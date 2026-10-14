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
    fn limits(self) -> (f64, f64) {
        match self {
            Bucket::Game => (400.0, 25.0),
            Bucket::Account => (30.0, 0.5),
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
}

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
