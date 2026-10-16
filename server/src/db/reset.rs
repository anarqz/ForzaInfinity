//! Progress resets: a season reset (every account, `fh2-server moderation season-reset`; and every
//! new profile's first save), and a moderation reset (`fh2-server moderation reset`, a cheater).
//! Nobody is banned.
//!
//! The player's own cloud save is edited, so its career stays: the story, championships, events,
//! unlocks, tutorials and introductions, roads, barn finds, bucket list. What starts again:
//! - XP and level (ProfileData XP, the XP the game syncs and shows the level from, its copy in
//!   WristbandLevel, and Level), level rewards, skill points and perks (PerkUnlock), credits;
//! - the garage: only the first starter car the player got (by garage Id: 289 Camaro, 461 Supra,
//!   1218 Z4; else the oldest car), with its own parts; the career car and the last used car
//!   become it;
//! - records: each track's best lap and top speed (Career_Track_Stats), speed trap and zone
//!   bests (SaveState), and on the server the scores (leaderboards), ghosts, XP and the synced
//!   profile and garage;
//! - badges and titles beyond the 51 every profile starts with.
//! Liveries, designs and shared files stay (the profile's Livery_* folders and the storefront are
//! not in the save). A moderation reset also deletes the account's drivatar.
//!
//! The profile gates (forza/gates.rs) start again from the reset's numbers, so bringing the old
//! progress back (a local backup) is refused as a jump. Until the game downloads the reset save
//! (GET save), uploads are refused, and the hook says why at its next start.

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::{Store, save_bests::bundle_file, saves::SAVE_FILES};
use crate::unix_ms;

/// The game's starter cars (gamedb IEAutoShowCars).
pub const STARTERS: [i64; 3] = [289, 461, 1218];
/// The badges and titles a new profile has (a fresh save after the intro), kept by a reset.
pub const DEFAULT_BADGES: std::ops::RangeInclusive<i64> = 374..=421;
pub const DEFAULT_BADGES_EXTRA: [i64; 3] = [326, 335, 353];
/// PlayerDatabase: "ssmc", u32 SQLite size, the SQLite file, zeros up to this size.
const CONTAINER_SIZE: usize = 6_291_464;
const LZ_MIN_MATCH: usize = 4;
const LZ_HASH_BITS: u32 = 16;

/// accounts.reset_pending: why the game has a reset save to download.
pub const PENDING_MODERATION: i64 = 1;
pub const PENDING_SEASON: i64 = 2;
/// A new profile's first save (no notice: it is how every profile starts).
pub const PENDING_NEW: i64 = 3;
/// An upload refused by the save gates (check_upload): the server's save wins again.
pub const PENDING_REFUSED: i64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetKind {
    Moderation,
    Season,
}

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

/// ProfileData names an int as u32 length, the name, " \0\0\0\0\x03\0\0\0", then an i32 (the
/// length keeps "Level" from matching "WristbandLevel").
fn profile_marker(name: &str) -> Vec<u8> {
    let mut m = (name.len() as u32).to_le_bytes().to_vec();
    m.extend_from_slice(name.as_bytes());
    m.extend_from_slice(b" \0\0\0\0\x03\0\0\0");
    m
}

fn find_once(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let mut found = hay.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// An int of ProfileData: it must be there once (the tests read the reset's numbers with it).
#[cfg_attr(not(test), allow(dead_code))]
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

/// A SaveState with every camera's PersonalBest at 0.
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

fn temp_path(what: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("fh2-{what}-{}-{}-{n}.sqlite", std::process::id(), unix_ms()))
}

/// The SQLite file inside PlayerDatabase.
fn container_sqlite(container: &[u8]) -> Option<&[u8]> {
    if container.get(..4)? != b"ssmc" {
        return None;
    }
    let size = u32::from_le_bytes(container.get(4..8)?.try_into().ok()?) as usize;
    container.get(8..8 + size)
}

