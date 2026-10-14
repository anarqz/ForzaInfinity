//! Player slots and the waiting line (--max-players). Everything is in memory: a restart
//! empties both, and every player simply takes a slot or a place again.
//!
//! - A slot belongs to an account while it keeps making requests. It is freed after IDLE
//!   without one (the game closed or crashed); coming back within IDLE keeps it.
//! - Without a free slot the account waits in line, first come first served. It stays there
//!   while it asks (GET /infinity/v1/queue, or any game request) at least every STALE.
//! - The first `free slots` places of the line take a slot when they next ask, so nobody can
//!   jump the line, and a place that stops asking is dropped after STALE.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

/// A slot is freed this long after the account's last request.
pub const IDLE: Duration = Duration::from_secs(5 * 60);
/// A place in line is dropped this long after its last question.
pub const STALE: Duration = Duration::from_secs(90);
/// At most this many accounts wait; more are told the server is full.
pub const MAX_LINE: usize = 20_000;
/// Admissions from the line over this window give the wait estimate.
const RATE_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Has a slot: serve it.
    Playing,
    /// Waiting: 1-based position in line.
    Waiting(usize),
    /// The line itself is full.
    Full,
}

#[derive(Debug, Clone, Copy)]
pub struct Status {
    pub place: Place,
    pub online: usize,
    pub waiting: usize,
    pub capacity: usize,
    /// Seconds until the slot, from the recent pace (None: no pace yet).
    pub eta_s: Option<u64>,
    /// When to ask again, in seconds (later for places further back).
    pub retry_s: u64,
}

pub struct Admission {
    /// 0: no limit.
    capacity: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// xuid -> last request.
    playing: HashMap<u64, Instant>,
    /// Waiting xuids, in order.
    line: VecDeque<u64>,
    /// xuid -> last question, for every xuid in `line`.
    asked: HashMap<u64, Instant>,
    /// xuid -> its index in `line` (rebuilt whenever the line closes up).
    index: HashMap<u64, usize>,
    /// When each recent admission from the line happened.
    admitted: VecDeque<Instant>,
    /// The last sweep of idle slots and stale places (at most one a second: the line can be
    /// long, and every request comes through here).
    swept: Option<Instant>,
}

