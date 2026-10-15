//! Speed trap and speed zone bests from the cloud save. The game submits a speed trap or zone
//! (Scoreboard.SubmitScore) only for a new personal best, and a profile that started from an
//! offline save already holds its bests there: without them the boards would not know the
//! player's own speed. Each save the server takes is read for them.
//!
//! The SaveState (XML) keeps one CameraState per camera, `<Object Id="speed_trap_11"
//! Type="CameraState">` with `PersonalBest` (m/s, as SubmitScore sends it). The board of each
//! camera is its LeaderboardID in media/EntityModel.zip speed_cameras*.xml: speed traps 1-30
//! (Storm Island 31-35), speed zones 36-50 (Storm Island 51-55). scoreboardType 9, scoreType 5
//! for traps and 6 for zones, as the game asks for them.

use serde_json::{Value, json};

use super::Store;
use super::saves::SAVE_FILES;

/// The leaderboards of speed traps and zones.
const BOARD_TYPE: i64 = 9;
const TRAP: i64 = 5;
const ZONE: i64 = 6;
/// Highest speed taken from a save (m/s; about 500 km/h).
const MAX_SPEED: f64 = 140.0;

/// (scoreType, board) of a camera id from the save.
fn camera_board(id: &str) -> Option<(i64, i64)> {
    let number = |prefix: &str| id.strip_prefix(prefix).and_then(|n| n.parse::<i64>().ok());
    let in_range = |n: i64, max: i64| (1..=max).contains(&n).then_some(n);
    if let Some(n) = number("pdlc_speed_trap_").and_then(|n| in_range(n, 5)) {
        Some((TRAP, 30 + n))
    } else if let Some(n) = number("pdlc_speed_zone_").and_then(|n| in_range(n, 5)) {
        Some((ZONE, 50 + n))
    } else if let Some(n) = number("speed_trap_").and_then(|n| in_range(n, 30)) {
        Some((TRAP, n))
    } else {
        number("speed_zone_").and_then(|n| in_range(n, 15)).map(|n| (ZONE, 35 + n))
    }
}

/// (scoreType, board, best) of every camera with a personal best in a SaveState.
pub fn camera_bests(xml: &str) -> Vec<(i64, i64, f64)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<Object Id=\"") {
        rest = &rest[at + 12..];
        let Some(end) = rest.find('"') else { break };
        let id = &rest[..end];
        let body = &rest[end..rest.find("</Object>").unwrap_or(rest.len())];
        if !body.starts_with("\" Type=\"CameraState\"") {
            continue;
        }
        let Some((score_type, board)) = camera_board(id) else { continue };
        let best = body
            .find("Name=\"PersonalBest\" Value=\"")
            .and_then(|p| body[p + 27..].split('"').next())
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_SPEED);
        if let Some(best) = best {
            out.push((score_type, board, best));
        }
    }
    out
}

/// One file of a bundle (see saves.rs), unpacked.
pub fn bundle_file(data: &[u8], wanted: &str) -> Option<Vec<u8>> {
    let mut at = 20usize;
    let count = u32::from_le_bytes(data.get(16..20)?.try_into().ok()?);
    if data.get(..8)? != b"FH2SAVE1" || count as usize > SAVE_FILES.len() {
        return None;
    }
    for _ in 0..count {
        let n = u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as usize;
        let name = data.get(at + 2..at + 2 + n)?;
        at += 2 + n;
        let raw = u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize;
        let stored = u32::from_le_bytes(data.get(at + 4..at + 8)?.try_into().ok()?) as usize;
        let method = *data.get(at + 8)?;
        let body = data.get(at + 9..at + 9 + stored)?;
        at += 9 + stored;
        if name == wanted.as_bytes() {
            return match method {
                0 => Some(body.to_vec()),
                1 => xpress_huffman(body, raw),
                2 => lz(body, raw),
                _ => None,
            };
        }
    }
    None
}

