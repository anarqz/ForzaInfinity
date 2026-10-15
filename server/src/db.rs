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
//! - events: the activity feed shown live in the web UI;
//! - friends: who each player follows (Xbox "People"); the manager adds both directions;
//! - presence: the last rich presence each player's game posted.

use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::{gamedb::Catalog, unix_ms};

pub mod accounts;
pub mod clubs;
pub mod integrity;
pub mod news;
pub mod notifications;
pub mod save_bests;
pub mod saves;
pub mod stats;
pub mod storefront;
pub use clubs::{ClubFields, ROLE_COLEADER, ROLE_LEADER, ROLE_MEMBER};

const MAX_EVENTS: i64 = 5000;
/// Liveries and tunes kept per player (the newest).
const MAX_PER_PLAYER: i64 = 200;
/// A player counts as online while their game posted presence within this window.
pub const PRESENCE_TTL_MS: i64 = 5 * 60 * 1000;

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
             CREATE INDEX IF NOT EXISTS events_user ON events (xuid, id);
             CREATE TABLE IF NOT EXISTS ghosts (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL,
                 scoreboard_type INTEGER NOT NULL, score_type INTEGER NOT NULL,
                 game_scoreboard_id INTEGER NOT NULL, track_id INTEGER NOT NULL,
                 score_id TEXT NOT NULL, score REAL NOT NULL, car_id INTEGER NOT NULL,
                 time INTEGER NOT NULL, data BLOB NOT NULL,
                 UNIQUE (xuid, scoreboard_type, score_type, game_scoreboard_id, track_id));
             CREATE TABLE IF NOT EXISTS drivatars (
                 xuid INTEGER PRIMARY KEY, data TEXT NOT NULL, updated INTEGER NOT NULL,
                 saves INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS liveries (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, time INTEGER NOT NULL,
                 name TEXT NOT NULL, car_id INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS transactions (
                 id TEXT PRIMARY KEY, xuid INTEGER NOT NULL, time INTEGER NOT NULL,
                 method TEXT NOT NULL, body BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS friends (
                 xuid INTEGER NOT NULL, friend INTEGER NOT NULL, favorite INTEGER NOT NULL DEFAULT 0,
                 created INTEGER NOT NULL, PRIMARY KEY (xuid, friend));
             CREATE TABLE IF NOT EXISTS presence (
                 xuid INTEGER PRIMARY KEY, state TEXT NOT NULL, rich TEXT NOT NULL,
                 updated INTEGER NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch(clubs::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(notifications::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(accounts::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(saves::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(news::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(integrity::SCHEMA).map_err(|e| e.to_string())?;
        db.execute_batch(storefront::SCHEMA).map_err(|e| e.to_string())?;
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

    /// The name the game shows: its gamertag fields hold 15 characters (like Xbox's), so a
    /// "Name#12345" longer than that shows as the name alone (cut at 15).
    pub fn game_tag(&self, xuid: u64) -> String {
        game_tag(&self.gamertag(xuid))
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

    /// User.SetPlayerCardInfo: level, badge and title, kept with the login data where the
    /// UserData builder reads them.
    pub fn set_player_card(&self, xuid: u64, card: &Value) {
        let mut login = self.login(xuid);
        if !login.is_object() {
            login = json!({});
        }
        for key in ["currentCareerLevel", "currentBadgeId", "currentPlayerTitleId"] {
            if !card[key].is_null() {
                login[key] = card[key].clone();
            }
        }
        self.touch_user(xuid);
        self.with(|db| db.execute("UPDATE users SET login = ?2 WHERE xuid = ?1", params![xuid as i64, login.to_string()]));
    }

    /// The last OnlineProfile.UpdateAll, or Null.
    pub fn profile_data(&self, xuid: u64) -> Value {
        self.with(|db| {
            db.query_row("SELECT data FROM profile WHERE xuid = ?1", [xuid as i64], |r| r.get::<_, String>(0)).optional()
        })
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
            )?;
            // per event keep only the player's best and this new one (SubmitScore returns
            // before UploadFile names it)
            let order = if lower_is_better(score_type) { "ASC" } else { "DESC" };
            db.execute(
                &format!(
                    "DELETE FROM scores WHERE xuid = ?1 AND score_type = ?2 AND scoreboard_type = ?3
                       AND game_scoreboard_id = ?4 AND track_id = ?5 AND score_id <> ?6
                       AND id <> (SELECT id FROM scores WHERE xuid = ?1 AND score_type = ?2 AND scoreboard_type = ?3
                                    AND game_scoreboard_id = ?4 AND track_id = ?5 ORDER BY score {order}, id ASC LIMIT 1)"
                ),
                params![xuid as i64, score_type, int("scoreboardType"), int("gameScoreboardId"), int("trackId"), score_id],
            )
        });
        score_id
    }

    /// Best score of each player on a board. `track_id` None = any track.
    /// Hypothesis: score type 1 is a time (lower is better); the others are points.
    /// A player's best time (score type 1, lower is better) on each route event: one row per
    /// scoreboard type, event and track (data holds those and the car).
    pub fn route_bests(&self, xuid: u64) -> Vec<ScoreRow> {
        self.with(|db| {
            // SQLite: the bare columns come from the row holding MIN(score)
            let mut stmt = db.prepare(
                "SELECT score, time, data, MIN(score) FROM scores
                 WHERE xuid = ?1 AND score_type = 1 AND track_id >= 0
                 GROUP BY scoreboard_type, game_scoreboard_id, track_id",
            )?;
            let found = stmt
                .query_map([xuid as i64], |r| {
                    Ok(ScoreRow {
                        xuid,
                        score: r.get(0)?,
                        time: r.get(1)?,
                        data: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    })
                })?
                .flatten()
                .collect::<Vec<_>>();
            Ok(found)
        })
        .unwrap_or_default()
    }

    pub fn leaderboard(&self, scoreboard_type: i64, score_type: i64, track_id: Option<i64>, game_scoreboard_id: Option<i64>) -> Vec<ScoreRow> {
        let lower = lower_is_better(score_type);
        let mut rows: Vec<ScoreRow> = self
            .with(|db| {
                let mut stmt = db.prepare(
                    "SELECT xuid, score, time, data FROM scores
                     WHERE scoreboard_type = ?1 AND score_type = ?2
                       AND (?3 IS NULL OR track_id = ?3) AND (?4 IS NULL OR game_scoreboard_id = ?4)
                       AND xuid NOT IN (SELECT xuid FROM accounts WHERE banned = 1)",
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

    /// The full reply (12-byte header + results) sent to a request carrying X-TransactionId,
    /// so Transaction.Reconcile can hand it back if the client missed it.
    pub fn save_transaction(&self, id: &str, xuid: u64, method: &str, body: &[u8]) {
        self.with(|db| {
            db.execute(
                "INSERT OR REPLACE INTO transactions (id, xuid, time, method, body) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![transaction_key(id), xuid as i64, unix_ms(), method, body],
            )?;
            // the game reconciles within its session: a week is plenty
            db.execute("DELETE FROM transactions WHERE time < ?1", [unix_ms() - 7 * 86_400_000])
        });
    }

    pub fn transaction(&self, id: &str) -> Option<Vec<u8>> {
        self.with(|db| {
            db.query_row("SELECT body FROM transactions WHERE id = ?1", [transaction_key(id)], |r| r.get(0)).optional()
        })
        .flatten()
    }

    /// True when `score_id` is this player's best on its event (board, score type, event, track).
    pub fn is_personal_best(&self, score_id: &str) -> bool {
        self.with(|db| {
            let row = db
                .query_row(
                    "SELECT xuid, scoreboard_type, score_type, game_scoreboard_id, track_id FROM scores WHERE score_id = ?1",
                    [score_id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?)),
                )
                .optional()?;
            let Some((xuid, sb, st, gs, track)) = row else { return Ok(false) };
            let order = if lower_is_better(st) { "ASC" } else { "DESC" };
            let best: Option<String> = db
                .query_row(
                    &format!(
                        "SELECT score_id FROM scores WHERE xuid = ?1 AND scoreboard_type = ?2 AND score_type = ?3
                           AND game_scoreboard_id = ?4 AND track_id = ?5 ORDER BY score {order}, id ASC LIMIT 1"
                    ),
                    params![xuid, sb, st, gs, track],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(best.as_deref() == Some(score_id))
        })
        .unwrap_or(false)
    }

    /// Scoreboard.UploadFile: keeps the ghost only if its score is still the player's best on
    /// that event, replacing the previous one (the ghost id of an event stays the same).
    /// Returns (ghost id, event key description) or None when the score is unknown/not best.
    pub fn save_ghost(&self, owner: u64, score_id: &str, data: &[u8]) -> Option<(i64, Value)> {
        if !self.is_personal_best(score_id) {
            return None;
        }
        self.with(|db| {
            let (xuid, sb, st, gs, track, score, car): (i64, i64, i64, i64, i64, f64, i64) = db.query_row(
                "SELECT xuid, scoreboard_type, score_type, game_scoreboard_id, track_id, score, car_id FROM scores
                 WHERE score_id = ?1 AND xuid = ?2",
                params![score_id, owner as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )?;
            db.execute(
                "INSERT INTO ghosts (xuid, scoreboard_type, score_type, game_scoreboard_id, track_id, score_id, score, car_id, time, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (xuid, scoreboard_type, score_type, game_scoreboard_id, track_id)
                 DO UPDATE SET score_id = ?6, score = ?7, car_id = ?8, time = ?9, data = ?10",
                params![xuid, sb, st, gs, track, score_id, score, car, unix_ms(), data],
            )?;
            let id: i64 = db.query_row(
                "SELECT id FROM ghosts WHERE xuid = ?1 AND scoreboard_type = ?2 AND score_type = ?3 AND game_scoreboard_id = ?4 AND track_id = ?5",
                params![xuid, sb, st, gs, track],
                |r| r.get(0),
            )?;
            Ok((id, json!({ "scoreboardType": sb, "scoreType": st, "gameScoreboardId": gs, "trackId": track, "score": score, "carId": car })))
        })
    }

    /// (owner, data) of a ghost.
    pub fn ghost(&self, id: i64) -> Option<(u64, Vec<u8>)> {
        self.with(|db| {
            db.query_row("SELECT xuid, data FROM ghosts WHERE id = ?1", [id], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))
                .optional()
        })
        .flatten()
    }

    /// Ghost id of a player's best on an event, if uploaded.
    pub fn ghost_id(&self, xuid: u64, scoreboard_type: i64, score_type: i64, game_scoreboard_id: i64, track_id: i64) -> Option<i64> {
        self.with(|db| {
            db.query_row(
                "SELECT id FROM ghosts WHERE xuid = ?1 AND scoreboard_type = ?2 AND score_type = ?3 AND game_scoreboard_id = ?4 AND track_id = ?5",
                params![xuid as i64, scoreboard_type, score_type, game_scoreboard_id, track_id],
                |r| r.get(0),
            )
            .optional()
        })
        .flatten()
    }

    /// The ghost of one of `owners` on a route (event ignored) whose score is closest to
    /// `score`: (ghost id, its score, its car).
    /// The ghost closest to `score` on a route: among `owners`, or (None) everyone not banned.
    pub fn nearest_ghost(&self, owners: Option<&[u64]>, scoreboard_type: i64, score_type: i64, track_id: i64, score: f64) -> Option<(i64, f64, i64)> {
        let whose = match owners {
            Some([]) => return None,
            // integers only: safe to inline
            Some(owners) => format!("xuid IN ({})", owners.iter().map(|x| (*x as i64).to_string()).collect::<Vec<_>>().join(",")),
            None => "xuid NOT IN (SELECT xuid FROM accounts WHERE banned = 1)".to_owned(),
        };
        self.with(|db| {
            db.query_row(
                &format!(
                    "SELECT id, score, car_id FROM ghosts WHERE {whose} AND scoreboard_type = ?1 AND score_type = ?2
                       AND track_id = ?3 ORDER BY abs(score - ?4) LIMIT 1"
                ),
                params![scoreboard_type, score_type, track_id, score],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })
        .flatten()
    }

    /// The player's ghosts (no data), for the web UI.
    pub fn ghosts(&self, xuid: u64, catalog: &Catalog) -> Vec<Value> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT id, scoreboard_type, score_type, game_scoreboard_id, track_id, score, car_id, time, length(data)
                 FROM ghosts WHERE xuid = ?1 ORDER BY time DESC",
            )?;
            let rows = stmt
                .query_map([xuid as i64], |r| {
                    let track: i64 = r.get(4)?;
                    let car: i64 = r.get(6)?;
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?, "scoreboardType": r.get::<_, i64>(1)?, "scoreType": r.get::<_, i64>(2)?,
                        "gameScoreboardId": r.get::<_, i64>(3)?, "trackId": track, "track": catalog.tracks.get(&track),
                        "score": r.get::<_, f64>(5)?, "carId": car, "car": catalog.car(car),
                        "time": r.get::<_, i64>(7)?, "bytes": r.get::<_, i64>(8)?,
                    }))
                })?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// Drivatar.SaveDrivatarInformation: what the game learned about how this player drives
    /// ({info, curveTypes, segmentInformation}). The latest upload replaces the previous one.
    pub fn save_drivatar(&self, xuid: u64, data: &Value) {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO drivatars (xuid, data, updated, saves) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT (xuid) DO UPDATE SET data = ?2, updated = ?3, saves = saves + 1",
                params![xuid as i64, data.to_string(), unix_ms()],
            )
        });
    }

    /// The players with a drivatar (accounts not banned), the most recently trained first.
    pub fn drivatar_owners(&self, limit: usize) -> Vec<u64> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT d.xuid FROM drivatars d JOIN accounts a ON a.xuid = d.xuid
                 WHERE a.banned = 0 ORDER BY d.updated DESC LIMIT ?1",
            )?;
            let found = stmt.query_map([limit as i64], |r| r.get::<_, i64>(0))?.flatten().map(|x| x as u64).collect::<Vec<_>>();
            Ok(found)
        })
        .unwrap_or_default()
    }

    /// (data, updated, saves) of a player's drivatar.
    pub fn drivatar(&self, xuid: u64) -> Option<(Value, i64, i64)> {
        self.with(|db| {
            db.query_row("SELECT data, updated, saves FROM drivatars WHERE xuid = ?1", [xuid as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
            })
            .optional()
        })
        .flatten()
        .map(|(d, u, s)| (serde_json::from_str(&d).unwrap_or(Value::Null), u, s))
    }


    pub fn add_xp(&self, xuid: u64, xp: f64) {
        self.touch_user(xuid);
        self.with(|db| db.execute("INSERT INTO xp (xuid, time, xp) VALUES (?1, ?2, ?3)", params![xuid as i64, unix_ms(), xp]));
    }

    /// OnlineProfile.UpdateLivery (with layer data) and ContentVersioning.CreateVersionedLivery
    /// (the id the game keeps) both land here; returns the row id.
    pub fn add_livery(&self, xuid: u64, name: &str, car_id: i64, data: &Value) -> i64 {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO liveries (xuid, time, name, car_id, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, unix_ms(), name.chars().take(64).collect::<String>(), car_id, data.to_string()],
            )?;
            let id = db.last_insert_rowid();
            db.execute(
                // a livery a shared file points at is kept
                "DELETE FROM liveries WHERE xuid = ?1 AND id NOT IN (SELECT id FROM liveries WHERE xuid = ?1 ORDER BY id DESC LIMIT ?2)
                 AND id NOT IN (SELECT versioned_id FROM storefront_files WHERE kind = 'livery' AND removed = 0)",
                params![xuid as i64, MAX_PER_PLAYER],
            )?;
            Ok(id)
        })
        .unwrap_or(0)
    }

    /// A versioned livery by id: (owner, parameters with its data). CreateVersionedLivery only
    /// names the livery; its bytes came in the UpdateLivery of the same name just before, so a
    /// row without data takes the newest data of that owner and name.
    pub fn livery(&self, id: i64) -> Option<(u64, Value)> {
        self.with(|db| {
            let Some((xuid, name, data)) = db
                .query_row("SELECT xuid, name, data FROM liveries WHERE id = ?1", [id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })
                .optional()?
            else {
                return Ok(None);
            };
            let mut params: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
            if params["data"].as_str().is_none_or(str::is_empty) {
                let mut stmt = db.prepare("SELECT data FROM liveries WHERE xuid = ?1 AND name = ?2 ORDER BY id DESC")?;
                let rows = stmt.query_map(params![xuid, name], |r| r.get::<_, String>(0))?;
                if let Some(full) = rows.flatten().filter_map(|t| serde_json::from_str::<Value>(&t).ok()).find(|v| {
                    v["data"].as_str().is_some_and(|d| !d.is_empty())
                }) {
                    params = full;
                }
            }
            Ok(Some((xuid as u64, params)))
        })
        .flatten()
        .filter(|(_, p)| p["data"].as_str().is_some_and(|d| !d.is_empty()))
    }

    pub fn set_livery_data(&self, id: i64, data: &Value) {
        self.with(|db| db.execute("UPDATE liveries SET data = ?2 WHERE id = ?1", params![id, data.to_string()]));
    }

    /// A versioned tune by id: (owner, its parameters: carId, installedParts, tuning...).
    pub fn tune(&self, id: i64) -> Option<(u64, Value)> {
        self.with(|db| {
            db.query_row("SELECT xuid, data FROM tunes WHERE id = ?1", [id], |r| {
                Ok((r.get::<_, i64>(0)? as u64, serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or(Value::Null)))
            })
            .optional()
        })
        .flatten()
    }

    pub fn add_tune(&self, xuid: u64, car_id: i64, data: &Value) -> i64 {
        self.touch_user(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO tunes (xuid, time, car_id, data) VALUES (?1, ?2, ?3, ?4)",
                params![xuid as i64, unix_ms(), car_id, data.to_string()],
            )?;
            let id = db.last_insert_rowid();
            db.execute(
                "DELETE FROM tunes WHERE xuid = ?1 AND id NOT IN (SELECT id FROM tunes WHERE xuid = ?1 ORDER BY id DESC LIMIT ?2)
                 AND id NOT IN (SELECT versioned_id FROM storefront_files WHERE kind = 'tune' AND removed = 0)",
                params![xuid as i64, MAX_PER_PLAYER],
            )?;
            Ok(id)
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

    /// How many `kind` events a player caused since a time (rate limits).
    pub fn events_since(&self, xuid: u64, kind: &str, since: i64) -> i64 {
        self.with(|db| {
            db.query_row(
                "SELECT COUNT(*) FROM events WHERE xuid = ?1 AND kind = ?2 AND time >= ?3",
                params![xuid as i64, kind, since],
                |r| r.get(0),
            )
        })
        .unwrap_or(0)
    }

    pub fn users(&self) -> Value {
        let rows = self
            .with(|db| {
                let mut stmt = db.prepare(
                    "SELECT u.xuid, u.gamertag, u.first_seen, u.last_seen, u.logons,
                            (SELECT COUNT(*) FROM friends f WHERE f.xuid = u.xuid)
                     FROM users u ORDER BY u.last_seen DESC",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)? as u64,
                            json!({
                                "xuid": (r.get::<_, i64>(0)? as u64).to_string(),
                                "gamertag": r.get::<_, String>(1)?,
                                "firstSeen": r.get::<_, i64>(2)?,
                                "lastSeen": r.get::<_, i64>(3)?,
                                "logons": r.get::<_, i64>(4)?,
                                "friends": r.get::<_, i64>(5)?,
                            }),
                        ))
                    })?
                    .flatten()
                    .collect::<Vec<_>>();
                Ok(rows)
            })
            .unwrap_or_default();
        Value::Array(
            rows.into_iter()
                .map(|(xuid, mut user)| {
                    user["presence"] = self.presence_json(xuid);
                    user
                })
                .collect(),
        )
    }

    pub fn xuid_by_gamertag(&self, gamertag: &str) -> Option<u64> {
        self.with(|db| {
            db.query_row("SELECT xuid FROM users WHERE gamertag = ?1 COLLATE NOCASE", [gamertag.trim()], |r| r.get::<_, i64>(0))
                .optional()
        })
        .flatten()
        .map(|x| x as u64)
    }

    pub fn user_exists(&self, xuid: u64) -> bool {
        self.with(|db| db.query_row("SELECT 1 FROM users WHERE xuid = ?1", [xuid as i64], |_| Ok(())).optional())
            .flatten()
            .is_some()
    }

    /// A player made in the manager (a friend who has not played yet, or a test account).
    /// XUIDs come from a range real consoles never used (2535000000000001...).
    pub fn create_user(&self, gamertag: &str) -> Result<u64, String> {
        let now = unix_ms();
        let db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let taken: bool = db
            .query_row("SELECT 1 FROM users WHERE gamertag = ?1 COLLATE NOCASE", [gamertag], |_| Ok(()))
            .optional()
            .map_err(|e| e.to_string())?
            .is_some();
        if taken {
            return Err("gamertag já existe".into());
        }
        let base: i64 = 2_535_000_000_000_000;
        let next: i64 = db
            .query_row("SELECT MAX(xuid) FROM users WHERE xuid > ?1", [base], |r| r.get::<_, Option<i64>>(0))
            .map_err(|e| e.to_string())?
            .unwrap_or(base)
            + 1;
        db.execute(
            "INSERT INTO users (xuid, gamertag, first_seen, last_seen) VALUES (?1, ?2, ?3, ?3)",
            params![next, gamertag, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(next as u64)
    }

    /// Removes a player and everything that is theirs: the account and its devices, profile,
    /// save, scores, ghosts, drivatar, liveries, tunes, storefront files (and the likes,
    /// downloads and reports on them), libraries, friendships and requests both ways, club
    /// membership (the club passes on as when the leader leaves), notifications to them and the
    /// messages they sent. Their likes and downloads of other players' files go too, and those
    /// files' counters are recounted. One transaction: all or nothing.
    pub fn delete_user(&self, xuid: u64) -> bool {
        self.leave_club(xuid, clubs::ACTION_REMOVED);
        let x = xuid as i64;
        self.with(|db| {
            let tx = db.unchecked_transaction()?;
            let n = tx.execute("DELETE FROM users WHERE xuid = ?1", [x])?;
            for sql in [
                // what points at the player's shared files, then the files
                "DELETE FROM storefront_likes WHERE file_id IN (SELECT id FROM storefront_files WHERE xuid = ?1) OR xuid = ?1",
                "DELETE FROM storefront_downloads WHERE file_id IN (SELECT id FROM storefront_files WHERE xuid = ?1) OR xuid = ?1",
                "DELETE FROM storefront_reports WHERE file_id IN (SELECT id FROM storefront_files WHERE xuid = ?1) OR xuid = ?1",
                "DELETE FROM storefront_files WHERE xuid = ?1",
                "DELETE FROM followed_creators WHERE xuid = ?1 OR creator = ?1",
                "DELETE FROM livery_groups WHERE xuid = ?1",
                "DELETE FROM versioned_groups WHERE xuid = ?1",
                "DELETE FROM car_setups WHERE xuid = ?1",
                "DELETE FROM liveries WHERE xuid = ?1",
                "DELETE FROM tunes WHERE xuid = ?1",
                "DELETE FROM scores WHERE xuid = ?1",
                "DELETE FROM xp WHERE xuid = ?1",
                "DELETE FROM transactions WHERE xuid = ?1",
                "DELETE FROM friends WHERE xuid = ?1 OR friend = ?1",
                "DELETE FROM presence WHERE xuid = ?1",
                "DELETE FROM profile WHERE xuid = ?1",
                "DELETE FROM profile_tables WHERE xuid = ?1",
                "DELETE FROM drivatars WHERE xuid = ?1",
                "DELETE FROM ghosts WHERE xuid = ?1",
                "DELETE FROM events WHERE xuid = ?1",
                "DELETE FROM club_invites WHERE xuid = ?1 OR sender = ?1",
                "DELETE FROM club_requests WHERE xuid = ?1",
                "DELETE FROM club_history WHERE xuid = ?1",
                "DELETE FROM club_notices WHERE xuid = ?1",
                "DELETE FROM user_settings WHERE xuid = ?1",
                // their inbox, and what they sent to others (messages, rival times)
                "DELETE FROM notifications WHERE xuid = ?1 OR json_extract(params, '$.senderXuid') = ?1",
                "DELETE FROM accounts WHERE xuid = ?1",
                "DELETE FROM account_lang WHERE xuid = ?1",
                "DELETE FROM devices WHERE xuid = ?1",
                "DELETE FROM web_keys WHERE xuid = ?1",
                "DELETE FROM saves WHERE xuid = ?1",
                "DELETE FROM integrity WHERE xuid = ?1",
                "DELETE FROM friend_requests WHERE from_xuid = ?1 OR to_xuid = ?1",
            ] {
                tx.execute(sql, [x])?;
            }
            tx.execute_batch(
                "UPDATE storefront_files SET
                     likes = (SELECT COUNT(*) FROM storefront_likes WHERE file_id = storefront_files.id),
                     downloads = (SELECT COUNT(*) FROM storefront_downloads WHERE file_id = storefront_files.id)",
            )?;
            tx.commit()?;
            Ok(n > 0)
        })
        .unwrap_or(false)
    }

    /// Friendship as the manager makes it: each one follows the other.
    pub fn add_friend(&self, a: u64, b: u64) -> bool {
        let now = unix_ms();
        self.with(|db| {
            for (x, y) in [(a, b), (b, a)] {
                db.execute(
                    "INSERT OR IGNORE INTO friends (xuid, friend, created) VALUES (?1, ?2, ?3)",
                    params![x as i64, y as i64, now],
                )?;
            }
            Ok(true)
        })
        .unwrap_or(false)
    }

    pub fn remove_friend(&self, a: u64, b: u64) -> bool {
        self.with(|db| {
            db.execute(
                "DELETE FROM friends WHERE (xuid = ?1 AND friend = ?2) OR (xuid = ?2 AND friend = ?1)",
                params![a as i64, b as i64],
            )
        })
        .is_some_and(|n| n > 0)
    }

    pub fn set_favorite(&self, xuid: u64, friend: u64, favorite: bool) -> bool {
        self.with(|db| {
            db.execute(
                "UPDATE friends SET favorite = ?3 WHERE xuid = ?1 AND friend = ?2",
                params![xuid as i64, friend as i64, favorite as i64],
            )
        })
        .is_some_and(|n| n > 0)
    }

    /// Who `xuid` follows: (friend, favorite, follows back).
    pub fn friends(&self, xuid: u64) -> Vec<(u64, bool, bool)> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT f.friend, f.favorite,
                        EXISTS (SELECT 1 FROM friends b WHERE b.xuid = f.friend AND b.friend = f.xuid)
                 FROM friends f WHERE f.xuid = ?1 ORDER BY f.created",
            )?;
            let rows = stmt
                .query_map([xuid as i64], |r| {
                    Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? != 0, r.get::<_, i64>(2)? != 0))
                })?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// The friends list as the manager shows it.
    pub fn friends_json(&self, xuid: u64) -> Value {
        Value::Array(
            self.friends(xuid)
                .into_iter()
                .map(|(friend, favorite, mutual)| {
                    json!({
                        "xuid": friend.to_string(),
                        "gamertag": self.gamertag(friend),
                        "favorite": favorite,
                        "mutual": mutual,
                        "presence": self.presence_json(friend),
                    })
                })
                .collect(),
        )
    }

    /// Stores the game's rich presence. Returns true when it changed (not just refreshed).
    pub fn set_presence(&self, xuid: u64, state: &str, rich: &str) -> bool {
        let before = self.presence(xuid);
        self.with(|db| {
            db.execute(
                "INSERT INTO presence (xuid, state, rich, updated) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (xuid) DO UPDATE SET state = ?2, rich = ?3, updated = ?4",
                params![xuid as i64, state, rich, unix_ms()],
            )
        });
        !matches!(before, Some((s, r, t)) if s == state && r == rich && unix_ms() - t < PRESENCE_TTL_MS)
    }

    /// (state, rich presence id, updated) as the game last posted it.
    pub fn presence(&self, xuid: u64) -> Option<(String, String, i64)> {
        self.with(|db| {
            db.query_row("SELECT state, rich, updated FROM presence WHERE xuid = ?1", [xuid as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()
        })
        .flatten()
    }

    /// Online while the game keeps posting (it does every few seconds while running).
    pub fn is_online(&self, xuid: u64) -> bool {
        self.presence(xuid).is_some_and(|(state, _, t)| state != "offline" && unix_ms() - t < PRESENCE_TTL_MS)
    }

    pub fn presence_json(&self, xuid: u64) -> Value {
        match self.presence(xuid) {
            Some((_, rich, updated)) => json!({
                "online": self.is_online(xuid),
                "rich": rich,
                "updated": updated,
            }),
            None => json!({ "online": false, "rich": "", "updated": 0 }),
        }
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
            "ghosts": self.ghosts(xuid, catalog),
            "drivatar": self.drivatar(xuid).map(|(d, updated, saves)| json!({
                "updated": updated,
                "saves": saves,
                "segments": d["segmentInformation"].as_array().map(Vec::len).unwrap_or(0),
                "curves": d["curveTypes"].as_array().map(Vec::len).unwrap_or(0),
                "info": d["info"],
            })),
            // Xbox achievements unlock locally (Windows.Xbox.Achievements in Azimuth) and do
            // not reach this server yet; the hook will have to forward them.
            "achievements": [],
            "friends": self.friends_json(xuid),
            "presence": self.presence_json(xuid),
            "notifications": self.notifications_json(xuid),
            // refused syncs (forza/gates.rs)
            "integrity": self.integrity_json(xuid),
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

/// The header sends "248398cef9d0...", Reconcile a GUID "248398ce-f9d0-...": compare bare hex.
fn transaction_key(id: &str) -> String {
    id.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_lowercase()).collect()
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

/// See Store::game_tag.
pub fn game_tag(gamertag: &str) -> String {
    if gamertag.chars().count() <= GAME_TAG_CHARS {
        return gamertag.to_owned();
    }
    gamertag.split('#').next().unwrap_or("").chars().take(GAME_TAG_CHARS).collect()
}

/// Characters a gamertag field of the game holds (UserData.wzGamerTag and the others).
pub const GAME_TAG_CHARS: usize = 15;
