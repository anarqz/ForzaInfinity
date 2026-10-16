//! A moderation reset (db/moderation.rs, `fh2-server moderation reset`): the account goes back to a
//! new profile's progress, never banned.
//!
//! - Its cloud save becomes the starting save of the starter car it chose at the beginning
//!   (server/templates/starter-<car>.bundle: past the intro and the first championship, only
//!   that car, stock) with 0 credits. The car is the first starter (289 Camaro, 461 Supra, 1218
//!   Z4) in the Career_Garage of its current save, by garage Id; 461 when there is none. Its
//!   liveries and designs stay: they are in the profile's shared folder (player-1/Livery_*), not
//!   in User_<xuid>, and the storefront's are the server's.
//! - Its records on the server go: scores (leaderboards, speed traps), ghosts, drivatar, XP,
//!   transactions, the synced profile and garage. Shared creations (liveries, designs, tunes)
//!   are not touched.
//! - The profile gates (forza/gates.rs) start again from the reset save's numbers, so bringing
//!   the old progress back (a local backup) is refused as a jump.
//! - Until the game downloads the reset save (GET save), uploads are refused (a game still
//!   running with the old progress cannot put it back); the hook shows why.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};

use super::{Store, save_bests::bundle_file, saves::SAVE_FILES};
use crate::unix_ms;

/// The game's starter cars (gamedb IEAutoShowCars), the starting saves made for them.
pub const STARTERS: [i64; 3] = [289, 461, 1218];
const DEFAULT_STARTER: i64 = 461;
const LZ_MIN_MATCH: usize = 4;
const LZ_HASH_BITS: u32 = 16;

// ------------------------------------------------------------------ the hook's LZ (method 2)

fn put_varint(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let byte = (v & 127) as u8;
        v >>= 7;
        out.push(byte | if v != 0 { 128 } else { 0 });
        if v == 0 {
            break;
        }
    }
}

fn read32(p: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]])
}

fn lz_hash(p: &[u8], i: usize) -> usize {
    (read32(p, i).wrapping_mul(2654435761) >> (32 - LZ_HASH_BITS)) as usize
}

/// hook/adapters/azimuth_codec.c's codec_lz_encode: varint literal count, the literals, then
/// varint length - 4 and varint offset - 1; no empty run after a final match.
pub fn lz_encode(input: &[u8]) -> Vec<u8> {
    let size = input.len();
    let mut out = Vec::with_capacity(size / 8 + 16);
    let mut table = vec![0u32; 1 << LZ_HASH_BITS];
    let (mut anchor, mut i) = (0usize, 0usize);
    let literals = |out: &mut Vec<u8>, from: usize, to: usize| {
        put_varint(out, to - from);
        out.extend_from_slice(&input[from..to]);
    };
    while i + LZ_MIN_MATCH <= size {
        let h = lz_hash(input, i);
        let candidate = table[h] as usize;
        table[h] = (i + 1) as u32;
        if candidate == 0 || read32(input, candidate - 1) != read32(input, i) {
            i += 1;
            continue;
        }
        let from = candidate - 1;
        let mut length = LZ_MIN_MATCH;
        while i + length < size && input[from + length] == input[i + length] {
            length += 1;
        }
        literals(&mut out, anchor, i);
        put_varint(&mut out, length - LZ_MIN_MATCH);
        put_varint(&mut out, i - from - 1);
        let end = i + length;
        let mut p = i + 1;
        while p + LZ_MIN_MATCH <= size && p < end && p < i + 16 {
            table[lz_hash(input, p)] = (p + 1) as u32;
            p += 1;
        }
        if end - 1 > i + 16 && end - 1 + LZ_MIN_MATCH <= size {
            table[lz_hash(input, end - 1)] = end as u32;
        }
        i = end;
        anchor = end;
    }
    if anchor < size {
        literals(&mut out, anchor, size);
    }
    out
}

// --------------------------------------------------------------------------- bundles

/// Every file of a bundle, unpacked, in SAVE_FILES order.
pub fn unpack(bundle: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let files: Vec<(String, Vec<u8>)> =
        SAVE_FILES.iter().filter_map(|n| bundle_file(bundle, n).map(|d| (n.to_string(), d))).collect();
    files.iter().any(|(n, _)| n == "commit").then_some(files)
}

