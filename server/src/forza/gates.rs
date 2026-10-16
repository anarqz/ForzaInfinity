//! Sanity gates on what a player's game reports. Nobody outside a player's friends sees their
//! numbers, so these only stop the plain cheats: money or XP out of nowhere, impossible speeds
//! or times, cars that do not exist. A refused report is not stored (the game is answered as
//! usual), and the player gets a strike with the reason.
//!
//! Allowances are generous: a session of normal play stays far below them.

use serde_json::Value;

use super::Ctx;
use crate::db::integrity::Baseline;
use crate::gamedb::Catalog;

/// Credits (or race winnings) a player may gain: this much at once...
pub const CREDITS_BASE: i64 = 250_000;
/// ...plus this much per hour since the last accepted sync.
pub const CREDITS_PER_HOUR: i64 = 500_000;
pub const XP_BASE: i64 = 150_000;
pub const XP_PER_HOUR: i64 = 400_000;
pub const LEVELS_BASE: i64 = 3;
pub const LEVELS_PER_HOUR: i64 = 10;
/// XP the game may report with Scoreboard.SubmitXpScore in any hour.
pub const XP_REPORTED_PER_HOUR: f64 = 400_000.0;
/// Over a car's stock top speed: upgrades, drafting, downhill.
pub const SPEED_MARGIN: f64 = 1.3;
pub const CARS_BASE: i64 = 10;
pub const CARS_PER_HOUR: i64 = 20;
/// Hours after an account is made in which its offline progress would arrive as it is. None since
/// the season resets (db/reset.rs): every profile starts from 0 like everyone, so even its first
/// syncs are measured from there.
pub const ARRIVAL_HOURS: i64 = 0;
/// Highest car id the game uses (base cars go to 2501; car packs add their own in between and
/// above, which the base database the server reads does not list).
const MAX_CAR_ID: i64 = 4095;
/// Hard limits whatever the history.
pub const MAX_CREDITS: i64 = 999_999_999;
pub const MAX_XP: i64 = 200_000_000;
pub const MAX_LEVEL: i64 = 999;

/// The numbers of an UpdateAll that the gates look at.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub credits: i64,
    pub xp: i64,
    pub level: i64,
    pub winnings: i64,
    /// m/s, the fastest the player ever went
    pub top_speed: f64,
}

pub fn progress(params: &Value) -> Result<Progress, String> {
    let history = &params["playerHistory"][0];
    let race = &params["playerRaceHistory"][0];
    let int = |v: &Value, name: &str| -> Result<i64, String> {
        match v.as_i64().or_else(|| v.as_u64().map(|u| u.min(i64::MAX as u64) as i64)) {
            Some(n) if n >= 0 => Ok(n),
            Some(_) => Err(format!("negative {name}")),
            None => Ok(0),
        }
    };
    let top_speed = race["topSpeed"].as_f64().unwrap_or(0.0);
    if !top_speed.is_finite() || top_speed < 0.0 {
        return Err("invalid top speed".into());
    }
    let p = Progress {
        credits: int(&history["credits"], "credits")?,
        xp: int(&race["xp"], "XP")?,
        level: int(&race["level"], "level")?,
        winnings: int(&race["totalWinnings"], "winnings")?,
        top_speed,
    };
    if p.credits > MAX_CREDITS || p.winnings > MAX_CREDITS || p.xp > MAX_XP || p.level > MAX_LEVEL {
        return Err("numbers beyond the game's limits".into());
    }
    Ok(p)
}

/// Whether an account made at `created` is still arriving (ARRIVAL_HOURS).
pub fn arriving(created: Option<i64>, now: i64) -> bool {
    created.is_some_and(|c| now - c < ARRIVAL_HOURS * 3_600_000)
}

fn hours_since(then: i64, now: i64) -> f64 {
    ((now - then).max(0) as f64 / 3_600_000.0).min(24.0 * 30.0)
}

/// Growth since the last accepted sync, within the allowances. While the account is arriving
/// anything within the limits is taken; a first sync with no baseline is checked against a new
/// season's start: 0 credits, XP and level.
pub fn check_growth(previous: Option<&Baseline>, new: &Progress, now: i64, arriving: bool) -> Result<(), String> {
    if arriving {
        return Ok(());
    }
    let start = Baseline { credits: 0, xp: 0, level: 0, winnings: 0, synced: now };
    let b = previous.unwrap_or(&start);
    let hours = hours_since(b.synced, now);
    let allowed = |base: i64, per_hour: i64| base + (per_hour as f64 * hours) as i64;
    if new.credits - b.credits > allowed(CREDITS_BASE, CREDITS_PER_HOUR) {
        return Err(format!("credits {} -> {} in {:.1} h", b.credits, new.credits, hours));
    }
    if previous.is_some() && new.winnings - b.winnings > allowed(CREDITS_BASE, CREDITS_PER_HOUR) {
        return Err(format!("winnings {} -> {} in {:.1} h", b.winnings, new.winnings, hours));
    }
    if new.xp - b.xp > allowed(XP_BASE, XP_PER_HOUR) {
        return Err(format!("XP {} -> {} in {:.1} h", b.xp, new.xp, hours));
    }
    if new.level - b.level > allowed(LEVELS_BASE, LEVELS_PER_HOUR) {
        return Err(format!("level {} -> {} in {:.1} h", b.level, new.level, hours));
    }
    Ok(())
}

/// The fastest stock car among `cars` (m/s), or of the whole game when none is known.
pub fn fastest(catalog: &Catalog, cars: &[i64]) -> f64 {
    let known: Vec<f64> = cars.iter().filter_map(|c| catalog.car(*c)).map(|c| c.top_speed_mps).collect();
    let list: Vec<f64> = if known.is_empty() { catalog.cars.values().map(|c| c.top_speed_mps).collect() } else { known };
    list.into_iter().fold(0.0, f64::max)
}

