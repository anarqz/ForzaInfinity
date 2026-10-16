//! Players' reports and moderation (an account reported from the game, PLAYER ACTIONS > REPORT
//! PLAYER, e.g. on a leaderboard).
//!
//! - player_reports: one row per (reporter, reported): a player reports another once (a second
//!   time changes nothing). A reporter must be a real player: an account at least
//!   REPORTER_MIN_AGE old that has played online (its profile synced at least once), and sends
//!   at most REPORTS_PER_DAY a day. Once FLAG_AT different players reported an account, its
//!   accounts.flagged is set (with flagged_at): a moderator (a person, or an agent they run)
//!   looks at it with `fh2-server moderation list|show` and either clears it or resets the
//!   account (`moderation clear|reset`).
//! - moderation_log: what was done to an account, by whom and why.
//!
//! Nobody is banned by a report: the worst a reset does is take the account back to a new
//! profile's progress (moderation_reset below). Shared creations (liveries, designs, tunes) have
//! their own reports (storefront.rs) and are never touched here.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::Store;
use crate::unix_ms;

/// Different players' reports that flag an account for review.
pub const FLAG_AT: i64 = 5;
/// Reports one player may send in a day.
pub const REPORTS_PER_DAY: i64 = 20;
/// How old an account must be to report anyone.
pub const REPORTER_MIN_AGE_MS: i64 = 24 * 3600 * 1000;
const DAY_MS: i64 = 24 * 3600 * 1000;
const REASON_MAX: usize = 120;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS player_reports (
        reporter INTEGER NOT NULL, reported INTEGER NOT NULL, reason TEXT NOT NULL,
        created INTEGER NOT NULL, PRIMARY KEY (reporter, reported));
    CREATE INDEX IF NOT EXISTS player_reports_reported ON player_reports (reported);
    CREATE TABLE IF NOT EXISTS moderation_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, action TEXT NOT NULL,
        note TEXT NOT NULL, created INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS moderation_log_xuid ON moderation_log (xuid);";

/// accounts.flagged / flagged_at / reset_pending, added to databases made before them.
pub fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(accounts)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    if !columns.iter().any(|c| c == "flagged") {
        db.execute_batch("ALTER TABLE accounts ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0;")?;
    }
    if !columns.iter().any(|c| c == "flagged_at") {
        db.execute_batch("ALTER TABLE accounts ADD COLUMN flagged_at INTEGER;")?;
    }
    // a moderation reset the game has not downloaded yet (db/reset.rs)
    if !columns.iter().any(|c| c == "reset_pending") {
        db.execute_batch("ALTER TABLE accounts ADD COLUMN reset_pending INTEGER NOT NULL DEFAULT 0;")?;
    }
    Ok(())
}

