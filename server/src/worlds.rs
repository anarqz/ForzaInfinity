//! Player-hosted worlds (PLANO_FREEROAM.md, phase F1). A player clicks HOSPEDAR MUNDO; the hook
//! starts dedicated_server (the relay) on their PC and announces it here every 15 s. This
//! server then sends players to the right world (freeroam.rs: `Freeroam::world_for`).
//!
//! Account routes (X-Fh2-Token; api.rs), under /infinity/v1/ (and the legacy /fh2online/v1/):
//!   POST   worlds                {name, message, visibility, password, maxPlayers, port, lan,
//!                                 version [, banned]} -> {id, address, players, expiresIn}
//!   DELETE worlds                -> 204 (the host stops; members are released)
//!   GET    worlds                -> {worlds: [{id, name, message, host, players, maxPlayers,
//!                                 friendsOnly, sameNetwork}]}
//!   GET    worlds/of/<xuid>      -> {world: null | {id, name, host, players, maxPlayers,
//!                                 password, canJoin, why}}
//!   POST   worlds/<id>/join      {password} -> {ok: true}: the caller's next FREE ROAM ONLINE
//!                                 goes there
//!   POST   worlds/punch          {candidates} -> {world: [ip:port...]} (F3): a joining player's
//!                                 proxy sends its public addresses (STUN) and gets the world's
//!                                 (its LAN, STUN and UPnP addresses); the host is asked to punch
//!                                 towards the player's
//!   GET    worlds/punches        -> {punch: [ip:port...]}: the host's pending punches (also in
//!                                 the heartbeat's reply), polled every few seconds
//!
//! POST worlds also takes `public` (the relay's STUN and UPnP addresses, dedicated_server.net)
//! and `nat` (cone, symmetric or unknown). A public IPv4 address, the host's or a player's, must
//! be the one this server sees them at: nobody can point the others' punches at someone else.
//!
//! Everything is in memory: a world without a heartbeat for 45 s is gone. Passwords are kept
//! only as a salted SHA-1 and never logged.

use std::{
    cell::OnceCell,
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use crate::freeroam::Reply;

/// A world without a heartbeat for this long is gone (the hook sends one every 15 s).
pub const WORLD_TTL: Duration = Duration::from_secs(45);
/// A join choice waits this long for the player's next FREE ROAM ONLINE.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
/// A player sent to a world counts in it this long after their last SmartMatch ticket.
const MEMBER_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_WORLDS: usize = 256;
const MAX_MEMBERS: usize = 4096;
const MAX_PENDING: usize = 4096;
const MAX_FAILURES: usize = 4096;
/// Wrong passwords a player may send in FAILURE_WINDOW (then 429).
const MAX_WRONG_PASSWORDS: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// The game's free roam session holds 12 (gsiSets.maxAllowedPlayers).
const MAX_PLAYERS: u64 = 12;
const MAX_LAN: usize = 4;
/// Public addresses per host or player (STUN IPv4, STUN IPv6, UPnP).
const MAX_PUBLIC: usize = 3;
/// Punches waiting for one host, and how long one waits.
const MAX_PUNCHES: usize = 64;
const PUNCH_TTL: Duration = Duration::from_secs(30);
const MAX_BANNED: usize = 100;
/// Most worlds in one list.
const MAX_LIST: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    Public,
    Friends,
    Off,
}

impl Visibility {
    fn name(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Friends => "friends",
            Visibility::Off => "off",
        }
    }
}

struct World {
    name: String,
    message: String,
    visibility: Visibility,
    /// On the list and in the automatic join. A private session (the game's "Sessão online
    /// particular") is not: only ENTRAR NO MUNDO reaches it.
    listed: bool,
    salt: String,
    /// SHA-1 hex of salt:password; None: no password.
    password: Option<String>,
    max_players: usize,
    port: u16,
    lan: Vec<IpAddr>,
    version: String,
    /// The host's car database (Caller::car_db).
    car_db: Option<String>,
    /// The host's address as this server sees it.
    host_ip: IpAddr,
    /// Its relay's public addresses (STUN, UPnP) and NAT kind (F3).
    public: Vec<SocketAddr>,
    nat: String,
    heartbeat: Instant,
    banned: Vec<u64>,
}

/// Who is asking: the account, its address as this server sees it, and lookups in the store.
pub struct Caller<'a> {
    pub xuid: u64,
    pub ip: IpAddr,
    gamertag: &'a dyn Fn(u64) -> String,
    friends: &'a dyn Fn(u64) -> Vec<u64>,
    friend_list: OnceCell<Vec<u64>>,
    /// The SHA-256 of the caller's car database (X-Fh2-CarDb): players only meet with the
    /// same cars installed. None: an older hook, not checked.
    car_db: Option<String>,
}

impl<'a> Caller<'a> {
    /// `friends(x)`: the players x is friends with (both added each other).
    pub fn new(xuid: u64, ip: IpAddr, gamertag: &'a dyn Fn(u64) -> String, friends: &'a dyn Fn(u64) -> Vec<u64>) -> Self {
        Caller { xuid, ip: ip.to_canonical(), gamertag, friends, friend_list: OnceCell::new(), car_db: None }
    }

    /// The X-Fh2-CarDb header (64 hex digits; anything else is ignored).
    pub fn with_car_db(mut self, header: Option<&str>) -> Self {
        self.car_db = header
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())) || h == CAR_DB_CHANGED);
        self
    }

    pub fn gamertag(&self, xuid: u64) -> String {
        (self.gamertag)(xuid)
    }

    /// The name the game shows (15 characters at most), as in the free roam sessions.
    fn game_tag(&self, xuid: u64) -> String {
        match self.gamertag(xuid) {
            t if t.is_empty() => format!("Driver {}", xuid % 10000),
            t => crate::db::game_tag(&t),
        }
    }

    fn is_friend(&self, other: u64) -> bool {
        self.friend_list.get_or_init(|| (self.friends)(self.xuid)).contains(&other)
    }
}

/// How `choose` picked a world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Via {
    /// The caller hosts it.
    Hosting,
    /// The caller chose it (POST worlds/<id>/join).
    Pending,
    /// The caller was sent there before and still counts in it.
    Member,
    /// The best open world for the caller.
    Auto,
}

#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub addr: SocketAddr,
    pub host: u64,
    pub via: Via,
}

#[derive(Default)]
struct State {
    /// By host xuid (one world per account).
    worlds: HashMap<u64, World>,
    /// player -> (host, last assigned)
    members: HashMap<u64, (u64, Instant)>,
    /// player -> (host, chosen)
    pending: HashMap<u64, (u64, Instant)>,
    /// player -> (wrong passwords, first of them)
    failures: HashMap<u64, (u32, Instant)>,
    /// host -> players its relay must drop (handed over in the next heartbeat's reply).
    kicks: HashMap<u64, Vec<u64>>,
    /// host -> (player's address, asked at) its relay should punch towards (F3).
    punches: HashMap<u64, Vec<(SocketAddr, Instant)>>,
}

impl State {
    fn purge(&mut self, now: Instant) {
        self.worlds.retain(|host, w| {
            let live = now.saturating_duration_since(w.heartbeat) < WORLD_TTL;
            if !live {
                println!("worlds: the world of {host} expired (no heartbeat for {} s)", WORLD_TTL.as_secs());
            }
            live
        });
        let worlds = &self.worlds;
        self.members.retain(|player, (host, at)| {
            worlds.contains_key(host) && player != host && now.saturating_duration_since(*at) < MEMBER_TTL
        });
        self.pending.retain(|_, (host, at)| worlds.contains_key(host) && now.saturating_duration_since(*at) < PENDING_TTL);
        self.failures.retain(|_, (_, at)| now.saturating_duration_since(*at) < FAILURE_WINDOW);
        let worlds = &self.worlds;
        self.kicks.retain(|host, _| worlds.contains_key(host));
        self.punches.retain(|host, list| {
            list.retain(|(_, at)| now.saturating_duration_since(*at) < PUNCH_TTL);
            worlds.contains_key(host) && !list.is_empty()
        });
    }

