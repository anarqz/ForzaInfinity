//! Cloud saves: the newest Azimuth save revision of each account, as the hook bundles it
//! (azimuth_cloud.c). The server does not read the game data; it checks the bundle's shape
//! and keeps the last few uploads, so a bad one can be rolled back by hand.
//!
//! Bundle (little endian): "FH2SAVE1", u64 revision, u32 count, then per file
//! u16 name length, name (one of SAVE_FILES), u32 raw size, u32 stored size, u8 method
//! (0 stored, 1 XPRESS Huffman), stored bytes.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS saves (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, revision INTEGER NOT NULL,
        commit_hash TEXT NOT NULL, data BLOB NOT NULL, created INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS saves_xuid ON saves (xuid, id);";

/// The files of an Azimuth save revision, the only names a bundle may hold.
pub const SAVE_FILES: &[&str] = &[
    "commit",
    "data/BinarySaveState",
    "data/PlayerDatabase",
    "data/ProfileData",
    "data/SaveState",
    "data/TransactionLogFile",
    "data/VersionFlags",
];
/// Uploads kept per account (the newest is the save).
const KEEP: i64 = 3;
/// Largest file inside a save once unpacked (PlayerDatabase is 6 MB).
const MAX_FILE: u32 = 32 << 20;

pub struct SaveInfo {
    pub revision: u64,
    pub commit: String,
    pub bytes: usize,
    pub created: i64,
}

/// Checks a bundle's layout; returns its revision.
pub fn check_bundle(data: &[u8]) -> Result<u64, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let end = at.checked_add(n).filter(|&e| e <= data.len()).ok_or("truncated bundle")?;
        let slice = &data[at..end];
        at = end;
        Ok(slice)
    };
    if take(8)? != b"FH2SAVE1" {
        return Err("unknown save format".into());
    }
    let revision = u64::from_le_bytes(take(8)?.try_into().unwrap());
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap());
    if count == 0 || count as usize > SAVE_FILES.len() {
        return Err("invalid file count".into());
    }
    let mut seen = Vec::new();
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let name = std::str::from_utf8(take(name_len)?).map_err(|_| "invalid name")?;
        if !SAVE_FILES.contains(&name) || seen.contains(&name.to_owned()) {
            return Err(format!("unexpected file in the save: {}", name.chars().take(40).collect::<String>()));
        }
        seen.push(name.to_owned());
        let raw = u32::from_le_bytes(take(4)?.try_into().unwrap());
        let stored = u32::from_le_bytes(take(4)?.try_into().unwrap());
        let method = take(1)?[0];
        if raw > MAX_FILE || method > 1 || (method == 0 && stored != raw) {
            return Err("invalid save file".into());
        }
        take(stored as usize)?;
    }
    if !seen.iter().any(|n| n == "commit") {
        return Err("save without a commit".into());
    }
    if at != data.len() {
        return Err("trailing data in the bundle".into());
    }
    Ok(revision)
}

pub fn valid_commit(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Store {
    pub fn save_info(&self, xuid: u64) -> Option<SaveInfo> {
        self.with(|db| {
            db.query_row(
                "SELECT revision, commit_hash, length(data), created FROM saves WHERE xuid = ?1 ORDER BY id DESC LIMIT 1",
                [xuid as i64],
                |r| {
                    Ok(SaveInfo {
                        revision: r.get::<_, i64>(0)? as u64,
                        commit: r.get(1)?,
                        bytes: r.get::<_, i64>(2)? as usize,
                        created: r.get(3)?,
                    })
                },
            )
            .optional()
        })
        .flatten()
    }

    pub fn save_data(&self, xuid: u64) -> Option<Vec<u8>> {
        self.with(|db| {
            db.query_row("SELECT data FROM saves WHERE xuid = ?1 ORDER BY id DESC LIMIT 1", [xuid as i64], |r| r.get(0))
                .optional()
        })
        .flatten()
    }

    /// Stores a checked bundle as the account's save. The same commit twice is a no-op.
    pub fn put_save(&self, xuid: u64, revision: u64, commit: &str, data: &[u8]) -> bool {
        if self.save_info(xuid).is_some_and(|s| s.commit.eq_ignore_ascii_case(commit)) {
            return true;
        }
        self.with(|db| {
            db.execute(
                "INSERT INTO saves (xuid, revision, commit_hash, data, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, revision as i64, commit.to_ascii_lowercase(), data, unix_ms()],
            )?;
            db.execute(
                "DELETE FROM saves WHERE xuid = ?1 AND id NOT IN (SELECT id FROM saves WHERE xuid = ?1 ORDER BY id DESC LIMIT ?2)",
                params![xuid as i64, KEEP],
            )
        })
        .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"FH2SAVE1".to_vec();
        out.extend_from_slice(&7u64.to_le_bytes());
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        for (name, data) in files {
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.push(0);
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn bundles() {
        assert_eq!(check_bundle(&bundle(&[("commit", b"x"), ("data/SaveState", b"abc")])), Ok(7));
        assert!(check_bundle(&bundle(&[("data/SaveState", b"abc")])).is_err());
        assert!(check_bundle(&bundle(&[("commit", b"x"), ("../../evil", b"abc")])).is_err());
        let mut long = bundle(&[("commit", b"x")]);
        long.push(0);
        assert!(check_bundle(&long).is_err());
        assert!(check_bundle(b"FH2SAVE1").is_err());
    }
}
