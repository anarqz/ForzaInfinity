//! Only the game's own cars play online. Every request of the hook carries X-Fh2-CarDb, the
//! SHA-256 of the player's media/stripped/gamedbRC.slt (hook: azimuth_world.c), or "changed"
//! once that file changed during the session. An account whose header is missing, "changed" or
//! not one of the accepted hashes is refused, on the game's services and on /infinity/v1/*
//! alike (main.rs, api.rs): the game plays offline and the hook says why.
//!
//! The accepted hash is the one of the server's own --gamedb file (the original game's),
//! plus any --cardb-allow <sha-256>; --cardb-gate off turns the check off. The header is only
//! as honest as the client sending it: this keeps modified databases out of the official hook,
//! it does not stop someone who builds their own (the server's gates on what a profile does,
//! forza/gates.rs, and the players' reports, db/moderation.rs, are for that).

use std::{collections::HashSet, sync::Mutex};

pub const CHANGED: &str = "changed";
/// Accounts already logged as refused (one line each per server run).
const LOGGED_MAX: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No X-Fh2-CarDb (a hook from before fh2online.46, or the file could not be read).
    Missing,
    /// The file changed while the game ran.
    Changed,
    /// Not the game's own file.
    Modified,
}

impl Refusal {
    pub fn name(self) -> &'static str {
        match self {
            Refusal::Missing => "missing",
            Refusal::Changed => "changed",
            Refusal::Modified => "modified",
        }
    }
}

#[derive(Default)]
pub struct Gate {
    /// Lowercase hex SHA-256s; empty: the gate is off.
    allowed: Vec<String>,
    logged: Mutex<HashSet<u64>>,
}

impl Gate {
    pub fn new(allowed: Vec<String>) -> Gate {
        Gate { allowed: allowed.into_iter().map(|a| a.to_ascii_lowercase()).collect(), logged: Mutex::default() }
    }

    pub fn on(&self) -> bool {
        !self.allowed.is_empty()
    }

    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    /// Why the request (its X-Fh2-CarDb header) may not play online; None: it may.
    pub fn refusal(&self, header: Option<&str>) -> Option<Refusal> {
        if !self.on() {
            return None;
        }
        let h = header.map(|h| h.trim().to_ascii_lowercase());
        match h.as_deref() {
            None | Some("") => Some(Refusal::Missing),
            Some(CHANGED) => Some(Refusal::Changed),
            Some(h) if self.allowed.iter().any(|a| a == h) => None,
            Some(_) => Some(Refusal::Modified),
        }
    }

    /// Whether this account's refusal is still to be logged (once per run).
    pub fn first_time(&self, xuid: u64) -> bool {
        let mut l = self.logged.lock().unwrap_or_else(|p| p.into_inner());
        if l.len() >= LOGGED_MAX {
            l.clear();
        }
        l.insert(xuid)
    }
}

/// A 64-digit hex SHA-256 (for --cardb-allow).
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "4d50013dc7705de0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn refusals() {
        let g = Gate::new(vec![GOOD.to_ascii_uppercase()]);
        assert_eq!(g.refusal(Some(GOOD)), None);
        assert_eq!(g.refusal(Some(&format!("  {}  ", GOOD.to_ascii_uppercase()))), None);
        assert_eq!(g.refusal(None), Some(Refusal::Missing));
        assert_eq!(g.refusal(Some("")), Some(Refusal::Missing));
        assert_eq!(g.refusal(Some("changed")), Some(Refusal::Changed));
        assert_eq!(g.refusal(Some(&"b".repeat(64))), Some(Refusal::Modified));
        assert_eq!(g.refusal(Some("x")), Some(Refusal::Modified));
        // off: everything goes
        assert_eq!(Gate::new(vec![]).refusal(None), None);
        assert!(g.first_time(5) && !g.first_time(5) && g.first_time(6));
        assert!(is_sha256(GOOD) && !is_sha256("abc") && !is_sha256(&"z".repeat(64)));
    }
}