/// An FH2SAVE1 bundle: method 2 where it is smaller, else stored.
pub fn pack(revision: u64, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"FH2SAVE1".to_vec();
    out.extend_from_slice(&revision.to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data) in files {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        let packed = if data.is_empty() { Vec::new() } else { lz_encode(data) };
        let (method, body) = if !packed.is_empty() && packed.len() < data.len() { (2u8, &packed) } else { (0u8, data) };
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.push(method);
        out.extend_from_slice(body);
    }
    out
}

fn profile_marker(name: &str) -> Vec<u8> {
    let mut m = name.as_bytes().to_vec();
    m.extend_from_slice(b" \0\0\0\0\x03\0\0\0");
    m
}

fn find_once(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let mut found = hay.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// An int of ProfileData ("<name> \0\0\0\0\x03\0\0\0" then an i32): it must be there once.
pub fn profile_int(profile: &[u8], name: &str) -> Option<i32> {
    let m = profile_marker(name);
    let at = find_once(profile, &m)? + m.len();
    Some(i32::from_le_bytes(profile.get(at..at + 4)?.try_into().ok()?))
}

pub fn set_profile_int(profile: &[u8], name: &str, value: i32) -> Option<Vec<u8>> {
    let m = profile_marker(name);
    let at = find_once(profile, &m)? + m.len();
    profile.get(at..at + 4)?;
    let mut out = profile.to_vec();
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    Some(out)
}

/// The commit with each listed file's "sha256 size name" line made for `files` (data/<name>).
pub fn commit_with(commit: &[u8], files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let text = String::from_utf8_lossy(commit);
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() == 3
                && let Some((_, data)) = files.iter().find(|(n, _)| n.strip_prefix("data/") == Some(parts[2]))
            {
                return format!("{} {} {}", crate::sha256::hex(data), data.len(), parts[2]);
            }
            line.to_owned()
        })
        .collect();
    lines.join("\n").into_bytes()
}

/// The first starter car (by garage Id) in a save's PlayerDatabase ("ssmc", u32 size, SQLite).
pub fn first_starter(player_db: &[u8]) -> Option<i64> {
    let size = u32::from_le_bytes(player_db.get(4..8)?.try_into().ok()?) as usize;
    let sqlite = player_db.get(8..8 + size)?;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("fh2-reset-{}-{}-{n}.sqlite", std::process::id(), unix_ms()));
    std::fs::write(&path, sqlite).ok()?;
    let cars: Option<Vec<i64>> = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok().and_then(|db| {
        let mut q = db.prepare("SELECT CarId FROM Career_Garage ORDER BY Id").ok()?;
        let rows = q.query_map([], |r| r.get::<_, i64>(0)).ok()?;
        rows.collect::<rusqlite::Result<Vec<i64>>>().ok()
    });
    let _ = std::fs::remove_file(&path);
    cars?.into_iter().find(|c| STARTERS.contains(c))
}