    /// Hands over (and forgets) the host's pending punches.
    fn take_punches(&mut self, host: u64) -> Vec<String> {
        self.punches.remove(&host).unwrap_or_default().iter().map(|(a, _)| a.to_string()).collect()
    }

    /// The host and the players sent there.
    fn players(&self, host: u64) -> usize {
        1 + self.members.values().filter(|(h, _)| *h == host).count()
    }

    fn is_member(&self, player: u64, host: u64) -> bool {
        self.members.get(&player).is_some_and(|(h, _)| *h == host)
    }

    /// Why the caller cannot be in the world of `host` ("" = they can; the password is apart).
    fn why(&self, host: u64, w: &World, caller: &Caller) -> &'static str {
        if caller.xuid == host {
            return "";
        }
        if cars_differ(w, caller) {
            return "cars";
        }
        if w.visibility == Visibility::Off {
            return "off";
        }
        if w.banned.contains(&caller.xuid) {
            return "banned";
        }
        if self.is_member(caller.xuid, host) {
            return "";
        }
        if w.visibility == Visibility::Friends && !caller.is_friend(host) {
            return "friends";
        }
        if self.players(host) >= w.max_players {
            return "full";
        }
        ""
    }

    /// The host itself: loopback (the relay runs on its PC). The same public address: the
    /// host's first LAN address. A host seen on loopback (a server on the host's own PC,
    /// reached at 127.0.0.1) is only reachable from the others by its LAN address too.
    /// Anybody else: the host's public address.
    fn address(host: u64, w: &World, caller: &Caller) -> SocketAddr {
        let ip = if caller.xuid == host {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else if caller.ip == w.host_ip || w.host_ip.is_loopback() {
            w.lan.first().copied().unwrap_or(w.host_ip)
        } else {
            w.host_ip
        };
        SocketAddr::new(ip, w.port)
    }
}

/// A private network address (security review L1): RFC 1918 and link-local IPv4, unique
/// local (fc00::/7) and link-local (fe80::/10) IPv6.
fn is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// The `public` (or `candidates`) addresses of a host or a player: at most MAX_PUBLIC
/// `ip:port`; IPv4 only at the address this server sees them at (`seen`), IPv6 only global.
/// A caller on loopback or a LAN address (a server on the same network) may send any, and one
/// seen over IPv6 (its HTTPS goes over IPv6, its game may well use IPv4) one IPv4 address.
fn public_addresses(v: &Value, seen: IpAddr) -> Result<Vec<SocketAddr>, String> {
    let list = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(a) if a.len() <= MAX_PUBLIC => a,
        _ => return Err(format!("public: at most {MAX_PUBLIC} addresses")),
    };
    let mut out: Vec<SocketAddr> = Vec::new();
    for item in list {
        let Some(a) = item.as_str().and_then(|s| s.trim().parse::<SocketAddr>().ok()) else {
            return Err("public: ip:port addresses".into());
        };
        let a = SocketAddr::new(a.ip().to_canonical(), a.port());
        let ok = a.port() != 0
            && match a.ip() {
                IpAddr::V4(v4) => {
                    seen.is_loopback()
                        || is_lan(&seen)
                        || (IpAddr::V4(v4) == seen && !v4.is_unspecified())
                        || (seen.is_ipv6() && !v4.is_unspecified() && !out.iter().any(|o| o.is_ipv4() && o.ip() != a.ip()))
                }
                IpAddr::V6(v6) => {
                    let first = v6.segments()[0];
                    // 2000::/3, the global unicast range
                    (first & 0xe000) == 0x2000
                }
            };
        if !ok {
            return Err("public: an IPv4 address must be the one this server sees you at".into());
        }
        if !out.contains(&a) {
            out.push(a);
        }
    }
    Ok(out)
}

/// Both the host and the caller sent their car database and the two differ: they cannot
/// play together (different cars installed, imported or modified).
fn cars_differ(w: &World, caller: &Caller) -> bool {
    caller.car_db.as_deref() == Some(CAR_DB_CHANGED)
        || matches!((&w.car_db, &caller.car_db), (Some(a), Some(b)) if a != b)
}

/// What a hook sends as X-Fh2-CarDb once the car database changed during the session: that
/// game joins no world until it restarts.
const CAR_DB_CHANGED: &str = "changed";
/// Most players waiting to be kicked from one world.
const MAX_KICKS: usize = 64;

pub struct Worlds {
    state: Mutex<State>,
    seq: AtomicU64,
}

