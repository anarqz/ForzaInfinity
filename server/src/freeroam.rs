//! Online free roam (PLANO_FREEROAM.md, phase F0): the Xbox Live game-server and session calls
//! that take the game into online free roam and to a world's relay.
//!
//! Ported from the fh2-mods research (`tools/online/server/fh2srv.py`, MIT; line refs below are
//! to that file):
//!   - `gameserverds.xboxlive.com …/variants`: one variant, `Release_6CC152FD.FreeRoam`;
//!   - `gameserverds.xboxlive.com /qosservers`: exactly one QoS server;
//!   - `gameserverms.xboxlive.com …/clusters` (POST): the cluster is "Fulfilled" at once;
//!   - `sessiondirectory.xboxlive.com /serviceconfigs/…` (MPSD, contract 105), in memory;
//!   - `momatch.xboxlive.com /serviceconfigs/…/hoppers/…` (SmartMatch, contract 103): every
//!     ticket is found at once (not in fh2srv.py; see `smartmatch`).
//!
//! The trick (fh2srv.py 203-211): when a session written by the game has
//! `constants.custom.gameServerPlatform`, we fill `servers.gameServerPlatform.properties.custom`
//! with `{status: "Active", datacenter, secureDeviceAddress, hostName}` as Xbox Live Compute
//! used to. The game (0x14183B360) reads it and AllocateServer (0x141836190) skips the cluster
//! allocation.
//!
//! **World address format.** `secureDeviceAddress` is base64 (standard alphabet, padded) of the
//! ASCII text `fh2inf:<ip>:<port>`, IPv6 in brackets: `fh2inf:192.168.68.40:5101`,
//! `fh2inf:[2001:db8::1]:5101`. The hook decodes it (SecureDeviceAddress.FromBase64String) and
//! its Association answers with that socket address. With no world configured we send
//! fh2srv.py's placeholder (base64 of "fh2net-game-server", line 94) and hostName 127.0.0.1.
//!
//! The world comes from `world_for(xuid)`: in F0 the server-wide `--freeroam-world HOST:PORT`;
//! in F1 the world the account chose.
//!
//! Only with `--freeroam` or `--freeroam-world` (main.rs). Without them, and for requests
//! without an account, these hosts keep their static stubs (responses/hosts/<host>/).
//! Paths not covered here also fall back to the stubs (fh2srv.py answered them 503).

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Map, Value, json};

pub const GAMESERVERDS: &str = "gameserverds.xboxlive.com";
pub const GAMESERVERMS: &str = "gameserverms.xboxlive.com";
pub const SESSIONDIRECTORY: &str = "sessiondirectory.xboxlive.com";
pub const MOMATCH: &str = "momatch.xboxlive.com";
/// Most match tickets kept at once (a ticket lives as long as its sessions' TTL).
const MAX_TICKETS: usize = 512;

// fh2srv.py 86-88
const GAME_VARIANT: &str = "00000000-0000-0000-0000-00000000f001";
const VARIANT_SCHEMA: &str = "00000000-0000-0000-0000-00000000f002";
const GSI_SET: &str = "00000000-0000-0000-0000-00000000f003";
/// fh2srv.py 89-91: the game takes server images named 'Release_' + its build hash + '.*'
/// (0x141838D70; the hash '6CC152FD' is the string at 0x142F4DAF0).
const GSI_NAME: &str = "Release_6CC152FD.FreeRoam";
/// fh2srv.py 92: the game's default datacenter (0x1422EDAE8).
const LOCATION: &str = "North Central US";
/// fh2srv.py 94-95: base64 of "fh2net-game-server", when no world is configured.
const PLACEHOLDER_ADDRESS: &str = "ZmgybmV0LWdhbWUtc2VydmVy";
const PLACEHOLDER_HOST: &str = "127.0.0.1";
/// Prefix of the address payload the hook decodes (see the module docs).
pub const ADDRESS_PREFIX: &str = "fh2inf:";

/// Sessions untouched this long are dropped.
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// Most sessions kept at once (the least recently touched goes first).
const MAX_SESSIONS: usize = 512;
/// Largest session document (serialized); a write that would grow past it is refused.
const MAX_DOC: usize = 128 << 10;

/// A world's relay (the dedicated server the game connects to).
pub type WorldAddr = SocketAddr;

/// An answer: status and JSON body (None: no body, for 204).
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Option<Value>,
}

impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Reply { status, body: Some(body) }
    }
}

struct Session {
    doc: Value,
    touched: Instant,
}

pub struct Freeroam {
    /// The world every player is sent to (F0): --freeroam-world.
    world: Option<WorldAddr>,
    /// MPSD sessions by path (lower-cased: session names are case-insensitive).
    sessions: Mutex<HashMap<String, Session>>,
    /// SmartMatch tickets by id: (ticket session key, its details, created).
    tickets: Mutex<HashMap<String, (String, Value, Instant)>>,
    ids: AtomicU64,
}

/// `fh2inf:<ip>:<port>` in base64 (the module docs).
pub fn secure_device_address(world: &WorldAddr) -> String {
    // SocketAddr's Display already brackets IPv6: "[2001:db8::1]:5101"
    B64.encode(format!("{ADDRESS_PREFIX}{world}"))
}