/// A SaveState with every camera's PersonalBest at 0: the starting save's own speed trap and
/// zone bests (its author's) must not become the reset player's (db/save_bests.rs imports them).
pub fn without_camera_bests(state: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(state);
    let key = "Name=\"PersonalBest\" Value=\"";
    let mut out = String::with_capacity(text.len());
    let mut rest: &str = &text;
    while let Some(at) = rest.find(key) {
        out.push_str(&rest[..at + key.len()]);
        rest = &rest[at + key.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        out.push('0');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// The reset save: the starting save of `car` with 0 credits, as revision `revision`.
pub struct ResetSave {
    pub bundle: Vec<u8>,
    pub commit: String,
    pub car: i64,
    pub level: i64,
}

pub fn reset_save(templates: &Path, car: i64, revision: u64) -> Result<ResetSave, String> {
    let car = if STARTERS.contains(&car) { car } else { DEFAULT_STARTER };
    let path = templates.join(format!("starter-{car}.bundle"));
    let template = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut files = unpack(&template).ok_or("the starting save cannot be read")?;
    let profile = files.iter_mut().find(|(n, _)| n == "data/ProfileData").ok_or("no ProfileData in the starting save")?;
    profile.1 = set_profile_int(&profile.1, "Credits", 0).ok_or("no Credits in the starting save's ProfileData")?;
    let level = profile_int(&profile.1, "Level").unwrap_or(1) as i64;
    if let Some(state) = files.iter_mut().find(|(n, _)| n == "data/SaveState") {
        state.1 = without_camera_bests(&state.1);
    }
    let data: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != "commit").cloned().collect();
    let commit_file = files.iter().find(|(n, _)| n == "commit").map(|(_, c)| c.clone()).unwrap_or_default();
    let commit = commit_with(&commit_file, &data);
    if let Some(c) = files.iter_mut().find(|(n, _)| n == "commit") {
        c.1 = commit.clone();
    }
    let bundle = pack(revision, &files);
    super::saves::check_bundle(&bundle)?;
    Ok(ResetSave { bundle, commit: crate::sha256::hex(&commit), car, level })
}

impl Store {
    /// Resets `xuid` (see above). Returns what was done, as JSON.
    pub fn moderation_reset(&self, xuid: u64, templates: &Path, note: &str) -> Result<Value, String> {
        let current = self.save_data(xuid);
        let car = current
            .as_deref()
            .and_then(|b| bundle_file(b, "data/PlayerDatabase"))
            .and_then(|db| first_starter(&db))
            .unwrap_or(DEFAULT_STARTER);
        let revision = self.save_info(xuid).map(|s| s.revision + 1).unwrap_or(1);
        let save = reset_save(templates, car, revision)?;
        let now = unix_ms();
        let x = xuid as i64;
        let mut db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let exists: i64 = tx.query_row("SELECT COUNT(*) FROM accounts WHERE xuid = ?1", params![x], |r| r.get(0)).map_err(|e| e.to_string())?;
        if exists == 0 {
            return Err("there is no such account".into());
        }
        let mut removed = serde_json::Map::new();
        for table in ["scores", "ghosts", "drivatars", "xp", "transactions", "profile", "profile_tables"] {
            let n = tx.execute(&format!("DELETE FROM {table} WHERE xuid = ?1"), params![x]).map_err(|e| e.to_string())?;
            removed.insert(table.to_owned(), Value::from(n));
        }
        tx.execute(
            "INSERT INTO saves (xuid, revision, commit_hash, data, created) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![x, revision as i64, save.commit, save.bundle, now],
        )
        .map_err(|e| e.to_string())?;
        // the gates start from the reset save's numbers
        tx.execute(
            "INSERT INTO integrity (xuid, credits, xp, level, winnings, synced, cars, tables_synced)
             VALUES (?1, 0, 0, ?2, 0, ?3, 1, ?3)
             ON CONFLICT (xuid) DO UPDATE SET credits = 0, xp = 0, level = ?2, winnings = 0, synced = ?3, cars = 1, tables_synced = ?3",
            params![x, save.level, now],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE accounts SET flagged = 0, flagged_at = NULL, reset_pending = 1 WHERE xuid = ?1", params![x])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM player_reports WHERE reported = ?1", params![x]).map_err(|e| e.to_string())?;
        let note: String = note.chars().filter(|c| !c.is_control()).take(200).collect();
        tx.execute(
            "INSERT INTO moderation_log (xuid, action, note, created) VALUES (?1, 'reset', ?2, ?3)",
            params![x, format!("starter car {}; {note}", save.car), now],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        self.add_event(xuid, "moderation", "Progresso zerado pela moderação", json!({ "car": save.car }));
        Ok(json!({ "xuid": xuid.to_string(), "starterCar": save.car, "saveRevision": revision, "commit": save.commit, "removed": removed }))
    }

    /// The account's progress was reset and the game has not downloaded it yet.
    pub fn reset_pending(&self, xuid: u64) -> bool {
        self.with(|db| db.query_row("SELECT reset_pending FROM accounts WHERE xuid = ?1", params![xuid as i64], |r| r.get::<_, i64>(0)))
            .is_some_and(|p| p != 0)
    }

    pub fn reset_downloaded(&self, xuid: u64) {
        self.with(|db| db.execute("UPDATE accounts SET reset_pending = 0 WHERE xuid = ?1 AND reset_pending = 1", params![xuid as i64]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lz_decode(input: &[u8], size: usize) -> Vec<u8> {
        // the server's own decoder (save_bests.rs) through a one-file bundle
        let files = vec![("commit".to_string(), b"c".to_vec()), ("data/ProfileData".to_string(), input.to_vec())];
        let _ = files;
        let mut b = b"FH2SAVE1".to_vec();
        b.extend_from_slice(&1u64.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        let name = b"data/ProfileData";
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(name);
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&(input.len() as u32).to_le_bytes());
        b.push(2);
        b.extend_from_slice(input);
        bundle_file(&b, "data/ProfileData").unwrap()
    }

    #[test]
    fn lz_round_trips() {
        let mut big = vec![0u8; 300_000];
        for (i, b) in big.iter_mut().enumerate().take(5000) {
            *b = (i * 7 % 251) as u8;
        }
        big.extend_from_slice(b"Credits \0\0\0\0\x03\0\0\0\x10\x27\0\0 tail");
        for data in [b"".to_vec(), b"abc".to_vec(), b"aaaaaaaaaaaaaaaaaaaaaaaa".to_vec(), big.clone()] {
            if data.is_empty() {
                continue;
            }
            let e = lz_encode(&data);
            assert_eq!(lz_decode(&e, data.len()), data);
        }
        assert!(lz_encode(&big).len() < big.len() / 10);
    }

    #[test]
    fn profile_ints_and_commit() {
        let p = b"xx Credits \0\0\0\0\x03\0\0\0\x50\xc3\0\0 Level \0\0\0\0\x03\0\0\0\x06\0\0\0".to_vec();
        assert_eq!(profile_int(&p, "Credits"), Some(50000));
        let q = set_profile_int(&p, "Credits", 0).unwrap();
        assert_eq!(profile_int(&q, "Credits"), Some(0));
        assert_eq!(profile_int(&q, "Level"), Some(6));
        assert_eq!(profile_int(b"none", "Credits"), None);
        let twice = [p.clone(), p.clone()].concat();
        assert_eq!(profile_int(&twice, "Credits"), None);
        let commit = b"Azimuth complete save revision v2\nblobs 2\naaaa 3 ProfileData\nbbbb 1 VersionFlags\n";
        let c = commit_with(commit, &[("data/ProfileData".into(), b"xyz".to_vec())]);
        let c = String::from_utf8(c).unwrap();
        assert!(c.contains(&format!("{} 3 ProfileData", crate::sha256::hex(b"xyz"))));
        assert!(c.contains("bbbb 1 VersionFlags"));
    }

    /// The real starting saves (server/templates): each unpacks, its reset has 0 credits, a
    /// commit matching its files, fits a download, and keeps its starter car.
    #[test]
    fn starting_saves_reset() {
        let templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for car in STARTERS {
            let r = reset_save(&templates, car, 9).unwrap();
            assert_eq!(r.car, car);
            assert!(r.bundle.len() <= super::super::saves::MAX_BUNDLE, "{}", r.bundle.len());
            let files = unpack(&r.bundle).unwrap();
            let get = |n: &str| files.iter().find(|(k, _)| k == n).map(|(_, d)| d.clone()).unwrap();
            assert_eq!(profile_int(&get("data/ProfileData"), "Credits"), Some(0));
            assert_eq!(first_starter(&get("data/PlayerDatabase")), Some(car));
            let state = String::from_utf8(get("data/SaveState")).unwrap();
            assert!(crate::db::save_bests::camera_bests(&state).is_empty());
            let commit = String::from_utf8(get("commit")).unwrap();
            for (n, d) in files.iter().filter(|(n, _)| n != "commit") {
                let line = format!("{} {} {}", crate::sha256::hex(d), d.len(), n.trim_start_matches("data/"));
                assert!(commit.contains(&line), "{line}");
            }
            assert_eq!(crate::sha256::hex(&get("commit")), r.commit);
        }
        // anything else: the Supra
        assert_eq!(reset_save(&templates, 9999, 1).unwrap().car, DEFAULT_STARTER);
    }
}
