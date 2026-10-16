//! FH2Online accounts: "name#12345", created from the game through the hook
//! (POST /infinity/v1/register). Each account is a player (users row, server-allocated XUID);
//! the hook then sends X-Fh2-Token on every request and the token, not the game's own XUID,
//! says who is playing.
//!
//! Tables:
//! - accounts: name (1-20 letters/digits, first a letter) + discriminator (5 digits, the start
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
    CREATE TABLE IF NOT EXISTS account_lang (xuid INTEGER PRIMARY KEY, lang TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS devices (
        token_hash TEXT PRIMARY KEY, xuid INTEGER NOT NULL, created INTEGER NOT NULL,
        last_seen INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS devices_xuid ON devices (xuid);
    CREATE TABLE IF NOT EXISTS friend_requests (
        from_xuid INTEGER NOT NULL, to_xuid INTEGER NOT NULL, created INTEGER NOT NULL,
        PRIMARY KEY (from_xuid, to_xuid));
    CREATE TABLE IF NOT EXISTS web_keys (
        xuid INTEGER PRIMARY KEY, key_hash TEXT NOT NULL, created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS account_expansions (
        xuid INTEGER NOT NULL, product TEXT NOT NULL, granted INTEGER NOT NULL,
        PRIMARY KEY (xuid, product));";

/// Expansions every profile on the server has: Storm Island (ContentOffers 31, whose data is in
/// the base game, MountPath game:\). The hook unlocks them in the game (azimuth_expansions.c).
pub const GRANTED_EXPANSIONS: &[&str] = &["06f06bff-35ee-4821-a289-fbc1765d6b76"];

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

/// Longest name. The game shows 15 characters of a gamertag: a longer "Name#12345" keeps its
/// number there and the name is cut to fit (Store::game_tag).
pub const MAX_NAME: usize = 20;

/// 1 to MAX_NAME ASCII letters or digits starting with a letter.
pub fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
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

/// Profiles one PC (one device id) can have on this server.
pub const MAX_PROFILES_PER_PC: usize = 3;

/// The `accounts.device` values of a PC's profile slots: its first profile keeps the device's
/// hash as before (the column is UNIQUE), the 2nd and 3rd the hash of "device#2" / "device#3".
fn device_slots(device: &str) -> [String; MAX_PROFILES_PER_PC] {
    let device = device.trim().to_ascii_lowercase();
    [token_hash(&device), token_hash(&format!("{device}#2")), token_hash(&format!("{device}#3"))]
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
            return Err("The name must have 1 to 20 letters or digits and start with a letter.".into());
        }
        if !valid_discriminator(discriminator) {
            return Err("Invalid fingerprint (5 digits expected).".into());
        }
        if !valid_device(device) {
            return Err("Invalid device id.".into());
        }
        let slots = device_slots(device);
        let mine: Vec<Account> = slots.iter().filter_map(|slot| self.account_by_device(slot)).collect();
        let Some(device) = slots.into_iter().find(|slot| self.account_by_device(slot).is_none()) else {
            let names: Vec<String> = mine.iter().map(Account::gamertag).collect();
            return Err(format!(
                "This PC already has {MAX_PROFILES_PER_PC} profiles on this server ({}). Play one of them with Switch profile, or delete one there first.",
                names.join(", ")
            ));
        };
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
        self.grant_expansions();
        Ok((Account { xuid, name: name.into(), discriminator: discriminator.into(), banned: false }, token))
    }

    /// Gives every account the GRANTED_EXPANSIONS it does not have yet (a new account, and at
    /// start the ones made before). Returns how many grants were added.
    pub fn grant_expansions(&self) -> usize {
        let now = unix_ms();
        GRANTED_EXPANSIONS
            .iter()
            .map(|product| {
                self.with(|db| {
                    db.execute(
                        "INSERT OR IGNORE INTO account_expansions (xuid, product, granted) SELECT xuid, ?1, ?2 FROM accounts",
                        params![product, now],
                    )
                })
                .unwrap_or(0)
            })
            .sum()
    }

    /// The expansions (product ids) an account has.
    pub fn expansions(&self, xuid: u64) -> Vec<String> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT product FROM account_expansions WHERE xuid = ?1 ORDER BY product")?;
            let rows = stmt.query_map([xuid as i64], |r| r.get::<_, String>(0))?.flatten().collect();
            Ok(rows)
        })
        .unwrap_or_default()
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

    /// The profiles created from this PC (its device id), oldest slot first.
    pub fn device_accounts(&self, device: &str) -> Vec<Account> {
        if !valid_device(device) {
            return Vec::new();
        }
        device_slots(device).iter().filter_map(|slot| self.account_by_device(slot)).collect()
    }

    /// The PC that created a profile plays it again without its web key (its device id is the
    /// proof): as a sign-in, every other token of the account stops working and the web key is
    /// replaced. For a player whose PC lost FH2Online_Accounts.ini, or whose profile was
    /// signed in elsewhere: (account, token, new web key).
    pub fn recover_on_device(&self, device: &str, xuid: u64) -> Result<(Account, String, String), String> {
        let account = self
            .device_accounts(device)
            .into_iter()
            .find(|a| a.xuid == xuid)
            .ok_or("This profile was not created on this PC: sign in with it and its site key.")?;
        if account.banned {
            return Err("Account suspended on this server.".into());
        }
        let token = self.replace_tokens(account.xuid).ok_or("Could not save the sign-in.")?;
        let key = self.rotate_web_key(account.xuid).ok_or("Could not make a key.")?;
        Ok((account, token, key))
    }

    /// The accounts behind token hashes (SHA-1 hex, as `devices` keeps them): a PC that played
    /// this server under another address (127.0.0.1, its LAN address, a name) asks which of
    /// its saved profiles are this server's, without sending the tokens themselves.
    pub fn accounts_by_token_hash(&self, hashes: &[String]) -> Vec<(String, Account)> {
        hashes
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .filter_map(|h| {
                self.with(|db| {
                    db.query_row(
                        "SELECT a.xuid, a.name, a.discriminator, a.banned FROM devices d JOIN accounts a ON a.xuid = d.xuid
                         WHERE d.token_hash = ?1",
                        [&h],
                        Self::account_row,
                    )
                    .optional()
                })
                .flatten()
                .map(|a| (h, a))
            })
            .collect()
    }

    /// Every token of the account is dropped for one new one, returned.
    fn replace_tokens(&self, xuid: u64) -> Option<String> {
        let token = new_token();
        let now = unix_ms();
        self.with(|db| {
            db.execute("DELETE FROM devices WHERE xuid = ?1", [xuid as i64])?;
            db.execute(
                "INSERT INTO devices (token_hash, xuid, created, last_seen) VALUES (?1, ?2, ?3, ?3)",
                params![token_hash(&token), xuid as i64, now],
            )
        })?;
        Some(token)
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

    /// When the account was made (ms), for the gates' arrival window.
    pub fn account_created(&self, xuid: u64) -> Option<i64> {
        self.with(|db| db.query_row("SELECT created FROM accounts WHERE xuid = ?1", [xuid as i64], |r| r.get(0)).optional())
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
        let token = self.replace_tokens(account.xuid).ok_or("Could not save the sign-in.")?;
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
    /// Where `me` stands with `other`: "friends" (both added each other), "requested" (`me`
    /// asked), "incoming" (`other` asked) or "none".
    pub fn friend_state(&self, me: u64, other: u64) -> &'static str {
        if self.friends(me).iter().any(|f| f.0 == other && f.2) {
            return "friends";
        }
        let asked = |from: u64, to: u64| {
            self.with(|db| {
                db.query_row(
                    "SELECT 1 FROM friend_requests WHERE from_xuid = ?1 AND to_xuid = ?2",
                    params![from as i64, to as i64],
                    |_| Ok(()),
                )
                .optional()
            })
            .flatten()
            .is_some()
        };
        if asked(me, other) {
            "requested"
        } else if asked(other, me) {
            "incoming"
        } else {
            "none"
        }
    }

    /// Ends a friendship and any request between the two (either way). True when something
    /// was there.
    pub fn unfriend(&self, a: u64, b: u64) -> bool {
        let requests = self
            .with(|db| {
                db.execute(
                    "DELETE FROM friend_requests WHERE (from_xuid = ?1 AND to_xuid = ?2) OR (from_xuid = ?2 AND to_xuid = ?1)",
                    params![a as i64, b as i64],
                )
            })
            .unwrap_or(0);
        self.remove_friend(a, b) || requests > 0
    }

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

    /// The game language the account's hook last reported (X-Fh2-Lang), "" when none.
    pub fn lang(&self, xuid: u64) -> String {
        self.with(|db| db.query_row("SELECT lang FROM account_lang WHERE xuid = ?1", [xuid as i64], |r| r.get(0)).optional())
            .flatten()
            .unwrap_or_default()
    }

    pub fn set_lang(&self, xuid: u64, lang: &str) {
        let lang: String = lang.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect();
        self.with(|db| {
            db.execute(
                "INSERT INTO account_lang (xuid, lang) VALUES (?1, ?2) ON CONFLICT (xuid) DO UPDATE SET lang = ?2",
                params![xuid as i64, lang],
            )
        });
    }

    /// A player everyone sees (leaderboards, ghosts, drivatars, shared files): an account that
    /// is not banned.
    pub fn is_public_player(&self, xuid: u64) -> bool {
        self.with(|db| {
            db.query_row("SELECT banned FROM accounts WHERE xuid = ?1", [xuid as i64], |r| r.get::<_, i64>(0)).optional()
        })
        .flatten()
            == Some(0)
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
        assert!(valid_name("DominicToretto"));
        assert!(valid_name("A2345678901234567890"));
        assert!(!valid_name("A23456789012345678901"));
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
        // a PC has up to three profiles on a server
        let (second, second_token) = store.register_account("Outra", "01234", &pc(1)).unwrap();
        let (third, _) = store.register_account("Terceira", "01234", &pc(1)).unwrap();
        let full = store.register_account("Quarta", "01234", &pc(1)).err().unwrap();
        assert!(full.contains("Alex#01234") && full.contains("Terceira#01234"), "{full}");
        let mine: Vec<u64> = store.device_accounts(&pc(1)).iter().map(|a| a.xuid).collect();
        assert_eq!(mine, vec![acc.xuid, second.xuid, third.xuid]);
        assert_eq!(store.device_accounts(&pc(1).to_uppercase()).len(), 3);
        assert!(store.device_accounts(&pc(9)).is_empty());
        // the PC that made a profile plays it again without its key; the old token stops
        let (back, new_token_, key) = store.recover_on_device(&pc(1), second.xuid).unwrap();
        assert_eq!(back.xuid, second.xuid);
        assert!(store.account_by_token(&second_token).is_none());
        assert_eq!(store.account_by_token(&new_token_).unwrap().xuid, second.xuid);
        assert!(store.account_by_web_key("Outra#01234", &key).is_some());
        assert!(store.recover_on_device(&pc(2), second.xuid).is_err());
        // which saved tokens are this server's, by their hash
        let known = store.accounts_by_token_hash(&[token_hash(&token), token_hash(&second_token), "zz".into()]);
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].1.xuid, acc.xuid);
        // a deleted profile frees its slot
        assert!(store.delete_user(third.xuid));
        assert!(store.register_account("Quarta", "01234", &pc(1)).is_ok());
        assert!(store.register_account("Alex", "99999", &pc(3)).is_ok());
        assert!(store.register_account("Alex", "55555", "curto").is_err());
        let found = store.account_by_token(&token).unwrap();
        assert_eq!(found.xuid, acc.xuid);
        assert_eq!(store.gamertag(acc.xuid), "Alex#01234");
        assert!(store.account_by_token("nope").is_none());
        // every profile has Storm Island: a new one at once, older ones from the backfill
        assert_eq!(store.expansions(acc.xuid), GRANTED_EXPANSIONS);
        store.with(|db| db.execute("DELETE FROM account_expansions", []));
        assert!(store.expansions(acc.xuid).is_empty());
        assert_eq!(store.grant_expansions(), 4); // the four accounts left above
        assert_eq!(store.expansions(acc.xuid), GRANTED_EXPANSIONS);
        assert_eq!(store.grant_expansions(), 0);
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
