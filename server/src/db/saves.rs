//! Cloud saves: the newest Azimuth save revision of each account, as the hook bundles it
//! (azimuth_cloud.c). The server does not read the game data; it checks the bundle's shape
//! and keeps the last few uploads, so a bad one can be rolled back by hand.
//!
//! Bundle (little endian): "FH2SAVE1", u64 revision, u32 count, then per file
//! u16 name length, name (one of SAVE_FILES), u32 raw size, u32 stored size, u8 method
//! (0 stored, 1 XPRESS Huffman from older hooks on Windows, 2 the hook's LZ: azimuth_codec.c),
//! stored bytes.

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
/// Largest bundle taken: a real save is about 28 KB (6.5 MB unpacked, mostly zeros).
pub const MAX_BUNDLE: usize = 384 << 10;
/// (name, smallest, largest) unpacked size of each file, from real saves: PlayerDatabase,
/// ProfileData and VersionFlags always have the same size; the others stay well inside these (an
/// early game's BinarySaveState is about 4.5 KB, a later one about 113 KB).
pub const FILE_SIZES: &[(&str, u32, u32)] = &[
    ("commit", 256, 2048),
    ("data/BinarySaveState", 1 << 10, 256 << 10),
    ("data/PlayerDatabase", 6_291_464, 6_291_464),
    ("data/ProfileData", 20_488, 20_488),
    ("data/SaveState", 4 << 10, 256 << 10),
    ("data/TransactionLogFile", 0, 64 << 10),
    ("data/VersionFlags", 4, 4),
];
/// Files every save has.
const REQUIRED: &[&str] = &["commit", "data/PlayerDatabase", "data/ProfileData", "data/SaveState"];
/// Largest file inside a save once unpacked (PlayerDatabase is 6 291 464 bytes, always).
const MAX_FILE: u32 = 8 << 20;
/// Largest save once unpacked, all files together.
const MAX_UNPACKED: u64 = 16 << 20;

pub struct SaveInfo {
    pub revision: u64,
    pub commit: String,
    pub bytes: usize,
    pub created: i64,
}

/// Checks a bundle's layout; returns its revision.
pub fn check_bundle(data: &[u8]) -> Result<u64, String> {
    if data.len() > MAX_BUNDLE {
        return Err("save too big".into());
    }
    let mut unpacked = 0u64;
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
        // a packed file never takes more room than it unpacks to (the hook stores it as is then)
        if raw > MAX_FILE || method > 2 || (method == 0 && stored != raw) || stored > raw {
            return Err("invalid save file".into());
        }
        // each file's own size, as real saves have it
        if let Some((_, min, max)) = FILE_SIZES.iter().find(|(n, _, _)| *n == name)
            && (raw < *min || raw > *max)
        {
            return Err(format!("{name}: {raw} bytes is not a real save's size"));
        }
        unpacked += raw as u64;
        if unpacked > MAX_UNPACKED {
            return Err("save too big".into());
        }
        take(stored as usize)?;
    }
    if !seen.iter().any(|n| n == "commit") {
        return Err("save without a commit".into());
    }
    if let Some(missing) = REQUIRED.iter().find(|r| !seen.iter().any(|n| n == *r)) {
        return Err(format!("save without {missing}"));
    }
    if at != data.len() {
        return Err("trailing data in the bundle".into());
    }
    Ok(revision)
}