impl Admission {
    pub fn new(capacity: usize) -> Admission {
        Admission { capacity, state: Mutex::new(State::default()) }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// A request from `xuid`: takes or keeps its slot, or keeps its place in line.
    pub fn arrive(&self, xuid: u64) -> Status {
        self.arrive_at(xuid, Instant::now())
    }

    /// (playing, waiting) right now.
    pub fn counts(&self) -> (usize, usize) {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.expire(now);
        (s.playing.len(), s.line.len())
    }

    fn arrive_at(&self, xuid: u64, now: Instant) -> Status {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.expire(now);
        let place = if self.capacity == 0 || s.playing.contains_key(&xuid) {
            s.playing.insert(xuid, now);
            Place::Playing
        } else {
            let free = self.capacity.saturating_sub(s.playing.len());
            let index = match s.index.get(&xuid) {
                Some(&i) => Some(i),
                None if s.line.is_empty() && free > 0 => None, // nobody waits: straight in
                None if s.line.len() >= MAX_LINE => return s.status(self.capacity, Place::Full, now),
                None => {
                    s.line.push_back(xuid);
                    let i = s.line.len() - 1;
                    s.index.insert(xuid, i);
                    Some(i)
                }
            };
            match index {
                Some(i) if i < free => {
                    s.line.remove(i);
                    s.asked.remove(&xuid);
                    s.reindex();
                    s.admitted.push_back(now);
                    s.playing.insert(xuid, now);
                    Place::Playing
                }
                Some(i) => {
                    s.asked.insert(xuid, now);
                    Place::Waiting(i + 1)
                }
                None => {
                    s.playing.insert(xuid, now);
                    Place::Playing
                }
            }
        };
        s.status(self.capacity, place, now)
    }
}

impl State {
    fn expire(&mut self, now: Instant) {
        if self.swept.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1)) {
            return;
        }
        self.swept = Some(now);
        self.playing.retain(|_, at| now.duration_since(*at) < IDLE);
        // `asked` holds exactly the xuids in line, so dropping the stale ones from both keeps
        // them in step
        let before = self.line.len();
        self.asked.retain(|_, at| now.duration_since(*at) < STALE);
        let asked = &self.asked;
        self.line.retain(|x| asked.contains_key(x));
        if self.line.len() != before {
            self.reindex();
        }
        while self.admitted.front().is_some_and(|at| now.duration_since(*at) > RATE_WINDOW) {
            self.admitted.pop_front();
        }
    }

    fn reindex(&mut self) {
        self.index = self.line.iter().enumerate().map(|(i, &x)| (x, i)).collect();
    }

    fn status(&self, capacity: usize, place: Place, now: Instant) -> Status {
        // the pace of the line: admissions since the oldest recent one (at least a minute)
        let eta_s = match (place, self.admitted.front()) {
            (Place::Waiting(n), Some(first)) => {
                let span = now.duration_since(*first).as_secs_f64().max(60.0);
                let rate = self.admitted.len() as f64 / span; // per second
                Some((n as f64 / rate).ceil() as u64)
            }
            _ => None,
        };
        let retry_s = match place {
            Place::Waiting(n) if n <= 50 => 10,
            Place::Waiting(n) if n <= 500 => 30,
            _ => 60,
        };
        Status { place, online: self.playing.len(), waiting: self.line.len(), capacity, eta_s, retry_s }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_and_line() {
        let a = Admission::new(2);
        let t = Instant::now();
        assert_eq!(a.arrive_at(1, t).place, Place::Playing);
        assert_eq!(a.arrive_at(2, t).place, Place::Playing);
        assert_eq!(a.arrive_at(3, t).place, Place::Waiting(1));
        assert_eq!(a.arrive_at(4, t).place, Place::Waiting(2));
        // a player who keeps playing keeps the slot; the line does not move
        assert_eq!(a.arrive_at(1, t + Duration::from_secs(60)).place, Place::Playing);
        assert_eq!(a.arrive_at(3, t + Duration::from_secs(60)).place, Place::Waiting(1));
        assert_eq!(a.arrive_at(4, t + Duration::from_secs(60)).place, Place::Waiting(2));
        // 2 goes quiet: its slot frees after IDLE, and only the head of the line may take it
        // (3 and 4 keep asking each minute meanwhile)
        for m in 2..=4 {
            let now = t + Duration::from_secs(60 * m);
            a.arrive_at(1, now);
            a.arrive_at(3, now);
            a.arrive_at(4, now);
        }
        let later = t + IDLE + Duration::from_secs(1);
        a.arrive_at(1, later);
        assert_eq!(a.arrive_at(4, later).place, Place::Waiting(2));
        assert_eq!(a.arrive_at(3, later).place, Place::Playing);
        assert_eq!(a.arrive_at(4, later).place, Place::Waiting(1));
    }

    #[test]
    fn stale_places_drop_out() {
        let a = Admission::new(1);
        let t = Instant::now();
        a.arrive_at(1, t);
        assert_eq!(a.arrive_at(2, t).place, Place::Waiting(1));
        assert_eq!(a.arrive_at(3, t).place, Place::Waiting(2));
        // 2 stops asking; 3 keeps asking and moves up
        assert_eq!(a.arrive_at(3, t + Duration::from_secs(60)).place, Place::Waiting(2));
        let t2 = t + STALE + Duration::from_secs(1);
        assert_eq!(a.arrive_at(3, t2).place, Place::Waiting(1));
        // 2 comes back: the back of the line
        assert_eq!(a.arrive_at(2, t2).place, Place::Waiting(2));
    }

    #[test]
    fn no_limit() {
        let a = Admission::new(0);
        let t = Instant::now();
        assert!((0..1000).all(|x| a.arrive_at(x, t).place == Place::Playing));
    }

    /// 3000 accounts at once on 150 slots: 150 play, 2850 wait in arrival order, and players
    /// leaving let the line in at the pace they leave.
    #[test]
    fn crowd() {
        let a = Admission::new(150);
        let t = Instant::now();
        for x in 0..3000u64 {
            let s = a.arrive_at(x, t);
            if x < 150 {
                assert_eq!(s.place, Place::Playing);
            } else {
                assert_eq!(s.place, Place::Waiting(x as usize - 149));
            }
        }
        let s = a.arrive_at(2999, t);
        assert_eq!((s.online, s.waiting, s.capacity), (150, 2850, 150));
        assert_eq!(s.retry_s, 60);
        assert_eq!(a.arrive_at(150, t).retry_s, 10);

        // everyone keeps asking each minute; players 0..30 leave (stop asking) at once
        let mut now = t;
        for _ in 0..5 {
            now += Duration::from_secs(60);
            for x in 30..3000u64 {
                a.arrive_at(x, now);
            }
        }
        now += Duration::from_secs(60); // past IDLE for players 0..30
        let mut admitted = 0;
        for x in 30..3000u64 {
            if x >= 150 && a.arrive_at(x, now).place == Place::Playing {
                admitted += 1;
                assert!(x < 180, "only the first 30 of the line get in, not {x}");
            } else {
                a.arrive_at(x, now);
            }
        }
        assert_eq!(admitted, 30);
        let s = a.arrive_at(2999, now);
        assert_eq!((s.online, s.waiting), (150, 2820));
        assert_eq!(s.place, Place::Waiting(2820));
        assert!(s.eta_s.is_none() || s.eta_s.unwrap() > 0);
    }

    #[test]
    fn line_has_an_end() {
        let a = Admission::new(1);
        let t = Instant::now();
        a.arrive_at(u64::MAX, t);
        for x in 0..MAX_LINE as u64 {
            a.arrive_at(x, t);
        }
        assert_eq!(a.arrive_at(MAX_LINE as u64, t).place, Place::Full);
    }
}
