//! Xls.WebServices.Notifications: delivers the inbox kept in db/notifications.rs.
//!
//! RetrieveForUser returns the notification headers plus one parameterBlock blob holding each
//! notification's parameter struct (bin/xtsw) back to back; cbParameters is each one's size.
//! sentDate / expirationDate are FILETIME (100 ns since 1601).

use std::sync::OnceLock;

use serde_json::{Value, json};

use super::Ctx;
use crate::{db::notifications::TTL_MS, xtsw::Schema};

/// How often the game polls (it also polls right after StartNewSession).
const POLL_SECONDS: i64 = 30;

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(Schema::load_embedded)
}

fn filetime(unix_ms: i64) -> i64 {
    (unix_ms + 11_644_473_600_000) * 10_000
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// XlsNotificationCategory list sent in the Logon reply (at most 7). `type` is the category a
/// notification falls in: (notificationType >> 8) & 0x7f.
pub fn categories() -> Value {
    let list: [(i64, &str, &str); 7] = [
        (2, "Messages", "Messages from other players and the server"),
        (4, "Rivals", "Challenges: friends who beat your times"),
        (7, "Club", "Your club's invites, members and notice board"),
        (5, "Gifts", "Credits, cars and items received"),
        (3, "Rewards", "Drivatar and shared content payouts"),
        (1, "News", "ForzaInfinity news"),
        (6, "Notices", "Account notices"),
    ];
    Value::Array(
        list.iter()
            .map(|&(t, name, description)| json!({ "type": t, "name": name, "description": description, "backgroundImageFileId": 0 }))
            .collect(),
    )
}

pub fn handle(ctx: &Ctx, method: &str, params: &Value) -> Option<Value> {
    let me = ctx.xuid;
    Some(match method {
        "StartNewSession" => json!({ "nonce": 1, "nextRetrieveSeconds": POLL_SECONDS }),
        "RetrieveForUser" => {
            // polled every 30 s while the game runs: the player is online
            ctx.store.touch_user(me);
            sync_rivals(ctx);
            let max = params["maxResults"].as_i64().filter(|&n| n > 0).unwrap_or(64);
            let budget = params["cbParameterBlockSize"].as_i64().filter(|&n| n > 0).unwrap_or(0x10000) as usize;
            let mut block = Vec::new();
            let mut results = Vec::new();
            let mut ids = Vec::new();
            for n in ctx.store.notifications(me, max) {
                let bytes = match schema().encode(&n.structure, &n.params) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("notification {}: {e}", n.id);
                        continue;
                    }
                };
                if block.len() + bytes.len() > budget || bytes.len() > u16::MAX as usize {
                    break;
                }
                results.push(json!({
                    "id": n.id,
                    "xuidRecipient": n.xuid,
                    "notificationType": n.kind,
                    "hasThumbnail": false,
                    "isRead": n.read,
                    "deleteAction": 0,
                    "expirationDate": filetime(n.sent + TTL_MS),
                    "sentDate": filetime(n.sent),
                    "cbParameters": bytes.len(),
                }));
                ids.push(n.id);
                block.extend_from_slice(&bytes);
            }
            json!({
                "results": results,
                "parameterBlock": hex(&block),
                "nextRetrieveSeconds": POLL_SECONDS,
                "reconcileActiveNotifications": true,
                "activeNotificationIds": ids,
            })
        }
        "MarkRead" => {
            ctx.store.mark_notification_read(me, params["notificationId"].as_i64().unwrap_or(0));
            json!({})
        }
        "Clear" => {
            ctx.store.clear_notification(me, params["notificationId"].as_i64().unwrap_or(0));
            json!({})
        }
        "GetCategories" => json!({ "results": [] }),
        "GetThumbnail" => json!({ "data": "", "required": 0 }),
        // A text message to a friend (30 an hour at most).
        "SendTextNotification" => {
            let to = params["xuidRecipient"].as_u64().unwrap_or(0);
            let text = super::clean_text(params["message"].as_str().unwrap_or(""), 200);
            if text.is_empty() || !ctx.friend_ids().contains(&to) {
                return Some(json!({ "__xtsw_error": { "code": 1, "message": "messages go to friends only" } }));
            }
            if ctx.store.events_since(me, "message", crate::unix_ms() - 3_600_000) >= 30 {
                return Some(json!({ "__xtsw_error": { "code": 1, "message": "too many messages" } }));
            }
            let id = ctx.store.notify(to, crate::db::notifications::MESSAGE, json!({ "Message": text, "SRAUri": "" }));
            ctx.event("message", format!("Mensagem para {}: {text}", ctx.store.gamertag(to)), json!({ "to": to.to_string() }));
            json!({ "notificationId": id })
        }
        _ => return None,
    })
}

