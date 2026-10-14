//! FH2Online accounts: "name#12345", created from the game through the hook
//! (POST /infinity/v1/register). Each account is a player (users row, server-allocated XUID);
//! the hook then sends X-Fh2-Token on every request and the token, not the game's own XUID,
//! says who is playing.
//!
//! Tables:
//! - accounts: name (1-9 letters/digits, first a letter) + discriminator (5 digits, the start
//!   of the device fingerprint), unique together; gamertag is "name#discriminator".
//!   One account per PC: `device` is the SHA-1 of the device id the hook sends (its full
//!   fingerprint hashed with this server's host, so servers cannot match players up);
//! - devices: one token per registered device, stored only as its SHA-1.
//!
//! A new account always starts from zero (no offline progress is imported).

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS accounts (
        xuid INTEGER PRIMARY KEY, name TEXT NOT NULL, discriminator TEXT NOT NULL,
        device TEXT NOT NULL UNIQUE, created INTEGER NOT NULL, banned INTEGER NOT NULL DEFAULT 0,
        UNIQUE (name COLLATE NOCASE, discriminator));
    CREATE TABLE IF NOT EXISTS devices (
        token_hash TEXT PRIMARY KEY, xuid INTEGER NOT NULL, created INTEGER NOT NULL,
        last_seen INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS devices_xuid ON devices (xuid);
    CREATE TABLE IF NOT EXISTS friend_requests (
        from_xuid INTEGER NOT NULL, to_xuid INTEGER NOT NULL, created INTEGER NOT NULL,
        PRIMARY KEY (from_xuid, to_xuid));
    CREATE TABLE IF NOT EXISTS web_keys (
        xuid INTEGER PRIMARY KEY, key_hash TEXT NOT NULL, created INTEGER NOT NULL);";

/// Web key alphabet: no 0/O or 1/I, so it can be read off a TV and typed.
const WEB_KEY_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// "K7QM-4XPA-9RT2": 12 random characters (60 bits) for logging in to the site with
/// name#12345. Separate from the device token, which never leaves the game.
pub fn new_web_key() -> String {
    let mut bytes = [0u8; 12];
    os_random(&mut bytes);
    let chars: Vec<char> = bytes.iter().map(|b| WEB_KEY_ALPHABET[(b & 31) as usize] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Canonical form for comparing: upper case, no dashes or spaces, O->0 is not needed (no 0).
fn web_key_canonical(key: &str) -> String {
    key.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

pub struct Account {
    pub xuid: u64,
    pub name: String,
    pub discriminator: String,
    pub banned: bool,
}

impl Account {
    pub fn gamertag(&self) -> String {
        format!("{}#{}", self.name, self.discriminator)
    }
}

/// 1 to 9 ASCII letters or digits starting with a letter: with "#12345" the gamertag stays
/// within the game's 15 characters.
pub fn valid_name(name: &str) -> bool {
    (1..=9).contains(&name.len())
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn valid_discriminator(d: &str) -> bool {
    d.len() == 5 && d.bytes().all(|b| b.is_ascii_digit())
}

/// The hook's per-server device id: 64 hex digits (SHA-256).
pub fn valid_device(d: &str) -> bool {
    d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 32 random bytes from the OS, as hex.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    os_random(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn token_hash(token: &str) -> String {
    sha1_smol::Sha1::from(token.trim().as_bytes()).digest().to_string()
}

#[cfg(windows)]
fn os_random(buf: &mut [u8]) {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        // RtlGenRandom
        fn SystemFunction036(buffer: *mut u8, length: u32) -> u8;
    }
    let ok = unsafe { SystemFunction036(buf.as_mut_ptr(), buf.len() as u32) };
    assert!(ok != 0, "RtlGenRandom failed");
}

#[cfg(not(windows))]
fn os_random(buf: &mut [u8]) {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).expect("/dev/urandom");
}

impl Store {
    /// Creates the account and its first device token. Err: a message for the player.
    pub fn register_account(&self, name: &str, discriminator: &str, device: &str) -> Result<(Account, String), String> {
        if !valid_name(name) {
            return Err("The name must have 1 to 9 letters or digits and start with a letter.".into());
        }
        if !valid_discriminator(discriminator) {
            return Err("Invalid fingerprint (5 digits expected).".into());
        }
        if !valid_device(device) {
            return Err("Invalid device id.".into());
        }
        let device = token_hash(&device.to_ascii_lowercase());
        if let Some(existing) = self.account_by_device(&device) {
            return Err(format!("This PC already has the account {} on this server.", existing.gamertag()));
        }
        if self.account_by_name(name, discriminator).is_some() {
            return Err(format!("{name}#{discriminator} already exists on this server."));
        }
        let gamertag = format!("{name}#{discriminator}");
        let xuid = self.create_user(&gamertag)?;
        let token = new_token();
        let now = unix_ms();
        self.with(|db| {
            db.execute(
                "INSERT INTO accounts (xuid, name, discriminator, device, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, name, discriminator, device, now],
            )?;
            db.execute(
                "INSERT INTO devices (token_hash, xuid, created, last_seen) VALUES (?1, ?2, ?3, ?3)",
                params![token_hash(&token), xuid as i64, now],
            )
        })
        .ok_or("Could not save the account.")?;
        Ok((Account { xuid, name: name.into(), discriminator: discriminator.into(), banned: false }, token))
    }

    fn account_row(r: &rusqlite::Row) -> rusqlite::Result<Account> {
        Ok(Account {
            xuid: r.get::<_, i64>(0)? as u64,
            name: r.get(1)?,
            discriminator: r.get(2)?,
            banned: r.get::<_, i64>(3)? != 0,
        })
    }

    pub fn account_by_name(&self, name: &str, discriminator: &str) -> Option<Account> {
        self.with(|db| {
            db.query_row(
                "SELECT xuid, name, discriminator, banned FROM accounts WHERE name = ?1 COLLATE NOCASE AND discriminator = ?2",
                params![name, discriminator],
                Self::account_row,
            )
            .optional()
        })
        .flatten()
    }

    fn account_by_device(&self, device_hash: &str) -> Option<Account> {
        self.with(|db| {
            db.query_row(
                "SELECT xuid, name, discriminator, banned FROM accounts WHERE device = ?1",
                [device_hash],
                Self::account_row,
            )
            .optional()
        })
        .flatten()
    }

    pub fn account(&self, xuid: u64) -> Option<Account> {
        self.with(|db| {
            db.query_row(
                "SELECT xuid, name, discriminator, banned FROM accounts WHERE xuid = ?1",
                [xuid as i64],
                Self::account_row,
            )
            .optional()
        })
        .flatten()
    }

    /// The account behind a device token (banned accounts included: the caller decides).
    pub fn account_by_token(&self, token: &str) -> Option<Account> {
        let token = token.trim();
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let hash = token_hash(token);
        let account = self
            .with(|db| {
                db.query_row(
                    "SELECT a.xuid, a.name, a.discriminator, a.banned FROM devices d JOIN accounts a ON a.xuid = d.xuid
                     WHERE d.token_hash = ?1",
                    [&hash],
                    Self::account_row,
                )
                .optional()
            })
            .flatten()?;
        self.with(|db| db.execute("UPDATE devices SET last_seen = ?1 WHERE token_hash = ?2", params![unix_ms(), hash]));
        Some(account)
    }

    /// A new web key for the account (the previous one stops working). Returns it in clear,
    /// once; only its hash is kept.
    pub fn rotate_web_key(&self, xuid: u64) -> Option<String> {
        let key = new_web_key();
        self.with(|db| {
            db.execute(
                "INSERT INTO web_keys (xuid, key_hash, created) VALUES (?1, ?2, ?3)
                 ON CONFLICT (xuid) DO UPDATE SET key_hash = ?2, created = ?3",
                params![xuid as i64, token_hash(&web_key_canonical(&key)), unix_ms()],
            )
        })?;
        Some(key)
    }

    /// The account for "name#12345" + web key (site login).
    pub fn account_by_web_key(&self, gamertag: &str, key: &str) -> Option<Account> {
        let (name, discriminator) = gamertag.trim().rsplit_once('#')?;
        let account = self.account_by_name(name, discriminator)?;
        let stored: String = self
            .with(|db| db.query_row("SELECT key_hash FROM web_keys WHERE xuid = ?1", [account.xuid as i64], |r| r.get(0)).optional())
            .flatten()?;
        (stored == token_hash(&web_key_canonical(key))).then_some(account)
    }

    /// Logs an existing account in on another PC (or again on this one): name#12345 plus the
    /// web key. Every token the account had stops working (the old PC goes offline), the web
    /// key is replaced (it was just typed somewhere), and the new token is returned:
    /// (account, token, new web key). The PC that created an account stays the only one that
    /// could create it: logging in never frees a PC to create another account.
    pub fn log_in_elsewhere(&self, gamertag: &str, web_key: &str) -> Result<(Account, String, String), String> {
        let account = self.account_by_web_key(gamertag, web_key).ok_or("Wrong profile or key.")?;
        if account.banned {
            return Err("Account suspended on this server.".into());
        }
        let token = new_token();
        let now = unix_ms();
        self.with(|db| {
            db.execute("DELETE FROM devices WHERE xuid = ?1", [account.xuid as i64])?;
            db.execute(
                "INSERT INTO devices (token_hash, xuid, created, last_seen) VALUES (?1, ?2, ?3, ?3)",
                params![token_hash(&token), account.xuid as i64, now],
            )
        })
        .ok_or("Could not save the sign-in.")?;
        let key = self.rotate_web_key(account.xuid).ok_or("Could not make a key.")?;
        Ok((account, token, key))
    }

    /// `from` asks to befriend `to`. Friends (and rivals) only once both asked: returns true
    /// when this request completed the pair. Pending requests older than 30 days lapse.
    pub fn request_friend(&self, from: u64, to: u64) -> bool {
        let now = unix_ms();
        let mutual = self
            .with(|db| {
                db.execute("DELETE FROM friend_requests WHERE created < ?1", [now - 30 * 86_400_000])?;
                let reverse = db
                    .query_row(
                        "SELECT 1 FROM friend_requests WHERE from_xuid = ?1 AND to_xuid = ?2",
                        params![to as i64, from as i64],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if reverse {
                    db.execute(
                        "DELETE FROM friend_requests WHERE (from_xuid = ?1 AND to_xuid = ?2) OR (from_xuid = ?2 AND to_xuid = ?1)",
                        params![from as i64, to as i64],
                    )?;
                } else {
                    db.execute(
                        "INSERT INTO friend_requests (from_xuid, to_xuid, created) VALUES (?1, ?2, ?3)
                         ON CONFLICT (from_xuid, to_xuid) DO UPDATE SET created = ?3",
                        params![from as i64, to as i64, now],
                    )?;
                }
                Ok(reverse)
            })
            .unwrap_or(false);
        if mutual {
            self.add_friend(from, to);
        }
        mutual
    }

    /// Friend requests `from` sent in the last `window_ms` (rate limit).
    pub fn friend_requests_since(&self, from: u64, window_ms: i64) -> i64 {
        self.with(|db| {
            db.query_row(
                "SELECT COUNT(*) FROM friend_requests WHERE from_xuid = ?1 AND created >= ?2",
                params![from as i64, unix_ms() - window_ms],
                |r| r.get(0),
            )
        })
        .unwrap_or(0)
    }

    pub fn set_banned(&self, xuid: u64, banned: bool) -> bool {
        self.with(|db| db.execute("UPDATE accounts SET banned = ?1 WHERE xuid = ?2", params![banned as i64, xuid as i64]))
            .is_some_and(|n| n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("Alex"));
        assert!(valid_name("A23456789"));
        assert!(!valid_name("A234567890"));
        assert!(!valid_name("1Alex"));
        assert!(!valid_name("Al ex"));
        assert!(!valid_name(""));
        assert!(valid_discriminator("00042"));
        assert!(!valid_discriminator("4242"));
    }

    #[test]
    fn register_and_token() {
        let dir = std::env::temp_dir().join(format!("fh2-accounts-{}", new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        let pc = |n: u8| format!("{n:02x}").repeat(32);
        let (acc, token) = store.register_account("Alex", "01234", &pc(1)).unwrap();
        assert_eq!(acc.gamertag(), "Alex#01234");
        assert_eq!(token.len(), 64);
        // same name#discriminator from another PC
        assert!(store.register_account("alex", "01234", &pc(2)).is_err());
        // second account on the same PC
        assert!(store.register_account("Outra", "01234", &pc(1)).is_err());
        assert!(store.register_account("Alex", "99999", &pc(3)).is_ok());
        assert!(store.register_account("Alex", "55555", "curto").is_err());
        let found = store.account_by_token(&token).unwrap();
        assert_eq!(found.xuid, acc.xuid);
        assert_eq!(store.gamertag(acc.xuid), "Alex#01234");
        assert!(store.account_by_token("nope").is_none());
        let key = store.rotate_web_key(acc.xuid).unwrap();
        assert_eq!(key.len(), 14);
        assert!(store.account_by_web_key("Alex#01234", &key).is_some());
        assert!(store.account_by_web_key("alex#01234", &key.to_lowercase().replace('-', "")).is_some());
        assert!(store.account_by_web_key("Alex#99999", &key).is_none());
        let newer = store.rotate_web_key(acc.xuid).unwrap();
        assert!(store.account_by_web_key("Alex#01234", &key).is_none() || key == newer);
        assert!(store.account_by_web_key("Alex#01234", &newer).is_some());
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
