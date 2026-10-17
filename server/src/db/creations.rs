//! Cloud creations: the player's own files the game keeps beside the profile's save, one Azimuth
//! container each under <saves>\player-1 (Livery_*, BaseLivery_*, GarageThumbnail_*,
//! LayerGroup_*, tunes: designs, vinyl groups and tuning setups), so they follow the profile
//! to any PC. The hook bundles each container's newest revision (azimuth_cloud.c, "FH2FILE1");
//! the server only checks its shape and keeps it.
//!
//! Bundle (little endian): "FH2FILE1", u32 count, then per file u16 name length, name
//! ("commit" or "data/<name>"), u32 raw size, u32 stored size, u8 method (0 stored, 2 the
//! hook's LZ), stored bytes.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS creations (
        xuid INTEGER NOT NULL, name TEXT NOT NULL, commit_hash TEXT NOT NULL, data BLOB NOT NULL,
        updated INTEGER NOT NULL, PRIMARY KEY (xuid, name));";

/// Largest bundle: a livery is about 180 KB (two PNG thumbnails that do not compress).
pub const MAX_BUNDLE: usize = 1 << 20;
/// Most containers an account keeps, and their bytes together.
pub const MAX_COUNT: usize = 300;
pub const MAX_TOTAL: i64 = 96 << 20;
const MAX_FILES: u32 = 16;

/// A container's folder name: "<Kind>_<id>", letters, digits and underscores, never the
/// profile's own save (User_*).
pub fn valid_name(name: &str) -> bool {
    let Some((kind, id)) = name.split_once('_') else {
        return false;
    };
    (1..=24).contains(&kind.len())
        && kind.bytes().all(|b| b.is_ascii_alphabetic())
        && (1..=48).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !kind.eq_ignore_ascii_case("User")
}

/// A bundle's shape: the magic, at most MAX_FILES entries with sane names and sizes, a commit,
/// and nothing after the last entry.
pub fn check_bundle(data: &[u8]) -> Result<(), String> {
    if data.len() < 12 || &data[..8] != b"FH2FILE1" {
        return Err("not a creation bundle".into());
    }
    let count = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if count == 0 || count > MAX_FILES {
        return Err(format!("{count} files"));
    }
    let mut at = 12;
    let mut commit = false;
    for _ in 0..count {
        let len = *data.get(at).ok_or("truncated")? as usize | (*data.get(at + 1).ok_or("truncated")? as usize) << 8;
        at += 2;
        let name = data.get(at..at + len).ok_or("truncated")?;
        let name = std::str::from_utf8(name).map_err(|_| "file name")?;
        let ok = name == "commit"
            || name.strip_prefix("data/").is_some_and(|n| {
                (1..=48).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            });
        if !ok {
            return Err(format!("file name {name:?}"));
        }
        commit |= name == "commit";
        at += len;
        let head = data.get(at..at + 9).ok_or("truncated")?;
        let raw = u32::from_le_bytes(head[0..4].try_into().unwrap()) as usize;
        let stored = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        let method = head[8];
        at += 9;
        if raw > MAX_BUNDLE * 4 || (method != 0 && method != 2) || (method == 0 && stored != raw) {
            return Err(format!("{name}: sizes"));
        }
        data.get(at..at + stored).ok_or("truncated")?;
        at += stored;
    }
    if at != data.len() {
        return Err("trailing bytes".into());
    }
    if !commit {
        return Err("no commit".into());
    }
    Ok(())
}

pub enum PutError {
    TooMany,
    TooBig,
}

impl Store {
    /// (name, commit) of every creation the account keeps, by name.
    pub fn creations(&self, xuid: u64) -> Vec<(String, String)> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT name, commit_hash FROM creations WHERE xuid = ?1 ORDER BY name")?;
            let found = stmt.query_map([xuid as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect::<Vec<_>>();
            Ok(found)
        })
        .unwrap_or_default()
    }

    pub fn creation(&self, xuid: u64, name: &str) -> Option<Vec<u8>> {
        self.with(|db| {
            db.query_row("SELECT data FROM creations WHERE xuid = ?1 AND name = ?2", params![xuid as i64, name], |r| r.get(0))
                .optional()
        })
        .flatten()
    }

    /// Stores (or replaces) a creation, within the account's MAX_COUNT and MAX_TOTAL.
    pub fn put_creation(&self, xuid: u64, name: &str, commit: &str, data: &[u8]) -> Result<(), PutError> {
        let (count, total, mine): (i64, i64, i64) = self
            .with(|db| {
                db.query_row(
                    "SELECT count(*), coalesce(sum(length(data)), 0),
                            coalesce(sum(CASE WHEN name = ?2 THEN length(data) ELSE 0 END), 0)
                     FROM creations WHERE xuid = ?1",
                    params![xuid as i64, name],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .unwrap_or((0, 0, 0));
        let exists = self.creation(xuid, name).is_some();
        if !exists && count as usize >= MAX_COUNT {
            return Err(PutError::TooMany);
        }
        if total - mine + data.len() as i64 > MAX_TOTAL {
            return Err(PutError::TooBig);
        }
        let _ = self.with(|db| {
            db.execute(
                "INSERT INTO creations (xuid, name, commit_hash, data, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (xuid, name) DO UPDATE SET commit_hash = ?3, data = ?4, updated = ?5",
                params![xuid as i64, name, commit, data, unix_ms()],
            )
        });
        Ok(())
    }

    pub fn delete_creation(&self, xuid: u64, name: &str) -> bool {
        self.with(|db| db.execute("DELETE FROM creations WHERE xuid = ?1 AND name = ?2", params![xuid as i64, name]))
            .map(|n| n > 0)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = b"FH2FILE1".to_vec();
        b.extend((files.len() as u32).to_le_bytes());
        for (name, data) in files {
            b.extend((name.len() as u16).to_le_bytes());
            b.extend(name.as_bytes());
            b.extend((data.len() as u32).to_le_bytes());
            b.extend((data.len() as u32).to_le_bytes());
            b.push(0);
            b.extend(*data);
        }
        b
    }

    #[test]
    fn names_and_bundles() {
        assert!(valid_name("Livery_0461_20261014184334"));
        assert!(valid_name("GarageThumbnail_1124_20261016001541"));
        assert!(!valid_name("User_90191AE817005"));
        assert!(!valid_name("Livery_../x"));
        assert!(!valid_name("Livery"));
        let good = bundle(&[("commit", b"Azimuth complete save revision v2\n"), ("data/C_livery", b"x"), ("data/bigThumb.png", b"png")]);
        assert!(check_bundle(&good).is_ok());
        assert!(check_bundle(&bundle(&[("data/C_livery", b"x")])).is_err());
        assert!(check_bundle(&bundle(&[("commit", b"c"), ("data/../x", b"x")])).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(check_bundle(&long).is_err());
    }

    #[test]
    fn kept_per_account() {
        let dir = std::env::temp_dir().join(format!("fh2-creations-{}", crate::db::accounts::new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        assert!(store.put_creation(1, "Livery_1", &"a".repeat(64), b"one").is_ok());
        assert!(store.put_creation(1, "Livery_1", &"b".repeat(64), b"two").is_ok());
        assert!(store.put_creation(2, "Tune_1", &"c".repeat(64), b"three").is_ok());
        assert_eq!(store.creations(1), vec![("Livery_1".to_string(), "b".repeat(64))]);
        assert_eq!(store.creation(1, "Livery_1").unwrap(), b"two");
        assert!(store.creation(1, "Tune_1").is_none());
        assert!(store.delete_creation(1, "Livery_1"));
        assert!(store.creations(1).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