/// LEB128 varint at `*at`.
fn varint(input: &[u8], at: &mut usize) -> Option<usize> {
    let mut v = 0usize;
    for shift in (0..35).step_by(7) {
        let b = *input.get(*at)?;
        *at += 1;
        v |= ((b & 127) as usize) << shift;
        if b & 128 == 0 {
            return Some(v);
        }
    }
    None
}

/// The hook's method 2 (azimuth_codec.c): varint literal count, the literals, then (unless the
/// output is complete) varint match length - 4 and varint offset - 1.
fn lz(input: &[u8], size: usize) -> Option<Vec<u8>> {
    let mut at = 0usize;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let n = varint(input, &mut at)?;
        if n > size - out.len() {
            return None;
        }
        out.extend_from_slice(input.get(at..at + n)?);
        at += n;
        if out.len() == size {
            break;
        }
        let length = varint(input, &mut at)? + 4;
        let offset = varint(input, &mut at)? + 1;
        if offset > out.len() || length > size - out.len() {
            return None;
        }
        for _ in 0..length {
            out.push(out[out.len() - offset]);
        }
    }
    (at == input.len()).then_some(out)
}

/// XPRESS Huffman (MS-XCA 2.2.4), what hooks before method 2 sent from Windows: blocks of 64 KiB
/// of output, each starting with 256 bytes of 4-bit code lengths for 512 symbols.
fn xpress_huffman(input: &[u8], size: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    let mut base = 0usize;
    let mut block_end = 0usize;
    while out.len() < size {
        let lengths = input.get(base..base + 256)?;
        let mut table = vec![(0u16, 0u8); 1 << 15];
        let mut code = 0usize;
        for length in 1..=15u8 {
            for symbol in 0..512usize {
                let l = if symbol % 2 == 0 { lengths[symbol / 2] & 15 } else { lengths[symbol / 2] >> 4 };
                if l == length {
                    let first = code << (15 - length);
                    let n = 1usize << (15 - length);
                    table.get_mut(first..first + n)?.fill((symbol as u16, length));
                    code += 1;
                }
            }
            code <<= 1;
        }
        let mut at = base + 256;
        let word = |at: &mut usize| -> u32 {
            let v = input.get(*at).copied().unwrap_or(0) as u32 | (input.get(*at + 1).copied().unwrap_or(0) as u32) << 8;
            *at += 2;
            v
        };
        let mut bits = word(&mut at) << 16 | word(&mut at);
        let mut extra = 16i32;
        let take = |n: u8, bits: &mut u32, extra: &mut i32, at: &mut usize| {
            *bits = bits.wrapping_shl(n as u32);
            *extra -= n as i32;
            if *extra < 0 {
                *bits |= word(at) << (-*extra) as u32;
                *extra += 16;
            }
        };
        block_end += 65536;
        let end = block_end.min(size);
        while out.len() < end {
            let (symbol, length) = table[(bits >> 17) as usize];
            if length == 0 {
                return None;
            }
            take(length, &mut bits, &mut extra, &mut at);
            if symbol < 256 {
                out.push(symbol as u8);
                continue;
            }
            let symbol = symbol - 256;
            let mut length = (symbol & 15) as usize;
            let offset_bits = (symbol >> 4) as u8;
            if length == 15 {
                length = *input.get(at)? as usize + 15;
                at += 1;
                if length == 270 {
                    length = u16::from_le_bytes(input.get(at..at + 2)?.try_into().ok()?) as usize;
                    at += 2;
                    if length == 0 {
                        length = u32::from_le_bytes(input.get(at..at + 4)?.try_into().ok()?) as usize;
                        at += 4;
                    }
                }
            }
            length += 3;
            let offset = if offset_bits > 0 { ((bits >> (32 - offset_bits as u32)) | (1 << offset_bits)) as usize } else { 1 };
            if offset_bits > 0 {
                take(offset_bits, &mut bits, &mut extra, &mut at);
            }
            if offset > out.len() || length > size.saturating_sub(out.len()) + 65536 {
                return None;
            }
            for _ in 0..length {
                out.push(out[out.len() - offset]);
            }
        }
        base = at;
    }
    out.truncate(size);
    Some(out)
}

