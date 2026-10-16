//! Storefront: the files players share (designs, vinyl groups, tuning setups), the design
//! libraries the game syncs (OnlineProfile.UpdateLiveryGroup / UpdateCarSetup), versioned vinyl
//! groups (ContentVersioning) and followed creators.
//!
//! A shared file points at versioned content (a livery, vinyl group or tune id); the content
//! itself stays in its own table and is never pruned while a file shares it. Versioned content
//! keeps its own copy, so renaming or deleting a library entry does not touch shared files.

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::Store;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS storefront_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, kind TEXT NOT NULL,
        type INTEGER NOT NULL, versioned_id INTEGER NOT NULL, car_id INTEGER NOT NULL,
        title TEXT NOT NULL, description TEXT NOT NULL, keyword_one INTEGER NOT NULL,
        keyword_two INTEGER NOT NULL, file_name TEXT NOT NULL, layers INTEGER NOT NULL DEFAULT 0,
        meta TEXT NOT NULL DEFAULT '{}', preview TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL,
        removed INTEGER NOT NULL DEFAULT 0, downloads INTEGER NOT NULL DEFAULT 0,
        likes INTEGER NOT NULL DEFAULT 0, uses INTEGER NOT NULL DEFAULT 0, views INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX IF NOT EXISTS storefront_kind ON storefront_files (kind, removed);
    CREATE TABLE IF NOT EXISTS storefront_reports (
        file_id INTEGER NOT NULL, xuid INTEGER NOT NULL, time INTEGER NOT NULL, PRIMARY KEY (file_id, xuid));
    CREATE TABLE IF NOT EXISTS storefront_downloads (
        file_id INTEGER NOT NULL, xuid INTEGER NOT NULL, PRIMARY KEY (file_id, xuid));
    CREATE TABLE IF NOT EXISTS storefront_likes (
        file_id INTEGER NOT NULL, xuid INTEGER NOT NULL, PRIMARY KEY (file_id, xuid));
    CREATE TABLE IF NOT EXISTS followed_creators (
        xuid INTEGER NOT NULL, creator INTEGER NOT NULL, PRIMARY KEY (xuid, creator));
    CREATE TABLE IF NOT EXISTS livery_groups (
        xuid INTEGER NOT NULL, name TEXT NOT NULL, data TEXT NOT NULL, updated INTEGER NOT NULL,
        PRIMARY KEY (xuid, name));
    CREATE TABLE IF NOT EXISTS versioned_groups (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, name TEXT NOT NULL,
        data TEXT NOT NULL, thumbnail TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS car_setups (
        xuid INTEGER NOT NULL, car_id INTEGER NOT NULL, name TEXT NOT NULL, data TEXT NOT NULL,
        updated INTEGER NOT NULL, PRIMARY KEY (xuid, car_id, name));";

/// Reports from different players that take a file off the storefront, for moderation to look at
/// (db/moderation.rs: `fh2-server moderation files`).
pub const STRIKES_TO_REMOVE: i64 = 3;
/// Reports a player may make in an hour.
pub const REPORTS_PER_HOUR: i64 = 20;
/// `removed`: 0 = shared, 1 = removed by its owner (may be shared again), 2 = removed by
/// moderation (never shared again; before moderation existed: after STRIKES_TO_REMOVE reports),
/// 3 = off the storefront after STRIKES_TO_REMOVE reports, waiting for moderation (it either
/// comes back, 0, or goes for good, 2).
pub const REMOVED_BY_REPORTS: i64 = 2;
pub const UNDER_REVIEW: i64 = 3;

pub enum Report {
    /// counted: the file now has this many strikes
    Counted(i64),
    /// the third strike: the file is off the storefront until moderation decides
    Removed,
    /// already reported by this player, or too many reports this hour
    Ignored,
}

/// Files a player may share at most (each one keeps its content and preview).
pub const MAX_SHARED: i64 = 100;
/// Largest preview image kept (the game reads up to 409600 bytes).
pub const MAX_PREVIEW_HEX: usize = 2 * 409_600;

#[derive(Debug, Clone)]
pub struct SharedFile {
    pub id: i64,
    pub xuid: u64,
    pub file_type: i64,
    pub versioned_id: i64,
    pub car_id: i64,
    pub title: String,
    pub description: String,
    pub keyword_one: i64,
    pub keyword_two: i64,
    pub layers: i64,
    pub meta: Value,
    pub created: i64,
    pub downloads: i64,
    pub likes: i64,
    pub uses: i64,
}

pub struct NewFile<'a> {
    pub kind: &'a str,
    pub file_type: i64,
    pub versioned_id: i64,
    pub car_id: i64,
    pub title: &'a str,
    pub description: &'a str,
    pub keyword_one: i64,
    pub keyword_two: i64,
    pub file_name: &'a str,
    pub layers: i64,
    pub meta: &'a Value,
    pub preview: &'a str,
}