/// Where "Ir para" in Meus Rivais takes the player (FUN_140d6b6b0): category 1 = Panama
/// (Storm Island), 4 = monthly rivals; otherwise timeTrialId != -1 = that showcase, -1 = the
/// festival route (trackId) with car select. Returns (category, timeTrialId).
pub fn challenge_target(board: i64, event: i64) -> (i64, i64) {
    match board {
        6 => (0, event), // showcase: the event is the showcase id
        _ => (0, -1),    // a festival route race
    }
}

/// Meus Rivais: each friend's best time on every route event the player has not beaten yet
/// (not raced there, or slower), as one "time to beat" (TimeTrialTimeBeaten) per friend and
/// event. Run on every RetrieveForUser (the game polls every 30 s and drops ids missing from
/// activeNotificationIds), so it follows new bests, beaten times and friendships both ways.
/// Only route times: the game has no rival notification for speed traps, zones or skills, and
/// "Ir para" on one opens a broken event (no GPS, no line, "1/- laps"); those boards are
/// compared on the friends' leaderboards. A time the player deleted is not sent again.
pub fn sync_rivals(ctx: &Ctx) {
    let me = ctx.xuid;
    let key = |board: i64, event: i64, track: i64| (board, event, track);
    let row_key = |d: &Value| {
        key(d["scoreboardType"].as_i64().unwrap_or(-1), d["gameScoreboardId"].as_i64().unwrap_or(-1), d["trackId"].as_i64().unwrap_or(-1))
    };
    let mine: Vec<((i64, i64, i64), f64)> = ctx.store.route_bests(me).iter().map(|r| (row_key(&r.data), r.score)).collect();
    let my_best = |k: (i64, i64, i64)| mine.iter().find(|(m, _)| *m == k).map(|m| m.1);
    // what the list should hold: (sender, event, the sender's best)
    let mut wanted = Vec::new();
    for friend in ctx.friend_ids() {
        for best in ctx.store.route_bests(friend) {
            let k = row_key(&best.data);
            if my_best(k).is_none_or(|m| best.score < m) {
                wanted.push((friend, k, best));
            }
        }
    }
    let same = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let existing = ctx.store.rival_notifications(me);
    let notice_key = |p: &Value| {
        (p["senderXuid"].as_u64().unwrap_or(0), key(p["rivalBoard"].as_i64().unwrap_or(i64::MIN), p["rivalEvent"].as_i64().unwrap_or(-1), p["trackId"].as_i64().unwrap_or(-1)))
    };
    // drop the times beaten, improved on, or from someone no longer a friend
    for (id, cleared, p) in &existing {
        let (sender, k) = notice_key(p);
        let score = p["score"].as_f64().unwrap_or(f64::NAN);
        if !cleared && !wanted.iter().any(|(f, wk, b)| *f == sender && *wk == k && same(b.score, score)) {
            ctx.store.clear_notification(me, *id);
        }
    }
    for (friend, k, best) in wanted {
        let sent_before = existing.iter().any(|(_, _, p)| notice_key(p) == (friend, k) && same(p["score"].as_f64().unwrap_or(f64::NAN), best.score));
        if sent_before {
            continue;
        }
        let (board, event, track) = k;
        let (category, time_trial) = challenge_target(board, event);
        let club = ctx.store.membership(friend).and_then(|(id, role)| ctx.store.club(id).map(|c| (c.tag, role)));
        let track_name = ctx.track_name(track);
        let params = json!({
            "senderXuid": friend,
            "senderGamertag": ctx.store.game_tag(friend),
            "senderClubTag": club.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
            "senderClubRole": club.as_ref().map(|c| c.1).unwrap_or(0),
            "senderFlags": 0,
            "sentTime": super::iso_utc(best.time),
            "timeTrialId": time_trial,
            "carId": best.data["carId"].as_i64().unwrap_or(-1),
            "carClassId": best.data["carClassId"].as_i64().unwrap_or(0),
            "localizedTimeTrialName": track_name,
            "trackName": track_name,
            "trackId": track,
            "score": best.score,
            "scoreType": 1,
            "category": category,
            // ours, not the game's: which event this is
            "rivalBoard": board,
            "rivalEvent": event,
        });
        ctx.store.notify(me, crate::db::notifications::TIME_BEATEN, params);
        ctx.event(
            "rival",
            format!("Tempo para bater: {} em {} ({})", ctx.store.gamertag(friend), track_name, super::format_score(best.score, 1)),
            json!({ "from": friend.to_string(), "score": best.score }),
        );
    }
}