/// A real player may report (a player or a shared file): an account REPORTER_MIN_AGE old at least
/// that has played online (its profile synced at least once).
pub fn real_reporter(db: &Connection, reporter: u64, now: i64) -> rusqlite::Result<bool> {
    let created: Option<i64> =
        db.query_row("SELECT created FROM accounts WHERE xuid = ?1", params![reporter as i64], |r| r.get(0)).optional()?;
    let played: i64 = db.query_row("SELECT COUNT(*) FROM profile WHERE xuid = ?1", params![reporter as i64], |r| r.get(0))?;
    Ok(created.is_some_and(|c| now - c >= REPORTER_MIN_AGE_MS) && played > 0)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReportError {
    Yourself,
    NoSuchPlayer,
    TooMany,
    /// The reporter's account is younger than REPORTER_MIN_AGE or never played online.
    TooNew,
    /// The reporter already reported this player.
    Already,
}

/// The reason as given: printable, at most REASON_MAX characters.
fn clean_reason(reason: &str) -> String {
    reason.chars().filter(|c| !c.is_control()).take(REASON_MAX).collect::<String>().trim().to_owned()
}

impl Store {
    /// `reporter` reports `reported`. Returns whether this report flagged the account.
    pub fn report_player(&self, reporter: u64, reported: u64, reason: &str) -> Result<bool, ReportError> {
        self.report_player_at(reporter, reported, reason, unix_ms())
    }

    pub fn report_player_at(&self, reporter: u64, reported: u64, reason: &str, now: i64) -> Result<bool, ReportError> {
        if reporter == reported {
            return Err(ReportError::Yourself);
        }
        let reason = clean_reason(reason);
        let db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let outcome = (|| -> rusqlite::Result<Result<bool, ReportError>> {
            let exists: Option<i64> =
                db.query_row("SELECT flagged FROM accounts WHERE xuid = ?1", params![reported as i64], |r| r.get(0)).optional()?;
            let Some(flagged) = exists else { return Ok(Err(ReportError::NoSuchPlayer)) };
            if !real_reporter(&db, reporter, now)? {
                return Ok(Err(ReportError::TooNew));
            }
            let already: i64 = db.query_row(
                "SELECT COUNT(*) FROM player_reports WHERE reporter = ?1 AND reported = ?2",
                params![reporter as i64, reported as i64],
                |r| r.get(0),
            )?;
            if already > 0 {
                return Ok(Err(ReportError::Already));
            }
            let today: i64 = db.query_row(
                "SELECT COUNT(*) FROM player_reports WHERE reporter = ?1 AND created > ?2",
                params![reporter as i64, now - DAY_MS],
                |r| r.get(0),
            )?;
            if today >= REPORTS_PER_DAY {
                return Ok(Err(ReportError::TooMany));
            }
            db.execute(
                "INSERT INTO player_reports (reporter, reported, reason, created) VALUES (?1, ?2, ?3, ?4)",
                params![reporter as i64, reported as i64, reason, now],
            )?;
            // different players who reported it, not counting suspended accounts
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM player_reports r JOIN accounts a ON a.xuid = r.reporter
                 WHERE r.reported = ?1 AND a.banned = 0",
                params![reported as i64],
                |r| r.get(0),
            )?;
            if flagged == 0 && count >= FLAG_AT {
                db.execute("UPDATE accounts SET flagged = 1, flagged_at = ?2 WHERE xuid = ?1", params![reported as i64, now])?;
                db.execute(
                    "INSERT INTO moderation_log (xuid, action, note, created) VALUES (?1, 'flagged', ?2, ?3)",
                    params![reported as i64, format!("{count} players reported this account"), now],
                )?;
                return Ok(Ok(true));
            }
            Ok(Ok(false))
        })();
        match outcome {
            Ok(r) => r,
            Err(e) => {
                eprintln!("db: report: {e}");
                Err(ReportError::NoSuchPlayer)
            }
        }
    }

    /// Flagged accounts, oldest flag first: [{xuid, gamertag, flaggedAt, reports}].
    pub fn flagged_accounts(&self) -> Vec<Value> {
        self.with(|db| {
            let mut q = db.prepare(
                "SELECT a.xuid, a.name || '#' || a.discriminator, a.flagged_at,
                        (SELECT COUNT(*) FROM player_reports r WHERE r.reported = a.xuid)
                 FROM accounts a WHERE a.flagged = 1 ORDER BY a.flagged_at",
            )?;
            let rows = q.query_map([], |r| {
                Ok(json!({
                    "xuid": r.get::<_, i64>(0)?.to_string(),
                    "gamertag": r.get::<_, String>(1)?,
                    "flaggedAt": r.get::<_, Option<i64>>(2)?,
                    "reports": r.get::<_, i64>(3)?,
                }))
            })?;
            rows.collect()
        })
        .unwrap_or_default()
    }

    /// Everything a moderator needs about one account (reports, its records, what the server's
    /// gates saw, the moderation log), as JSON.
    pub fn moderation_detail(&self, xuid: u64) -> Option<Value> {
        let x = xuid as i64;
        self.with(|db| {
            let account = db
                .query_row(
                    "SELECT name || '#' || discriminator, created, banned, flagged, flagged_at FROM accounts WHERE xuid = ?1",
                    params![x],
                    |r| {
                        Ok(json!({
                            "xuid": xuid.to_string(),
                            "gamertag": r.get::<_, String>(0)?,
                            "created": r.get::<_, i64>(1)?,
                            "banned": r.get::<_, i64>(2)? != 0,
                            "flagged": r.get::<_, i64>(3)? != 0,
                            "flaggedAt": r.get::<_, Option<i64>>(4)?,
                        }))
                    },
                )
                .optional()?;
            let Some(mut account) = account else { return Ok(None) };
            let reports: Vec<Value> = db
                .prepare(
                    "SELECT r.reporter, a.name || '#' || a.discriminator, r.reason, r.created
                     FROM player_reports r LEFT JOIN accounts a ON a.xuid = r.reporter
                     WHERE r.reported = ?1 ORDER BY r.created",
                )?
                .query_map(params![x], |r| {
                    Ok(json!({
                        "reporter": r.get::<_, i64>(0)?.to_string(),
                        "gamertag": r.get::<_, Option<String>>(1)?,
                        "reason": r.get::<_, String>(2)?,
                        "at": r.get::<_, i64>(3)?,
                    }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let count = |sql: &str| db.query_row(sql, params![x], |r| r.get::<_, i64>(0));
            let records = json!({
                "scores": count("SELECT COUNT(*) FROM scores WHERE xuid = ?1")?,
                "ghosts": count("SELECT COUNT(*) FROM ghosts WHERE xuid = ?1")?,
                "drivatars": count("SELECT COUNT(*) FROM drivatars WHERE xuid = ?1")?,
                "cloudSaves": count("SELECT COUNT(*) FROM saves WHERE xuid = ?1")?,
            });
            let profile: Option<String> =
                db.query_row("SELECT data FROM profile WHERE xuid = ?1", params![x], |r| r.get(0)).optional()?;
            let profile: Value = profile.and_then(|p| serde_json::from_str(&p).ok()).unwrap_or(Value::Null);
            let history = &profile["playerHistory"][0];
            let race = &profile["playerRaceHistory"][0];
            let summary = json!({
                "credits": history["credits"],
                "level": race["level"],
                "xp": race["xp"],
                "races": race["numRaces"],
                "distanceDriven": race["distanceDriven"],
                "totalWinnings": race["totalWinnings"],
            });
            let garage: Option<String> = db
                .query_row("SELECT data FROM profile_tables WHERE xuid = ?1 AND name = 'garageCars'", params![x], |r| r.get(0))
                .optional()?;
            // {"<garage id>": {"id", "carId", ...}} as the game syncs it (UpdateAllTables)
            let garage: Value = garage.and_then(|g| serde_json::from_str(&g).ok()).unwrap_or(Value::Null);
            let cars: Vec<Value> = match &garage {
                Value::Object(m) => m.values().map(|c| c["carId"].clone()).collect(),
                Value::Array(a) => a.iter().map(|c| c["carId"].clone()).collect(),
                _ => Vec::new(),
            };
            let garage_cars = cars.len();
            let top: Vec<Value> = db
                .prepare(
                    "SELECT scoreboard_type, track_id, car_id, score, time FROM scores WHERE xuid = ?1
                     ORDER BY time DESC LIMIT 20",
                )?
                .query_map(params![x], |r| {
                    Ok(json!({
                        "board": r.get::<_, i64>(0)?, "track": r.get::<_, i64>(1)?, "car": r.get::<_, i64>(2)?,
                        "score": r.get::<_, f64>(3)?, "at": r.get::<_, i64>(4)?,
                    }))
                })?
                .collect::<rusqlite::Result<_>>()
                .unwrap_or_default();
            let log: Vec<Value> = db
                .prepare("SELECT action, note, created FROM moderation_log WHERE xuid = ?1 ORDER BY id")?
                .query_map(params![x], |r| {
                    Ok(json!({ "action": r.get::<_, String>(0)?, "note": r.get::<_, String>(1)?, "at": r.get::<_, i64>(2)? }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            account["reports"] = Value::from(reports);
            account["records"] = records;
            account["profile"] = summary;
            account["garageCars"] = Value::from(garage_cars);
            account["garage"] = Value::from(cars);
            account["recentScores"] = Value::from(top);
            account["moderationLog"] = Value::from(log);
            Ok(Some(account))
        })
        .flatten()
        .map(|mut a| {
            a["integrity"] = self.integrity_json(xuid);
            a
        })
    }

    // ------------------------------------------------------------ shared files

    /// Shared files off the storefront after reports, waiting for moderation, oldest first.
    pub fn files_under_review(&self) -> Vec<Value> {
        self.with(|db| {
            let mut q = db.prepare(
                "SELECT f.id, f.kind, f.title, f.description, f.car_id, f.xuid, a.name || '#' || a.discriminator, f.created,
                        (SELECT COUNT(*) FROM storefront_reports r WHERE r.file_id = f.id),
                        (SELECT MAX(time) FROM storefront_reports r WHERE r.file_id = f.id)
                 FROM storefront_files f LEFT JOIN accounts a ON a.xuid = f.xuid
                 WHERE f.removed = ?1 ORDER BY 10",
            )?;
            let rows = q.query_map(params![super::storefront::UNDER_REVIEW], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
                    "description": r.get::<_, String>(3)?, "car": r.get::<_, i64>(4)?,
                    "owner": r.get::<_, i64>(5)?.to_string(), "ownerGamertag": r.get::<_, Option<String>>(6)?,
                    "created": r.get::<_, i64>(7)?, "reports": r.get::<_, i64>(8)?, "lastReport": r.get::<_, Option<i64>>(9)?,
                }))
            })?;
            rows.collect()
        })
        .unwrap_or_default()
    }

    /// One shared file for moderation: its details, who reported it and when, and its preview
    /// image (raw bytes, for `--preview FILE`).
    pub fn file_detail(&self, id: i64) -> Option<(Value, Vec<u8>)> {
        self.with(|db| {
            let file = db
                .query_row(
                    "SELECT f.kind, f.type, f.title, f.description, f.car_id, f.xuid, a.name || '#' || a.discriminator, f.created,
                            f.removed, f.downloads, f.likes, f.layers, f.preview
                     FROM storefront_files f LEFT JOIN accounts a ON a.xuid = f.xuid WHERE f.id = ?1",
                    params![id],
                    |r| {
                        let removed: i64 = r.get(8)?;
                        Ok((
                            json!({
                                "id": id, "kind": r.get::<_, String>(0)?, "type": r.get::<_, i64>(1)?,
                                "title": r.get::<_, String>(2)?, "description": r.get::<_, String>(3)?,
                                "car": r.get::<_, i64>(4)?, "owner": r.get::<_, i64>(5)?.to_string(),
                                "ownerGamertag": r.get::<_, Option<String>>(6)?, "created": r.get::<_, i64>(7)?,
                                "state": match removed { 0 => "shared", 1 => "removed by its owner", 2 => "removed by moderation", 3 => "under review", _ => "?" },
                                "downloads": r.get::<_, i64>(9)?, "likes": r.get::<_, i64>(10)?, "layers": r.get::<_, i64>(11)?,
                            }),
                            r.get::<_, String>(12)?,
                        ))
                    },
                )
                .optional()?;
            let Some((mut file, preview_hex)) = file else { return Ok(None) };
            let reports: Vec<Value> = db
                .prepare(
                    "SELECT r.xuid, a.name || '#' || a.discriminator, r.time FROM storefront_reports r
                     LEFT JOIN accounts a ON a.xuid = r.xuid WHERE r.file_id = ?1 ORDER BY r.time",
                )?
                .query_map(params![id], |r| {
                    Ok(json!({ "reporter": r.get::<_, i64>(0)?.to_string(), "gamertag": r.get::<_, Option<String>>(1)?, "at": r.get::<_, i64>(2)? }))
                })?
                .collect::<rusqlite::Result<_>>()?;
            file["reports"] = Value::from(reports);
            let preview: Vec<u8> = (0..preview_hex.len() / 2)
                .filter_map(|i| u8::from_str_radix(preview_hex.get(2 * i..2 * i + 2)?, 16).ok())
                .collect();
            file["previewBytes"] = Value::from(preview.len());
            Ok(Some((file, preview)))
        })
        .flatten()
    }

    /// Moderation's word on a file under review (or any shared file): `keep` puts it back on the
    /// storefront (its reports go), else it goes for good (its content can never be shared again).
    pub fn moderate_file(&self, id: i64, keep: bool, note: &str) -> bool {
        let now = unix_ms();
        self.with(|db| {
            let owner: Option<i64> = db.query_row("SELECT xuid FROM storefront_files WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
            let Some(owner) = owner else { return Ok(false) };
            if keep {
                db.execute("UPDATE storefront_files SET removed = 0 WHERE id = ?1 AND removed = ?2", params![id, super::storefront::UNDER_REVIEW])?;
                db.execute("DELETE FROM storefront_reports WHERE file_id = ?1", params![id])?;
            } else {
                db.execute("UPDATE storefront_files SET removed = ?2 WHERE id = ?1", params![id, super::storefront::REMOVED_BY_REPORTS])?;
            }
            db.execute(
                "INSERT INTO moderation_log (xuid, action, note, created) VALUES (?1, ?2, ?3, ?4)",
                params![owner, if keep { "file-kept" } else { "file-deleted" }, format!("file {id}; {}", clean_reason(note)), now],
            )?;
            Ok(true)
        })
        .unwrap_or(false)
    }

    /// The moderator found nothing wrong: the flag and the reports go (new reports start over).
    pub fn moderation_clear(&self, xuid: u64, note: &str) -> bool {
        let now = unix_ms();
        self.with(|db| {
            let n = db.execute("UPDATE accounts SET flagged = 0, flagged_at = NULL WHERE xuid = ?1", params![xuid as i64])?;
            if n == 0 {
                return Ok(false);
            }
            db.execute("DELETE FROM player_reports WHERE reported = ?1", params![xuid as i64])?;
            db.execute(
                "INSERT INTO moderation_log (xuid, action, note, created) VALUES (?1, 'cleared', ?2, ?3)",
                params![xuid as i64, clean_reason(note), now],
            )?;
            Ok(true)
        })
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(accounts: &[u64]) -> Store {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("fh2-moderation-{}-{}-{n}.sqlite", std::process::id(), unix_ms()));
        let s = Store::open(&path).unwrap();
        s.with(|db| {
            for x in accounts {
                db.execute(
                    "INSERT INTO accounts (xuid, name, discriminator, device, created) VALUES (?1, 'P', ?2, ?3, ?4)",
                    params![*x as i64, format!("{x:05}"), format!("d{x}"), unix_ms() - 2 * REPORTER_MIN_AGE_MS],
                )?;
                db.execute("INSERT INTO profile (xuid, data, updated) VALUES (?1, '{}', 0)", params![*x as i64])?;
            }
            Ok(())
        });
        s
    }

    #[test]
    fn five_players_flag_an_account() {
        let s = store_with(&[1, 2, 3, 4, 5, 6, 9]);
        assert_eq!(s.report_player(9, 9, "x"), Err(ReportError::Yourself));
        assert_eq!(s.report_player(1, 777, "x"), Err(ReportError::NoSuchPlayer));
        // a player reports another once
        for r in [1, 2, 3, 4] {
            assert_eq!(s.report_player(r, 9, "impossible time"), Ok(false));
        }
        assert_eq!(s.report_player(1, 9, "again"), Err(ReportError::Already));
        assert!(s.flagged_accounts().is_empty());
        assert_eq!(s.report_player(5, 9, "speed\u{7}hack"), Ok(true));
        let flagged = s.flagged_accounts();
        assert_eq!(flagged.len(), 1);
        assert_eq!((flagged[0]["xuid"].as_str(), flagged[0]["reports"].as_i64()), (Some("9"), Some(5)));
        // already flagged: no second flag
        assert_eq!(s.report_player(6, 9, ""), Ok(false));
        let d = s.moderation_detail(9).unwrap();
        assert_eq!(d["reports"].as_array().unwrap().len(), 6);
        assert_eq!(d["reports"][4]["reason"], "speedhack");
        assert_eq!(d["moderationLog"][0]["action"], "flagged");
        // cleared: the reports go, the count starts over
        assert!(s.moderation_clear(9, "looked fine"));
        assert!(s.flagged_accounts().is_empty());
        assert_eq!(s.moderation_detail(9).unwrap()["reports"].as_array().unwrap().len(), 0);
        assert!(!s.moderation_clear(777, ""));
    }

    #[test]
    fn reports_per_day_are_limited() {
        let s = store_with(&(1..=30).collect::<Vec<u64>>());
        let now = unix_ms();
        for k in 0..REPORTS_PER_DAY as u64 {
            assert!(s.report_player_at(1, 2 + k, "", now).is_ok());
        }
        assert_eq!(s.report_player_at(1, 25, "", now), Err(ReportError::TooMany));
        assert!(s.report_player_at(1, 25, "", now + DAY_MS + 1).is_ok());
    }

    #[test]
    fn only_real_players_report() {
        let s = store_with(&[1, 2, 3]);
        // a new account (an hour old) cannot report
        s.with(|db| db.execute("UPDATE accounts SET created = ?1 WHERE xuid = 1", params![unix_ms() - 3600 * 1000]));
        assert_eq!(s.report_player(1, 3, ""), Err(ReportError::TooNew));
        // an old account that never played online cannot either
        s.with(|db| db.execute("DELETE FROM profile WHERE xuid = 2", []));
        assert_eq!(s.report_player(2, 3, ""), Err(ReportError::TooNew));
        // a day later, having played: it can
        assert_eq!(s.report_player_at(1, 3, "", unix_ms() + REPORTER_MIN_AGE_MS), Ok(false));
    }

    #[test]
    fn migrate_adds_the_columns_once() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE accounts (xuid INTEGER PRIMARY KEY, banned INTEGER NOT NULL DEFAULT 0);").unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        db.execute("INSERT INTO accounts (xuid) VALUES (1)", []).unwrap();
        let f: i64 = db.query_row("SELECT flagged FROM accounts", [], |r| r.get(0)).unwrap();
        assert_eq!(f, 0);
    }
}
