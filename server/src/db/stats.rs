//! Server-wide totals for the public page (GET /infinity/v1/stats): counts and sums over all
//! players, nothing about any one of them.

use serde_json::{Value, json};

use super::Store;
use crate::unix_ms;

/// A player counts as online when the server heard from their game this recently (it polls
/// Notifications.RetrieveForUser every 30 s while running; presence only changes now and then).
const ONLINE_MS: i64 = 5 * 60 * 1000;

impl Store {
    pub fn server_stats(&self) -> Value {
        let now = unix_ms();
        let count = |sql: &str| -> i64 { self.with(|db| db.query_row(sql, [], |r| r.get::<_, i64>(0))).unwrap_or(0) };
        // career totals from each profile's last OnlineProfile.UpdateAll
        let (mut races, mut victories, mut metres, mut winnings, mut speed_traps) = (0i64, 0i64, 0i64, 0i64, 0i64);
        for data in self
            .with(|db| {
                let mut stmt = db.prepare("SELECT data FROM profile")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.flatten().collect::<Vec<_>>();
                Ok(rows)
            })
            .unwrap_or_default()
        {
            let p: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
            let race = &p["playerRaceHistory"][0];
            let n = |v: &Value| v.as_i64().unwrap_or(0).max(0);
            races += n(&race["numRaces"]);
            victories += n(&race["numVictories"]);
            metres += n(&race["distanceDriven"]);
            winnings += n(&race["totalWinnings"]);
            speed_traps += n(&p["compareStats"][0]["speedTrapsTriggered"]);
        }
        // garage sizes from each player's last UpdateAllTables
        let cars: i64 = self
            .with(|db| {
                let mut stmt = db.prepare("SELECT data FROM profile_tables WHERE name = 'garageCars'")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.flatten().collect::<Vec<_>>();
                Ok(rows)
            })
            .unwrap_or_default()
            .iter()
            .map(|t| match serde_json::from_str::<Value>(t) {
                Ok(Value::Array(a)) => a.len() as i64,
                Ok(Value::Object(o)) => o.len() as i64,
                _ => 0,
            })
            .sum();
        json!({
            "players": count("SELECT COUNT(*) FROM accounts WHERE banned = 0"),
            "online": self
                .with(|db| db.query_row(
                    "SELECT COUNT(*) FROM accounts a JOIN users u ON u.xuid = a.xuid
                     LEFT JOIN presence p ON p.xuid = a.xuid
                     WHERE a.banned = 0 AND MAX(u.last_seen, COALESCE(p.updated, 0)) >= ?1",
                    [now - ONLINE_MS],
                    |r| r.get::<_, i64>(0),
                ))
                .unwrap_or(0),
            "friendships": count("SELECT COUNT(*) / 2 FROM friends"),
            "clubs": count("SELECT COUNT(*) FROM clubs"),
            "races": races,
            "victories": victories,
            "kilometres": metres / 1000,
            "credits": winnings,
            "speedTraps": speed_traps,
            "cars": cars,
            // a design = a livery with layers the game saved (once per player and name)
            "designs": count(
                "SELECT COUNT(DISTINCT xuid || '/' || name) FROM liveries WHERE json_extract(data, '$.numberOfLayers') > 0",
            ),
            "sharedFiles": count("SELECT COUNT(*) FROM storefront_files WHERE removed = 0"),
            "downloads": count("SELECT COALESCE(SUM(downloads), 0) FROM storefront_files"),
            "scores": count("SELECT COUNT(*) FROM scores"),
            "ghosts": count("SELECT COUNT(*) FROM ghosts"),
            "drivatars": count("SELECT COUNT(*) FROM drivatars"),
            "updated": crate::forza::iso_utc(now),
        })
    }
}