/// What a reset kept, for the logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub garage_id: i64,
    pub car: i64,
    pub cars_removed: i64,
}

/// The garage, perks, badges and track records of PlayerDatabase reset (see above).
pub fn reset_player_database(container: &[u8]) -> Result<(Vec<u8>, Kept), String> {
    let sqlite = container_sqlite(container).ok_or("PlayerDatabase is not an ssmc container")?;
    let path = temp_path("reset");
    std::fs::write(&path, sqlite).map_err(|e| e.to_string())?;
    let done = (|| -> rusqlite::Result<Result<Kept, String>> {
        let db = Connection::open(&path)?;
        let garage: Vec<(i64, i64)> = db
            .prepare("SELECT Id, CarId FROM Career_Garage ORDER BY Id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let Some(&(garage_id, car)) = garage.iter().find(|(_, c)| STARTERS.contains(c)).or(garage.first()) else {
            return Ok(Err("the garage is empty".into()));
        };
        let tx = db.unchecked_transaction()?;
        let cars_removed = tx.execute("DELETE FROM Career_Garage WHERE Id != ?1", params![garage_id])? as i64;
        tx.execute("DELETE FROM Career_PurchasedParts WHERE GarageId != ?1", params![garage_id])?;
        tx.execute("UPDATE LastUsedGarageId SET LastUsedGarageId = ?1", params![garage_id])?;
        let mut badges: Vec<String> = DEFAULT_BADGES.map(|b| b.to_string()).collect();
        badges.extend(DEFAULT_BADGES_EXTRA.iter().map(|b| b.to_string()));
        tx.execute(&format!("DELETE FROM BadgeTitleUnlocks WHERE UnlockId NOT IN ({})", badges.join(",")), [])?;
        for table in ["PerkUnlock", "BadgeTitleProgress", "Career_Track_Stats", "ChangeTracking"] {
            // a table an older save lacks is fine
            let exists: i64 = tx.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", params![table], |r| r.get(0))?;
            if exists > 0 {
                tx.execute(&format!("DELETE FROM \"{table}\""), [])?;
            }
        }
        tx.commit()?;
        db.execute_batch("VACUUM")?;
        Ok(Ok(Kept { garage_id, car, cars_removed }))
    })();
    let bytes = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    let kept = done.map_err(|e| e.to_string())??;
    let bytes = bytes.map_err(|e| e.to_string())?;
    if bytes.len() + 8 > CONTAINER_SIZE.max(container.len()) {
        return Err("the reset database does not fit its container".into());
    }
    let mut out = Vec::with_capacity(container.len().max(CONTAINER_SIZE));
    out.extend_from_slice(b"ssmc");
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&bytes);
    out.resize(container.len().max(CONTAINER_SIZE), 0);
    Ok((out, kept))
}

/// XP, level, rewards, skill points, perks and credits of ProfileData reset; the garage's numbers
/// point at the kept car. Fields a save lacks (or has twice) are left alone.
pub fn reset_profile(profile: &[u8], kept: &Kept) -> Vec<u8> {
    let mut p = profile.to_vec();
    let fields: [(&str, i32); 13] = [
        ("Credits", 0),
        ("XP", 0),
        ("Level", 0),
        ("WristbandLevel", 0),
        ("LastRewardedLevel", 0),
        ("NumSkillTokens", 0),
        ("PerksUnlocked", 0),
        ("XpPendingUpload", 0),
        ("CareerCar", kept.garage_id as i32),
        ("CarsInGarage", 1),
        ("MostValuableCar", kept.car as i32),
        ("GaragePartsValue", 0),
        ("HighestSkillScore", 0),
    ];
    for (name, value) in fields {
        if let Some(next) = set_profile_int(&p, name, value) {
            p = next;
        }
    }
    p
}