impl Store {
    /// import_save_bests for every account's current save (at start: saves taken before the
    /// server read them). Returns how many bests were taken.
    pub fn import_all_save_bests(&self) -> usize {
        let owners: Vec<u64> = self
            .with(|db| {
                let mut stmt = db.prepare("SELECT DISTINCT xuid FROM saves")?;
                let found = stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten().map(|x| x as u64).collect::<Vec<_>>();
                Ok(found)
            })
            .unwrap_or_default();
        owners.into_iter().filter_map(|x| self.save_data(x).map(|d| self.import_save_bests(x, &d))).sum()
    }

    /// Takes a save's speed trap and zone bests as the player's scores where they beat what the
    /// boards have for them. Returns how many were taken.
    pub fn import_save_bests(&self, xuid: u64, bundle: &[u8]) -> usize {
        let Some(xml) = bundle_file(bundle, "data/SaveState") else { return 0 };
        let xml = String::from_utf8_lossy(&xml);
        // the car shown with them (none yet when the save comes before the garage: fill_save_cars)
        let car = self.most_raced_car(xuid).unwrap_or(Value::Null);
        let mut taken = 0;
        for (score_type, board, best) in camera_bests(&xml) {
            let current = self
                .leaderboard(BOARD_TYPE, score_type, None, Some(board))
                .into_iter()
                .find(|r| r.xuid == xuid)
                .map(|r| r.score);
            if current.is_some_and(|c| c >= best - 1e-3) {
                continue;
            }
            self.add_score(
                xuid,
                score_type,
                &json!({
                    "scoreboardType": BOARD_TYPE, "gameScoreboardId": board, "trackId": -1, "score": best,
                    "carId": car["carId"].as_i64().unwrap_or(-1), "carClassId": car["classID"].as_i64().unwrap_or(0),
                    "carPerformanceIndex": car["perfIndex"].as_f64().unwrap_or(0.0),
                    "usedSTM": false, "usedTCS": false, "usedABS": false, "usedAutoBrake": false,
                    "usedAutoShifting": false, "usedClutch": false, "isClean": true,
                    // from the save, not submitted while online
                    "fromSave": true,
                }),
            );
            taken += 1;
        }
        taken
    }

    /// The player's most raced garage car (UpdateAllTables garageCars row).
    fn most_raced_car(&self, xuid: u64) -> Option<Value> {
        let garage = self.tables(xuid).remove("garageCars")?;
        garage.as_array()?.iter().filter(|c| c["carId"].as_i64().is_some_and(|id| id > 0)).max_by_key(|c| c["numRaces"].as_i64().unwrap_or(0)).cloned()
    }