/// fh2srv.py 128-132: deep merge; a null removes the key.
fn merge(dst: &mut Map<String, Value>, src: Map<String, Value>) {
    for (k, v) in src {
        match v {
            Value::Null => {
                dst.remove(&k);
            }
            Value::Object(o) if dst.get(&k).is_some_and(Value::is_object) => {
                if let Some(Value::Object(d)) = dst.get_mut(&k) {
                    merge(d, o);
                }
            }
            v => {
                dst.insert(k, v);
            }
        }
    }
}

fn now_iso() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    // ISO 8601 with milliseconds, as the session directory wrote them (fh2srv.py 171: six-digit
    // fractions are rejected)
    crate::forza::iso_utc(ms)
}

/// The fake device token of a member (fh2srv.py 200 used md5 of 'fh2net-device-' + xuid): 32
/// upper-case hex digits, stable per player.
fn device_token(xuid: u64) -> String {
    let digest = sha1_smol::Sha1::from(format!("fh2inf-device-{xuid}")).digest().to_string();
    digest[..32].to_ascii_uppercase()
}

impl Freeroam {
    pub fn new(world: Option<WorldAddr>) -> Self {
        Freeroam { world, sessions: Mutex::new(HashMap::new()), tickets: Mutex::new(HashMap::new()), ids: AtomicU64::new(0) }
    }

    /// The world a player is sent to. F0: the configured one for everybody; F1 will pick the
    /// account's chosen world here.
    pub fn world_for(&self, _xuid: u64) -> Option<WorldAddr> {
        self.world
    }

    /// `properties.custom` of servers.gameServerPlatform (fh2srv.py 138-140, 209-211): the game
    /// (0x14183B360) reads status (Active / Queued / Error / Quarantined), datacenter,
    /// secureDeviceAddress (base64) and hostName.
    fn server_entry(&self, xuid: u64, location: &str) -> Value {
        let (address, host) = match self.world_for(xuid) {
            Some(w) => (secure_device_address(&w), w.ip().to_string()),
            None => (PLACEHOLDER_ADDRESS.to_owned(), PLACEHOLDER_HOST.to_owned()),
        };
        json!({
            "constants": { "system": {}, "custom": {} },
            "properties": { "system": {}, "custom": {
                "status": "Active", "datacenter": location, "secureDeviceAddress": address, "hostName": host,
            } },
        })
    }