/// A save with the season reset applied (see above), as revision `revision`: (bundle, commit
/// SHA-256, what was kept).
pub fn reset_save(bundle: &[u8], revision: u64) -> Result<(Vec<u8>, String, Kept), String> {
    let mut files = unpack(bundle).ok_or("the save cannot be read")?;
    let mut kept = None;
    for (name, data) in files.iter_mut() {
        match name.as_str() {
            "data/PlayerDatabase" => {
                let (db, k) = reset_player_database(data)?;
                *data = db;
                kept = Some(k);
            }
            "data/SaveState" => *data = without_camera_bests(data),
            _ => {}
        }
    }
    let kept = kept.ok_or("the save has no PlayerDatabase")?;
    if let Some((_, p)) = files.iter_mut().find(|(n, _)| n == "data/ProfileData") {
        *p = reset_profile(p, &kept);
    }
    let data: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != "commit").cloned().collect();
    let commit = files.iter().find(|(n, _)| n == "commit").map(|(_, c)| commit_with(c, &data)).unwrap_or_default();
    if let Some(c) = files.iter_mut().find(|(n, _)| n == "commit") {
        c.1 = commit.clone();
    }
    let packed = pack(revision, &files);
    super::saves::check_bundle(&packed)?;
    Ok((packed, crate::sha256::hex(&commit), kept))
}

/// What the save gates look at: ProfileData's credits, XP and level, and the cars in
/// Career_Garage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveProgress {
    pub credits: i64,
    pub xp: i64,
    pub level: i64,
    pub cars: i64,
}

pub fn save_progress(bundle: &[u8]) -> Option<SaveProgress> {
    let profile = bundle_file(bundle, "data/ProfileData")?;
    let db = bundle_file(bundle, "data/PlayerDatabase")?;
    let sqlite = container_sqlite(&db)?;
    let path = temp_path("progress");
    std::fs::write(&path, sqlite).ok()?;
    let cars = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()
        .and_then(|c| c.query_row("SELECT COUNT(*) FROM Career_Garage", [], |r| r.get::<_, i64>(0)).ok());
    let _ = std::fs::remove_file(&path);
    Some(SaveProgress {
        credits: profile_int(&profile, "Credits")? as i64,
        xp: profile_int(&profile, "XP")? as i64,
        level: profile_int(&profile, "Level")? as i64,
        cars: cars?,
    })
}

/// The save gates: an upload's progress against the last save the server took (`previous`, saved
/// at `then`), with forza/gates.rs's allowances per hour. Err: why it is refused.
pub fn check_upload(previous: &SaveProgress, then: i64, new: &SaveProgress, now: i64) -> Result<(), String> {
    use crate::forza::gates;
    if new.credits < 0 || new.credits > gates::MAX_CREDITS || new.xp < 0 || new.xp > gates::MAX_XP || new.level < 0 || new.level > gates::MAX_LEVEL {
        return Err("numbers beyond the game's limits".into());
    }
    let baseline = super::integrity::Baseline { credits: previous.credits, xp: previous.xp, level: previous.level, winnings: 0, synced: then };
    let p = gates::Progress { credits: new.credits, xp: new.xp, level: new.level, winnings: 0, top_speed: 0.0 };
    gates::check_growth(Some(&baseline), &p, now, false)?;
    let hours = ((now - then).max(0) as f64 / 3_600_000.0).min(24.0 * 30.0);
    let allowed = gates::CARS_BASE + (gates::CARS_PER_HOUR as f64 * hours) as i64;
    if new.cars - previous.cars > allowed {
        return Err(format!("{} -> {} cars in {hours:.1} h", previous.cars, new.cars));
    }
    Ok(())
}