impl Default for Worlds {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_password(salt: &str, password: &str) -> String {
    sha1_smol::Sha1::from(format!("{salt}:{password}")).digest().to_string()
}

/// A string field: at most `max` characters, no control characters (`trim` the ends first).
fn text(body: &Value, key: &str, max: usize, trim: bool) -> Result<String, String> {
    let v = match &body[key] {
        Value::Null => "",
        Value::String(s) => s.as_str(),
        _ => return Err(format!("{key}: expected a string")),
    };
    let v = if trim { v.trim() } else { v };
    if v.chars().count() > max || v.chars().any(char::is_control) {
        return Err(format!("{key}: at most {max} characters"));
    }
    Ok(v.to_owned())
}

fn bad(error: String) -> Reply {
    Reply::json(400, json!({ "error": error }))
}

/// 403 with a sentence and the reason's code (the same codes as `why`).
fn refused(why: &str) -> Reply {
    let error = match why {
        "off" => "This world is closed.",
        "banned" => "You cannot join this world.",
        "friends" => "Only the host's friends can join this world.",
        "full" => "This world is full.",
        _ => "wrong password",
    };
    Reply::json(403, json!({ "error": error, "why": why }))
}

impl Worlds {
    pub fn new() -> Self {
        Worlds { state: Mutex::new(State::default()), seq: AtomicU64::new(0) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The world the caller goes to, in order: the one they host, the one they chose (still
    /// open to them), the one they were sent to before, the best open world (same public
    /// address first, then the most players, then the latest heartbeat). None: no world.
    pub fn choose(&self, caller: &Caller, now: Instant) -> Option<Choice> {
        let mut s = self.lock();
        s.purge(now);
        let pick = |host: u64, via: Via| {
            let w = &s.worlds[&host];
            Choice { addr: State::address(host, w, caller), host, via }
        };
        if s.worlds.contains_key(&caller.xuid) {
            return Some(pick(caller.xuid, Via::Hosting));
        }
        for (map, via) in [(&s.pending, Via::Pending), (&s.members, Via::Member)] {
            if let Some(&(host, _)) = map.get(&caller.xuid) {
                if s.worlds.get(&host).is_some_and(|w| s.why(host, w, caller).is_empty()) {
                    return Some(pick(host, via));
                }
            }
        }
        let best = s
            .worlds
            .iter()
            // FREE ROAM ONLINE goes straight only to a friend's world: a stranger's relay could be
            // hostile, so those are offered one by one (worlds/browse) with a warning instead
            .filter(|(host, w)| {
                w.listed && w.password.is_none() && caller.is_friend(**host) && s.why(**host, w, caller).is_empty()
            })
            // a tie goes to the oldest account (the lowest XUID): the same answer every time
            .max_by_key(|(host, w)| (w.host_ip == caller.ip, s.players(**host), w.heartbeat, std::cmp::Reverse(**host)))
            .map(|(host, _)| *host)?;
        Some(pick(best, Via::Auto))
    }

    /// The caller was sent to `choice` (None: to no hosted world): they count in that world
    /// now, and their join choice is used up.
    pub fn assign(&self, player: u64, choice: Option<&Choice>, now: Instant) {
        let mut s = self.lock();
        s.pending.remove(&player);
        match choice {
            Some(c) if c.via != Via::Hosting => {
                if !s.members.contains_key(&player) && s.members.len() >= MAX_MEMBERS {
                    if let Some(oldest) = s.members.iter().min_by_key(|(_, m)| m.1).map(|(k, _)| *k) {
                        s.members.remove(&oldest);
                    }
                }
                s.members.insert(player, (c.host, now));
            }
            _ => {
                s.members.remove(&player);
            }
        }
    }

    /// The account routes (`route`: "worlds", "worlds/of/<xuid>", "worlds/<id>/join").
    pub fn api(&self, method: &str, route: &str, body: &[u8], caller: &Caller) -> Reply {
        self.api_at(method, route, body, caller, Instant::now())
    }

    pub fn api_at(&self, method: &str, route: &str, body: &[u8], caller: &Caller, now: Instant) -> Reply {
        let parts: Vec<&str> = route.split('/').filter(|p| !p.is_empty()).collect();
        let body: Value = if body.is_empty() { json!({}) } else { serde_json::from_slice(body).unwrap_or(Value::Null) };
        match (method, parts.as_slice()) {
            ("POST", ["worlds"]) => self.host(&body, caller, now),
            ("DELETE", ["worlds"]) => {
                self.stop(caller.xuid, now);
                Reply { status: 204, body: None }
            }
            ("POST", ["worlds", "left"]) => self.left(&body, caller, now),
            ("POST", ["worlds", "punch"]) => self.punch(&body, caller, now),
            ("GET", ["worlds", "punches"]) => {
                let mut s = self.lock();
                s.purge(now);
                if !s.worlds.contains_key(&caller.xuid) {
                    return Reply::json(404, json!({ "error": "You are not hosting a world." }));
                }
                Reply::json(200, json!({ "punch": s.take_punches(caller.xuid) }))
            }
            ("GET", ["worlds"]) => self.list(caller, now),
            ("GET", ["worlds", "browse", after]) => match after.parse::<u64>() {
                Ok(after) => self.browse(after, caller, now),
                Err(_) => bad("bad xuid".into()),
            },
            ("GET", ["worlds", "of", who]) => match who.parse::<u64>() {
                Ok(who) => self.of(who, caller, now),
                Err(_) => bad("bad xuid".into()),
            },
            ("POST", ["worlds", id, "join"]) => match id.parse::<u64>() {
                Ok(id) => self.join(id, &body, caller, now),
                Err(_) => Reply::json(404, json!({ "error": "There is no such world." })),
            },
            _ => Reply::json(404, json!({ "error": "not found" })),
        }
    }

    /// POST worlds: start hosting, or the heartbeat.
    fn host(&self, body: &Value, caller: &Caller, now: Instant) -> Reply {
        if !body.is_object() {
            return bad("the body is not a JSON object".into());
        }
        // server.cfg's Name= is empty by default: the world is named after its host
        let name = match text(body, "name", 40, true) {
            Ok(n) if n.is_empty() => caller.game_tag(caller.xuid),
            Ok(n) => n,
            Err(e) => return bad(e),
        };
        let message = match text(body, "message", 120, true) {
            Ok(m) => m,
            Err(e) => return bad(e),
        };
        let visibility = match body["visibility"].as_str() {
            Some("public") => Visibility::Public,
            Some("friends") => Visibility::Friends,
            Some("off") => Visibility::Off,
            _ => return bad("visibility: public, friends or off".into()),
        };
        let listed = match &body["listed"] {
            Value::Null => true,
            Value::Bool(b) => *b,
            _ => return bad("listed: true or false".into()),
        };
        let password = match text(body, "password", 32, false) {
            Ok(p) => p,
            Err(e) => return bad(e),
        };
        let max_players = match body["maxPlayers"].as_u64() {
            Some(n) if (1..=MAX_PLAYERS).contains(&n) => n as usize,
            _ => return bad(format!("maxPlayers: 1 to {MAX_PLAYERS}")),
        };
        let port = match body["port"].as_u64() {
            Some(p) if (1..=65535).contains(&p) => p as u16,
            _ => return bad("port: 1 to 65535".into()),
        };
        let lan = match &body["lan"] {
            Value::Null => Vec::new(),
            Value::Array(a) if a.len() <= MAX_LAN => {
                let parsed: Option<Vec<IpAddr>> =
                    a.iter().map(|v| v.as_str().and_then(|s| s.trim().parse::<IpAddr>().ok()).map(|ip| ip.to_canonical())).collect();
                match parsed {
                    Some(l) if l.iter().all(is_lan) => l,
                    Some(_) => return bad("lan: private addresses only".into()),
                    None => return bad("lan: IP addresses".into()),
                }
            }
            _ => return bad(format!("lan: at most {MAX_LAN} IP addresses")),
        };
        let version = match text(body, "version", 40, true) {
            Ok(v) => v,
            Err(e) => return bad(e),
        };
        let public = match public_addresses(&body["public"], caller.ip) {
            Ok(p) => p,
            Err(e) => return bad(e),
        };
        let nat = match body["nat"].as_str() {
            None => "unknown".to_owned(),
            Some(k @ ("cone" | "symmetric" | "unknown")) => k.to_owned(),
            Some(_) => return bad("nat: cone, symmetric or unknown".into()),
        };
        let banned = match &body["banned"] {
            Value::Null => Vec::new(),
            Value::Array(a) if a.len() <= MAX_BANNED => {
                let parsed: Option<Vec<u64>> =
                    a.iter().map(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64())).collect();
                match parsed {
                    Some(b) => b,
                    None => return bad("banned: XUIDs".into()),
                }
            }
            _ => return bad(format!("banned: at most {MAX_BANNED} XUIDs")),
        };

        let mut s = self.lock();
        s.purge(now);
        let host = caller.xuid;
        let old = s.worlds.remove(&host);
        if old.is_none() && s.worlds.len() >= MAX_WORLDS {
            return Reply::json(503, json!({ "error": "Too many worlds on this server right now. Try again later." }));
        }
        let salt = match &old {
            Some(w) => w.salt.clone(),
            None => {
                let n = self.seq.fetch_add(1, Ordering::Relaxed);
                let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
                sha1_smol::Sha1::from(format!("fh2inf-world|{host}|{t}|{n}")).digest().to_string()[..16].to_owned()
            }
        };
        let world = World {
            password: (!password.is_empty()).then(|| hash_password(&salt, &password)),
            salt,
            name,
            message,
            visibility,
            listed,
            max_players,
            port,
            lan,
            version,
            car_db: caller.car_db.clone(),
            host_ip: caller.ip,
            public,
            nat,
            heartbeat: now,
            banned,
        };
        let changed = old.as_ref().is_none_or(|o| {
            o.name != world.name
                || o.visibility != world.visibility
                || o.listed != world.listed
                || o.password.is_some() != world.password.is_some()
                || o.port != world.port
                || o.host_ip != world.host_ip
                || o.max_players != world.max_players
                || o.public != world.public
                || o.nat != world.nat
        });
        if changed {
            println!(
                "worlds: {host} {} \"{}\" at {} ({}, {} players max, password {}, version {}, nat {}, public [{}])",
                if old.is_none() { "hosts" } else { "changed" },
                world.name,
                SocketAddr::new(world.host_ip, world.port),
                world.visibility.name(),
                world.max_players,
                if world.password.is_some() { "yes" } else { "no" },
                if world.version.is_empty() { "?" } else { &world.version },
                world.nat,
                world.public.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "),
            );
        }
        let address = SocketAddr::new(world.host_ip, world.port);
        s.worlds.insert(host, world);
        // the host plays in their own world
        s.members.remove(&host);
        s.pending.remove(&host);
        let kicks: Vec<String> = s.kicks.remove(&host).unwrap_or_default().iter().map(|x| x.to_string()).collect();
        let punch = s.take_punches(host);
        Reply::json(200, json!({
            "id": host.to_string(),
            "address": address.to_string(),
            "players": s.players(host),
            "expiresIn": WORLD_TTL.as_secs(),
            "kick": kicks,
            "punch": punch,
        }))
    }

    /// POST worlds/punch {candidates}: the caller's proxy is about to reach the world it was
    /// sent to (a member, a pending choice, or its own). It gets the world's addresses to try,
    /// best first; the host's relay is asked to punch towards the caller's.
    fn punch(&self, body: &Value, caller: &Caller, now: Instant) -> Reply {
        let candidates = match public_addresses(&body["candidates"], caller.ip) {
            Ok(c) => c,
            Err(e) => return bad(e.replace("public", "candidates")),
        };
        let mut s = self.lock();
        s.purge(now);
        let host = if s.worlds.contains_key(&caller.xuid) {
            caller.xuid
        } else if let Some(&(h, _)) = s.members.get(&caller.xuid).or(s.pending.get(&caller.xuid)) {
            h
        } else {
            return Reply::json(404, json!({ "error": "You are not on your way to any world." }));
        };
        let Some(w) = s.worlds.get(&host) else {
            return Reply::json(404, json!({ "error": "There is no such world." }));
        };
        if host != caller.xuid && !s.why(host, w, caller).is_empty() {
            return refused(s.why(host, w, caller));
        }
        // the world's addresses, best first: its LAN (same public address), its own port at the
        // address we see it at, then what STUN and UPnP found
        let mut world: Vec<SocketAddr> = Vec::new();
        let mut add = |a: SocketAddr| {
            if !world.contains(&a) {
                world.push(a);
            }
        };
        if host == caller.xuid {
            add(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), w.port));
        } else {
            if caller.ip == w.host_ip || w.host_ip.is_loopback() {
                for ip in &w.lan {
                    add(SocketAddr::new(*ip, w.port));
                }
            }
            if !w.host_ip.is_loopback() {
                add(SocketAddr::new(w.host_ip, w.port));
            }
            for a in &w.public {
                add(*a);
            }
        }
        let world: Vec<String> = world.iter().map(|a| a.to_string()).collect();
        let nat = w.nat.clone();
        // the host punches towards what the caller sent, and its address as we see it
        if host != caller.xuid {
            let mut targets = candidates;
            if !caller.ip.is_loopback() && !targets.iter().any(|t| t.ip() == caller.ip) {
                // no STUN answer: at least the player's address, at its game port
                if let Some(port) = body["port"].as_u64().filter(|p| (1..=65535).contains(p)) {
                    targets.push(SocketAddr::new(caller.ip, port as u16));
                }
            }
            let list = s.punches.entry(host).or_default();
            for t in targets {
                if list.len() >= MAX_PUNCHES {
                    list.remove(0);
                }
                list.retain(|(a, _)| *a != t);
                list.push((t, now));
            }
            println!("worlds: {} punches towards the world of {host}", caller.xuid);
        }
        Reply::json(200, json!({ "world": world, "nat": nat }))
    }

