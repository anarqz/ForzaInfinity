//! Abuse limits: open connections per address and token-bucket rate limits per (address,
//! bucket). Everything is in memory and forgotten on restart.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::Instant,
};

/// At most this many connections open at once, in total and per address.
pub const MAX_CONNECTIONS: usize = 512;
pub const MAX_CONNECTIONS_PER_IP: usize = 24;

/// (burst, refill per second) of each bucket.
#[derive(Clone, Copy)]
pub enum Bucket {
    /// Proxied game traffic: the boot sequence fires a few hundred requests.
    Game,
    /// Account endpoints that only read or change the caller's own account.
    Account,
    /// Account creation: a handful per address per hour.
    Register,
    /// Checks of a name + web key: slow enough that guessing is pointless.
    Login,
    /// Cloud save uploads (PUT /infinity/v1/save, up to db::saves::MAX_BUNDLE each).
    Save,
}

impl Bucket {
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Game => "game",
            Bucket::Account => "account",
            Bucket::Register => "register",
            Bucket::Login => "login",
            Bucket::Save => "save",
        }
    }

    fn limits(self) -> (f64, f64) {
        match self {
            Bucket::Game => (400.0, 25.0),
            // per account when the request carries its token (api::allowed), else per address
            Bucket::Account => (60.0, 2.0),
            Bucket::Register => (3.0, 3.0 / 3600.0),
            Bucket::Login => (10.0, 10.0 / 600.0),
            Bucket::Save => (5.0, 1.0 / 30.0),
        }
    }
}

#[derive(Default)]
pub struct Guard {
    connections: Mutex<HashMap<IpAddr, usize>>,
    buckets: Mutex<HashMap<(IpAddr, u8), (f64, Instant)>>,
    /// Refusals logged: (key, bucket) -> (when last logged, refusals since).
    refusals: Mutex<HashMap<(IpAddr, u8), (Instant, u32)>>,
}

/// A refused request is logged at most this often per (key, bucket); the line counts the rest.
const REFUSAL_LOG_EVERY_SECS: u64 = 60;

/// Releases the connection slot when dropped.
pub struct Slot<'a> {
    guard: &'a Guard,
    ip: IpAddr,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut open = self.guard.connections.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(n) = open.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                open.remove(&self.ip);
            }
        }
    }
}

impl Guard {
    /// A slot for a new connection from `ip`, or None when over the limits (`per_ip`: the
    /// per-address one too).
    pub fn connect(&self, ip: IpAddr, per_ip: bool) -> Option<Slot<'_>> {
        let mut open = self.connections.lock().unwrap_or_else(|p| p.into_inner());
        let total: usize = open.values().sum();
        let mine = open.entry(ip).or_insert(0);
        if total >= MAX_CONNECTIONS || (per_ip && *mine >= MAX_CONNECTIONS_PER_IP) {
            if *mine == 0 {
                open.remove(&ip);
            }
            return None;
        }
        *mine += 1;
        Some(Slot { guard: self, ip })
    }

    /// Takes one token from `ip`'s bucket; false when it is empty. Loopback is never limited
    /// (the manager and local tests).
    /// A request refused by `allow`: Some(refusals since the last line) when a line is due.
    pub fn refusal(&self, key: IpAddr, bucket: Bucket) -> Option<u32> {
        let now = Instant::now();
        let mut refusals = self.refusals.lock().unwrap_or_else(|p| p.into_inner());
        if refusals.len() > 10_000 {
            refusals.retain(|_, (at, _)| now.duration_since(*at).as_secs() < REFUSAL_LOG_EVERY_SECS);
        }
        match refusals.get_mut(&(key, bucket as u8)) {
            Some((at, held)) if now.duration_since(*at).as_secs() < REFUSAL_LOG_EVERY_SECS => {
                *held += 1;
                None
            }
            Some((at, held)) => {
                let before = *held;
                *at = now;
                *held = 0;
                Some(before)
            }
            None => {
                refusals.insert((key, bucket as u8), (now, 0));
                Some(0)
            }
        }
    }

    pub fn allow(&self, ip: IpAddr, bucket: Bucket) -> bool {
        if ip.is_loopback() {
            return true;
        }
        let (burst, rate) = bucket.limits();
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if buckets.len() > 50_000 {
            // forget addresses whose bucket has refilled anyway
            buckets.retain(|&(_, b), (tokens, at)| {
                let (burst, rate) = bucket_by_id(b).limits();
                *tokens + now.duration_since(*at).as_secs_f64() * rate < burst
            });
        }
        let (tokens, at) = buckets.entry((ip, bucket as u8)).or_insert((burst, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * rate).min(burst);
        *at = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }
}

fn bucket_by_id(id: u8) -> Bucket {
    match id {
        0 => Bucket::Game,
        1 => Bucket::Account,
        2 => Bucket::Register,
        3 => Bucket::Login,
        _ => Bucket::Save,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn refusals_are_logged_once_a_minute_with_the_count() {
        let g = Guard::default();
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        assert_eq!(g.refusal(ip, Bucket::Register), Some(0));
        assert_eq!(g.refusal(ip, Bucket::Register), None);
        assert_eq!(g.refusal(ip, Bucket::Register), None);
        // another bucket or address gets its own line
        assert_eq!(g.refusal(ip, Bucket::Login), Some(0));
        assert_eq!(g.refusal("203.0.113.10".parse().unwrap(), Bucket::Register), Some(0));
        // a minute later: one line, counting the two held back
        g.refusals.lock().unwrap().get_mut(&(ip, Bucket::Register as u8)).unwrap().0 -=
            std::time::Duration::from_secs(REFUSAL_LOG_EVERY_SECS);
        assert_eq!(g.refusal(ip, Bucket::Register), Some(2));
    }

    use super::*;

    #[test]
    fn limits() {
        let g = Guard::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let slots: Vec<_> = (0..MAX_CONNECTIONS_PER_IP).map(|_| g.connect(ip, true).unwrap()).collect();
        assert!(g.connect(ip, true).is_none());
        drop(slots);
        assert!(g.connect(ip, true).is_some());
        assert!((0..3).all(|_| g.allow(ip, Bucket::Register)));
        assert!(!g.allow(ip, Bucket::Register));
        assert!(g.allow("127.0.0.1".parse().unwrap(), Bucket::Register));
    }

    #[test]
    fn shared_proxy() {
        let g = Guard::default();
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        let slots: Vec<_> = (0..MAX_CONNECTIONS_PER_IP * 2).map(|_| g.connect(lo, false).unwrap()).collect();
        assert!(g.connect(lo, true).is_none());
        assert_eq!(slots.len(), MAX_CONNECTIONS_PER_IP * 2);
    }
}
