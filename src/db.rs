//! Player data in SQLite (data/fh2online.sqlite). Temporary by design: delete the file to start
//! over; the game uploads its profile again (UpdateAll / UpdateAllTables) on the next session.
//!
//! Tables:
//! - users: one row per XUID (gamertag, logons, last login data, profile checkpoint);
//! - profile: the last OnlineProfile.UpdateAll (settings, credits, race history, stats);
//! - profile_tables: OnlineProfile.UpdateAllTables, one JSON map per table keyed by row id
//!   (garageCars, trackStats, affinities...). Uploads are deltas: rows are merged and
//!   delete* lists applied;
//! - scores, xp, tunes: Scoreboard.SubmitScore / SubmitXpScore, ContentVersioning tunes;
//! - events: the activity feed shown live in the web UI.

use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::{gamedb::Catalog, unix_ms};

const MAX_EVENTS: i64 = 5000;

pub struct Store {
    db: Mutex<Connection>,
}

/// One leaderboard entry: a player's best score on a board.
pub struct ScoreRow {
    pub xuid: u64,
    pub score: f64,
    pub time: i64,
    pub data: Value,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS users (
                 xuid INTEGER PRIMARY KEY, gamertag TEXT NOT NULL DEFAULT '',
                 first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
                 logons INTEGER NOT NULL DEFAULT 0, login TEXT, profile_tran_id INTEGER);
             CREATE TABLE IF NOT EXISTS profile (
                 xuid INTEGER PRIMARY KEY, data TEXT NOT NULL, updated INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS profile_tables (
                 xuid INTEGER NOT NULL, name TEXT NOT NULL, data TEXT NOT NULL, updated INTEGER NOT NULL,
                 PRIMARY KEY (xuid, name));
             CREATE TABLE IF NOT EXISTS scores (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, score_id TEXT NOT NULL, xuid INTEGER NOT NULL,
                 time INTEGER NOT NULL, score_type INTEGER NOT NULL, scoreboard_type INTEGER NOT NULL,
                 game_scoreboard_id INTEGER NOT NULL, track_id INTEGER NOT NULL, car_id INTEGER NOT NULL,
                 car_class_id INTEGER NOT NULL, score REAL NOT NULL, is_clean INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS scores_board ON scores (scoreboard_type, score_type, track_id);
             CREATE TABLE IF NOT EXISTS xp (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, time INTEGER NOT NULL, xp REAL NOT NULL);
             CREATE TABLE IF NOT EXISTS tunes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, time INTEGER NOT NULL,
                 car_id INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, time INTEGER NOT NULL,
                 kind TEXT NOT NULL, title TEXT NOT NULL, detail TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS events_user ON events (xuid, id);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Store { db: Mutex::new(db) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
        let db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        f(&db).map_err(|e| eprintln!("db: {e}")).ok()
    }

    /// Creates the user if needed and marks it as seen now.
    pub fn touch_user(&self, xuid: u64) {
        let now = unix_ms();
        self.with(|db| {
            db.execute(
                "INSERT INTO users (xuid, first_seen, last_seen) VALUES (?1, ?2, ?2)
                 ON CONFLICT (xuid) DO UPDATE SET last_seen = ?2",
                params![xuid as i64, now],
            )
        });
    }

    pub fn record_logon(&self, xuid: u64, gamertag: &str, login: &Value) {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "UPDATE users SET gamertag = ?2, logons = logons + 1, login = ?3 WHERE xuid = ?1",
                params![xuid as i64, gamertag, login.to_string()],
            )
        });
    }

    /// transactionId of the last UpdateAllTables we stored (the profile checkpoint).
    pub fn profile_tran_id(&self, xuid: u64) -> Option<i64> {
        self.with(|db| {
            db.query_row("SELECT profile_tran_id FROM users WHERE xuid = ?1", [xuid as i64], |r| r.get(0))
                .optional()
        })
        .flatten()
        .flatten()
    }

    pub fn gamertag(&self, xuid: u64) -> String {
        self.with(|db| {
            db.query_row("SELECT gamertag FROM users WHERE xuid = ?1", [xuid as i64], |r| r.get(0)).optional()
        })
        .flatten()
        .unwrap_or_default()
    }

    /// The login data of the last Logon (career level, badge, title...).
    pub fn login(&self, xuid: u64) -> Value {
        self.with(|db| {
            db.query_row("SELECT login FROM users WHERE xuid = ?1", [xuid as i64], |r| r.get::<_, Option<String>>(0))
                .optional()
        })
        .flatten()
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
    }

    pub fn save_profile(&self, xuid: u64, data: &Value) {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO profile (xuid, data, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT (xuid) DO UPDATE SET data = ?2, updated = ?3",
                params![xuid as i64, data.to_string(), unix_ms()],
            )
        });
    }

    /// Merges an UpdateAllTables upload. Returns (rows upserted, rows deleted) per table.
    pub fn apply_tables(&self, xuid: u64, params: &Value) -> Vec<(String, usize, usize)> {
        self.touch_user(xuid);
        let Some(obj) = params.as_object() else { return Vec::new() };
        let now = unix_ms();
        let mut summary = Vec::new();
        let db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        for (name, value) in obj {
            if name == "transactionId" || name.starts_with("delete") {
                continue;
            }
            let Some(rows) = value.as_array() else { continue };
            // deleteGarageCars -> garageCars, deletePurchasedParts -> purchasedParts
            let delete_key = format!("delete{}{}", name[..1].to_uppercase(), &name[1..]);
            let deletes: Vec<String> =
                obj.get(&delete_key).and_then(Value::as_array).map(|a| a.iter().map(id_text).collect()).unwrap_or_default();
            if rows.is_empty() && deletes.is_empty() {
                continue;
            }
            let mut table: Map<String, Value> = db
                .query_row(
                    "SELECT data FROM profile_tables WHERE xuid = ?1 AND name = ?2",
                    params![xuid as i64, name],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .ok()
                .flatten()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            for (i, row) in rows.iter().enumerate() {
                let key = ["id", "Id", "trackId"]
                    .iter()
                    .find_map(|k| row.get(*k))
                    .map(id_text)
                    .unwrap_or_else(|| format!("#{i}"));
                table.insert(key, row.clone());
            }
            let mut deleted = 0;
            for key in &deletes {
                deleted += table.remove(key).is_some() as usize;
            }
            let _ = db
                .execute(
                    "INSERT INTO profile_tables (xuid, name, data, updated) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (xuid, name) DO UPDATE SET data = ?3, updated = ?4",
                    params![xuid as i64, name, Value::Object(table).to_string(), now],
                )
                .map_err(|e| eprintln!("db: {e}"));
            summary.push((name.clone(), rows.len(), deleted));
        }
        if let Some(tran) = obj.get("transactionId").and_then(Value::as_i64) {
            let _ = db.execute("UPDATE users SET profile_tran_id = ?2 WHERE xuid = ?1", params![xuid as i64, tran]);
        }
        summary
    }

    /// All profile tables as {name: [rows]}.
    pub fn tables(&self, xuid: u64) -> Map<String, Value> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT name, data FROM profile_tables WHERE xuid = ?1 ORDER BY name")?;
            let rows = stmt.query_map([xuid as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Map::new();
            for (name, data) in rows.flatten() {
                let map: Map<String, Value> = serde_json::from_str(&data).unwrap_or_default();
                out.insert(name, Value::Array(map.into_iter().map(|(_, v)| v).collect()));
            }
            Ok(out)
        })
        .unwrap_or_default()
    }

    /// Stores a score; returns its new scoreId (a GUID string).
    pub fn add_score(&self, xuid: u64, score_type: i64, data: &Value) -> String {
        self.touch_user(xuid);
        let now = unix_ms();
        let score_id = new_guid(xuid, now);
        let int = |k: &str| data.get(k).and_then(Value::as_i64).unwrap_or(-1);
        self.with(|db| {
            db.execute(
                "INSERT INTO scores (score_id, xuid, time, score_type, scoreboard_type, game_scoreboard_id,
                     track_id, car_id, car_class_id, score, is_clean, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    score_id,
                    xuid as i64,
                    now,
                    score_type,
                    int("scoreboardType"),
                    int("gameScoreboardId"),
                    int("trackId"),
                    int("carId"),
                    int("carClassId"),
                    data.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                    data.get("isClean").and_then(Value::as_bool).unwrap_or(false),
                    data.to_string()
                ],
            )
        });
        score_id
    }

    /// Best score of each player on a board. `track_id` None = any track.
    /// Hypothesis: score type 1 is a time (lower is better); the others are points.
    pub fn leaderboard(&self, scoreboard_type: i64, score_type: i64, track_id: Option<i64>, game_scoreboard_id: Option<i64>) -> Vec<ScoreRow> {
        let lower = lower_is_better(score_type);
        let mut rows: Vec<ScoreRow> = self
            .with(|db| {
                let mut stmt = db.prepare(
                    "SELECT xuid, score, time, data FROM scores
                     WHERE scoreboard_type = ?1 AND score_type = ?2
                       AND (?3 IS NULL OR track_id = ?3) AND (?4 IS NULL OR game_scoreboard_id = ?4)",
                )?;
                let found = stmt
                    .query_map(params![scoreboard_type, score_type, track_id, game_scoreboard_id], |r| {
                        Ok(ScoreRow {
                            xuid: r.get::<_, i64>(0)? as u64,
                            score: r.get(1)?,
                            time: r.get(2)?,
                            data: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
                        })
                    })?
                    .flatten()
                    .collect::<Vec<_>>();
                Ok(found)
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            let o = a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal);
            if lower { o } else { o.reverse() }
        });
        let mut seen = std::collections::HashSet::new();
        rows.retain(|r| seen.insert(r.xuid));
        rows
    }

    pub fn add_xp(&self, xuid: u64, xp: f64) {
        self.touch_user(xuid);
        self.with(|db| db.execute("INSERT INTO xp (xuid, time, xp) VALUES (?1, ?2, ?3)", params![xuid as i64, unix_ms(), xp]));
    }

    pub fn add_tune(&self, xuid: u64, car_id: i64, data: &Value) -> i64 {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO tunes (xuid, time, car_id, data) VALUES (?1, ?2, ?3, ?4)",
                params![xuid as i64, unix_ms(), car_id, data.to_string()],
            )?;
            Ok(db.last_insert_rowid())
        })
        .unwrap_or(0)
    }

    /// Appends to the activity feed; returns the event as JSON (for the WebSocket).
    pub fn add_event(&self, xuid: u64, kind: &str, title: &str, detail: Value) -> Value {
        let now = unix_ms();
        let id = self
            .with(|db| {
                db.execute(
                    "INSERT INTO events (xuid, time, kind, title, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![xuid as i64, now, kind, title, detail.to_string()],
                )?;
                let id = db.last_insert_rowid();
                db.execute("DELETE FROM events WHERE id <= ?1", [id - MAX_EVENTS])?;
                Ok(id)
            })
            .unwrap_or(0);
        json!({ "id": id, "xuid": xuid.to_string(), "time": now, "kind": kind, "title": title, "detail": detail })
    }

    pub fn users(&self) -> Value {
        self.with(|db| {
            let mut stmt =
                db.prepare("SELECT xuid, gamertag, first_seen, last_seen, logons FROM users ORDER BY last_seen DESC")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "xuid": (r.get::<_, i64>(0)? as u64).to_string(),
                        "gamertag": r.get::<_, String>(1)?,
                        "firstSeen": r.get::<_, i64>(2)?,
                        "lastSeen": r.get::<_, i64>(3)?,
                        "logons": r.get::<_, i64>(4)?,
                    }))
                })?
                .flatten()
                .collect::<Vec<_>>();
            Ok(Value::Array(rows))
        })
        .unwrap_or(Value::Array(Vec::new()))
    }

    /// Everything the web UI shows for one player.
    pub fn user_profile(&self, xuid: u64, catalog: &Catalog) -> Option<Value> {
        let user = self
            .with(|db| {
                db.query_row(
                    "SELECT gamertag, first_seen, last_seen, logons, login, profile_tran_id FROM users WHERE xuid = ?1",
                    [xuid as i64],
                    |r| {
                        let login: Value =
                            r.get::<_, Option<String>>(4)?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
                        Ok(json!({
                            "xuid": xuid.to_string(),
                            "gamertag": r.get::<_, String>(0)?,
                            "firstSeen": r.get::<_, i64>(1)?,
                            "lastSeen": r.get::<_, i64>(2)?,
                            "logons": r.get::<_, i64>(3)?,
                            "careerLevel": login["currentCareerLevel"],
                            "badgeId": login["currentBadgeId"],
                            "titleId": login["currentPlayerTitleId"],
                            "profileTransaction": r.get::<_, Option<i64>>(5)?,
                        }))
                    },
                )
                .optional()
            })
            .flatten()?;

        let mut tables = self.tables(xuid);
        let garage: Vec<Value> = tables
            .remove("garageCars")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|mut car| {
                let id = car["carId"].as_i64().unwrap_or(-1);
                car["car"] = catalog.car(id).map(|c| json!(c)).unwrap_or(Value::Null);
                car
            })
            .collect();

        let scores = self
            .with(|db| {
                let mut stmt = db.prepare(
                    "SELECT score_id, time, score_type, scoreboard_type, game_scoreboard_id, track_id, car_id,
                            car_class_id, score, is_clean
                     FROM scores WHERE xuid = ?1 ORDER BY id DESC LIMIT 100",
                )?;
                let rows = stmt
                    .query_map([xuid as i64], |r| {
                        let track_id: i64 = r.get(5)?;
                        let car_id: i64 = r.get(6)?;
                        Ok(json!({
                            "scoreId": r.get::<_, String>(0)?,
                            "time": r.get::<_, i64>(1)?,
                            "scoreType": r.get::<_, i64>(2)?,
                            "scoreboardType": r.get::<_, i64>(3)?,
                            "gameScoreboardId": r.get::<_, i64>(4)?,
                            "trackId": track_id,
                            "track": catalog.tracks.get(&track_id),
                            "carId": car_id,
                            "car": catalog.car(car_id),
                            "carClassId": r.get::<_, i64>(7)?,
                            "score": r.get::<_, f64>(8)?,
                            "isClean": r.get::<_, bool>(9)?,
                        }))
                    })?
                    .flatten()
                    .collect::<Vec<_>>();
                Ok(rows)
            })
            .unwrap_or_default();

        let xp = self
            .with(|db| {
                let mut stmt = db.prepare("SELECT time, xp FROM xp WHERE xuid = ?1 ORDER BY id DESC LIMIT 300")?;
                let mut rows = stmt
                    .query_map([xuid as i64], |r| Ok(json!({ "time": r.get::<_, i64>(0)?, "xp": r.get::<_, f64>(1)? })))?
                    .flatten()
                    .collect::<Vec<_>>();
                rows.reverse();
                Ok(rows)
            })
            .unwrap_or_default();

        let tunes: i64 = self
            .with(|db| db.query_row("SELECT COUNT(*) FROM tunes WHERE xuid = ?1", [xuid as i64], |r| r.get(0)))
            .unwrap_or(0);

        let (profile, profile_updated) = self
            .with(|db| {
                db.query_row("SELECT data, updated FROM profile WHERE xuid = ?1", [xuid as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .optional()
            })
            .flatten()
            .map(|(d, t)| (serde_json::from_str(&d).unwrap_or(Value::Null), Some(t)))
            .unwrap_or((Value::Null, None));

        Some(json!({
            "user": user,
            "profile": profile,
            "profileUpdated": profile_updated,
            "garage": garage,
            "tables": tables,
            "scores": scores,
            "xp": xp,
            "tunes": tunes,
            "events": self.events(xuid, 100),
            // Xbox achievements unlock locally (Windows.Xbox.Achievements in Azimuth) and do
            // not reach this server yet; the hook will have to forward them.
            "achievements": [],
        }))
    }

    pub fn events(&self, xuid: u64, limit: i64) -> Vec<Value> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT id, time, kind, title, detail FROM events WHERE xuid = ?1 OR xuid = 0 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![xuid as i64, limit], |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "xuid": xuid.to_string(),
                        "time": r.get::<_, i64>(1)?,
                        "kind": r.get::<_, String>(2)?,
                        "title": r.get::<_, String>(3)?,
                        "detail": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                    }))
                })?
                .flatten()
                .collect::<Vec<_>>();
            Ok(rows)
        })
        .unwrap_or_default()
    }
}

pub fn lower_is_better(score_type: i64) -> bool {
    score_type == 1
}

fn id_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Not cryptographic: unique enough for score ids on a test server.
fn new_guid(xuid: u64, now: i64) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let a = (now as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ xuid;
    let b = n.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ (xuid.rotate_left(17));
    let h = format!("{a:016x}{b:016x}");
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}