    /// POST worlds/left {"reason"}: the caller's game left its world on its own (the car
    /// database changed during the session). It is no longer a member, and that world's host
    /// is asked to kick it at once (the relay may still hear it). A host ends its world.
    fn left(&self, body: &Value, caller: &Caller, now: Instant) -> Reply {
        let reason = body["reason"].as_str().unwrap_or("").chars().take(16).collect::<String>();
        let mut s = self.lock();
        s.purge(now);
        if s.worlds.contains_key(&caller.xuid) {
            drop(s);
            self.stop(caller.xuid, now);
            println!("worlds: {} left their own world ({reason})", caller.xuid);
            return Reply { status: 204, body: None };
        }
        s.pending.remove(&caller.xuid);
        if let Some((host, _)) = s.members.remove(&caller.xuid) {
            let list = s.kicks.entry(host).or_default();
            if list.len() < MAX_KICKS && !list.contains(&caller.xuid) {
                list.push(caller.xuid);
            }
            println!("worlds: {} left the world of {host} ({reason}); its relay is asked to kick them", caller.xuid);
        }
        Reply { status: 204, body: None }
    }

    /// DELETE worlds: the world ends; its players are released.
    fn stop(&self, host: u64, now: Instant) {
        let mut s = self.lock();
        s.purge(now);
        s.kicks.remove(&host);
        if s.worlds.remove(&host).is_some() {
            let before = s.members.len();
            s.members.retain(|_, (h, _)| *h != host);
            s.pending.retain(|_, (h, _)| *h != host);
            println!("worlds: {host} stopped hosting ({} player(s) released)", before - s.members.len());
        }
    }