pub fn check_top_speed(catalog: &Catalog, cars: &[i64], top_speed: f64) -> Result<(), String> {
    let limit = fastest(catalog, cars) * SPEED_MARGIN;
    if limit > 0.0 && top_speed > limit {
        return Err(format!("top speed {:.0} km/h, cars allow {:.0} km/h", top_speed * 3.6, limit * 3.6));
    }
    Ok(())
}

/// A race time: the track length can not be covered faster than the car's top speed allows.
pub fn check_race_time(catalog: &Catalog, car: i64, track: i64, seconds: f64) -> Result<(), String> {
    let (Some(car), Some(track)) = (catalog.car(car), catalog.tracks.get(&track)) else {
        return Ok(());
    };
    let fastest = car.top_speed_mps * SPEED_MARGIN;
    if fastest > 0.0 && track.length_m > 0 && seconds < track.length_m as f64 / fastest {
        return Err(format!("{:.2} s over {} m with {}", seconds, track.length_m, car.name));
    }
    Ok(())
}

/// garageCars of an UpdateAllTables: car ids the game can have (the base database's, or a car
/// pack's), and not too many new ones at once (unless the account is arriving).
pub fn check_garage(
    catalog: &Catalog, params: &Value, stored: usize, previous: Option<(i64, i64)>, now: i64, arriving: bool,
) -> Result<(), String> {
    let rows = params["garageCars"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if let Some(bad) =
        rows.iter().filter_map(|r| r["carId"].as_i64()).find(|c| catalog.car(*c).is_none() && !(1..=MAX_CAR_ID).contains(c))
    {
        return Err(format!("unknown car {bad}"));
    }
    if arriving {
        return Ok(());
    }
    let (before, then) = previous.unwrap_or((stored as i64, now));
    let added = rows.len() as i64 - params["deleteGarageCars"].as_array().map(Vec::len).unwrap_or(0) as i64;
    let allowed = CARS_BASE + (CARS_PER_HOUR as f64 * hours_since(then, now)) as i64;
    if (stored as i64 + added.max(0)) - before > allowed {
        return Err(format!("{} cars added at once", stored as i64 + added - before));
    }
    Ok(())
}

/// Records a refused report: event for the manager, strike for the player.
pub fn refuse(ctx: &Ctx, what: &str, reason: &str) {
    let strikes = ctx.store.strike(ctx.xuid, &format!("{what}: {reason}"));
    eprintln!("integrity: {} {what} refused ({reason}), strike {strikes}", ctx.xuid);
    ctx.event(
        "integrity",
        format!("{what} recusado: {reason}"),
        serde_json::json!({ "what": what, "reason": reason, "strikes": strikes }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(credits: i64, xp: i64, level: i64) -> Progress {
        Progress { credits, xp, level, winnings: 0, top_speed: 0.0 }
    }

    #[test]
    fn growth() {
        let now = 10 * 3_600_000;
        let base = Baseline { credits: 10_000, xp: 100_000, level: 6, winnings: 0, synced: now - 1_800_000 };
        // 10k -> 1M in half an hour: refused
        assert!(check_growth(Some(&base), &p(1_000_000, 100_000, 6), now, false).is_err());
        // a good race in half an hour: fine
        assert!(check_growth(Some(&base), &p(80_000, 130_000, 7), now, false).is_ok());
        // spending is never a problem
        assert!(check_growth(Some(&base), &p(0, 100_000, 6), now, false).is_ok());
        // XP and levels
        assert!(check_growth(Some(&base), &p(10_000, 2_000_000, 6), now, false).is_err());
        assert!(check_growth(Some(&base), &p(10_000, 120_000, 40), now, false).is_err());
        // a day later much more is fine
        let later = Baseline { synced: now - 24 * 3_600_000, ..base };
        assert!(check_growth(Some(&later), &p(5_000_000, 3_000_000, 30), now, false).is_ok());
        // a first sync with no baseline: measured from a season's start (0)
        assert!(check_growth(None, &p(60_000, 100_717, 2), now, false).is_ok());
        assert!(check_growth(None, &p(5_000_000, 100_717, 2), now, false).is_err());
        assert!(check_growth(None, &p(60_000, 100_717, 6), now, false).is_err());
        // no arrival window any more: a new account brings nothing as it is
        assert!(!arriving(Some(now - 1), now) && !arriving(None, now));
    }

    #[test]
    fn garage() {
        let catalog = Catalog::default();
        let cars = |ids: &[i64]| serde_json::json!({ "garageCars": ids.iter().map(|c| serde_json::json!({ "carId": c })).collect::<Vec<_>>() });
        let many: Vec<i64> = (1..=60).collect();
        // a car pack's car, a whole garage while arriving
        assert!(check_garage(&catalog, &cars(&[2284, 2546]), 0, None, 0, false).is_ok());
        assert!(check_garage(&catalog, &cars(&many), 0, None, 0, true).is_ok());
        assert!(check_garage(&catalog, &cars(&many), 0, None, 0, false).is_err());
        assert!(check_garage(&catalog, &cars(&[99_999]), 0, None, 0, true).is_err());
    }

    #[test]
    fn limits() {
        let bad = serde_json::json!({ "playerHistory": [{ "credits": -5 }], "playerRaceHistory": [{}] });
        assert!(progress(&bad).is_err());
        let huge = serde_json::json!({ "playerHistory": [{ "credits": 5_000_000_000u64 }], "playerRaceHistory": [{}] });
        assert!(progress(&huge).is_err());
    }
}
