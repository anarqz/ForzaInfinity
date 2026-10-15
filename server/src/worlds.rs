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
    salt: String,
    /// SHA-1 hex of salt:password; None: no password.
    password: Option<String>,
    max_players: usize,
    port: u16,
    lan: Vec<IpAddr>,
    version: String,
    /// The host's address as this server sees it.
    host_ip: IpAddr,
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
}

impl<'a> Caller<'a> {
    /// `friends(x)`: the players x is friends with (both added each other).
    pub fn new(xuid: u64, ip: IpAddr, gamertag: &'a dyn Fn(u64) -> String, friends: &'a dyn Fn(u64) -> Vec<u64>) -> Self {
        Caller { xuid, ip: ip.to_canonical(), gamertag, friends, friend_list: OnceCell::new() }
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
            .filter(|(host, w)| w.password.is_none() && s.why(**host, w, caller).is_empty())
            .max_by_key(|(host, w)| (w.host_ip == caller.ip, s.players(**host), w.heartbeat))
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
            ("GET", ["worlds"]) => self.list(caller, now),
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
                    Some(l) => l,
                    None => return bad("lan: IP addresses".into()),
                }
            }
            _ => return bad(format!("lan: at most {MAX_LAN} IP addresses")),
        };
        let version = match text(body, "version", 40, true) {
            Ok(v) => v,
            Err(e) => return bad(e),
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
            max_players,
            port,
            lan,
            version,
            host_ip: caller.ip,
            heartbeat: now,
            banned,
        };
        let changed = old.as_ref().is_none_or(|o| {
            o.name != world.name
                || o.visibility != world.visibility
                || o.password.is_some() != world.password.is_some()
                || o.port != world.port
                || o.host_ip != world.host_ip
                || o.max_players != world.max_players
        });
        if changed {
            println!(
                "worlds: {host} {} \"{}\" at {} ({}, {} players max, password {}, version {})",
                if old.is_none() { "hosts" } else { "changed" },
                world.name,
                SocketAddr::new(world.host_ip, world.port),
                world.visibility.name(),
                world.max_players,
                if world.password.is_some() { "yes" } else { "no" },
                if world.version.is_empty() { "?" } else { &world.version },
            );
        }
        let address = SocketAddr::new(world.host_ip, world.port);
        s.worlds.insert(host, world);
        // the host plays in their own world
        s.members.remove(&host);
        s.pending.remove(&host);
        Reply::json(200, json!({
            "id": host.to_string(),
            "address": address.to_string(),
            "players": s.players(host),
            "expiresIn": WORLD_TTL.as_secs(),
        }))
    }

    /// DELETE worlds: the world ends; its players are released.
    fn stop(&self, host: u64, now: Instant) {
        let mut s = self.lock();
        s.purge(now);
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
                w.password.is_none()
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

    #[test]
    fn host_and_heartbeat() {
        let w = Worlds::new();
        let t0 = Instant::now();
        let b = host_world(&w, HOST, HOME, json!({}), t0);
        assert_eq!(b, json!({"id": "100", "address": "203.0.113.5:5101", "players": 1, "expiresIn": 45}));
        // heartbeat keeps it past 45 s
        host_world(&w, HOST, HOME, json!({}), t0 + Duration::from_secs(30));
        assert!(w.choose(&who(5, AWAY), t0 + Duration::from_secs(60)).is_some());
        assert_eq!(w.counts().0, 1);
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
        let c = w.choose(&who(HOST, HOME), t).unwrap();
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
        let c = w.choose(&who(7, "192.168.68.93"), t).unwrap();
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
        let c = w.choose(&who(5, AWAY), t).unwrap();
        assert_eq!((c.host, c.via), (300, Via::Auto));
        assert_eq!(c.addr, "192.0.2.2:5101".parse().unwrap());
        // somebody behind the same public address as host 200: its LAN address
        let c = w.choose(&who(6, "192.0.2.1"), t).unwrap();
        assert_eq!(c.host, 200);
        assert_eq!(c.addr, "192.168.68.40:5101".parse().unwrap());
        // a tie in players: the latest heartbeat
        let w = Worlds::new();
        host_world(&w, 200, "192.0.2.1", json!({}), t);
        host_world(&w, 300, "192.0.2.2", json!({}), t + Duration::from_secs(1));
        assert_eq!(w.choose(&who(5, AWAY), t + Duration::from_secs(2)).unwrap().host, 300);
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
        assert_eq!(w.choose(&who(1, AWAY), t).unwrap().host, HOST);
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
        let c = w.choose(&who(5, AWAY), t).unwrap();
        w.assign(5, Some(&c), t);
        // 200 holds 2: the host and player 5
        assert_eq!(join(200, 6), (403, "full".into()));
        // a pending choice for a world that filled up is skipped
        let w2 = Worlds::new();
        host_world(&w2, 200, "192.0.2.1", json!({"maxPlayers": 2}), t);
        call(&w2, "POST", "worlds/200/join", json!({}), &who(6, AWAY), t);
        let c = w2.choose(&who(5, AWAY), t).unwrap();
        w2.assign(5, Some(&c), t);
        assert!(w2.choose(&who(6, AWAY), t).is_none());
    }

    #[test]
    fn world_of() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, HOST, "192.0.2.3", json!({"visibility": "friends", "password": "pw"}), t);
        let of = |x: u64, caller: u64| call(&w, "GET", &format!("worlds/of/{x}"), Value::Null, &who(caller, AWAY), t).body.unwrap();
        assert_eq!(of(5, 1), json!({"world": null}));
        assert_eq!(of(HOST, 5), json!({"world": {
            "id": "100", "name": "Mundo do Alex", "host": "Player#100", "players": 1, "maxPlayers": 12,
            "password": true, "canJoin": false, "why": "friends",
        }}));
        assert_eq!(of(HOST, 1)["world"]["canJoin"], true);
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
        let c = w.choose(&who(5, AWAY), t0).unwrap();
        w.assign(5, Some(&c), t0);
        assert!(w.choose(&who(5, AWAY), t0 + Duration::from_secs(44)).is_some());
        let later = t0 + WORLD_TTL + Duration::from_secs(1);
        assert!(w.choose(&who(5, AWAY), later).is_none());
        assert_eq!(w.counts(), (0, 0, 0));
        assert_eq!(call(&w, "GET", "worlds", Value::Null, &who(5, AWAY), later).body.unwrap(), json!({"worlds": []}));
    }

    #[test]
    fn delete_releases_members() {
        let w = Worlds::new();
        let t = Instant::now();
        host_world(&w, 200, "192.0.2.1", json!({}), t);
        let c = w.choose(&who(5, AWAY), t).unwrap();
        w.assign(5, Some(&c), t);
        call(&w, "POST", "worlds/200/join", json!({}), &who(6, AWAY), t);
        assert_eq!(w.counts(), (1, 1, 1));
        let r = call(&w, "DELETE", "worlds", Value::Null, &who(200, "192.0.2.1"), t);
        assert_eq!(r.status, 204);
        assert!(r.body.is_none());
        assert_eq!(w.counts(), (0, 0, 0));
        assert!(w.choose(&who(5, AWAY), t).is_none());
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