    /// GET worlds: the worlds in the caller's list (public, or friends-only of a friend; no
    /// password, not off, not banning the caller).
    fn list(&self, caller: &Caller, now: Instant) -> Reply {
        let mut s = self.lock();
        s.purge(now);
        let mut items: Vec<(usize, Instant, Value)> = s
            .worlds
            .iter()
            .filter(|(host, w)| {
                w.listed
                    && !cars_differ(w, caller)
                    && w.password.is_none()
                    && w.visibility != Visibility::Off
                    && !w.banned.contains(&caller.xuid)
                    && (w.visibility == Visibility::Public || **host == caller.xuid || caller.is_friend(**host))
            })
            .map(|(host, w)| {
                let players = s.players(*host);
                (players, w.heartbeat, json!({
                    "id": host.to_string(),
                    "name": w.name,
                    "message": w.message,
                    "host": caller.game_tag(*host),
                    "players": players,
                    "maxPlayers": w.max_players,
                    "friendsOnly": w.visibility == Visibility::Friends,
                    "sameNetwork": w.host_ip == caller.ip,
                }))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        items.truncate(MAX_LIST);
        Reply::json(200, json!({ "worlds": items.into_iter().map(|i| i.2).collect::<Vec<_>>() }))
    }

    /// GET worlds/of/<xuid>: the world that player hosts or is in, judged for the caller.
    fn of(&self, who: u64, caller: &Caller, now: Instant) -> Reply {
        let mut s = self.lock();
        s.purge(now);
        let host = if s.worlds.contains_key(&who) { Some(who) } else { s.members.get(&who).map(|m| m.0) };
        let Some((host, w)) = host.and_then(|h| s.worlds.get(&h).map(|w| (h, w))) else {
            return Reply::json(200, json!({ "world": null }));
        };
        // where a player is, only for their friends, or when that world is open to the caller on
        // the list anyway (security review L3)
        let open = w.listed && w.password.is_none() && w.visibility == Visibility::Public;
        if !(open || caller.xuid == who || caller.is_friend(who)) {
            return Reply::json(200, json!({ "world": null }));
        }
        let why = s.why(host, w, caller);
        Reply::json(200, json!({ "world": {
            "id": host.to_string(),
            "name": w.name,
            "host": caller.game_tag(host),
            "players": s.players(host),
            "maxPlayers": w.max_players,
            "password": w.password.is_some(),
            "canJoin": why.is_empty(),
            "why": why,
        } }))
    }

    /// GET worlds/browse/<after>: the next open world the caller could join, after the one of
    /// host `after` (0: the first), in a stable order (host xuid): listed, no password, the same
    /// cars, not full, not the caller's. The INFINITY screen's FIND WORLD walks them one by one,
    /// with a warning for a stranger's (`friend`: false).
    fn browse(&self, after: u64, caller: &Caller, now: Instant) -> Reply {
        let mut s = self.lock();
        s.purge(now);
        let next = s
            .worlds
            .iter()
            .filter(|(host, w)| {
                **host > after && **host != caller.xuid && w.listed && w.password.is_none() && s.why(**host, w, caller).is_empty()
            })
            .min_by_key(|(host, _)| **host);
        let Some((host, w)) = next else {
            return Reply::json(200, json!({ "world": null }));
        };
        Reply::json(200, json!({ "world": {
            "id": host.to_string(),
            "name": w.name,
            "host": caller.game_tag(*host),
            "players": s.players(*host),
            "maxPlayers": w.max_players,
            "friend": caller.is_friend(*host),
        } }))
    }

    /// POST worlds/<id>/join {password}: the caller's next FREE ROAM ONLINE goes there.
    fn join(&self, host: u64, body: &Value, caller: &Caller, now: Instant) -> Reply {
        let password = body["password"].as_str().unwrap_or("");
        let mut s = self.lock();
        s.purge(now);
        let Some(w) = s.worlds.get(&host) else {
            return Reply::json(404, json!({ "error": "There is no such world." }));
        };
        let why = s.why(host, w, caller);
        if !why.is_empty() {
            return refused(why);
        }
        if caller.xuid != host {
            if let Some(hash) = &w.password {
                if s.failures.get(&caller.xuid).is_some_and(|f| f.0 >= MAX_WRONG_PASSWORDS) {
                    return Reply::json(429, json!({ "error": "Too many wrong passwords. Try again later." }));
                }
                if hash_password(&w.salt, password) != *hash {
                    if s.failures.len() >= MAX_FAILURES && !s.failures.contains_key(&caller.xuid) {
                        if let Some(oldest) = s.failures.iter().min_by_key(|(_, f)| f.1).map(|(k, _)| *k) {
                            s.failures.remove(&oldest);
                        }
                    }
                    s.failures.entry(caller.xuid).or_insert((0, now)).0 += 1;
                    return refused("password");
                }
            }
        }
        if !s.pending.contains_key(&caller.xuid) && s.pending.len() >= MAX_PENDING {
            if let Some(oldest) = s.pending.iter().min_by_key(|(_, p)| p.1).map(|(k, _)| *k) {
                s.pending.remove(&oldest);
            }
        }
        s.pending.insert(caller.xuid, (host, now));
        println!("worlds: {} chose the world of {host}", caller.xuid);
        Reply::json(200, json!({ "ok": true }))
    }

    /// For /stats (the landing page): the worlds alive now (public, friends-only and with a
    /// password alike; only numbers) and the players in them, hosts included.
    pub fn active(&self) -> (usize, usize) {
        self.active_at(Instant::now())
    }

    fn active_at(&self, now: Instant) -> (usize, usize) {
        let mut s = self.lock();
        s.purge(now);
        (s.worlds.len(), s.worlds.len() + s.members.len())
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize, usize) {
        let s = self.lock();
        (s.worlds.len(), s.members.len(), s.pending.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: u64 = 100;
    const HOME: &str = "203.0.113.5";
    const AWAY: &str = "198.51.100.9";

    fn tag(x: u64) -> String {
        format!("Player#{x}")
    }
    fn friends(x: u64) -> Vec<u64> {
        match x {
            1 => vec![HOST],
            HOST => vec![1],
            _ => Vec::new(),
        }
    }

    fn who(xuid: u64, ip: &str) -> Caller<'static> {
        Caller::new(xuid, ip.parse().unwrap(), &tag, &friends)
    }

    fn everyone(x: u64) -> Vec<u64> {
        (0..=1000).filter(|&y| y != x).collect()
    }

    /// A player who is everybody's friend: FREE ROAM ONLINE goes straight to friends' worlds
    /// only, and these tests are about the rest of the choice.
    fn pal(xuid: u64, ip: &str) -> Caller<'static> {
        Caller::new(xuid, ip.parse().unwrap(), &tag, &everyone)
    }

    fn world(extra: Value) -> Value {
        let mut b = json!({
            "name": "Mundo do Alex", "message": "bem-vindos", "visibility": "public", "password": "",
            "maxPlayers": 12, "port": 5101, "lan": ["192.168.68.40"], "version": "0.1.4",
        });
        if let (Value::Object(b), Value::Object(e)) = (&mut b, extra) {
            b.extend(e);
        }
        b
    }

    fn call(w: &Worlds, method: &str, route: &str, body: Value, c: &Caller, now: Instant) -> Reply {
        let body = if body.is_null() { Vec::new() } else { serde_json::to_vec(&body).unwrap() };
        w.api_at(method, route, &body, c, now)
    }

    fn host_world(w: &Worlds, host: u64, ip: &str, extra: Value, now: Instant) -> Value {
        let r = call(w, "POST", "worlds", world(extra), &who(host, ip), now);
        assert_eq!(r.status, 200, "{:?}", r.body);
        r.body.unwrap()
    }

    /// F3: the host announces its STUN/UPnP addresses; a joining player sends its own and gets
    /// the world's; the host's next poll (or heartbeat) has the punch, once.
    #[test]
    fn punch_exchange() {
        let w = Worlds::new();
        let t = Instant::now();
        // a public IPv4 address other than the one we see the host at is refused
        let r = call(&w, "POST", "worlds", world(json!({"public": ["198.51.100.66:5101"]})), &who(HOST, HOME), t);
        assert_eq!(r.status, 400);
        let r = call(&w, "POST", "worlds", world(json!({"nat": "open"})), &who(HOST, HOME), t);
        assert_eq!(r.status, 400);
        host_world(&w, HOST, HOME, json!({"public": ["203.0.113.5:40001", "[2001:db8::5]:5101"], "nat": "cone"}), t);
        // not on the way to the world yet
        let me = pal(5, AWAY);
        assert_eq!(call(&w, "POST", "worlds/punch", json!({"candidates": []}), &me, t).status, 404);
        let c = w.choose(&me, t).unwrap();
        w.assign(5, Some(&c), t);
        // someone else's IPv4 address as a candidate: refused
        let r = call(&w, "POST", "worlds/punch", json!({"candidates": ["203.0.113.99:5100"]}), &me, t);
        assert_eq!(r.status, 400);
        let r = call(&w, "POST", "worlds/punch", json!({"candidates": ["198.51.100.9:61000", "[2001:db8::9]:5100"]}), &me, t);
        assert_eq!(r.status, 200);
        assert_eq!(
            r.body.unwrap(),
            json!({"world": ["203.0.113.5:5101", "203.0.113.5:40001", "[2001:db8::5]:5101"], "nat": "cone"})
        );
        // the host gets the punches once
        let p = call(&w, "GET", "worlds/punches", Value::Null, &who(HOST, HOME), t).body.unwrap();
        assert_eq!(p, json!({"punch": ["198.51.100.9:61000", "[2001:db8::9]:5100"]}));
        assert_eq!(call(&w, "GET", "worlds/punches", Value::Null, &who(HOST, HOME), t).body.unwrap(), json!({"punch": []}));
        // without STUN: the address we see, at the game's port; it also rides the heartbeat
        call(&w, "POST", "worlds/punch", json!({"port": 5100}), &me, t);
        assert_eq!(host_world(&w, HOST, HOME, json!({}), t)["punch"], json!(["198.51.100.9:5100"]));
        // the same network: the host's LAN address first
        let neighbour = pal(6, HOME);
        let c = w.choose(&neighbour, t).unwrap();
        w.assign(6, Some(&c), t);
        let r = call(&w, "POST", "worlds/punch", json!({}), &neighbour, t).body.unwrap();
        assert_eq!(r["world"][0], "192.168.68.40:5101");
        // punches wait PUNCH_TTL at most; a non-host has none to poll
        call(&w, "POST", "worlds/punch", json!({"port": 5100}), &me, t);
        let later = t + PUNCH_TTL + Duration::from_secs(1);
        host_world(&w, HOST, HOME, json!({}), later - Duration::from_secs(1));
        assert_eq!(call(&w, "GET", "worlds/punches", Value::Null, &who(HOST, HOME), later).body.unwrap(), json!({"punch": []}));
        assert_eq!(call(&w, "GET", "worlds/punches", Value::Null, &me, t).status, 404);
        // bounded
        for k in 0..(MAX_PUNCHES as u16 + 10) {
            call(&w, "POST", "worlds/punch", json!({"candidates": [format!("198.51.100.9:{}", 1000 + k)]}), &me, later);
        }
        let p = call(&w, "GET", "worlds/punches", Value::Null, &who(HOST, HOME), later).body.unwrap();
        assert_eq!(p["punch"].as_array().unwrap().len(), MAX_PUNCHES);
    }

    #[test]
    fn public_address_rules() {
        let seen: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(public_addresses(&json!(["203.0.113.5:1", "[2001:db8::1]:2"]), seen).is_ok());
        assert!(public_addresses(&json!(["[::ffff:203.0.113.5]:1"]), seen).is_ok());
        for bad in [json!(["203.0.113.6:1"]), json!(["203.0.113.5:0"]), json!(["[fe80::1]:5"]), json!(["x"]), json!("a"), json!(["203.0.113.5:1", "203.0.113.5:2", "203.0.113.5:3", "203.0.113.5:4"])] {
            assert!(public_addresses(&bad, seen).is_err(), "{bad}");
        }
        // seen over IPv6: one IPv4 address (unverifiable), not two different ones
        let v6: IpAddr = "2001:db8::7".parse().unwrap();
        assert!(public_addresses(&json!(["198.51.100.4:5101", "[2001:db8::7]:5101"]), v6).is_ok());
        assert!(public_addresses(&json!(["198.51.100.4:5101", "198.51.100.5:5101"]), v6).is_err());
        // a server on the same LAN (or loopback) sees private addresses: anything goes
        assert!(public_addresses(&json!(["191.251.57.219:5101"]), "192.168.68.93".parse().unwrap()).is_ok());
    }

    #[test]
    fn host_and_heartbeat() {
        let w = Worlds::new();
        let t0 = Instant::now();
        let b = host_world(&w, HOST, HOME, json!({}), t0);
        assert_eq!(b, json!({"id": "100", "address": "203.0.113.5:5101", "players": 1, "expiresIn": 45, "kick": [], "punch": []}));
        // heartbeat keeps it past 45 s
        host_world(&w, HOST, HOME, json!({}), t0 + Duration::from_secs(30));
        assert!(w.choose(&pal(5, AWAY), t0 + Duration::from_secs(60)).is_some());
        assert_eq!(w.counts().0, 1);
    }

    #[test]
    fn unlisted_world_is_joined_only_by_choice() {
        // a private session: not on the list, not picked automatically, joinable by a friend
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, HOME, json!({"listed": false, "visibility": "friends"}), t);
        let r = call(&w, "GET", "worlds", Value::Null, &who(1, AWAY), t);
        assert_eq!(r.body.unwrap()["worlds"], json!([]));
        assert!(w.choose(&pal(1, AWAY), t).is_none());
        assert_eq!(call(&w, "POST", "worlds/100/join", json!({"password": ""}), &who(1, AWAY), t).status, 200);
        assert_eq!(w.choose(&pal(1, AWAY), t).unwrap().host, HOST);
        // not a friend: no
        assert_eq!(call(&w, "POST", "worlds/100/join", json!({"password": ""}), &who(5, AWAY), t).status, 403);
        // the host still goes to it
        assert_eq!(w.choose(&pal(HOST, HOME), t).unwrap().via, Via::Hosting);
    }

    #[test]
    fn worlds_only_with_the_same_cars() {
        let stock = "4d50013dc7705de0".repeat(4);
        let modded = "0123456789abcdef".repeat(4);
        let w = Worlds::new();
        let t = Instant::now();
        let host = who(HOST, HOME).with_car_db(Some(&stock));
        assert_eq!(call(&w, "POST", "worlds", world(json!({})), &host, t).status, 200);
        // the same cars: listed and picked
        let same = pal(5, AWAY).with_car_db(Some(&stock.to_uppercase()));
        assert_eq!(call(&w, "GET", "worlds", Value::Null, &same, t).body.unwrap()["worlds"].as_array().unwrap().len(), 1);
        assert!(w.choose(&same, t).is_some());
        // other cars: not listed, not picked, and a join says why
        let other = who(6, AWAY).with_car_db(Some(&modded));
        assert_eq!(call(&w, "GET", "worlds", Value::Null, &other, t).body.unwrap()["worlds"], json!([]));
        assert!(w.choose(&other, t).is_none());
        let of = call(&w, "GET", "worlds/of/100", Value::Null, &other, t).body.unwrap();
        assert_eq!(of["world"]["why"], json!("cars"));
        assert_eq!(call(&w, "POST", "worlds/100/join", json!({"password": ""}), &other, t).status, 403);
        // an older hook (no header) is not checked; a bad header is ignored
        assert!(w.choose(&pal(7, AWAY), t).is_some());
        assert!(w.choose(&pal(8, AWAY).with_car_db(Some("nope")), t).is_some());
    }

    #[test]
    fn changed_car_db_leaves_and_is_kicked() {
        let stock = "4d50013dc7705de0".repeat(4);
        let w = Worlds::new();
        let t = Instant::now();
        let host = who(HOST, HOME).with_car_db(Some(&stock));
        assert_eq!(call(&w, "POST", "worlds", world(json!({})), &host, t).status, 200);
        let guest = pal(5, AWAY).with_car_db(Some(&stock));
        let c = w.choose(&guest, t).unwrap();
        w.assign(5, Some(&c), t);
        // the guest's database changed: it says so and leaves
        let changed = who(5, AWAY).with_car_db(Some("changed"));
        assert!(w.choose(&changed, t).is_none());
        assert_eq!(call(&w, "POST", "worlds/left", json!({"reason": "cars"}), &changed, t).status, 204);
        // the host's next heartbeat carries the kick, once
        let b = call(&w, "POST", "worlds", world(json!({})), &host, t).body.unwrap();
        assert_eq!(b["kick"], json!(["5"]));
        let b = call(&w, "POST", "worlds", world(json!({})), &host, t).body.unwrap();
        assert_eq!(b["kick"], json!([]));
        // a host whose database changed ends its world
        assert_eq!(call(&w, "POST", "worlds/left", json!({"reason": "cars"}), &who(HOST, HOME), t).status, 204);
        assert_eq!(w.counts().0, 0);
    }

    #[test]
    fn lan_must_be_private_and_of_needs_friendship() {
        let w = Worlds::new();
        let t = Instant::now();
        let c = who(HOST, HOME);
        for lan in [json!(["8.8.8.8"]), json!(["2001:db8::1"]), json!(["192.168.0.2", "1.1.1.1"])] {
            assert_eq!(call(&w, "POST", "worlds", world(json!({"lan": lan})), &c, t).status, 400, "{lan}");
        }
        for lan in [json!(["10.0.0.2"]), json!(["172.16.5.1"]), json!(["169.254.1.1"]), json!(["fd00::1"]), json!(["fe80::1"])] {
            assert_eq!(call(&w, "POST", "worlds", world(json!({"lan": lan})), &c, t).status, 200, "{lan}");
        }
        // a private session: a stranger does not learn where the host is; a friend does
        assert_eq!(call(&w, "POST", "worlds", world(json!({"listed": false, "visibility": "friends"})), &c, t).status, 200);
        let r = call(&w, "GET", "worlds/of/100", Value::Null, &who(5, AWAY), t).body.unwrap();
        assert_eq!(r["world"], Value::Null);
        let r = call(&w, "GET", "worlds/of/100", Value::Null, &who(1, AWAY), t).body.unwrap();
        assert_eq!(r["world"]["id"], json!("100"));
    }

    #[test]
    fn auto_join_only_friends_and_browse_the_rest() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, HOME, json!({}), t);
        host_world(&w, 300, "192.0.2.30", json!({}), t);
        host_world(&w, 400, "192.0.2.40", json!({"password": "x"}), t);
        // player 1 is HOST's friend: FREE ROAM ONLINE takes them there
        assert_eq!(w.choose(&pal(1, AWAY), t).unwrap().host, HOST);
        // player 5 has no friend hosting: nothing automatic
        assert!(w.choose(&who(5, AWAY), t).is_none());
        // FIND WORLD walks the open ones (not the password one), then null
        let b = |after: u64| call(&w, "GET", &format!("worlds/browse/{after}"), Value::Null, &who(5, AWAY), t).body.unwrap();
        assert_eq!(b(0)["world"]["id"], json!("100"));
        assert_eq!(b(0)["world"]["friend"], json!(false));
        assert_eq!(b(100)["world"]["id"], json!("300"));
        assert_eq!(b(300)["world"], Value::Null);
        // choosing one sends them there
        assert_eq!(call(&w, "POST", "worlds/300/join", json!({"password": ""}), &who(5, AWAY), t).status, 200);
        assert_eq!(w.choose(&pal(5, AWAY), t).unwrap().host, 300);
        // a friend sees it as a friend's
        let f = call(&w, "GET", "worlds/browse/0", Value::Null, &who(1, AWAY), t).body.unwrap();
        assert_eq!(f["world"]["friend"], json!(true));
    }