const FILE_COLUMNS: &str =
    "id, xuid, type, versioned_id, car_id, title, description, keyword_one, keyword_two, layers, meta, created, downloads, likes, uses";

fn file_row(r: &rusqlite::Row) -> rusqlite::Result<SharedFile> {
    Ok(SharedFile {
        id: r.get(0)?,
        xuid: r.get::<_, i64>(1)? as u64,
        file_type: r.get(2)?,
        versioned_id: r.get(3)?,
        car_id: r.get(4)?,
        title: r.get(5)?,
        description: r.get(6)?,
        keyword_one: r.get(7)?,
        keyword_two: r.get(8)?,
        layers: r.get(9)?,
        meta: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Null),
        created: r.get(11)?,
        downloads: r.get(12)?,
        likes: r.get(13)?,
        uses: r.get(14)?,
    })
}

impl Store {
    /// Shares a file; the same content shared again by its owner updates that file instead.
    /// Returns the file id, 0 when the player shares too many files, -1 when this content was
    /// taken off the storefront after reports.
    pub fn share_file(&self, xuid: u64, f: &NewFile) -> i64 {
        let now = unix_ms();
        self.with(|db| {
            let existing: Option<i64> = db
                .query_row(
                    "SELECT id FROM storefront_files WHERE xuid = ?1 AND kind = ?2 AND versioned_id = ?3 AND removed = 0",
                    params![xuid as i64, f.kind, f.versioned_id],
                    |r| r.get(0),
                )
                .optional()?;
            let banned: Option<i64> = db
                .query_row(
                    "SELECT id FROM storefront_files WHERE xuid = ?1 AND kind = ?2 AND versioned_id = ?3 AND removed IN (?4, ?5)",
                    params![xuid as i64, f.kind, f.versioned_id, REMOVED_BY_REPORTS, UNDER_REVIEW],
                    |r| r.get(0),
                )
                .optional()?;
            if banned.is_some() {
                return Ok(-1);
            }
            if let Some(id) = existing {
                db.execute(
                    "UPDATE storefront_files SET title = ?2, description = ?3, keyword_one = ?4, keyword_two = ?5,
                     preview = ?6, meta = ?7 WHERE id = ?1",
                    params![id, f.title, f.description, f.keyword_one, f.keyword_two, f.preview, f.meta.to_string()],
                )?;
                return Ok(id);
            }
            let count: i64 = db.query_row(
                "SELECT COUNT(*) FROM storefront_files WHERE xuid = ?1 AND removed = 0",
                [xuid as i64],
                |r| r.get(0),
            )?;
            if count >= MAX_SHARED {
                return Ok(0);
            }
            db.execute(
                "INSERT INTO storefront_files (xuid, kind, type, versioned_id, car_id, title, description, keyword_one,
                 keyword_two, file_name, layers, meta, preview, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    xuid as i64,
                    f.kind,
                    f.file_type,
                    f.versioned_id,
                    f.car_id,
                    f.title,
                    f.description,
                    f.keyword_one,
                    f.keyword_two,
                    f.file_name,
                    f.layers,
                    f.meta.to_string(),
                    f.preview,
                    now
                ],
            )?;
            Ok(db.last_insert_rowid())
        })
        .unwrap_or(0)
    }

    /// Live (not removed) shared files, newest first; `kind` None = any kind, `owner` None = anyone.
    pub fn shared_files(&self, kind: Option<&str>, owner: Option<u64>) -> Vec<SharedFile> {
        self.with(|db| {
            let mut stmt = db.prepare(&format!(
                "SELECT {FILE_COLUMNS} FROM storefront_files WHERE removed = 0
                 AND (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR xuid = ?2) ORDER BY id DESC"
            ))?;
            let rows = stmt.query_map(params![kind, owner.map(|x| x as i64)], file_row)?.flatten().collect::<Vec<_>>();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    pub fn shared_file(&self, id: i64) -> Option<SharedFile> {
        self.with(|db| {
            db.query_row(&format!("SELECT {FILE_COLUMNS} FROM storefront_files WHERE id = ?1 AND removed = 0"), [id], file_row)
                .optional()
        })
        .flatten()
    }

    pub fn shared_file_preview(&self, id: i64) -> String {
        self.with(|db| db.query_row("SELECT preview FROM storefront_files WHERE id = ?1", [id], |r| r.get(0)).optional())
            .flatten()
            .unwrap_or_default()
    }

    /// The live file sharing this versioned content, if any.
    pub fn shared_file_of(&self, kind: &str, versioned_id: i64) -> Option<SharedFile> {
        self.with(|db| {
            db.query_row(
                &format!("SELECT {FILE_COLUMNS} FROM storefront_files WHERE kind = ?1 AND versioned_id = ?2 AND removed = 0 ORDER BY id DESC"),
                params![kind, versioned_id],
                file_row,
            )
            .optional()
        })
        .flatten()
    }

    /// Owner-only edits: "remove", "title", "description". True when the file was the owner's.
    pub fn edit_shared_file(&self, xuid: u64, id: i64, field: &str, value: &str) -> bool {
        let (id, xuid) = (id, xuid as i64);
        self.with(|db| match field {
            "remove" => db.execute("UPDATE storefront_files SET removed = 1 WHERE id = ?1 AND xuid = ?2 AND removed = 0", params![id, xuid]),
            "title" => db.execute("UPDATE storefront_files SET title = ?3 WHERE id = ?1 AND xuid = ?2 AND removed = 0", params![id, xuid, value]),
            "description" => {
                db.execute("UPDATE storefront_files SET description = ?3 WHERE id = ?1 AND xuid = ?2 AND removed = 0", params![id, xuid, value])
            }
            _ => Ok(0),
        })
        .is_some_and(|n| n > 0)
    }

    /// counter: "downloads", "uses", "views"; a like and a download count once per player.
    pub fn count_file(&self, id: i64, xuid: u64, counter: &str) {
        self.with(|db| {
            match counter {
                "likes" => {
                    if db.execute("INSERT OR IGNORE INTO storefront_likes (file_id, xuid) VALUES (?1, ?2)", params![id, xuid as i64])? > 0 {
                        db.execute("UPDATE storefront_files SET likes = likes + 1 WHERE id = ?1", [id])?;
                    }
                }
                // a download counts once per player
                "downloads" => {
                    if db.execute("INSERT OR IGNORE INTO storefront_downloads (file_id, xuid) VALUES (?1, ?2)", params![id, xuid as i64])? > 0 {
                        db.execute("UPDATE storefront_files SET downloads = downloads + 1 WHERE id = ?1", [id])?;
                    }
                }
                "uses" => {
                    db.execute("UPDATE storefront_files SET uses = uses + 1 WHERE id = ?1", [id])?;
                }
                "views" => {
                    db.execute("UPDATE storefront_files SET views = views + 1 WHERE id = ?1", [id])?;
                }
                _ => {}
            }
            Ok(())
        });
    }

    /// A player's report of a file (the caller checked they can see it and do not own it). Only a
    /// real player's counts (db/moderation.rs: a day-old account that has played online).
    pub fn report_file(&self, file: i64, reporter: u64) -> Report {
        let now = unix_ms();
        self.with(|db| {
            if !super::moderation::real_reporter(db, reporter, now)? {
                return Ok(Report::Ignored);
            }
            let recent: i64 = db.query_row(
                "SELECT COUNT(*) FROM storefront_reports WHERE xuid = ?1 AND time >= ?2",
                params![reporter as i64, now - 3_600_000],
                |r| r.get(0),
            )?;
            if recent >= REPORTS_PER_HOUR
                || db.execute(
                    "INSERT OR IGNORE INTO storefront_reports (file_id, xuid, time) VALUES (?1, ?2, ?3)",
                    params![file, reporter as i64, now],
                )? == 0
            {
                return Ok(Report::Ignored);
            }
            let strikes: i64 = db.query_row("SELECT COUNT(*) FROM storefront_reports WHERE file_id = ?1", [file], |r| r.get(0))?;
            if strikes >= STRIKES_TO_REMOVE {
                let n = db.execute("UPDATE storefront_files SET removed = ?2 WHERE id = ?1 AND removed = 0", params![file, UNDER_REVIEW])?;
                if n > 0 {
                    return Ok(Report::Removed);
                }
            }
            Ok(Report::Counted(strikes))
        })
        .unwrap_or(Report::Ignored)
    }

    pub fn set_followed(&self, xuid: u64, creator: u64, follow: bool) {
        self.with(|db| {
            if follow {
                db.execute("INSERT OR IGNORE INTO followed_creators (xuid, creator) VALUES (?1, ?2)", params![xuid as i64, creator as i64])
            } else {
                db.execute("DELETE FROM followed_creators WHERE xuid = ?1 AND creator = ?2", params![xuid as i64, creator as i64])
            }
        });
    }

    pub fn follows(&self, xuid: u64, creator: u64) -> bool {
        self.with(|db| {
            db.query_row(
                "SELECT 1 FROM followed_creators WHERE xuid = ?1 AND creator = ?2",
                params![xuid as i64, creator as i64],
                |_| Ok(()),
            )
            .optional()
        })
        .flatten()
        .is_some()
    }

    // ------------------------------------------------------------ libraries

    /// A vinyl group the player saved (OnlineProfile.UpdateLiveryGroup), replaced by name.
    pub fn save_livery_group(&self, xuid: u64, name: &str, data: &Value) {
        self.with(|db| {
            db.execute(
                "INSERT INTO livery_groups (xuid, name, data, updated) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (xuid, name) DO UPDATE SET data = ?3, updated = ?4",
                params![xuid as i64, name, data.to_string(), unix_ms()],
            )
        });
    }

    pub fn livery_group(&self, xuid: u64, name: &str) -> Option<Value> {
        self.with(|db| {
            db.query_row("SELECT data FROM livery_groups WHERE xuid = ?1 AND name = ?2", params![xuid as i64, name], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        })
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
    }

    /// A tuning setup the player saved (OnlineProfile.UpdateCarSetup), replaced by car and name.
    pub fn save_car_setup(&self, xuid: u64, car: i64, name: &str, data: &Value) {
        self.with(|db| {
            db.execute(
                "INSERT INTO car_setups (xuid, car_id, name, data, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (xuid, car_id, name) DO UPDATE SET data = ?4, updated = ?5",
                params![xuid as i64, car, name, data.to_string(), unix_ms()],
            )
        });
    }

    pub fn car_setup(&self, xuid: u64, car: i64, name: &str) -> Option<Value> {
        self.with(|db| {
            db.query_row(
                "SELECT data FROM car_setups WHERE xuid = ?1 AND car_id = ?2 AND name = ?3",
                params![xuid as i64, car, name],
                |r| r.get::<_, String>(0),
            )
            .optional()
        })
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
    }

    /// Vinyl group library edits: rename to `new_name`, or delete (None).
    pub fn edit_livery_group(&self, xuid: u64, name: &str, new_name: Option<&str>) {
        self.with(|db| match new_name {
            Some(to) => db.execute("UPDATE livery_groups SET name = ?3 WHERE xuid = ?1 AND name = ?2", params![xuid as i64, name, to]),
            None => db.execute("DELETE FROM livery_groups WHERE xuid = ?1 AND name = ?2", params![xuid as i64, name]),
        });
    }

    /// Tuning setup library edits: rename to `new_name`, or delete (None).
    pub fn edit_car_setup(&self, xuid: u64, car: i64, name: &str, new_name: Option<&str>) {
        self.with(|db| match new_name {
            Some(to) => db.execute(
                "UPDATE car_setups SET name = ?4 WHERE xuid = ?1 AND car_id = ?2 AND name = ?3",
                params![xuid as i64, car, name, to],
            ),
            None => db.execute("DELETE FROM car_setups WHERE xuid = ?1 AND car_id = ?2 AND name = ?3", params![xuid as i64, car, name]),
        });
    }

    // ------------------------------------------------------------ versioned vinyl groups

    pub fn add_versioned_group(&self, xuid: u64, name: &str, data: &Value, thumbnail: &str) -> i64 {
        self.with(|db| {
            db.execute(
                "INSERT INTO versioned_groups (xuid, name, data, thumbnail, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, name, data.to_string(), thumbnail, unix_ms()],
            )?;
            Ok(db.last_insert_rowid())
        })
        .unwrap_or(0)
    }

    /// (owner, name, data, thumbnail)
    pub fn versioned_group(&self, id: i64) -> Option<(u64, String, Value, String)> {
        self.with(|db| {
            db.query_row("SELECT xuid, name, data, thumbnail FROM versioned_groups WHERE id = ?1", [id], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, String>(1)?,
                    serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    r.get::<_, String>(3)?,
                ))
            })
            .optional()
        })
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_strikes() {
        let path = std::env::temp_dir().join(format!("fh2-storefront-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).unwrap();
        let meta = serde_json::json!({});
        let new = |versioned| NewFile {
            kind: "livery",
            file_type: 2,
            versioned_id: versioned,
            car_id: 461,
            title: "t",
            description: "",
            keyword_one: 1,
            keyword_two: 25,
            file_name: "f",
            layers: 4,
            meta: &meta,
            preview: "",
        };
        // real players (a day-old account that has played online) and one that is not (9)
        store.with(|db| {
            for x in [2i64, 3, 4, 9] {
                let created = if x == 9 { unix_ms() } else { unix_ms() - 2 * 24 * 3600 * 1000 };
                db.execute(
                    "INSERT INTO accounts (xuid, name, discriminator, device, created) VALUES (?1, 'P', ?2, ?3, ?4)",
                    params![x, format!("{x:05}"), format!("d{x}"), created],
                )?;
                db.execute("INSERT INTO profile (xuid, data, updated) VALUES (?1, '{}', 0)", params![x])?;
            }
            Ok(())
        });
        let id = store.share_file(1, &new(6));
        assert!(id > 0);
        assert!(matches!(store.report_file(id, 9), Report::Ignored)); // a new account does not count
        assert!(matches!(store.report_file(id, 2), Report::Counted(1)));
        assert!(matches!(store.report_file(id, 2), Report::Ignored)); // once per player
        assert!(matches!(store.report_file(id, 3), Report::Counted(2)));
        assert!(matches!(store.report_file(id, 4), Report::Removed));
        assert!(store.shared_file(id).is_none()); // off the storefront, under review
        assert_eq!(store.share_file(1, &new(6)), -1); // not shared again while it waits
        let queue = store.files_under_review();
        assert_eq!((queue.len(), queue[0]["reports"].as_i64()), (1, Some(3)));
        let (detail, _) = store.file_detail(id).unwrap();
        assert_eq!(detail["state"], "under review");
        // moderation keeps it: back on the storefront, the reports go
        assert!(store.moderate_file(id, true, "fine"));
        assert!(store.shared_file(id).is_some() && store.files_under_review().is_empty());
        // or deletes it: never shared again
        assert!(store.moderate_file(id, false, "offensive"));
        assert!(store.shared_file(id).is_none());
        assert_eq!(store.file_detail(id).unwrap().0["state"], "removed by moderation");
        assert!(!store.moderate_file(99999, false, ""));
        assert_eq!(store.share_file(1, &new(6)), -1); // never shared again
        assert!(!store.edit_shared_file(1, id, "remove", "")); // and the owner cannot reset it
        assert_eq!(store.share_file(1, &new(6)), -1);
        assert!(store.share_file(1, &new(7)) > 0); // other content is fine
        let _ = std::fs::remove_file(&path);
    }
}
