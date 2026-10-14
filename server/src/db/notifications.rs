//! Xls notifications: the game's inbox (rival challenges, club invites and news, messages,
//! gifts). Each one has a notificationType and a parameter struct, kept here as the struct's
//! schema name and its JSON; the Notifications handler encodes it (bin/xtsw) on delivery.
//!
//! notificationType -> parameter struct (switch in FUN_140f15ec0):
//!   0x0000 PermissionsDirty · 0x0100 NewDlc · 0x0200/0x0201/0x0203/0x0602/0x8401 Message
//!   0x0600 Banned · 0x0601 Unbanned
//!   0x0700 ClubMembershipRequest · 0x0701 ClubInvitation · 0x0702 ClubMemberRemoveRecommendation
//!   0x0703 ClubKick · 0x0704 ClubDemotion · 0x0705 ClubMemberAccepted · 0x0706 ClubMemberQuit
//!   0x0707 ClubNewMember · 0x0708 ClubNewPromotion · 0x0709 ClubNoticeBoardMessage
//!   0x8000 UserBadgeGranted · 0x8001 SetCredits · 0x8301 SharedCar · 0x8302 CurationPayout
//!   0x8303 DrivatarPayout · 0x8402 TimeTrialRecommend · 0x8403 TimeTrialTimeBeaten
//!   0x8500 GiftReceiveMoney · 0x8501 GiftReceiveItem · 0x8502 GiftReceiveCar
//!   0x8503 StorefrontOutOfStock · 0x8504 StorefrontMoneyWaiting

use rusqlite::params;
use serde_json::{Value, json};

use super::Store;
use crate::unix_ms;

pub const MESSAGE: i64 = 0x0200;
pub const CLUB_MEMBERSHIP_REQUEST: i64 = 0x0700;
pub const CLUB_INVITATION: i64 = 0x0701;
pub const CLUB_REMOVE_RECOMMENDATION: i64 = 0x0702;
pub const CLUB_KICK: i64 = 0x0703;
pub const CLUB_DEMOTION: i64 = 0x0704;
pub const CLUB_MEMBER_ACCEPTED: i64 = 0x0705;
pub const CLUB_MEMBER_QUIT: i64 = 0x0706;
pub const CLUB_NEW_MEMBER: i64 = 0x0707;
pub const CLUB_NEW_PROMOTION: i64 = 0x0708;
pub const CLUB_NOTICE_BOARD: i64 = 0x0709;
pub const TIME_BEATEN: i64 = 0x8403;
pub const GIFT_MONEY: i64 = 0x8500;