/// What check_bundle cannot see without unpacking: each file unpacks to its declared size, the
/// commit lists every file with its SHA-256 and size, PlayerDatabase is an ssmc container holding a
/// SQLite database, SaveState is XML.
pub fn check_contents(data: &[u8]) -> Result<(), String> {
    let files = super::reset::unpack(data).ok_or("the save does not unpack")?;
    let commit = files.iter().find(|(n, _)| n == "commit").map(|(_, c)| String::from_utf8_lossy(c).into_owned()).unwrap_or_default();
    for (name, body) in files.iter().filter(|(n, _)| n != "commit") {
        let short = name.trim_start_matches("data/");
        let line = format!("{} {} {short}", crate::sha256::hex(body), body.len());
        if !commit.lines().any(|l| l == line) {
            return Err(format!("{short} does not match the save's commit"));
        }
    }
    let get = |n: &str| files.iter().find(|(k, _)| k == n).map(|(_, d)| d.as_slice());
    let db = get("data/PlayerDatabase").unwrap_or(&[]);
    let inner = u32::from_le_bytes(db.get(4..8).and_then(|b| b.try_into().ok()).unwrap_or([0; 4])) as usize;
    if db.get(..4) != Some(b"ssmc") || inner + 8 > db.len() || db.get(8..24) != Some(b"SQLite format 3\0") {
        return Err("PlayerDatabase is not the game's database".into());
    }
    let state = get("data/SaveState").unwrap_or(&[]);
    if !state.iter().skip_while(|b| b.is_ascii_whitespace() || **b == 0xef || **b == 0xbb || **b == 0xbf).take(1).any(|b| *b == b'<') {
        return Err("SaveState is not XML".into());
    }
    Ok(())
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
        // a real save: every check passes (the fixtures are real saves)
        let real = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/starter-461.bundle")).unwrap();
        assert_eq!(check_bundle(&real), Ok(1));
        assert_eq!(check_contents(&real), Ok(()));
        assert!(check_bundle(&bundle(&[("data/SaveState", b"abc")])).is_err());
        assert!(check_bundle(&bundle(&[("commit", b"x"), ("../../evil", b"abc")])).is_err());
        let mut long = real.clone();
        long.push(0);
        assert!(check_bundle(&long).is_err());
        assert!(check_bundle(b"FH2SAVE1").is_err());
        // sizes no real save has
        let files = crate::db::reset::unpack(&real).unwrap();
        let with = |name: &str, data: Vec<u8>| {
            let f: Vec<(String, Vec<u8>)> = files.iter().map(|(n, d)| (n.clone(), if n == name { data.clone() } else { d.clone() })).collect();
            crate::db::reset::pack(1, &f)
        };
        assert!(check_bundle(&with("data/ProfileData", vec![1; 30_000])).is_err());
        assert!(check_bundle(&with("data/SaveState", vec![b'<'; 300 << 10])).is_err());
        assert!(check_bundle(&with("data/PlayerDatabase", vec![0; 6_291_465])).is_err());
        assert!(check_bundle(&with("commit", vec![b'x'; 10])).is_err());
        // without a file every save has
        let no_profile: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != "data/ProfileData").cloned().collect();
        assert!(check_bundle(&crate::db::reset::pack(1, &no_profile)).unwrap_err().contains("ProfileData"));
        // right sizes, wrong contents: the commit does not match, or the database is not one
        let mut profile = files.iter().find(|(n, _)| n == "data/ProfileData").unwrap().1.clone();
        profile[100] ^= 1;
        let edited = with("data/ProfileData", profile);
        assert!(check_bundle(&edited).is_ok());
        assert!(check_contents(&edited).unwrap_err().contains("ProfileData"));
        let mut db = files.iter().find(|(n, _)| n == "data/PlayerDatabase").unwrap().1.clone();
        db[8] = b'X';
        let f: Vec<(String, Vec<u8>)> = files.iter().map(|(n, d)| (n.clone(), if n == "data/PlayerDatabase" { db.clone() } else { d.clone() })).collect();
        let others: Vec<(String, Vec<u8>)> = f.iter().filter(|(n, _)| n != "commit").cloned().collect();
        let commit = crate::db::reset::commit_with(&files.iter().find(|(n, _)| n == "commit").unwrap().1, &others);
        let f: Vec<(String, Vec<u8>)> = f.into_iter().map(|(n, d)| if n == "commit" { (n, commit.clone()) } else { (n, d) }).collect();
        assert!(check_contents(&crate::db::reset::pack(1, &f)).unwrap_err().contains("PlayerDatabase"));
    }

    /// A bundle whose files declare (raw size, stored bytes, method).
    fn declared(files: &[(&str, u32, &[u8], u8)]) -> Vec<u8> {
        let mut out = b"FH2SAVE1".to_vec();
        out.extend_from_slice(&7u64.to_le_bytes());
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        for (name, raw, stored, method) in files {
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&raw.to_le_bytes());
            out.extend_from_slice(&(stored.len() as u32).to_le_bytes());
            out.push(*method);
            out.extend_from_slice(stored);
        }
        out
    }

    #[test]
    fn size_limits() {
        // a real save's sizes: 6 MB database packed small
        let real = [
            ("commit", 548, &[0u8; 64][..], 2),
            ("data/PlayerDatabase", 6_291_464, &[0u8; 64][..], 2),
            ("data/ProfileData", 20_488, &[0u8; 64][..], 2),
            ("data/SaveState", 70_000, &[0u8; 64][..], 2),
        ];
        assert!(check_bundle(&declared(&real)).is_ok());
        // too big to take at all
        assert!(check_bundle(&vec![0u8; MAX_BUNDLE + 1]).is_err());
        // one file declaring more than MAX_FILE unpacked
        assert!(check_bundle(&declared(&[("commit", 1, b"x", 0), ("data/PlayerDatabase", 9 << 20, &[0; 64], 2)])).is_err());
        // all files together declaring more than MAX_UNPACKED
        let big = [
            ("commit", 1, &b"x"[..], 0),
            ("data/PlayerDatabase", 8 << 20, &[0u8; 64][..], 2),
            ("data/SaveState", 8 << 20, &[0u8; 64][..], 2),
            ("data/ProfileData", 1 << 20, &[0u8; 64][..], 2),
        ];
        assert!(check_bundle(&declared(&big)).is_err());
        // "packed" bytes larger than what they unpack to
        assert!(check_bundle(&declared(&[("commit", 1, b"x", 0), ("data/SaveState", 10, &[0; 64], 2)])).is_err());
    }
}