impl Store {
    /// Resets one account (see above). Returns what was done, as JSON.
    pub fn reset_account(&self, xuid: u64, kind: ResetKind, note: &str) -> Result<Value, String> {
        let current = self.save_data(xuid);
        let revision = self.save_info(xuid).map(|s| s.revision + 1).unwrap_or(1);
        let save = match &current {
            Some(b) => Some(reset_save(b, revision)?),
            None => None,
        };
        let now = unix_ms();
        let x = xuid as i64;
        let mut db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let exists: i64 = tx.query_row("SELECT COUNT(*) FROM accounts WHERE xuid = ?1", params![x], |r| r.get(0)).map_err(|e| e.to_string())?;
        if exists == 0 {
            return Err("there is no such account".into());
        }
        let mut tables = vec!["scores", "ghosts", "xp", "transactions", "profile", "profile_tables"];
        if kind == ResetKind::Moderation {
            tables.push("drivatars");
        }
        let mut removed = serde_json::Map::new();
        for table in tables {
            let n = tx.execute(&format!("DELETE FROM {table} WHERE xuid = ?1"), params![x]).map_err(|e| e.to_string())?;
            removed.insert(table.to_owned(), Value::from(n));
        }
        if let Some((bundle, commit, _)) = &save {
            tx.execute(
                "INSERT INTO saves (xuid, revision, commit_hash, data, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![x, revision as i64, commit, bundle, now],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM saves WHERE xuid = ?1 AND id NOT IN (SELECT id FROM saves WHERE xuid = ?1 ORDER BY id DESC LIMIT 3)",
                params![x],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE accounts SET reset_pending = ?2 WHERE xuid = ?1",
                params![x, if kind == ResetKind::Moderation { PENDING_MODERATION } else { PENDING_SEASON }],
            )
            .map_err(|e| e.to_string())?;
        }
        // the gates start from the reset's numbers
        tx.execute(
            "INSERT INTO integrity (xuid, credits, xp, level, winnings, synced, cars, tables_synced)
             VALUES (?1, 0, 0, 0, 0, ?2, 1, ?2)
             ON CONFLICT (xuid) DO UPDATE SET credits = 0, xp = 0, level = 0, winnings = 0, synced = ?2, cars = 1, tables_synced = ?2",
            params![x, now],
        )
        .map_err(|e| e.to_string())?;
        if kind == ResetKind::Moderation {
            tx.execute("UPDATE accounts SET flagged = 0, flagged_at = NULL WHERE xuid = ?1", params![x]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM player_reports WHERE reported = ?1", params![x]).map_err(|e| e.to_string())?;
        }
        let note: String = note.chars().filter(|c| !c.is_control()).take(200).collect();
        let kept = save.as_ref().map(|s| s.2.clone());
        let what = match &kept {
            Some(k) => format!("car {} kept, {} removed; {note}", k.car, k.cars_removed),
            None => format!("no cloud save; {note}"),
        };
        tx.execute(
            "INSERT INTO moderation_log (xuid, action, note, created) VALUES (?1, ?2, ?3, ?4)",
            params![x, if kind == ResetKind::Moderation { "reset" } else { "season-reset" }, what, now],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        let label = if kind == ResetKind::Moderation { "Progresso zerado pela moderação" } else { "Nova temporada: progresso zerado" };
        self.add_event(xuid, "moderation", label, json!({ "car": kept.as_ref().map(|k| k.car) }));
        Ok(json!({
            "xuid": xuid.to_string(),
            "keptCar": kept.as_ref().map(|k| k.car),
            "carsRemoved": kept.as_ref().map(|k| k.cars_removed),
            "saveRevision": save.as_ref().map(|_| revision),
            "removed": removed,
        }))
    }

    /// The first save of a new profile (its offline progress) as it is kept: the season reset
    /// applied. None: not a new profile's first save, or the save cannot be reset (kept as sent).
    pub fn new_profile_save(&self, xuid: u64, bundle: &[u8], revision: u64) -> Option<(Vec<u8>, String)> {
        if self.save_info(xuid).is_some() {
            return None;
        }
        match reset_save(bundle, revision) {
            Ok((b, c, kept)) => {
                println!("  save: a new profile ({xuid}) starts from its offline career, car {} kept", kept.car);
                // the gates start from the season's 0 like everyone's
                self.with(|db| {
                    db.execute(
                        "INSERT INTO integrity (xuid, credits, xp, level, winnings, synced, cars, tables_synced)
                         VALUES (?1, 0, 0, 0, 0, ?2, 1, ?2)
                         ON CONFLICT (xuid) DO UPDATE SET credits = 0, xp = 0, level = 0, winnings = 0, synced = ?2, cars = 1, tables_synced = ?2",
                        params![xuid as i64, unix_ms()],
                    )
                });
                Some((b, c))
            }
            Err(e) => {
                eprintln!("  save: a new profile's first save could not be reset ({e}); kept as sent");
                None
            }
        }
    }

    /// Why the account has a reset save to download (0: none).
    pub fn reset_pending(&self, xuid: u64) -> i64 {
        self.with(|db| db.query_row("SELECT reset_pending FROM accounts WHERE xuid = ?1", params![xuid as i64], |r| r.get::<_, i64>(0)))
            .unwrap_or(0)
    }

    pub fn set_reset_pending(&self, xuid: u64, why: i64) {
        self.with(|db| db.execute("UPDATE accounts SET reset_pending = ?2 WHERE xuid = ?1", params![xuid as i64, why]));
    }

    pub fn reset_downloaded(&self, xuid: u64) {
        self.with(|db| db.execute("UPDATE accounts SET reset_pending = 0 WHERE xuid = ?1 AND reset_pending != 0", params![xuid as i64]));
    }

    /// Every account, for the season reset.
    pub fn all_accounts(&self) -> Vec<u64> {
        self.with(|db| {
            db.prepare("SELECT xuid FROM accounts ORDER BY xuid")?.query_map([], |r| r.get::<_, i64>(0).map(|x| x as u64))?.collect()
        })
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(car: i64) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/starter-{car}.bundle"))).unwrap()
    }

    fn get(files: &[(String, Vec<u8>)], n: &str) -> Vec<u8> {
        files.iter().find(|(k, _)| k == n).map(|(_, d)| d.clone()).unwrap()
    }

    /// A real save given a career's spoils: more cars (the starter not first), parts, perks, a
    /// badge, track records, credits, XP and level.
    fn rich_save() -> Vec<u8> {
        let mut files = unpack(&fixture(461)).unwrap();
        for (name, data) in files.iter_mut() {
            match name.as_str() {
                "data/PlayerDatabase" => {
                    let path = temp_path("rich");
                    std::fs::write(&path, container_sqlite(data).unwrap()).unwrap();
                    let db = Connection::open(&path).unwrap();
                    let cols: Vec<String> =
                        db.prepare("PRAGMA table_info(Career_Garage)").unwrap().query_map([], |r| r.get(1)).unwrap().map(|c| c.unwrap()).collect();
                    let id: i64 = db.query_row("SELECT Id FROM Career_Garage", [], |r| r.get(0)).unwrap();
                    // a bought car with a lower garage Id than the starter, two more after it
                    for (new_id, car) in [(id - 1, 1304), (id + 1, 297), (id + 2, 1124)] {
                        let others: Vec<String> = cols.iter().filter(|c| *c != "Id" && *c != "CarId").cloned().collect();
                        db.execute(
                            &format!(
                                "INSERT INTO Career_Garage (Id, CarId, {}) SELECT ?1, ?2, {} FROM Career_Garage WHERE Id = ?3",
                                others.join(","),
                                others.join(",")
                            ),
                            params![new_id, car, id],
                        )
                        .unwrap();
                        db.execute("INSERT INTO Career_PurchasedParts (GarageId, PartEnum, PartId, ManufacturerId, PricePaid) VALUES (?1, 1, 2, 3, 4)", params![new_id]).unwrap();
                    }
                    db.execute("INSERT INTO Career_PurchasedParts (GarageId, PartEnum, PartId, ManufacturerId, PricePaid) VALUES (?1, 9, 9, 9, 9)", params![id]).unwrap();
                    db.execute("UPDATE LastUsedGarageId SET LastUsedGarageId = ?1", params![id + 2]).unwrap();
                    db.execute("INSERT OR IGNORE INTO PerkUnlock (UnlockId) VALUES (9), (23), (31)", []).unwrap();
                    db.execute("INSERT OR IGNORE INTO BadgeTitleUnlocks (UnlockId, IsNewBadge, IsNewTitle) VALUES (900, 1, 1)", []).unwrap();
                    db.execute("INSERT INTO Career_Track_Stats (TrackId, TotalRaces, TotalDistanceDriven, TotalTimeDriven, TopSpeed, FastestLap, NumCareerRaces, NumDrivatarLaps) VALUES (1, 2, 3, 4, 90.0, 61.5, 2, 0)", []).unwrap();
                    drop(db);
                    let sqlite = std::fs::read(&path).unwrap();
                    let _ = std::fs::remove_file(&path);
                    let mut c = b"ssmc".to_vec();
                    c.extend_from_slice(&(sqlite.len() as u32).to_le_bytes());
                    c.extend_from_slice(&sqlite);
                    c.resize(CONTAINER_SIZE, 0);
                    *data = c;
                }
                "data/ProfileData" => {
                    for (f, v) in [("Credits", 987_654), ("XP", 250_000), ("Level", 12), ("WristbandLevel", 250_000), ("NumSkillTokens", 7), ("CareerCar", 99)] {
                        *data = set_profile_int(data, f, v).unwrap_or_else(|| panic!("{f}"));
                    }
                }
                _ => {}
            }
        }
        let data: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != "commit").cloned().collect();
        let commit = commit_with(&get(&files, "commit"), &data);
        files.iter_mut().find(|(n, _)| n == "commit").unwrap().1 = commit;
        pack(5, &files)
    }

    fn query(db_container: &[u8], sql: &str) -> Vec<i64> {
        let path = temp_path("q");
        std::fs::write(&path, container_sqlite(db_container).unwrap()).unwrap();
        let db = Connection::open(&path).unwrap();
        let v: Vec<i64> = db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        drop(db);
        let _ = std::fs::remove_file(&path);
        v
    }

    #[test]
    fn season_reset_keeps_the_career() {
        let before = unpack(&rich_save()).unwrap();
        let (bundle, commit, kept) = reset_save(&rich_save(), 6).unwrap();
        assert!(bundle.len() <= super::super::saves::MAX_BUNDLE, "{}", bundle.len());
        let after = unpack(&bundle).unwrap();
        let db = get(&after, "data/PlayerDatabase");
        assert_eq!(db.len(), CONTAINER_SIZE);
        // the starter stays (not the lower-Id bought car), with its own parts
        assert_eq!(kept.car, 461);
        assert_eq!(kept.cars_removed, 3);
        assert_eq!(query(&db, "SELECT CarId FROM Career_Garage"), vec![461]);
        assert_eq!(query(&db, "SELECT GarageId FROM Career_PurchasedParts"), vec![kept.garage_id]);
        assert_eq!(query(&db, "SELECT LastUsedGarageId FROM LastUsedGarageId"), vec![kept.garage_id]);
        assert!(query(&db, "SELECT UnlockId FROM PerkUnlock").is_empty());
        assert!(query(&db, "SELECT TrackId FROM Career_Track_Stats").is_empty());
        assert!(!query(&db, "SELECT UnlockId FROM BadgeTitleUnlocks").contains(&900));
        // the career is untouched
        for table in ["Career_ChampionshipsCompleted", "Career_EventUnlock", "Career_EventResults", "Career_DestinationUnlock", "BucketListProgress"] {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            assert_eq!(query(&db, &sql), query(&get(&before, "data/PlayerDatabase"), &sql), "{table}");
        }
        let p = get(&after, "data/ProfileData");
        for (f, v) in [("Credits", 0), ("XP", 0), ("Level", 0), ("WristbandLevel", 0), ("NumSkillTokens", 0), ("CarsInGarage", 1)] {
            assert_eq!(profile_int(&p, f), Some(v), "{f}");
        }
        assert_eq!(profile_int(&p, "CareerCar"), Some(kept.garage_id as i32));
        assert_eq!(profile_int(&p, "MostValuableCar"), Some(461));
        // the rest of ProfileData (tutorials, options) as it was
        assert_eq!(p.len(), get(&before, "data/ProfileData").len());
        let state = String::from_utf8(get(&after, "data/SaveState")).unwrap();
        assert!(crate::db::save_bests::camera_bests(&state).is_empty());
        // the commit matches every file
        let c = String::from_utf8(get(&after, "commit")).unwrap();
        for (n, d) in after.iter().filter(|(n, _)| n != "commit") {
            assert!(c.contains(&format!("{} {} {}", crate::sha256::hex(d), d.len(), n.trim_start_matches("data/"))), "{n}");
        }
        assert_eq!(crate::sha256::hex(&get(&after, "commit")), commit);
        // a second reset changes nothing more
        let (again, _, k2) = reset_save(&bundle, 7).unwrap();
        assert_eq!(k2.cars_removed, 0);
        assert_eq!(query(&get(&unpack(&again).unwrap(), "data/PlayerDatabase"), "SELECT CarId FROM Career_Garage"), vec![461]);
    }

    #[test]
    fn save_gates() {
        let reset = reset_save(&rich_save(), 6).unwrap().0;
        let start = save_progress(&reset).unwrap();
        assert_eq!(start, SaveProgress { credits: 0, xp: 0, level: 0, cars: 1 });
        let rich = save_progress(&rich_save()).unwrap();
        assert_eq!((rich.credits, rich.xp, rich.level, rich.cars), (987_654, 250_000, 12, 4));
        let hour = 3_600_000;
        // the old (or an edited) progress right after a reset: refused
        assert!(check_upload(&start, 0, &rich, 60_000).is_err());
        // a session of play: fine
        let played = SaveProgress { credits: 40_000, xp: 30_000, level: 2, cars: 2 };
        assert!(check_upload(&start, 0, &played, hour).is_ok());
        // too many cars at once
        assert!(check_upload(&start, 0, &SaveProgress { cars: 40, ..played }, hour).is_err());
        // days of play add up
        assert!(check_upload(&start, 0, &SaveProgress { credits: 900_000, xp: 250_000, level: 12, cars: 4 }, 3 * 24 * hour).is_ok());
        assert!(check_upload(&start, 0, &SaveProgress { credits: -1, ..played }, hour).is_err());
    }

    #[test]
    fn every_starter_fixture_resets() {
        for car in STARTERS {
            let (_, _, kept) = reset_save(&fixture(car), 2).unwrap();
            assert_eq!(kept.car, car);
        }
    }

    #[test]
    fn lz_round_trips() {
        let mut big = vec![0u8; 300_000];
        for (i, b) in big.iter_mut().enumerate().take(5000) {
            *b = (i * 7 % 251) as u8;
        }
        let packed = pack(1, &[("commit".into(), b"c".to_vec()), ("data/ProfileData".into(), big.clone())]);
        assert_eq!(bundle_file(&packed, "data/ProfileData").unwrap(), big);
        assert!(packed.len() < big.len() / 10);
    }

    #[test]
    fn profile_markers_are_exact() {
        let mut p = Vec::new();
        for (name, v) in [("WristbandLevel", 250_000i32), ("Level", 12), ("LastRewardedLevel", 11)] {
            p.extend_from_slice(&profile_marker(name));
            p.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(profile_int(&p, "Level"), Some(12));
        let q = set_profile_int(&p, "Level", 0).unwrap();
        assert_eq!((profile_int(&q, "Level"), profile_int(&q, "WristbandLevel")), (Some(0), Some(250_000)));
        assert_eq!(profile_int(b"none", "Credits"), None);
    }
}