    /// Gives the bests taken from a save before the player's garage reached the server (a new
    /// profile uploads its save first) the player's most raced car, as import_save_bests does:
    /// without one the boards showed "DLC Car", class D, PI 100. Returns how many were filled.
    pub fn fill_save_cars(&self, xuid: u64) -> usize {
        let Some(car) = self.most_raced_car(xuid) else { return 0 };
        let id = car["carId"].as_i64().unwrap_or(-1);
        let class = car["classID"].as_i64().unwrap_or(0);
        let pi = car["perfIndex"].as_f64().unwrap_or(0.0);
        self.with(|db| {
            let mut stmt = db.prepare("SELECT id, data FROM scores WHERE xuid = ?1 AND car_id < 0")?;
            let rows: Vec<(i64, String)> =
                stmt.query_map([xuid as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
            let mut filled = 0;
            for (row, data) in rows {
                let Ok(mut data) = serde_json::from_str::<Value>(&data) else { continue };
                if data["fromSave"] != true {
                    continue;
                }
                data["carId"] = json!(id);
                data["carClassId"] = json!(class);
                data["carPerformanceIndex"] = json!(pi);
                filled += db.execute(
                    "UPDATE scores SET car_id = ?2, car_class_id = ?3, data = ?4 WHERE id = ?1",
                    rusqlite::params![row, id, class, data.to_string()],
                )?;
            }
            Ok(filled)
        })
        .unwrap_or(0)
    }

    /// fill_save_cars for every player (at start).
    pub fn fill_all_save_cars(&self) -> usize {
        let owners: Vec<u64> = self
            .with(|db| {
                let mut stmt = db.prepare("SELECT DISTINCT xuid FROM scores WHERE car_id < 0")?;
                let found = stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten().map(|x| x as u64).collect::<Vec<_>>();
                Ok(found)
            })
            .unwrap_or_default();
        owners.into_iter().map(|x| self.fill_save_cars(x)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A best taken from the save before the garage came gets the most raced car with it.
    #[test]
    fn save_bests_get_a_car() {
        let dir = std::env::temp_dir().join(format!("fh2-savecar-{}", super::super::accounts::new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        let x = 2535000000000042;
        store.add_score(x, TRAP, &json!({
            "scoreboardType": BOARD_TYPE, "gameScoreboardId": 3, "trackId": -1, "score": 40.0, "carId": -1, "fromSave": true,
        }));
        assert_eq!(store.fill_all_save_cars(), 0);
        store.apply_tables(x, &json!({ "garageCars": [
            { "id": 1, "carId": 461, "classID": 2, "perfIndex": 0.608, "numRaces": 3 },
            { "id": 2, "carId": 1218, "classID": 3, "perfIndex": 0.65, "numRaces": 9 },
        ]}));
        assert_eq!(store.fill_all_save_cars(), 1);
        let row = store.leaderboard(BOARD_TYPE, TRAP, None, Some(3)).remove(0);
        assert_eq!(row.data["carId"], 1218);
        assert_eq!(row.data["carClassId"], 3);
        assert_eq!(store.fill_all_save_cars(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn boards() {
        assert_eq!(camera_board("speed_trap_11"), Some((TRAP, 11)));
        assert_eq!(camera_board("speed_zone_11"), Some((ZONE, 46)));
        assert_eq!(camera_board("speed_zone_01"), Some((ZONE, 36)));
        assert_eq!(camera_board("pdlc_speed_trap_01"), Some((TRAP, 31)));
        assert_eq!(camera_board("pdlc_speed_zone_05"), Some((ZONE, 55)));
        assert_eq!(camera_board("speed_trap_31"), None);
        assert_eq!(camera_board("speed_trap_tutorial"), None);
        let xml = r#"<Object Id="speed_trap_11" Type="CameraState">
    <Property Name="Discovered" Value="true"/>
    <Property Name="PersonalBest" Value="52.7177"/>
  </Object>
  <Object Id="speed_trap_12" Type="CameraState">
    <Property Name="PersonalBest" Value="0"/>
  </Object>
  <Object Id="SpeedTrapTutorial" Type="TutorialSaveState">
    <Property Name="PersonalBest" Value="9"/>
  </Object>
  <Object Id="speed_zone_11" Type="CameraState">
    <Property Name="PersonalBest" Value="999"/>
  </Object>"#;
        assert_eq!(camera_bests(xml), vec![(TRAP, 11, 52.7177)]);
    }

    #[test]
    fn method_2() {
        // "abcabcabcX": 3 literals, a match of 6 at offset 3, 1 literal
        let packed = [3, b'a', b'b', b'c', 2, 2, 1, b'X'];
        assert_eq!(lz(&packed, 10).unwrap(), b"abcabcabcX");
        assert!(lz(&packed, 9).is_none());
    }
}