    #[test]
    fn empty_name_is_the_host() {
        // server.cfg's default (Name= empty), as the hook sends it
        let w = Worlds::new();
        let t0 = Instant::now();
        host_world(&w, HOST, HOME, json!({"name": ""}), t0);
        let r = call(&w, "GET", "worlds", Value::Null, &who(5, AWAY), t0);
        assert_eq!(r.body.unwrap()["worlds"][0]["name"], json!("Player#100"));
    }

    #[test]
    fn bad_input() {
        let w = Worlds::new();
        let c = who(HOST, HOME);
        let t = Instant::now();
        for extra in [
            json!({"name": "x".repeat(41)}),
            json!({"message": "x".repeat(121)}),
            json!({"visibility": "secret"}),
            json!({"password": "x".repeat(33)}),
            json!({"maxPlayers": 0}),
            json!({"maxPlayers": 13}),
            json!({"port": 0}),
            json!({"port": 70000}),
            json!({"lan": ["not an ip"]}),
            json!({"lan": ["1.1.1.1", "1.1.1.2", "1.1.1.3", "1.1.1.4", "1.1.1.5"]}),
            json!({"name": "a\nb"}),
        ] {
            assert_eq!(call(&w, "POST", "worlds", world(extra.clone()), &c, t).status, 400, "{extra}");
        }
        assert_eq!(call(&w, "POST", "worlds", json!([1]), &c, t).status, 400);
        assert_eq!(w.counts().0, 0);
        // IPv6 LAN is fine
        assert_eq!(call(&w, "POST", "worlds", world(json!({"lan": ["fe80::1", "192.168.0.2"]})), &c, t).status, 200);
        assert_eq!(call(&w, "GET", "worlds/of/abc", Value::Null, &c, t).status, 400);
        assert_eq!(call(&w, "PUT", "worlds", Value::Null, &c, t).status, 404);
    }

