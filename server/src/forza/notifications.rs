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
        (2, "Mensagens", "Mensagens de outros jogadores e do servidor"),
        (4, "Rivais", "Desafios: amigos que bateram seus tempos"),
        (7, "Clube", "Convites, membros e mural do seu clube"),
        (5, "Presentes", "Créditos, carros e itens recebidos"),
        (3, "Recompensas", "Pagamentos de Drivatar e de conteúdo compartilhado"),
        (1, "Novidades", "Novidades do FH2Online"),
        (6, "Avisos", "Avisos da conta"),
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
        // A text message to another player.
        "SendTextNotification" => {
            let to = params["xuidRecipient"].as_u64().unwrap_or(0);
            let text = params["message"].as_str().unwrap_or("");
            let id = ctx.store.notify(to, crate::db::notifications::MESSAGE, json!({ "Message": text, "SRAUri": "" }));
            ctx.event("message", format!("Mensagem para {}: {text}", ctx.store.gamertag(to)), json!({ "to": to.to_string() }));
            json!({ "notificationId": id })
        }
        _ => return None,
    })
}

/// "Your rival beat your time": after `xuid` posted `score`, tell each friend whose best on the
/// same event is now worse.
pub fn rivals_beaten(ctx: &Ctx, score_type: i64, data: &Value) {
    let me = ctx.xuid;
    let score = data["score"].as_f64().unwrap_or(0.0);
    let board = data["scoreboardType"].as_i64().unwrap_or(0);
    let event = data["gameScoreboardId"].as_i64().unwrap_or(-1);
    let track = data["trackId"].as_i64().unwrap_or(-1);
    let lower = score_type == 1;
    let friends: Vec<u64> = ctx.store.friends(me).into_iter().map(|f| f.0).collect();
    if friends.is_empty() {
        return;
    }
    let club = ctx.store.membership(me).and_then(|(id, role)| ctx.store.club(id).map(|c| (c.tag, role)));
    let track_name = if track >= 0 { ctx.track_name(track) } else { String::new() };
    for row in ctx.store.leaderboard(board, score_type, Some(track), Some(event)) {
        if row.xuid == me || !friends.contains(&row.xuid) {
            continue;
        }
        let beaten = if lower { score < row.score } else { score > row.score };
        if !beaten {
            continue;
        }
        let params = json!({
            "senderXuid": me,
            "senderGamertag": ctx.store.gamertag(me),
            "senderClubTag": club.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
            "senderClubRole": club.as_ref().map(|c| c.1).unwrap_or(0),
            "senderFlags": 0,
            "sentTime": super::iso_utc(crate::unix_ms()),
            "timeTrialId": event,
            "carId": data["carId"].as_i64().unwrap_or(-1),
            "carClassId": data["carClassId"].as_i64().unwrap_or(0),
            "localizedTimeTrialName": track_name,
            "trackName": track_name,
            "trackId": track,
            "score": score,
            "scoreType": score_type,
            "category": board,
        });
        ctx.store.notify(row.xuid, crate::db::notifications::TIME_BEATEN, params);
        ctx.event_for(
            row.xuid,
            "rival",
            format!("{} bateu seu tempo em {}", ctx.store.gamertag(me), if track_name.is_empty() { "um evento" } else { &track_name }),
            json!({ "from": me.to_string(), "score": score }),
        );
    }
}