    /// A request for one of the hosts above; None: not ours (the static stub answers).
    /// `gamertag` is asked only when a session member is written.
    pub fn handle(
        &self,
        method: &str,
        host: &str,
        path: &str,
        body: &[u8],
        xuid: u64,
        gamertag: &dyn Fn(u64) -> String,
    ) -> Option<Reply> {
        self.handle_at(method, host, path, body, xuid, gamertag, Instant::now())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_at(
        &self,
        method: &str,
        host: &str,
        path: &str,
        body: &[u8],
        xuid: u64,
        gamertag: &dyn Fn(u64) -> String,
        now: Instant,
    ) -> Option<Reply> {
        let path = path.split('?').next().unwrap_or(path);
        match host {
            GAMESERVERDS => gsds(path),
            GAMESERVERMS => self.gsms(method, path, body, xuid, now),
            SESSIONDIRECTORY if path.starts_with("/serviceconfigs/") => {
                self.mpsd(method, path, body, xuid, gamertag, now)
            }
            MOMATCH if path.to_ascii_lowercase().starts_with("/serviceconfigs/") => {
                self.smartmatch(method, path, body, xuid, gamertag, now)
            }
            _ => None,
        }
    }

    /// fh2srv.py 143-158: XSAPI AllocateClusterAsync, POST /titles/<id>/clusters {sessionId}.
    /// The session named gets its server entry and the cluster is fulfilled at once.
    fn gsms(&self, method: &str, path: &str, body: &[u8], xuid: u64, now: Instant) -> Option<Reply> {
        if method != "POST" || !path.ends_with("/clusters") {
            return None;
        }
        let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let sid = request["sessionId"].as_str().unwrap_or("").to_ascii_lowercase();
        let last = sid.rsplit('/').next().unwrap_or("");
        if !sid.is_empty() {
            let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(session) = sessions.iter_mut().find(|(k, _)| k.ends_with(last) || k.contains(&sid)).map(|(_, s)| s) {
                let location = session.doc["constants"]["custom"]["gameServerPlatform"]["location"]
                    .as_str()
                    .unwrap_or(LOCATION)
                    .to_owned();
                session.doc["servers"]["gameServerPlatform"] = self.server_entry(xuid, &location);
                session.touched = now;
                println!("freeroam: cluster for session {last} (xuid {xuid})");
            }
        }
        let (address, host) = match self.world_for(xuid) {
            Some(w) => (secure_device_address(&w), w.ip().to_string()),
            None => (PLACEHOLDER_ADDRESS.to_owned(), PLACEHOLDER_HOST.to_owned()),
        };
        // fh2srv.py 156-158 (portMappings as there: the game's own port, not read for the relay)
        Some(Reply::json(200, json!({
            "pollIntervalMilliseconds": 1000,
            "fulfillmentState": "Fulfilled",
            "hostName": host,
            "region": LOCATION,
            "secureDeviceAddress": address,
            "portMappings": [{ "name": "Game", "value": 5100, "internal": 5100, "external": 5100 }],
        })))
    }

    /// fh2srv.py 165-216: the multiplayer session directory (contract 105), in memory.
    fn mpsd(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        xuid: u64,
        gamertag: &dyn Fn(u64) -> String,
        now: Instant,
    ) -> Option<Reply> {
        if method == "GET" && path.ends_with("/sessions") {
            // fh2srv.py 167-169: the sessions a user is in. Always none: listing an earlier
            // run's sessions made the game try to rejoin them at startup.
            return Some(Reply::json(200, json!({ "results": [] })));
        }
        if !path.contains("/sessions/") {
            return None;
        }
        let key = path.to_ascii_lowercase();
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        sessions.retain(|_, s| now.saturating_duration_since(s.touched) < SESSION_TTL);
        match method {
            "GET" => {
                return Some(match sessions.get_mut(&key) {
                    Some(s) => {
                        s.touched = now;
                        Reply::json(200, s.doc.clone())
                    }
                    None => Reply::json(404, json!({})),
                });
            }
            "DELETE" => {
                sessions.remove(&key);
                // fh2srv.py 178 sent "{}" with its 204; a 204 carries no body
                return Some(Reply { status: 204, body: None });
            }
            "PUT" | "POST" => {}
            _ => return None,
        }
        let Ok(Value::Object(mut request)) = serde_json::from_slice::<Value>(if body.is_empty() { b"{}" } else { body })
        else {
            return Some(Reply::json(400, json!({ "error": "the body is not a JSON object" })));
        };
        let created = !sessions.contains_key(&key);
        let mut doc = match sessions.get(&key) {
            Some(s) => s.doc.clone(),
            None => {
                let ts = now_iso();
                // fh2srv.py 181-187: the fields Microsoft.Xbox.Services.dll's session deserializer
                // reads (0x180112A90, 0x1801236E0)
                json!({
                    "contractVersion": 105, "correlationId": self.uuid(&key), "startTime": ts, "nextTimer": ts,
                    "constants": { "system": {}, "custom": {} },
                    "properties": { "system": { "keywords": [], "turn": [], "joinRestriction": "none" }, "custom": {} },
                    "servers": {}, "members": {}, "hostCandidates": [],
                    "membersInfo": { "first": 0, "next": 0, "count": 0, "accepted": 0, "active": 0 },
                })
            }
        };
        let members = match request.remove("members") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        if let Value::Object(d) = &mut doc {
            merge(d, request);
            // a body may null these out; the code below and the game need them
            for (field, empty) in [("constants", json!({})), ("servers", json!({})), ("members", json!({}))] {
                if !d.get(field).is_some_and(Value::is_object) {
                    d.insert(field.into(), empty);
                }
            }
            if !d.get("membersInfo").is_some_and(Value::is_object) {
                d.insert("membersInfo".into(), json!({ "first": 0, "next": 0, "count": 0, "accepted": 0, "active": 0 }));
            }
        }
        // fh2srv.py 189-202: "me" (and "reserve_N") is the caller. A reservation that names a
        // player (constants.system.xuid) is that player (fh2srv.py used the caller for those too).
        for (name, member) in members {
            if name != "me" && !name.starts_with("reserve_") {
                continue;
            }
            let who = if name == "me" {
                xuid
            } else {
                member["constants"]["system"]["xuid"]
                    .as_str()
                    .and_then(|x| x.parse().ok())
                    .or_else(|| member["constants"]["system"]["xuid"].as_u64())
                    .unwrap_or(xuid)
            };
            let index = doc["members"]
                .as_object()
                .and_then(|m| m.iter().find(|(_, v)| v["constants"]["system"]["xuid"] == json!(who.to_string())))
                .map(|(i, _)| i.clone());
            let Value::Object(member) = member else {
                // null (or anything else): the member leaves
                if let (Some(i), Some(m)) = (index, doc["members"].as_object_mut()) {
                    m.remove(&i);
                }
                continue;
            };
            let index = match index {
                Some(i) => i,
                None => {
                    let next = doc["membersInfo"]["next"].as_u64().unwrap_or(0);
                    doc["membersInfo"]["next"] = json!(next + 1);
                    let i = next.to_string();
                    doc["members"][&i] = json!({
                        "constants": { "system": { "xuid": who.to_string(), "index": next }, "custom": {} },
                        "properties": { "system": { "active": false, "ready": false }, "custom": {} },
                        "gamertag": match gamertag(who) {
                            t if t.is_empty() => format!("Driver {}", who % 10000),
                            t => crate::db::game_tag(&t),
                        },
                        "deviceToken": device_token(who),
                        "nat": "open",
                        "activeTitleId": "446059611",
                        "reserved": false,
                        "joinTime": now_iso(),
                        "turn": false,
                        "initializationEpisode": 0,
                    });
                    i
                }
            };
            if let Some(Value::Object(m)) = doc["members"].get_mut(&index) {
                merge(m, member);
            }
        }
        // the trick (fh2srv.py 203-211): a game session gets its server at once
        let location = match &doc["constants"]["custom"]["gameServerPlatform"] {
            Value::Null => None,
            gsp => Some(gsp["location"].as_str().unwrap_or(LOCATION).to_owned()),
        };
        if let Some(location) = location {
            if doc["servers"].get("gameServerPlatform").is_none() {
                doc["servers"]["gameServerPlatform"] = self.server_entry(xuid, &location);
                let to = self.world_for(xuid).map(|w| w.to_string()).unwrap_or_else(|| "placeholder".into());
                println!("freeroam: session {} -> world {to} (xuid {xuid})", short(&key));
            }
        }
        // fh2srv.py 212-214
        let (count, active, first) = match doc["members"].as_object() {
            Some(m) => (
                m.len(),
                m.values().filter(|v| v["properties"]["system"]["active"] == json!(true)).count(),
                m.keys().filter_map(|k| k.parse::<u64>().ok()).min(),
            ),
            None => (0, 0, None),
        };
        doc["membersInfo"]["count"] = json!(count);
        doc["membersInfo"]["accepted"] = json!(count);
        doc["membersInfo"]["active"] = json!(active);
        if let Some(first) = first {
            doc["membersInfo"]["first"] = json!(first);
        }
        if serde_json::to_vec(&doc).map(|v| v.len()).unwrap_or(usize::MAX) > MAX_DOC {
            return Some(Reply::json(413, json!({ "error": "session too large" })));
        }
        if created {
            if sessions.len() >= MAX_SESSIONS {
                if let Some(oldest) = sessions.iter().min_by_key(|(_, s)| s.touched).map(|(k, _)| k.clone()) {
                    sessions.remove(&oldest);
                }
            }
            println!("freeroam: session {} created (xuid {xuid}, {} in memory)", short(&key), sessions.len() + 1);
        }
        sessions.insert(key, Session { doc: doc.clone(), touched: now });
        // fh2srv.py 216
        Some(Reply::json(if created { 201 } else { 200 }, doc))
    }

    /// SmartMatch (momatch, contract 103), as Microsoft.Xbox.Services.dll calls it
    /// (MatchmakingService: CreateMatchTicketAsync, GetMatchTicketDetailsAsync,
    /// DeleteMatchTicketAsync, GetHopperStatisticsAsync). After writing its ticket session
    /// (properties.system.matchmaking.targetSessionConstants), the game creates a ticket for a
    /// hopper (FreeRoam_qos, CarMeet_qos, RoadTrip_qos); with no answer it deletes the ticket and
    /// shows an error. Here every ticket is found at once: the hopper has one shared target
    /// session (the world), made from the ticket's targetSessionConstants with the player
    /// reserved in it and its game server filled, and the ticket session's
    /// servers.matchmaking says "found" with the targetSessionRef, as SmartMatch wrote it.
    ///   POST   /serviceconfigs/{scid}/hoppers/{hopper}              -> {ticketId, waitTime}
    ///   GET    /serviceconfigs/{scid}/hoppers/{hopper}/tickets/{id} -> ticket details
    ///   DELETE /serviceconfigs/{scid}/hoppers/{hopper}/tickets/{id} -> 204
    ///   GET    /serviceconfigs/{scid}/hoppers/{hopper}/stats        -> {name, waitTime, population}
    fn smartmatch(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        xuid: u64,
        gamertag: &dyn Fn(u64) -> String,
        now: Instant,
    ) -> Option<Reply> {
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        // serviceconfigs, scid, hoppers, hopper [, tickets, id | stats]
        if parts.len() < 4 || !parts[2].eq_ignore_ascii_case("hoppers") {
            return None;
        }
        let (scid, hopper) = (parts[1], parts[3]);
        let what = parts.get(4).map(|p| p.to_ascii_lowercase());
        match (method, what.as_deref(), parts.get(5).filter(|id| !id.is_empty())) {
            ("POST", None, _) => {}
            ("GET", Some("stats"), _) => {
                let population = self.tickets.lock().unwrap_or_else(|p| p.into_inner()).len();
                return Some(Reply::json(200, json!({ "name": hopper, "waitTime": 0, "population": population })));
            }
            ("GET", Some("tickets"), Some(id)) => {
                let tickets = self.tickets.lock().unwrap_or_else(|p| p.into_inner());
                return Some(match tickets.get(*id) {
                    Some((_, details, _)) => Reply::json(200, details.clone()),
                    None => Reply::json(404, json!({})),
                });
            }
            ("DELETE", Some("tickets"), id) => {
                if let Some(id) = id {
                    self.tickets.lock().unwrap_or_else(|p| p.into_inner()).remove(*id);
                }
                return Some(Reply { status: 204, body: None });
            }
            _ => return None,
        }
        let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let ticket_ref = &request["ticketSessionRef"];
        let (Some(ticket_template), Some(ticket_name)) =
            (ticket_ref["templateName"].as_str(), ticket_ref["name"].as_str())
        else {
            return Some(Reply::json(400, json!({ "error": "ticketSessionRef is missing" })));
        };
        let ticket_scid = ticket_ref["scid"].as_str().unwrap_or(scid);
        let ticket_key = format!("/serviceconfigs/{ticket_scid}/sessiontemplates/{ticket_template}/sessions/{ticket_name}")
            .to_ascii_lowercase();
        // one world per hopper: everybody who looks for FreeRoam_qos meets in the same session
        let target_name = format!("fh2inf-{}", hopper.to_ascii_lowercase());
        let target_ref = json!({ "scid": ticket_scid, "templateName": ticket_template, "name": target_name });
        let target_key = format!("/serviceconfigs/{ticket_scid}/sessiontemplates/{ticket_template}/sessions/{target_name}")
            .to_ascii_lowercase();

        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        sessions.retain(|_, s| now.saturating_duration_since(s.touched) < SESSION_TTL);
        let Some(ticket_session) = sessions.get_mut(&ticket_key) else {
            return Some(Reply::json(404, json!({ "error": "the ticket session does not exist" })));
        };
        let target_constants = ticket_session.doc["properties"]["system"]["matchmaking"]["targetSessionConstants"].clone();
        // the ticket session: SmartMatch's result (MultiplayerSession reads
        // servers.matchmaking.properties.system: status, targetSessionRef)
        ticket_session.doc["servers"]["matchmaking"] = json!({
            "constants": { "system": {}, "custom": {} },
            "properties": { "system": {
                "status": "found", "statusDetails": "", "typicalWait": 0, "targetSessionRef": target_ref.clone(),
            }, "custom": {} },
        });
        ticket_session.touched = now;

        // the target session (the world), made or refreshed
        if !sessions.contains_key(&target_key) {
            let ts = now_iso();
            println!("freeroam: world session {target_name} created for hopper {hopper}");
            sessions.insert(target_key.clone(), Session {
                doc: json!({
                    "contractVersion": 105, "correlationId": self.uuid(&target_key), "startTime": ts, "nextTimer": ts,
                    "constants": { "system": { "visibility": "open", "maxMembersCount": 12 }, "custom": {} },
                    "properties": { "system": { "keywords": [], "turn": [], "joinRestriction": "none" }, "custom": {} },
                    "servers": {}, "members": {}, "hostCandidates": [],
                    "membersInfo": { "first": 0, "next": 0, "count": 0, "accepted": 0, "active": 0 },
                }),
                touched: now,
            });
        }
        let server = |location: &str| self.server_entry(xuid, location);
        let Some(target) = sessions.get_mut(&target_key) else {
            return Some(Reply::json(500, json!({})));
        };
        target.touched = now;
        if let (Value::Object(constants), Value::Object(from)) = (&mut target.doc["constants"], target_constants) {
            merge(constants, from);
        }
        let location = target.doc["constants"]["custom"]["gameServerPlatform"]["location"]
            .as_str()
            .unwrap_or(LOCATION)
            .to_owned();
        target.doc["servers"]["gameServerPlatform"] = server(&location);
        // the player, reserved in the world as SmartMatch did (the game then writes "me" active)
        let present = target.doc["members"]
            .as_object()
            .is_some_and(|m| m.values().any(|v| v["constants"]["system"]["xuid"] == json!(xuid.to_string())));
        if !present {
            let next = target.doc["membersInfo"]["next"].as_u64().unwrap_or(0);
            target.doc["membersInfo"]["next"] = json!(next + 1);
            target.doc["members"][next.to_string()] = json!({
                "constants": { "system": { "xuid": xuid.to_string(), "index": next }, "custom": {} },
                "properties": { "system": { "active": false, "ready": false }, "custom": {} },
                "gamertag": match gamertag(xuid) {
                    t if t.is_empty() => format!("Driver {}", xuid % 10000),
                    t => crate::db::game_tag(&t),
                },
                "deviceToken": device_token(xuid),
                "nat": "open",
                "activeTitleId": "446059611",
                "reserved": true,
                "joinTime": now_iso(),
                "turn": false,
                "initializationEpisode": 0,
            });
            let count = target.doc["members"].as_object().map(|m| m.len()).unwrap_or(0);
            target.doc["membersInfo"]["count"] = json!(count);
            target.doc["membersInfo"]["accepted"] = json!(count);
        }
        drop(sessions);

        let ticket_id = self.uuid(&ticket_key);
        let details = json!({
            "ticketStatus": "found",
            "waitTime": 0,
            "preserveSession": request["preserveSession"].clone(),
            "ticketSessionRef": ticket_ref.clone(),
            "targetSessionRef": target_ref,
            "ticketAttributes": request["ticketAttributes"].clone(),
        });
        let mut tickets = self.tickets.lock().unwrap_or_else(|p| p.into_inner());
        tickets.retain(|_, (_, _, made)| now.saturating_duration_since(*made) < SESSION_TTL);
        if tickets.len() >= MAX_TICKETS {
            if let Some(oldest) = tickets.iter().min_by_key(|(_, t)| t.2).map(|(k, _)| k.clone()) {
                tickets.remove(&oldest);
            }
        }
        tickets.insert(ticket_id.clone(), (ticket_key, details, now));
        println!("freeroam: ticket {ticket_id} for {hopper} found -> {target_name} (xuid {xuid})");
        Some(Reply::json(200, json!({ "ticketId": ticket_id, "waitTime": 0 })))
    }

    /// A random-looking version 4 UUID (no rand crate): the session path, the time and a counter.
    fn uuid(&self, key: &str) -> String {
        let n = self.ids.fetch_add(1, Ordering::Relaxed);
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let h = sha1_smol::Sha1::from(format!("{key}|{t}|{n}")).digest().bytes();
        let mut b = [0u8; 16];
        b.copy_from_slice(&h[..16]);
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let x: String = b.iter().map(|v| format!("{v:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &x[..8], &x[8..12], &x[12..16], &x[16..20], &x[20..])
    }

    #[cfg(test)]
    fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

/// fh2srv.py 98-121: the game server directory (no state).
fn gsds(path: &str) -> Option<Reply> {
    if path.ends_with("/variants") {
        // fh2srv.py 99-107: the game reads these into GSDSVariant / GSDSVariantSchema /
        // GSDSGSISet, whose fields are all strings (a JSON number or bool in a string field
        // throws in its parser): every value is a string.
        return Some(Reply::json(200, json!({
            "variants": [{
                "gamevariantId": GAME_VARIANT, "name": GSI_NAME, "isPublisher": "false", "rank": "1",
                "gameVariantSchemaId": VARIANT_SCHEMA,
            }],
            "variantSchemas": [{ "variantSchemaId": VARIANT_SCHEMA, "schemaContent": "{}", "name": GSI_NAME }],
            "gsiSets": [{
                "minRequiredPlayers": "1", "maxAllowedPlayers": "12", "gsiSetId": GSI_SET, "gsiSetName": GSI_NAME,
                "selectionOrder": "1", "variantSchemaId": VARIANT_SCHEMA,
            }],
        })));
    }
    if path == "/qosservers" {
        // fh2srv.py 113-120: ForzaLiveThunderheadGetQosServers_Results.qosServers, GSDSQosServer
        // (serverFqdn, serverSecureDeviceAddress (base64), targetLocation), all strings.
        // Microsoft.Xbox.Services.dll reads the array named qosServers (0x1800989C0 ->
        // 0x18009FF00); with a second 'qosservers' key too, the game got an empty list.
        return Some(Reply::json(200, json!({
            "qosServers": [{
                "serverFqdn": "qos.fh2net.local",
                "serverSecureDeviceAddress": "ZmgybmV0LXFvcy1zZXJ2ZXI=",
                "targetLocation": LOCATION,
            }],
        })));
    }
    None
}

/// The session's name, for the log.
fn short(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "/serviceconfigs/788f0100-a4bc-46c3-b19b-d1001a96545b/sessiontemplates/FreeRoam/sessions/ABC123";

    fn tag(xuid: u64) -> String {
        format!("Player#{xuid}")
    }

    fn call(f: &Freeroam, method: &str, host: &str, path: &str, body: Value, xuid: u64) -> Reply {
        let body = if body.is_null() { Vec::new() } else { serde_json::to_vec(&body).unwrap() };
        f.handle(method, host, path, &body, xuid, &tag).expect("handled")
    }

    fn all_strings(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.values().all(all_strings),
            Value::Array(a) => a.iter().all(all_strings),
            Value::String(_) => true,
            _ => false,
        }
    }

    #[test]
    fn variants_are_strings() {
        let f = Freeroam::new(None);
        let r = call(&f, "POST", GAMESERVERDS, "/titles/1368890819/variants", json!({"locale": "en-us", "maxVariants": 8, "publisherOnly": false}), 1);
        assert_eq!(r.status, 200);
        let b = r.body.unwrap();
        assert!(all_strings(&b), "{b}");
        assert_eq!(b["variants"][0]["name"], GSI_NAME);
        assert_eq!(b["variants"].as_array().unwrap().len(), 1);
        assert_eq!(b["gsiSets"][0]["maxAllowedPlayers"], "12");
        assert_eq!(b["gsiSets"][0]["gsiSetName"], "Release_6CC152FD.FreeRoam");
        assert_eq!(b["variantSchemas"][0]["variantSchemaId"], VARIANT_SCHEMA);
    }

    #[test]
    fn qosservers_one_key_one_server() {
        let f = Freeroam::new(None);
        let b = call(&f, "GET", GAMESERVERDS, "/qosservers", Value::Null, 1).body.unwrap();
        let obj = b.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert_eq!(b["qosServers"].as_array().unwrap().len(), 1);
        assert!(all_strings(&b));
        assert_eq!(b["qosServers"][0]["targetLocation"], LOCATION);
        // anything else on the host is the stub's
        assert!(f.handle("GET", GAMESERVERDS, "/other", b"", 1, &tag).is_none());
    }

    #[test]
    fn listing_is_empty() {
        let f = Freeroam::new(None);
        call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"members": {"me": {}}}), 7);
        let path = "/serviceconfigs/788f0100-a4bc-46c3-b19b-d1001a96545b/sessiontemplates/defaultTemplate_qos/sessions";
        let r = f.handle("GET", SESSIONDIRECTORY, &format!("{path}?xuid=7&private=true&take=32"), b"", 7, &tag).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body.unwrap(), json!({"results": []}));
        // not a session path: the stub answers
        assert!(f.handle("POST", SESSIONDIRECTORY, "/handles", b"{}", 7, &tag).is_none());
    }

    #[test]
    fn put_merges_and_get_returns() {
        let f = Freeroam::new(None);
        let r = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"properties": {"custom": {"a": 1, "b": {"x": 1}}}}), 7);
        assert_eq!(r.status, 201);
        let doc = r.body.unwrap();
        assert_eq!(doc["contractVersion"], 105);
        assert!(doc["startTime"].as_str().unwrap().ends_with('Z'));
        assert_eq!(doc["startTime"].as_str().unwrap().len(), 24);
        let r = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"properties": {"custom": {"a": null, "b": {"y": 2}}}}), 7);
        assert_eq!(r.status, 200);
        // a different case is the same session
        let doc = call(&f, "GET", SESSIONDIRECTORY, &SESSION.to_lowercase(), Value::Null, 7).body.unwrap();
        assert_eq!(doc["properties"]["custom"], json!({"b": {"x": 1, "y": 2}}));
        assert_eq!(doc["properties"]["system"]["joinRestriction"], "none");
        let r = call(&f, "GET", SESSIONDIRECTORY, &format!("{SESSION}X"), Value::Null, 7);
        assert_eq!(r.status, 404);
        assert_eq!(call(&f, "DELETE", SESSIONDIRECTORY, SESSION, Value::Null, 7).status, 204);
        assert_eq!(call(&f, "GET", SESSIONDIRECTORY, SESSION, Value::Null, 7).status, 404);
    }

    #[test]
    fn me_is_the_caller() {
        let f = Freeroam::new(None);
        let me = json!({"members": {"me": {"constants": {"system": {"initialize": true}}, "properties": {"system": {"active": true}}}}});
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, me.clone(), 111).body.unwrap();
        let m = &doc["members"]["0"];
        assert_eq!(m["constants"]["system"]["xuid"], "111");
        assert_eq!(m["constants"]["system"]["initialize"], true);
        assert_eq!(m["gamertag"], "Player#111");
        assert_eq!(m["nat"], "open");
        assert_eq!(m["deviceToken"].as_str().unwrap().len(), 32);
        // the same player again: same member; another player: a second one
        call(&f, "PUT", SESSIONDIRECTORY, SESSION, me.clone(), 111);
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, me, 222).body.unwrap();
        assert_eq!(doc["members"]["1"]["constants"]["system"]["xuid"], "222");
        assert_eq!(doc["membersInfo"]["count"], 2);
        assert_eq!(doc["membersInfo"]["active"], 2);
        assert_eq!(doc["membersInfo"]["next"], 2);
        // a reservation naming a player is that player
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"members": {"reserve_0": {"constants": {"system": {"xuid": "333"}}}}}), 111)
            .body
            .unwrap();
        assert_eq!(doc["members"]["2"]["gamertag"], "Player#333");
        // leaving
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"members": {"me": null}}), 111).body.unwrap();
        assert!(doc["members"].get("0").is_none());
        assert_eq!(doc["membersInfo"]["count"], 2);
        assert_eq!(doc["membersInfo"]["first"], 1);
    }

    #[test]
    fn game_server_placeholder_without_world() {
        let f = Freeroam::new(None);
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"constants": {"custom": {"gameServerPlatform": {"location": "West Europe"}}}}), 7)
            .body
            .unwrap();
        let c = &doc["servers"]["gameServerPlatform"]["properties"]["custom"];
        assert_eq!(*c, json!({"status": "Active", "datacenter": "West Europe", "secureDeviceAddress": PLACEHOLDER_ADDRESS, "hostName": "127.0.0.1"}));
        // a session without it gets no server
        let doc = call(&f, "PUT", SESSIONDIRECTORY, &format!("{SESSION}2"), json!({"members": {"me": {}}}), 7).body.unwrap();
        assert_eq!(doc["servers"], json!({}));
    }

    #[test]
    fn game_server_with_world() {
        let world: WorldAddr = "192.168.68.40:5101".parse().unwrap();
        let f = Freeroam::new(Some(world));
        let doc = call(&f, "PUT", SESSIONDIRECTORY, SESSION, json!({"constants": {"custom": {"gameServerPlatform": {}}}}), 7).body.unwrap();
        let c = &doc["servers"]["gameServerPlatform"]["properties"]["custom"];
        assert_eq!(c["status"], "Active");
        assert_eq!(c["datacenter"], LOCATION);
        assert_eq!(c["hostName"], "192.168.68.40");
        let decoded = B64.decode(c["secureDeviceAddress"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, b"fh2inf:192.168.68.40:5101");
        // and the cluster allocation names the same world
        let r = call(&f, "POST", GAMESERVERMS, "/titles/1368890819/clusters", json!({"sessionId": "x/abc123"}), 7);
        let b = r.body.unwrap();
        assert_eq!(b["fulfillmentState"], "Fulfilled");
        assert_eq!(b["secureDeviceAddress"], c["secureDeviceAddress"]);
        assert!(f.handle("GET", GAMESERVERMS, "/titles/1/clusters", b"", 7, &tag).is_none());
    }

    #[test]
    fn address_format() {
        let v4: WorldAddr = "10.0.0.2:5101".parse().unwrap();
        assert_eq!(secure_device_address(&v4), B64.encode("fh2inf:10.0.0.2:5101"));
        assert_eq!(secure_device_address(&v4), "ZmgyaW5mOjEwLjAuMC4yOjUxMDE=");
        let v6: WorldAddr = "[2001:db8::1]:5101".parse().unwrap();
        assert_eq!(B64.decode(secure_device_address(&v6)).unwrap(), b"fh2inf:[2001:db8::1]:5101");
    }

    #[test]
    fn sessions_expire_and_are_capped() {
        let f = Freeroam::new(None);
        let t0 = Instant::now();
        let put = |path: &str, at: Instant| f.handle_at("PUT", SESSIONDIRECTORY, path, b"{}", 7, &tag, at).unwrap();
        assert_eq!(put(SESSION, t0).status, 201);
        let later = t0 + SESSION_TTL + Duration::from_secs(1);
        let r = f.handle_at("GET", SESSIONDIRECTORY, SESSION, b"", 7, &tag, later).unwrap();
        assert_eq!(r.status, 404);
        assert_eq!(f.session_count(), 0);
        for i in 0..MAX_SESSIONS + 5 {
            put(&format!("{SESSION}{i}"), t0 + Duration::from_millis(i as u64));
        }
        assert_eq!(f.session_count(), MAX_SESSIONS);
        // the oldest went first
        let r = f.handle_at("GET", SESSIONDIRECTORY, &format!("{SESSION}0"), b"", 7, &tag, t0).unwrap();
        assert_eq!(r.status, 404);
    }

    #[test]
    fn smartmatch_finds_the_world() {
        let world: WorldAddr = "192.168.68.40:5101".parse().unwrap();
        let f = Freeroam::new(Some(world));
        let scid = "788f0100-a4bc-46c3-b19b-d1001a96545b";
        let ticket_session = format!("/serviceconfigs/{scid}/sessionTemplates/defaultTemplate_qos/sessions/T1");
        let constants = json!({ "custom": { "hopperId": 2, "gameServerPlatform": { "gameVariantId": GAME_VARIANT, "maxAllowedPlayers": 12 } } });
        call(&f, "PUT", SESSIONDIRECTORY, &ticket_session, json!({
            "properties": { "system": { "matchmaking": { "targetSessionConstants": constants } } },
            "members": { "me": { "properties": { "system": { "active": true } } } },
        }), 7);
        let hopper = format!("/Serviceconfigs/{scid}/hoppers/FreeRoam_qos");
        let r = call(&f, "POST", MOMATCH, &hopper, json!({
            "preserveSession": "never", "scid": scid, "giveUpDuration": 3600000,
            "ticketSessionRef": { "name": "T1", "scid": scid, "templateName": "defaultTemplate_qos" },
            "ticketAttributes": { "hopperId": 2 },
        }), 7);
        assert_eq!(r.status, 200);
        let body = r.body.unwrap();
        let id = body["ticketId"].as_str().unwrap().to_owned();
        assert_eq!(body["waitTime"], json!(0));
        // the ticket session says found, with the target
        let t = call(&f, "GET", SESSIONDIRECTORY, &ticket_session, Value::Null, 7).body.unwrap();
        let mm = &t["servers"]["matchmaking"]["properties"]["system"];
        assert_eq!(mm["status"], json!("found"));
        assert_eq!(mm["targetSessionRef"]["name"], json!("fh2inf-freeroam_qos"));
        // the target: the ticket's constants, the world, the player reserved
        let target = format!("/serviceconfigs/{scid}/sessiontemplates/defaultTemplate_qos/sessions/fh2inf-freeroam_qos");
        let w = call(&f, "GET", SESSIONDIRECTORY, &target, Value::Null, 7).body.unwrap();
        assert_eq!(w["constants"]["custom"]["hopperId"], json!(2));
        assert_eq!(
            w["servers"]["gameServerPlatform"]["properties"]["custom"]["secureDeviceAddress"],
            json!(secure_device_address(&world))
        );
        assert_eq!(w["members"]["0"]["constants"]["system"]["xuid"], json!("7"));
        assert_eq!(w["members"]["0"]["reserved"], json!(true));
        // a second player meets in the same world; the first joining with "me" is not doubled
        call(&f, "PUT", SESSIONDIRECTORY, &format!("/serviceconfigs/{scid}/sessionTemplates/defaultTemplate_qos/sessions/T2"),
            json!({ "properties": { "system": { "matchmaking": { "targetSessionConstants": constants } } } }), 8);
        call(&f, "POST", MOMATCH, &hopper,
            json!({ "ticketSessionRef": { "name": "T2", "scid": scid, "templateName": "defaultTemplate_qos" } }), 8);
        call(&f, "PUT", SESSIONDIRECTORY, &target, json!({ "members": { "me": { "properties": { "system": { "active": true } } } } }), 7);
        let w = call(&f, "GET", SESSIONDIRECTORY, &target, Value::Null, 7).body.unwrap();
        assert_eq!(w["membersInfo"]["count"], json!(2));
        assert_eq!(w["membersInfo"]["active"], json!(1));
        // ticket details, stats, delete
        let d = call(&f, "GET", MOMATCH, &format!("{hopper}/tickets/{id}"), Value::Null, 7).body.unwrap();
        assert_eq!(d["ticketStatus"], json!("found"));
        assert_eq!(call(&f, "GET", MOMATCH, &format!("{hopper}/stats"), Value::Null, 7).status, 200);
        assert_eq!(call(&f, "DELETE", MOMATCH, &format!("{hopper}/tickets/{id}"), Value::Null, 7).status, 204);
        assert_eq!(call(&f, "GET", MOMATCH, &format!("{hopper}/tickets/{id}"), Value::Null, 7).status, 404);
        // the empty ticket id the game deletes after a failed create
        assert_eq!(call(&f, "DELETE", MOMATCH, &format!("{hopper}/tickets/"), Value::Null, 7).status, 204);
        // a ticket for a session that does not exist
        let r = call(&f, "POST", MOMATCH, &hopper, json!({ "ticketSessionRef": { "name": "nope", "templateName": "x" } }), 7);
        assert_eq!(r.status, 404);
    }

    #[test]
    fn bad_body() {
        let f = Freeroam::new(None);
        assert_eq!(f.handle("PUT", SESSIONDIRECTORY, SESSION, b"[1]", 7, &tag).unwrap().status, 400);
    }
}