/// Notifications live this long (expirationDate).
pub const TTL_MS: i64 = 30 * 24 * 3600 * 1000;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS notifications (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, type INTEGER NOT NULL,
        struct TEXT NOT NULL, params TEXT NOT NULL, sent INTEGER NOT NULL,
        read INTEGER NOT NULL DEFAULT 0, cleared INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX IF NOT EXISTS notifications_user ON notifications (xuid, id);
    CREATE TABLE IF NOT EXISTS race_events (
        scoreboard_type INTEGER NOT NULL, score_type INTEGER NOT NULL, game_scoreboard_id INTEGER NOT NULL,
        track_id INTEGER NOT NULL, car_class_id INTEGER NOT NULL, last_seen INTEGER NOT NULL,
        PRIMARY KEY (scoreboard_type, game_scoreboard_id, track_id));";

pub struct Notification {
    pub id: i64,
    pub xuid: u64,
    pub kind: i64,
    pub structure: String,
    pub params: Value,
    pub sent: i64,
    pub read: bool,
}

/// The struct each type carries.
pub fn struct_for(kind: i64) -> Option<&'static str> {
    Some(match kind {
        0x0000 => "PermissionsDirtyNotificationParameters",
        0x0100 => "NewDlcNotificationParameters",
        0x0200 | 0x0201 | 0x0203 | 0x0602 | 0x8401 => "MessageNotificationParameters",
        0x0600 => "BannedNotificationParameters",
        0x0601 => "UnbannedNotificationParameters",
        CLUB_MEMBERSHIP_REQUEST => "ClubMembershipRequestParameters",
        CLUB_INVITATION => "ClubInvitationParameters",
        CLUB_REMOVE_RECOMMENDATION => "ClubMemberRemoveRecommendationParameters",
        CLUB_KICK => "ClubKickParameters",
        CLUB_DEMOTION => "ClubDemotionParameters",
        CLUB_MEMBER_ACCEPTED => "ClubMemberAcceptedParameters",
        CLUB_MEMBER_QUIT => "ClubMemberQuitParameters",
        CLUB_NEW_MEMBER => "ClubNewMemberParameters",
        CLUB_NEW_PROMOTION => "ClubNewPromotionParameters",
        CLUB_NOTICE_BOARD => "ClubNoticeBoardMessageParameters",
        0x8000 => "UserBadgeGrantedNotificationParameters",
        0x8001 => "SetCreditsNotificationParameters",
        0x8301 => "SharedCarNotificationParameters",
        0x8302 => "CurationPayoutNotificationParameters",
        0x8303 => "DrivatarPayoutNotificationParameters",
        0x8402 => "TimeTrialRecommendNotificationParameters",
        TIME_BEATEN => "TimeTrialTimeBeatenNotificationParameters",
        GIFT_MONEY => "GiftReceiveMoneyNotificationParameters",
        0x8501 => "GiftReceiveItemNotificationParameters",
        0x8502 => "GiftReceiveCarNotificationParameters",
        0x8503 => "StorefrontOutOfStockNotificationParameters",
        0x8504 => "StorefrontMoneyWaitingNotificationParameters",
        _ => return None,
    })
}

/// A short text for the manager's feed.
pub fn describe(kind: i64, p: &Value) -> String {
    let s = |k: &str| p[k].as_str().unwrap_or("").to_owned();
    match kind {
        CLUB_MEMBERSHIP_REQUEST => format!("{} pediu para entrar no clube", s("requestorGamertag")),
        CLUB_INVITATION => format!("{} convidou para [{}] {}", s("sponsorGamertag"), s("clubTag"), s("clubName")),
        CLUB_REMOVE_RECOMMENDATION => format!("{} sugeriu remover {}", s("recommendingRemovalGamertag"), s("recommendedForRemovalGamertag")),
        CLUB_KICK => format!("Removido do clube [{}] {}", s("clubTag"), s("clubName")),
        CLUB_DEMOTION => format!("Rebaixado no clube [{}]", s("clubTag")),
        CLUB_MEMBER_ACCEPTED => format!("Aceito no clube [{}] {}", s("clubTag"), s("clubName")),
        CLUB_MEMBER_QUIT => format!("{} saiu do clube [{}]", s("exMemberGamertag"), s("clubTag")),
        CLUB_NEW_MEMBER => format!("{} entrou no clube [{}]", s("newMemberGamertag"), s("clubTag")),
        CLUB_NEW_PROMOTION => format!("{} foi promovido no clube [{}]", s("newPromotedGamertag"), s("clubTag")),
        CLUB_NOTICE_BOARD => format!("Mural: {}: {}", s("senderGamertag"), s("noticeBoardMessage")),
        TIME_BEATEN => format!("{} bateu seu tempo em {}", s("senderGamertag"), s("trackName")),
        GIFT_MONEY => format!("{} enviou {} créditos", s("SenderGamerTag"), p["Amount"]),
        0x0200 | 0x0201 | 0x0203 | 0x0602 | 0x8401 => format!("Mensagem: {}", s("Message")),
        _ => format!("Notificação {kind:#06x}"),
    }
}

impl Store {
    /// Queues a notification for a player. Returns its id (0 if the type is unknown).
    pub fn notify(&self, xuid: u64, kind: i64, params: Value) -> i64 {
        let Some(structure) = struct_for(kind) else { return 0 };
        if xuid == 0 {
            return 0;
        }
        self.with(|db| {
            db.execute(
                "INSERT INTO notifications (xuid, type, struct, params, sent) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, kind, structure, params.to_string(), unix_ms()],
            )?;
            Ok(db.last_insert_rowid())
        })
        .unwrap_or(0)
    }