    #[test]
    fn hosting_goes_to_loopback() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, HOME, json!({"visibility": "off", "password": "x"}), t);
        let c = w.choose(&pal(HOST, HOME), t).unwrap();
        assert_eq!(c.via, Via::Hosting);
        assert_eq!(c.addr, "127.0.0.1:5101".parse().unwrap());
    }

    #[test]
    fn host_on_the_servers_pc_is_reached_by_lan() {
        // the server runs on the host's PC: the host announces from 127.0.0.1, a Deck on the
        // LAN must get the host's LAN address, not loopback (seen 2026-10-15)
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, "127.0.0.1", json!({}), t);
        let c = w.choose(&pal(7, "192.168.68.93"), t).unwrap();
        assert_eq!(c.addr, "192.168.68.40:5101".parse().unwrap());
    }

    #[test]
    fn auto_prefers_same_network_then_players() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({}), t);
        host_world(&w, 300, "192.0.2.2", json!({}), t);
        // 300 has two players sent to it
        for p in [11, 12] {
            let c = Choice { addr: "192.0.2.2:5101".parse().unwrap(), host: 300, via: Via::Auto };
            w.assign(p, Some(&c), t);
        }
        let c = w.choose(&pal(5, AWAY), t).unwrap();
        assert_eq!((c.host, c.via), (300, Via::Auto));
        assert_eq!(c.addr, "192.0.2.2:5101".parse().unwrap());
        // somebody behind the same public address as host 200: its LAN address
        let c = w.choose(&pal(6, "192.0.2.1"), t).unwrap();
        assert_eq!(c.host, 200);
        assert_eq!(c.addr, "192.168.68.40:5101".parse().unwrap());
        // a tie in players: the latest heartbeat
        let w = Worlds::new();
        host_world(&w, 200, "192.0.2.1", json!({}), t);
        host_world(&w, 300, "192.0.2.2", json!({}), t + Duration::from_secs(1));
        assert_eq!(w.choose(&pal(5, AWAY), t + Duration::from_secs(2)).unwrap().host, 300);
    }

    #[test]
    fn auto_skips_password_off_friends_full() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({"password": "pw"}), t);
        host_world(&w, 300, "192.0.2.2", json!({"visibility": "off"}), t);
        host_world(&w, HOST, "192.0.2.3", json!({"visibility": "friends"}), t);
        host_world(&w, 400, "192.0.2.4", json!({"maxPlayers": 1}), t);
        assert!(w.choose(&who(5, AWAY), t).is_none());
        // the host's friend goes to the friends-only world
        assert_eq!(w.choose(&pal(1, AWAY), t).unwrap().host, HOST);
    }

    #[test]
    fn list_visibility() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({"password": "pw", "name": "secret"}), t);
        host_world(&w, 300, "192.0.2.2", json!({"visibility": "off"}), t);
        host_world(&w, HOST, AWAY, json!({"visibility": "friends", "name": "amigos"}), t);
        host_world(&w, 400, "192.0.2.4", json!({"name": "aberto", "banned": ["9"]}), t);
        let list = |x: u64| call(&w, "GET", "worlds", Value::Null, &who(x, AWAY), t).body.unwrap()["worlds"].clone();
        let stranger = list(5);
        assert_eq!(stranger.as_array().unwrap().len(), 1);
        assert_eq!(stranger[0], json!({
            "id": "400", "name": "aberto", "message": "bem-vindos", "host": "Player#400", "players": 1,
            "maxPlayers": 12, "friendsOnly": false, "sameNetwork": false,
        }));
        let friend = list(1);
        assert_eq!(friend.as_array().unwrap().len(), 2);
        let mine = friend.as_array().unwrap().iter().find(|i| i["id"] == "100").unwrap();
        assert_eq!(mine["friendsOnly"], true);
        assert_eq!(mine["sameNetwork"], true);
        // banned from 400
        assert_eq!(list(9).as_array().unwrap().len(), 0);
    }

    #[test]
    fn password_world_join() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({"password": "segredo"}), t);
        let me = who(5, AWAY);
        let r = call(&w, "POST", "worlds/200/join", json!({"password": "errado"}), &me, t);
        assert_eq!(r.status, 403);
        assert_eq!(r.body.unwrap()["error"], "wrong password");
        assert!(w.choose(&me, t).is_none());
        let r = call(&w, "POST", "worlds/200/join", json!({"password": "segredo"}), &me, t);
        assert_eq!(r.status, 200);
        assert_eq!(r.body.unwrap(), json!({"ok": true}));
        let c = w.choose(&me, t).unwrap();
        assert_eq!((c.host, c.via), (200, Via::Pending));
        // assigned: the choice is used up, the player counts in the world and stays there
        w.assign(5, Some(&c), t);
        assert_eq!(w.counts(), (1, 1, 0));
        assert_eq!(w.choose(&me, t).unwrap().via, Via::Member);
        // too many wrong passwords
        let other = who(6, AWAY);
        for _ in 0..MAX_WRONG_PASSWORDS {
            assert_eq!(call(&w, "POST", "worlds/200/join", json!({"password": "x"}), &other, t).status, 403);
        }
        assert_eq!(call(&w, "POST", "worlds/200/join", json!({"password": "segredo"}), &other, t).status, 429);
        // unknown world
        assert_eq!(call(&w, "POST", "worlds/999/join", json!({}), &other, t).status, 404);
    }

    #[test]
    fn join_refusals() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({"maxPlayers": 2}), t);
        host_world(&w, 300, "192.0.2.2", json!({"visibility": "off"}), t);
        host_world(&w, HOST, "192.0.2.3", json!({"visibility": "friends", "banned": ["7"]}), t);
        let join = |id: u64, x: u64| {
            let r = call(&w, "POST", &format!("worlds/{id}/join"), json!({}), &who(x, AWAY), t);
            (r.status, r.body.unwrap()["why"].as_str().unwrap_or("").to_owned())
        };
        assert_eq!(join(300, 5), (403, "off".into()));
        assert_eq!(join(HOST, 5), (403, "friends".into()));
        assert_eq!(join(HOST, 7), (403, "banned".into()));
        assert_eq!(join(HOST, 1).0, 200);
        assert_eq!(join(200, 5).0, 200);
        let c = w.choose(&pal(5, AWAY), t).unwrap();
        w.assign(5, Some(&c), t);
        // 200 holds 2: the host and player 5
        assert_eq!(join(200, 6), (403, "full".into()));
        // a pending choice for a world that filled up is skipped
        let w2 = Worlds::new();
        host_world(&w2, 200, "192.0.2.1", json!({"maxPlayers": 2}), t);
        call(&w2, "POST", "worlds/200/join", json!({}), &who(6, AWAY), t);
        let c = w2.choose(&pal(5, AWAY), t).unwrap();
        w2.assign(5, Some(&c), t);
        assert!(w2.choose(&pal(6, AWAY), t).is_none());
    }

    #[test]
    fn world_of() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, "192.0.2.3", json!({"visibility": "friends", "password": "pw"}), t);
        let of = |x: u64, caller: u64| call(&w, "GET", &format!("worlds/of/{x}"), Value::Null, &who(caller, AWAY), t).body.unwrap();
        assert_eq!(of(5, 1), json!({"world": null}));
        // a stranger does not learn where the host of a friends-only world is (L3)
        assert_eq!(of(HOST, 5), json!({"world": null}));
        assert_eq!(of(HOST, 1), json!({"world": {
            "id": "100", "name": "Mundo do Alex", "host": "Player#100", "players": 1, "maxPlayers": 12,
            "password": true, "canJoin": true, "why": "",
        }}));
        // a player in the world: the same world
        let c = Choice { addr: "192.0.2.3:5101".parse().unwrap(), host: HOST, via: Via::Pending };
        w.assign(1, Some(&c), t);
        assert_eq!(of(1, 1)["world"]["id"], "100");
        assert_eq!(of(1, 1)["world"]["players"], 2);
    }

    #[test]
    fn expiry_after_45_s() {
        let w = Worlds::new();
        let t0 = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({}), t0);
        let c = w.choose(&pal(5, AWAY), t0).unwrap();
        w.assign(5, Some(&c), t0);
        assert!(w.choose(&pal(5, AWAY), t0 + Duration::from_secs(44)).is_some());
        // /stats: one world, its host and one guest
        assert_eq!(w.active_at(t0 + Duration::from_secs(44)), (1, 2));
        let later = t0 + WORLD_TTL + Duration::from_secs(1);
        assert_eq!(w.active_at(later), (0, 0));
        assert!(w.choose(&pal(5, AWAY), later).is_none());
        assert_eq!(w.counts(), (0, 0, 0));
        assert_eq!(call(&w, "GET", "worlds", Value::Null, &who(5, AWAY), later).body.unwrap(), json!({"worlds": []}));
    }

    #[test]
    fn delete_releases_members() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({}), t);
        let c = w.choose(&pal(5, AWAY), t).unwrap();
        w.assign(5, Some(&c), t);
        call(&w, "POST", "worlds/200/join", json!({}), &who(6, AWAY), t);
        assert_eq!(w.counts(), (1, 1, 1));
        let r = call(&w, "DELETE", "worlds", Value::Null, &who(200, "192.0.2.1"), t);
        assert_eq!(r.status, 204);
        assert!(r.body.is_none());
        assert_eq!(w.counts(), (0, 0, 0));
        assert!(w.choose(&pal(5, AWAY), t).is_none());
        // deleting nothing is fine too
        assert_eq!(call(&w, "DELETE", "worlds", Value::Null, &who(200, AWAY), t).status, 204);
    }

    #[test]
    fn caps() {
        let w = Worlds::new();
        let t = Instant::now();
        for host in 0..MAX_WORLDS as u64 {
            host_world(&w, 1000 + host, AWAY, json!({}), t);
        }
        assert_eq!(call(&w, "POST", "worlds", world(json!({})), &who(5, AWAY), t).status, 503);
        // a heartbeat of an existing world still goes through
        assert_eq!(call(&w, "POST", "worlds", world(json!({})), &who(1000, AWAY), t).status, 200);
        let c = Choice { addr: "192.0.2.2:5101".parse().unwrap(), host: 1000, via: Via::Auto };
        for p in 0..MAX_MEMBERS as u64 + 10 {
            w.assign(10_000 + p, Some(&c), t + Duration::from_millis(p));
        }
        assert_eq!(w.counts().1, MAX_MEMBERS);
    }

    #[test]
    fn password_is_not_stored_plainly() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, AWAY, json!({"password": "segredo"}), t);
        let s = w.lock();
        let world = &s.worlds[&200];
        assert_ne!(world.password.as_deref(), Some("segredo"));
        assert_eq!(world.password.as_deref().map(str::len), Some(40));
    }
}
