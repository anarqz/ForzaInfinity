//! What the server last accepted from each player's profile sync (OnlineProfile.UpdateAll /
//! UpdateAllTables), so the next one can be checked against it (forza/gates.rs), and the
//! strikes a player got for syncs that were refused.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS integrity (
        xuid INTEGER PRIMARY KEY, credits INTEGER, xp INTEGER, level INTEGER, winnings INTEGER,
        synced INTEGER, cars INTEGER, tables_synced INTEGER,
        strikes INTEGER NOT NULL DEFAULT 0, last_reason TEXT NOT NULL DEFAULT '', last_strike INTEGER);";

/// The last accepted numbers of a player's profile.
#[derive(Debug, Clone, Copy, Default)]
pub struct Baseline {
    pub credits: i64,
    pub xp: i64,
    pub level: i64,
    pub winnings: i64,
    /// unix ms of the UpdateAll they came from
    pub synced: i64,
}

impl Store {
    pub fn baseline(&self, xuid: u64) -> Option<Baseline> {
        self.with(|db| {
            db.query_row(
                "SELECT credits, xp, level, winnings, synced FROM integrity WHERE xuid = ?1 AND synced IS NOT NULL",
                [xuid as i64],
                |r| Ok(Baseline { credits: r.get(0)?, xp: r.get(1)?, level: r.get(2)?, winnings: r.get(3)?, synced: r.get(4)? }),
            )
            .optional()
        })
        .flatten()
    }

    pub fn set_baseline(&self, xuid: u64, b: &Baseline) {
        self.with(|db| {
            db.execute(
                "INSERT INTO integrity (xuid, credits, xp, level, winnings, synced) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (xuid) DO UPDATE SET credits = ?2, xp = ?3, level = ?4, winnings = ?5, synced = ?6",
                params![xuid as i64, b.credits, b.xp, b.level, b.winnings, b.synced],
            )
        });
    }

    /// (garage size, unix ms) of the last accepted UpdateAllTables.
    pub fn garage_baseline(&self, xuid: u64) -> Option<(i64, i64)> {
        self.with(|db| {
            db.query_row(
                "SELECT cars, tables_synced FROM integrity WHERE xuid = ?1 AND tables_synced IS NOT NULL",
                [xuid as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })
        .flatten()
    }

    pub fn set_garage_baseline(&self, xuid: u64, cars: i64) {
        self.with(|db| {
            db.execute(
                "INSERT INTO integrity (xuid, cars, tables_synced) VALUES (?1, ?2, ?3)
                 ON CONFLICT (xuid) DO UPDATE SET cars = ?2, tables_synced = ?3",
                params![xuid as i64, cars, unix_ms()],
            )
        });
    }

    /// Records a refused sync; returns the player's strike count.
    pub fn strike(&self, xuid: u64, reason: &str) -> i64 {
        self.with(|db| {
            db.execute(
                "INSERT INTO integrity (xuid, strikes, last_reason, last_strike) VALUES (?1, 1, ?2, ?3)
                 ON CONFLICT (xuid) DO UPDATE SET strikes = strikes + 1, last_reason = ?2, last_strike = ?3",
                params![xuid as i64, reason, unix_ms()],
            )?;
            db.query_row("SELECT strikes FROM integrity WHERE xuid = ?1", [xuid as i64], |r| r.get(0))
        })
        .unwrap_or(0)
    }

    pub fn integrity_json(&self, xuid: u64) -> serde_json::Value {
        let (strikes, reason) = self.strikes(xuid);
        serde_json::json!({ "strikes": strikes, "lastReason": reason })
    }

    /// (strikes, last reason) for the manager.
    pub fn strikes(&self, xuid: u64) -> (i64, String) {
        self.with(|db| {
            db.query_row("SELECT strikes, last_reason FROM integrity WHERE xuid = ?1", [xuid as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })
        .flatten()
        .unwrap_or((0, String::new()))
    }
}