    /// Live (not cleared, not expired) notifications of a player, oldest first.
    pub fn notifications(&self, xuid: u64, limit: i64) -> Vec<Notification> {
        let since = unix_ms() - TTL_MS;
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT id, xuid, type, struct, params, sent, read FROM notifications
                 WHERE xuid = ?1 AND cleared = 0 AND sent >= ?2 ORDER BY id DESC LIMIT ?3",
            )?;
            let mut rows: Vec<Notification> = stmt
                .query_map(params![xuid as i64, since, limit.max(1)], |r| {
                    Ok(Notification {
                        id: r.get(0)?,
                        xuid: r.get::<_, i64>(1)? as u64,
                        kind: r.get(2)?,
                        structure: r.get(3)?,
                        params: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                        sent: r.get(5)?,
                        read: r.get::<_, i64>(6)? != 0,
                    })
                })?
                .flatten()
                .collect();
            rows.reverse();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    pub fn mark_notification_read(&self, xuid: u64, id: i64) -> bool {
        self.with(|db| db.execute("UPDATE notifications SET read = 1 WHERE id = ?1 AND xuid = ?2", params![id, xuid as i64]))
            .is_some_and(|n| n > 0)
    }

    pub fn clear_notification(&self, xuid: u64, id: i64) -> bool {
        self.with(|db| db.execute("UPDATE notifications SET cleared = 1 WHERE id = ?1 AND xuid = ?2", params![id, xuid as i64]))
            .is_some_and(|n| n > 0)
    }

    /// A race event the game asked about (GetBestScore / GetRival before a race): the real
    /// (board, event, route) triples a rival challenge can point at.
    pub fn record_race_event(&self, board: i64, score_type: i64, event: i64, track: i64, class: i64) {
        if track < 0 {
            return;
        }
        self.with(|db| {
            db.execute(
                "INSERT INTO race_events (scoreboard_type, score_type, game_scoreboard_id, track_id, car_class_id, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (scoreboard_type, game_scoreboard_id, track_id) DO UPDATE SET
                    score_type = ?2, car_class_id = CASE WHEN ?5 >= 0 THEN ?5 ELSE car_class_id END, last_seen = ?6",
                params![board, score_type, event, track, class, unix_ms()],
            )
        });
    }

    /// The most recent race event: (scoreboard type, score type, event id, track, car class).
    pub fn last_race_event(&self) -> Option<(i64, i64, i64, i64, i64)> {
        self.with(|db| {
            use rusqlite::OptionalExtension;
            db.query_row(
                "SELECT scoreboard_type, score_type, game_scoreboard_id, track_id, car_class_id FROM race_events
                 WHERE game_scoreboard_id >= 0 ORDER BY last_seen DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
        })
        .flatten()
    }

    /// A pending "your rival beat your time" for this player on a route: its parameters.
    pub fn pending_challenge(&self, xuid: u64, track: i64, event: Option<i64>) -> Option<Value> {
        self.notifications(xuid, 512)
            .into_iter()
            .rev()
            .filter(|n| n.kind == TIME_BEATEN)
            .map(|n| n.params)
            .find(|p| p["trackId"].as_i64() == Some(track) && event.is_none_or(|e| e < 0 || p["timeTrialId"].as_i64() == Some(e)))
    }

    /// Deletes one notification (manager).
    pub fn delete_notification(&self, id: i64) -> bool {
        self.with(|db| db.execute("DELETE FROM notifications WHERE id = ?1", [id])).is_some_and(|n| n > 0)
    }

    /// The inbox as the manager shows it.
    pub fn notifications_json(&self, xuid: u64) -> Value {
        Value::Array(
            self.notifications(xuid, 100)
                .into_iter()
                .rev()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "type": n.kind,
                        "struct": n.structure,
                        "text": describe(n.kind, &n.params),
                        "params": n.params,
                        "sent": n.sent,
                        "read": n.read,
                    })
                })
                .collect(),
        )
    }
}
