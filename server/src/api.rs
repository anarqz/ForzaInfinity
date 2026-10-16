//! The manager side of the server: JSON API, WebSocket upgrade and the built web UI.
//!
//!   GET    /api/users                     players, most recent first (with presence)
//!   POST   /api/users {gamertag}          create a player (a friend who has not played yet)
//!   GET    /api/users/<xuid>              everything about one player (profile, garage, friends...)
//!   DELETE /api/users/<xuid>              remove a player and their data
//!   GET    /api/users/<xuid>/friends      who the player follows, with presence
//!   POST   /api/users/<xuid>/friends {xuid}   make two players friends (both directions)
//!   PUT    /api/users/<xuid>/friends/<f> {favorite}   mark a friend as favorite
//!   DELETE /api/users/<xuid>/friends/<f> undo a friendship
//!
//!   GET    /api/users/<xuid>/notifications   the player's in-game inbox
//!   POST   /api/users/<xuid>/notifications {kind: "message", message}
//!          or {kind: "rival", from}: a test "your rival beat your time" on the player's last event
//!   DELETE /api/notifications/<id>
//!   GET    /api/clubs                     clubs (members, invites, requests, notices) + keywords
//!   POST   /api/clubs {owner, tag, name, description, privacy, region, attitude, interest}
//!   PUT    /api/clubs/<id> {name, description, privacy, region, attitude, interest}
//!   DELETE /api/clubs/<id>
//!   POST   /api/clubs/<id>/members {xuid}         add (or approve a join request)
//!   PUT    /api/clubs/<id>/members/<xuid> {role}  1 leader, 2 co-leader, 3 member
//!   DELETE /api/clubs/<id>/members/<xuid>
//!   POST   /api/clubs/<id>/invites {xuid, sender}
//!   DELETE /api/clubs/<id>/invites/<invite id>
//!   DELETE /api/clubs/<id>/requests/<xuid>         decline a join request
//!   POST   /api/clubs/<id>/notices {xuid, message}
//!   DELETE /api/clubs/<id>/notices/<notice id>
//!
//! Every change is announced on /ws as {"type":"users", "users": [...]} and {"type":"clubs"}.
//!   GET /api/catalog          cars, classes and tracks from the game database
//!   GET /api/status           request counter, connected UIs
//!   GET /ws                   WebSocket: {"type":"request"|"event", ...} as they happen
//!   GET /ui/...               web/dist (React build); unknown paths fall back to index.html
//!
//! Account routes for the hook (/infinity/v1/..., X-Fh2-Token) are in `account_api`; the
//! player-hosted worlds (/infinity/v1/worlds...) are worlds.rs.

use std::{fs, path::Path};

use serde_json::{Value, json};

use crate::{App, Request, guard::Bucket, unix_ms};

/// Account requests carry a few short fields.
const MAX_ACCOUNT_BODY: usize = 4 << 10;

pub enum Outcome {
    Reply(Vec<u8>),
    /// WebSocket upgrade with this Sec-WebSocket-Key.
    Upgrade(String),
}

/// Requests from the hook carry X-Fh2-Original-Url; everything else on these paths is ours.
pub fn is_local(request: &Request) -> bool {
    if request.header("x-fh2-original-url").is_some() {
        return false;
    }
    let path = request.path_and_query.split('?').next().unwrap_or("");
    path == "/"
        || path == "/ws"
        || path.starts_with("/api/")
        || path == "/ui"
        || path.starts_with("/ui/")
        || account_route(path).is_some()
}

/// Rate limits of the non-game endpoints (the manager is loopback and never limited).
pub fn allowed(app: &App, request: &Request) -> bool {
    let path = request.path_and_query.split('?').next().unwrap_or("");
    let bucket = match account_route(path) {
        Some("register") => Bucket::Register,
        Some("weblogin" | "login") => Bucket::Login,
        // uploading a save: the hook sends at most one a minute
        Some("save") if request.method == "PUT" => Bucket::Save,
        // cheap reads every hook makes every few seconds (status line, waiting line)
        Some("info" | "queue" | "stats") => Bucket::Game,
        Some(_) => Bucket::Account,
        None => Bucket::Game,
    };
    // Account endpoints count per account (its token) when they carry one: players behind one
    // address (a home, a carrier's shared address) do not use up each other's requests.
    let key = match (bucket, request.header("x-fh2-token")) {
        (Bucket::Account, Some(token)) if !token.is_empty() => token_key(token),
        _ => request.client_ip(),
    };
    app.guard.allow(key, bucket)
}

/// A stand-in address for a token's rate-limit bucket (never a real one: fd00::/8, from the
/// token's SHA-1).
fn token_key(token: &str) -> std::net::IpAddr {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&sha1_smol::Sha1::from(token).digest().bytes()[..16]);
    bytes[0] = 0xfd;
    std::net::IpAddr::from(bytes)
}

/// The manager answers this machine only, under a local Host (a web page elsewhere cannot
/// reach it through DNS rebinding) and, from a browser, a local Origin (no cross-site posts).
fn manager_allowed(request: &Request) -> bool {
    let local_host = |h: &str| {
        let host = match h.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => h.split(':').next().unwrap_or(""),
        };
        matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
    };
    let origin_ok = match request.header("origin") {
        None => true,
        Some(o) => o.split_once("://").is_some_and(|(_, rest)| local_host(rest.split('/').next().unwrap_or(""))),
    };
    request.local_admin() && request.header("host").is_some_and(local_host) && origin_ok
}

/// Player text echoed back: at most `max` chars, letters/digits/'#' only.
fn clip(text: &str, max: usize) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '#').take(max).collect()
}

/// Account endpoints the hook calls directly (not proxied game traffic): /infinity/v1/...
const ACCOUNT_PREFIX: &str = "/infinity/v1/";
/// The prefix hooks before fh2online.33 call; still answered the same.
const LEGACY_ACCOUNT_PREFIX: &str = "/fh2online/v1/";

/// The route under the account prefix ("register", "save/info", ...), or None.
fn account_route(path: &str) -> Option<&str> {
    path.strip_prefix(ACCOUNT_PREFIX).or_else(|| path.strip_prefix(LEGACY_ACCOUNT_PREFIX))
}

/// POST register {name, discriminator, device} -> {xuid, gamertag, token} (one account per
/// device; `device` = SHA-256 hex of the full fingerprint + this server's host);
/// GET me (X-Fh2-Token) -> {xuid, gamertag}; GET info -> what the server is.
fn account_api(app: &App, request: &Request, path: &str) -> Vec<u8> {
    let lang = request.header("x-fh2-lang").unwrap_or("").to_owned();
    REPLY_PORTUGUESE.with(|p| p.set(crate::l10n::is_portuguese(&lang)));
    // remembered per account, for Message Center notices it is sent
    if !lang.is_empty() {
        if let Some(a) = request.header("x-fh2-token").and_then(|t| app.store.account_by_token(t)) {
            app.store.set_lang(a.xuid, &lang);
        }
    }
    let reply = account_api_inner(app, request, path);
    REPLY_PORTUGUESE.with(|p| p.set(false));
    reply
}

fn account_api_inner(app: &App, request: &Request, path: &str) -> Vec<u8> {
    let route = account_route(path).unwrap_or("");
    if request.body.len() > MAX_ACCOUNT_BODY && route != "save" {
        return json_reply(413, &json!({ "error": "too large" }));
    }
    let token_account = || request.header("x-fh2-token").and_then(|t| app.store.account_by_token(t));
    // only the game's own cars play online (cardb.rs): an account (or a new one) with another
    // car database is refused everything but the server's info and stats (updates, the landing
    // page) and deleting itself
    let open = matches!((request.method.as_str(), route), ("GET", "info") | ("GET", "stats") | ("POST", "delete"));
    if !open && app.car_db.on() {
        let account = if route == "register" { Some(0) } else { token_account().map(|a| a.xuid) };
        if let Some(xuid) = account
            && let Some(why) = app.car_db.refusal(request.header("x-fh2-cardb"))
        {
            if xuid == 0 || app.car_db.first_time(xuid) {
                println!("  {} /infinity/v1/{route}: refused, the car database is {} (xuid {xuid})", request.method, why.name());
            }
            return json_reply(403, &json!({ "error": "This game's car database is not the original one: only the game's own cars play online on this server. Put the original gamedbRC.slt back (or reinstall the game) and restart.", "why": "cars" }));
        }
    }
    // player-hosted worlds (worlds.rs), with free roam on
    if route == "worlds" || route.starts_with("worlds/") {
        return match (token_account(), &app.freeroam) {
            (Some(a), _) if a.banned => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            (None, _) => json_reply(401, &json!({ "error": "Unknown token." })),
            (Some(_), None) => json_reply(404, &json!({ "error": "Free roam is off on this server." })),
            (Some(a), Some(f)) => {
                let gamertag = |x| app.store.gamertag(x);
                let friends = |x| crate::mutual_friends(&app.store, x);
                let caller = crate::freeroam::Caller::new(a.xuid, request.client_ip(), &gamertag, &friends)
                    .with_car_db(request.header("x-fh2-cardb"));
                let reply = f.worlds.api(&request.method, route, &request.body, &caller);
                match reply.body {
                    Some(body) => json_reply(reply.status, &body),
                    None => response(reply.status, "application/json; charset=utf-8", "", b""),
                }
            }
        };
    }
    if route.starts_with("save") {
        return match token_account() {
            Some(a) if !a.banned => save_api(app, request, route, a.xuid),
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        };
    }
    match (request.method.as_str(), route) {
        ("GET", "info") => {
            let (online, queued) = app.admission.counts();
            let latest = latest_release(&app.root);
            json_reply(
                200,
                &json!({
                    "name": "ForzaInfinity",
                    "api": 1,
                    "registration": if app.registration_open { "open" } else { "closed" },
                    "online": online,
                    "queued": queued,
                    "capacity": app.admission.capacity(),
                    // online free roam on: players can host worlds (/infinity/v1/worlds)
                    "freeroam": app.freeroam.is_some(),
                    // the newest ForzaInfinity: the hook tells an outdated player to update
                    "latestHook": latest["hook"],
                    "latestRelease": latest["release"],
                }),
            )
        }
        // The caller's slot or place in line (asking keeps the place; see admission.rs):
        // {state: "playing" | "waiting" | "full", position, waiting, online, capacity, etaSeconds,
        // retrySeconds}
        ("GET", "queue") => match token_account() {
            Some(a) if !a.banned => {
                let s = app.admission.arrive(a.xuid);
                let (state, position) = match s.place {
                    crate::admission::Place::Playing => ("playing", 0),
                    crate::admission::Place::Waiting(n) => ("waiting", n),
                    crate::admission::Place::Full => ("full", 0),
                };
                json_reply(
                    200,
                    &json!({
                        "state": state,
                        "position": position,
                        "waiting": s.waiting,
                        "online": s.online,
                        "capacity": s.capacity,
                        "etaSeconds": s.eta_s,
                        "retrySeconds": s.retry_s,
                    }),
                )
            }
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // server-wide totals for the public page (any site may read them); Cloudflare keeps a
        // copy for 15 s (infra/cloudflare.tf), so a crowd on the page never reaches the VM
        ("GET", "stats") => response(
            200,
            "application/json; charset=utf-8",
            "Access-Control-Allow-Origin: *\r\nCache-Control: public, max-age=15, s-maxage=15\r\n",
            &serde_json::to_vec(&app.stats()).unwrap_or_default(),
        ),
        ("POST", "register") if !app.registration_open => {
            json_reply(403, &json!({ "error": "This server is not taking new profiles right now." }))
        }
        ("POST", "register") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let name = body["name"].as_str().unwrap_or("").trim();
            let discriminator = body["discriminator"].as_str().unwrap_or("").trim();
            let device = body["device"].as_str().unwrap_or("").trim();
            // the profile's save is the player's own offline one: the hook uploads it (PUT save)
            // right after, and deletes the account (POST delete) if that fails
            match app.store.register_account(name, discriminator, device) {
                Ok((account, token)) => {
                    let gamertag = account.gamertag();
                    let web_key = app.store.rotate_web_key(account.xuid).unwrap_or_default();
                    app.store.add_event(account.xuid, "account", &format!("Conta {gamertag} criada"), json!({}));
                    app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
                    json_reply(
                        200,
                        &json!({ "xuid": account.xuid.to_string(), "gamertag": gamertag, "token": token, "webKey": web_key, "expansions": app.store.expansions(account.xuid) }),
                    )
                }
                Err(e) => json_reply(400, &json!({ "error": e })),
            }
        }
        // The player deletes their own account from the game (Settings > INFINITY): everything
        // they have on the server goes (Store::delete_user); their offline save stays.
        ("POST", "delete") => match token_account() {
            Some(a) => {
                let gamertag = a.gamertag();
                if app.store.delete_user(a.xuid) {
                    println!("      account {gamertag} deleted by its player");
                    app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
                    json_reply(200, &json!({ "deleted": true }))
                } else {
                    json_reply(500, &json!({ "error": "Could not delete the account." }))
                }
            }
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // a new site key, asked from the game's options (the old one stops working)
        ("POST", "webkey") => match token_account() {
            Some(a) if !a.banned => match app.store.rotate_web_key(a.xuid) {
                Some(key) => json_reply(200, &json!({ "webKey": key })),
                None => json_reply(500, &json!({ "error": "Could not make a key." })),
            },
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // befriend "Name#12345" from the game: friends (and rivals) once both added each other
        // The friendship with one player, by XUID: {xuid} -> {state, gamertag} (state: friends,
        // requested, incoming, none). The game's player actions ask before offering add/remove.
        ("POST", "friends/status") => match token_account() {
            Some(me) if !me.banned => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                match body_xuid(&body, "xuid").filter(|&x| x != 0).and_then(|x| app.store.account(x)) {
                    Some(t) if t.xuid != me.xuid => json_reply(
                        200,
                        &json!({ "state": app.store.friend_state(me.xuid, t.xuid), "gamertag": t.gamertag() }),
                    ),
                    Some(_) => json_reply(400, &json!({ "error": "That is your own profile." })),
                    None => json_reply(404, &json!({ "error": "That player has no profile on this server." })),
                }
            }
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // Reports a player (PLAYER ACTIONS > REPORT PLAYER): {xuid, reason} -> {message}.
        // Five different players' reports flag the account for moderation (db/moderation.rs).
        ("POST", "report") => match token_account() {
            Some(me) if !me.banned => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let reason = body["reason"].as_str().unwrap_or("");
                match body_xuid(&body, "xuid").filter(|&x| x != 0).and_then(|x| app.store.account(x)) {
                    Some(t) => match app.store.report_player(me.xuid, t.xuid, reason) {
                        Ok(flagged) => {
                            println!(
                                "  report: {} reported {}{}",
                                me.xuid,
                                t.xuid,
                                if flagged { " (now flagged for moderation)" } else { "" }
                            );
                            json_reply(
                                200,
                                &json!({ "message": format!("Thanks. Your report on {} was sent to moderation.", t.gamertag()) }),
                            )
                        }
                        Err(crate::db::moderation::ReportError::Yourself) => {
                            json_reply(400, &json!({ "error": "That is your own profile." }))
                        }
                        Err(crate::db::moderation::ReportError::TooMany) => {
                            json_reply(429, &json!({ "error": "Too many reports today. Try again tomorrow." }))
                        }
                        Err(crate::db::moderation::ReportError::TooNew) => json_reply(
                            403,
                            &json!({ "error": "Only players whose profile is a day old and has played online can report." }),
                        ),
                        Err(crate::db::moderation::ReportError::Already) => {
                            json_reply(200, &json!({ "message": format!("You already reported {}.", t.gamertag()) }))
                        }
                        Err(crate::db::moderation::ReportError::NoSuchPlayer) => {
                            json_reply(404, &json!({ "error": "That player has no profile on this server." }))
                        }
                    },
                    None => json_reply(404, &json!({ "error": "That player has no profile on this server." })),
                }
            }
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // Ends a friendship (or a request either way): {xuid} -> {message}.
        ("POST", "friends/remove") => match token_account() {
            Some(me) if !me.banned => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                match body_xuid(&body, "xuid").filter(|&x| x != 0).and_then(|x| app.store.account(x)) {
                    Some(t) if t.xuid != me.xuid => {
                        let was = app.store.unfriend(me.xuid, t.xuid);
                        app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
                        let message = if was {
                            format!("{} is no longer your friend.", t.gamertag())
                        } else {
                            format!("{} was not your friend.", t.gamertag())
                        };
                        json_reply(200, &json!({ "message": message, "friends": false }))
                    }
                    Some(_) => json_reply(400, &json!({ "error": "That is your own profile." })),
                    None => json_reply(404, &json!({ "error": "That player has no profile on this server." })),
                }
            }
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        ("POST", "friends") => match token_account() {
            Some(me) if !me.banned => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let tag = body["gamertag"].as_str().unwrap_or("").trim();
                // by Name#12345 (typed), or by XUID (a player picked in the game's menus)
                let target = match body_xuid(&body, "xuid").filter(|&x| x != 0) {
                    Some(x) => app.store.account(x),
                    None => tag.rsplit_once('#').and_then(|(n, d)| app.store.account_by_name(n, d)),
                };
                match target {
                    _ if app.store.friend_requests_since(me.xuid, 3_600_000) >= 20 => {
                        json_reply(429, &json!({ "error": "Too many friend requests. Try again later." }))
                    }
                    None => json_reply(404, &json!({ "error": format!("There is no profile {} on this server.", clip(tag, 15)) })),
                    Some(t) if t.xuid == me.xuid => json_reply(400, &json!({ "error": "That is your own profile." })),
                    Some(t) if app.store.friends(me.xuid).iter().any(|f| f.0 == t.xuid && f.2) => {
                        json_reply(200, &json!({ "message": format!("You and {} are already friends and rivals.", t.gamertag()) }))
                    }
                    Some(t) => {
                        let mine = me.gamertag();
                        let theirs = t.gamertag();
                        if app.store.request_friend(me.xuid, t.xuid) {
                            for (to, other) in [(t.xuid, &mine), (me.xuid, &theirs)] {
                                let pt = crate::l10n::is_portuguese(&app.store.lang(to));
                                app.store.notify(to, crate::db::notifications::MESSAGE, json!({
                                    "Message": crate::l10n::tr(pt, &format!("You and {other} are now friends and rivals. Compare times in ghost races!")),
                                    "SRAUri": "",
                                }));
                            }
                            app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
                            json_reply(200, &json!({ "message": format!("You and {theirs} are now friends and rivals!"), "friends": true }))
                        } else {
                            let pt = crate::l10n::is_portuguese(&app.store.lang(t.xuid));
                            app.store.notify(t.xuid, crate::db::notifications::MESSAGE, json!({
                                "Message": crate::l10n::tr(pt, &format!("{mine} wants to be your friend. Add {mine} in Settings > Infinity to become rivals.")),
                                "SRAUri": "",
                            }));
                            json_reply(200, &json!({
                                "message": format!("Request sent to {theirs}. Once {theirs} adds you too, you become friends and rivals."),
                                "friends": false,
                            }))
                        }
                    }
                }
            }
            Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        // the game on another PC: name#12345 + web key -> a new token (the old PC goes offline)
        ("POST", "login") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let gamertag = body["gamertag"].as_str().unwrap_or("");
            let key = body["webKey"].as_str().unwrap_or("");
            match app.store.log_in_elsewhere(gamertag, key) {
                Ok((account, token, web_key)) => {
                    let gamertag = account.gamertag();
                    app.store.add_event(account.xuid, "account", &format!("{gamertag} entrou em outro PC"), json!({}));
                    json_reply(
                        200,
                        &json!({ "xuid": account.xuid.to_string(), "gamertag": gamertag, "token": token, "webKey": web_key, "expansions": app.store.expansions(account.xuid) }),
                    )
                }
                Err(e) => json_reply(401, &json!({ "error": e })),
            }
        }
        // site login check: name#12345 + web key
        ("POST", "weblogin") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let gamertag = body["gamertag"].as_str().unwrap_or("");
            let key = body["webKey"].as_str().unwrap_or("");
            match app.store.account_by_web_key(gamertag, key) {
                Some(a) if !a.banned => json_reply(200, &json!({ "xuid": a.xuid.to_string(), "gamertag": a.gamertag() })),
                Some(_) => json_reply(403, &json!({ "error": "Account suspended on this server." })),
                None => json_reply(401, &json!({ "error": "Wrong profile or key." })),
            }
        }
        ("GET", "me") => match token_account() {
            Some(a) if a.banned => json_reply(403, &json!({ "error": "Account suspended on this server." })),
            Some(a) => {
                let mut me = json!({ "xuid": a.xuid.to_string(), "gamertag": a.gamertag(), "expansions": app.store.expansions(a.xuid) });
                // a reset the game has not downloaded yet (db/reset.rs): the hook shows why
                let notice = match app.store.reset_pending(a.xuid) {
                    crate::db::reset::PENDING_MODERATION => Some("Moderation reset this profile's progress. Restart the game to get it; your liveries were kept."),
                    crate::db::reset::PENDING_REFUSED => Some("Your save had progress this server could not accept, so the server's save was restored. Restart the game to get it."),
                    crate::db::reset::PENDING_SEASON => Some("A new season started: XP, level, credits, records and the garage (all but your first car) start again; your career, liveries and creations stay. Restart the game to get it."),
                    _ => None,
                };
                if let Some(text) = notice {
                    me["notice"] = Value::from(crate::l10n::tr(REPLY_PORTUGUESE.with(|p| p.get()), text));
                }
                json_reply(200, &me)
            }
            None => json_reply(401, &json!({ "error": "Unknown token." })),
        },
        _ => json_reply(404, &json!({ "error": "not found" })),
    }
}

/// The account's cloud save (db/saves.rs), for the hook:
///   GET save/info -> {revision, commit, bytes, updated} or 404 (none yet);
///   GET save      -> the bundle (application/octet-stream);
///   PUT save      -> stores the bundle; X-Fh2-Save-Commit = SHA-256 of its commit file.
fn save_api(app: &App, request: &Request, route: &str, xuid: u64) -> Vec<u8> {
    use crate::db::saves;
    match (request.method.as_str(), route) {
        ("GET", "save/info") => match app.store.save_info(xuid) {
            // restore: the server's save wins over the game's (a reset, or a refused upload)
            Some(s) => json_reply(
                200,
                &json!({ "revision": s.revision.to_string(), "commit": s.commit, "bytes": s.bytes, "updated": s.created,
                         "restore": app.store.reset_pending(xuid) != 0 }),
            ),
            None => json_reply(404, &json!({ "error": "No cloud save yet." })),
        },
        ("GET", "save") => match app.store.save_data(xuid) {
            Some(data) => {
                // the game has the reset save now: its uploads are taken again (db/reset.rs)
                app.store.reset_downloaded(xuid);
                response(200, "application/octet-stream", "", &data)
            }
            None => json_reply(404, &json!({ "error": "No cloud save yet." })),
        },
        ("PUT", "save") => {
            // a game still running with the progress from before a reset (db/reset.rs)
            if app.store.reset_pending(xuid) != 0 {
                return json_reply(409, &json!({ "error": "Moderation reset this profile's progress. Restart the game to get it; your liveries were kept." }));
            }
            if request.body.len() > saves::MAX_BUNDLE {
                return json_reply(413, &json!({ "error": "save too big" }));
            }
            let commit = request.header("x-fh2-save-commit").unwrap_or("");
            if !saves::valid_commit(commit) {
                return json_reply(400, &json!({ "error": "invalid commit" }));
            }
            // a new profile's first save (its offline progress) starts the season like everyone's:
            // its career stays, XP, credits, records and all cars but the first start again
            let fresh = saves::check_bundle(&request.body)
                .ok()
                .and_then(|revision| app.store.new_profile_save(xuid, &request.body, revision).map(|(b, c)| (revision, b, c)));
            if let Some((revision, bundle, reset_commit)) = fresh {
                if !app.store.put_save(xuid, revision, &reset_commit, &bundle) {
                    return json_reply(500, &json!({ "error": "Could not store the save." }));
                }
                app.store.set_reset_pending(xuid, crate::db::reset::PENDING_NEW);
                app.store.add_event(xuid, "save", "Perfil novo: save na nuvem a partir da carreira offline, temporada do zero", json!({}));
                return json_reply(200, &json!({ "revision": revision.to_string(), "commit": reset_commit }));
            }
            // the save gates (db/reset.rs check_upload): the progress since the server's save, at most
            // what an hour of play gives; refused, the server's save is restored at the next start
            if let (Ok(_), Some(info)) = (saves::check_bundle(&request.body), app.store.save_info(xuid)) {
                let previous = app.store.save_data(xuid).and_then(|b| crate::db::reset::save_progress(&b));
                let new = crate::db::reset::save_progress(&request.body);
                if let (Some(previous), Some(new)) = (previous, new)
                    && let Err(why) = crate::db::reset::check_upload(&previous, info.created, &new, unix_ms())
                {
                    let strikes = app.store.strike(xuid, &format!("save: {why}"));
                    app.store.set_reset_pending(xuid, crate::db::reset::PENDING_REFUSED);
                    app.store.add_event(xuid, "integrity", &format!("Save recusado: {why}"), json!({ "strikes": strikes }));
                    eprintln!("integrity: {xuid} save refused ({why}), strike {strikes}");
                    return json_reply(409, &json!({ "error": "Your save had progress this server could not accept, so the server's save was restored. Restart the game to get it.", "why": "save" }));
                }
            }
            match saves::check_bundle(&request.body) {
                Ok(revision) if app.store.put_save(xuid, revision, commit, &request.body) => {
                    app.store.add_event(xuid, "save", &format!("Save na nuvem atualizado (revisão {revision})"), json!({ "bytes": request.body.len() }));
                    // speed trap and zone bests the game will not submit again (db/save_bests.rs)
                    let taken = app.store.import_save_bests(xuid, &request.body);
                    if taken > 0 {
                        app.store.add_event(xuid, "score", &format!("{taken} recorde(s) de radar e zona lidos do save"), json!({ "count": taken }));
                    }
                    json_reply(200, &json!({ "revision": revision.to_string(), "commit": commit.to_ascii_lowercase() }))
                }
                Ok(_) => json_reply(500, &json!({ "error": "Could not store the save." })),
                Err(e) => json_reply(400, &json!({ "error": e })),
            }
        }
        _ => json_reply(404, &json!({ "error": "not found" })),
    }
}

pub fn hello(app: &App) -> Value {
    json!({ "type": "hello", "time": unix_ms(), "users": app.store.users() })
}

pub fn handle(app: &App, request: &Request) -> Outcome {
    let path = request.path_and_query.split('?').next().unwrap_or("").to_owned();
    if account_route(&path).is_some() {
        let reply = account_api(app, request, &path);
        let status = std::str::from_utf8(&reply[..reply.len().min(12)]).unwrap_or("").get(9..12).unwrap_or("?").to_owned();
        println!("      {:<5} {:<60} {:>6} bytes -> {status} ({})", request.method, path, request.body.len(), request.client_ip());
        return Outcome::Reply(reply);
    }
    if !manager_allowed(request) {
        return Outcome::Reply(json_reply(404, &json!({ "error": "not found" })));
    }
    if request.method != "GET" && request.method != "HEAD" {
        // JSON only: a plain form post from a web page cannot pass as a manager request
        if !request.header("content-type").is_some_and(|c| c.to_ascii_lowercase().starts_with("application/json")) {
            return Outcome::Reply(json_reply(415, &json!({ "error": "expected application/json" })));
        }
        return Outcome::Reply(change(app, request, &path));
    }
    if path == "/ws" {
        let upgrade = request.header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        return match (upgrade, request.header("sec-websocket-key")) {
            (true, Some(key)) => Outcome::Upgrade(key.to_owned()),
            _ => Outcome::Reply(json_reply(400, &json!({ "error": "expected a WebSocket upgrade" }))),
        };
    }
    if path == "/" || path == "/ui" {
        return Outcome::Reply(redirect("/ui/"));
    }
    if let Some(rest) = path.strip_prefix("/ui/") {
        return Outcome::Reply(static_file(&app.web, rest));
    }
    let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').filter(|p| !p.is_empty()).collect();
    Outcome::Reply(match parts.as_slice() {
        ["users"] => json_reply(200, &app.store.users()),
        ["users", xuid, "friends"] => match xuid.parse::<u64>().ok().filter(|&x| app.store.user_exists(x)) {
            Some(x) => json_reply(200, &app.store.friends_json(x)),
            None => json_reply(404, &json!({ "error": "unknown player" })),
        },
        ["users", xuid, "notifications"] => match xuid.parse::<u64>().ok().filter(|&x| app.store.user_exists(x)) {
            Some(x) => json_reply(200, &app.store.notifications_json(x)),
            None => json_reply(404, &json!({ "error": "unknown player" })),
        },
        ["users", xuid] => match xuid.parse::<u64>().ok().and_then(|x| app.store.user_profile(x, &app.catalog)) {
            Some(profile) => json_reply(200, &profile),
            None => json_reply(404, &json!({ "error": "unknown player" })),
        },
        ["gamerpic", xuid] => {
            let xuid = xuid.trim_end_matches(".png").parse::<u64>().unwrap_or(0);
            let (png, from) = crate::gamerpic::get(&app.root, xuid);
            let extra = format!("X-Gamerpic-Source: {}\r\n", crate::gamerpic_source(&from));
            response(200, "image/png", &extra, &png)
        }
        ["catalog"] => json_reply(200, &json!(app.catalog)),
        ["clubs"] => json_reply(200, &clubs_reply(app)),
        ["status"] => json_reply(
            200,
            &json!({
                "requests": app.counter.load(std::sync::atomic::Ordering::Relaxed),
                "liveClients": app.hub.count(),
                "time": unix_ms(),
            }),
        ),
        _ => json_reply(404, &json!({ "error": "not found" })),
    })
}

/// POST / PUT / DELETE on the manager API.
fn change(app: &App, request: &Request, path: &str) -> Vec<u8> {
    let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
    let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').filter(|p| !p.is_empty()).collect();
    let xuid = |s: &str| s.parse::<u64>().ok().filter(|&x| app.store.user_exists(x));
    let club = |s: &str| s.parse::<i64>().ok().and_then(|id| app.store.club(id));
    let result: Result<Value, (u16, String)> = match (request.method.as_str(), parts.as_slice()) {
        ("POST", ["users"]) => {
            let tag = body["gamertag"].as_str().unwrap_or("").trim().to_owned();
            if !valid_gamertag(&tag) {
                Err((400, "gamertag: 1 a 15 letras, dígitos ou espaços, começando por letra".into()))
            } else {
                app.store.create_user(&tag).map(|x| json!({ "xuid": x.to_string() })).map_err(|e| (409, e))
            }
        }
        ("DELETE", ["users", x]) => match xuid(x) {
            Some(x) if app.store.delete_user(x) => Ok(json!({ "deleted": x.to_string() })),
            _ => Err((404, "jogador desconhecido".into())),
        },
        // Suspend / restore an FH2Online account (its token is refused while suspended).
        ("POST" | "DELETE", ["users", x, "ban"]) => match xuid(x) {
            Some(x) if app.store.set_banned(x, request.method == "POST") => Ok(json!({ "banned": request.method == "POST" })),
            _ => Err((404, "conta desconhecida".into())),
        },
        ("POST", ["users", x, "friends"]) => {
            let other = body["xuid"].as_str().map(str::to_owned).unwrap_or_else(|| body["xuid"].to_string());
            match (xuid(x), xuid(&other)) {
                (Some(a), Some(b)) if a != b => {
                    app.store.add_friend(a, b);
                    Ok(app.store.friends_json(a))
                }
                (Some(_), Some(_)) => Err((400, "um jogador não pode ser amigo de si mesmo".into())),
                _ => Err((404, "jogador desconhecido".into())),
            }
        }
        ("PUT", ["users", x, "friends", f]) => match (xuid(x), xuid(f)) {
            (Some(a), Some(b)) if app.store.set_favorite(a, b, body["favorite"].as_bool().unwrap_or(false)) => {
                Ok(app.store.friends_json(a))
            }
            _ => Err((404, "amizade desconhecida".into())),
        },
        ("DELETE", ["users", x, "friends", f]) => match (xuid(x), xuid(f)) {
            (Some(a), Some(b)) if app.store.remove_friend(a, b) => Ok(app.store.friends_json(a)),
            _ => Err((404, "amizade desconhecida".into())),
        },
        ("POST", ["users", x, "notifications"]) => match xuid(x) {
            Some(to) => match body["kind"].as_str().unwrap_or("message") {
                "message" => {
                    let text = body["message"].as_str().unwrap_or("").trim().to_owned();
                    if text.is_empty() {
                        Err((400, "mensagem vazia".into()))
                    } else {
                        let id = app.store.notify(to, crate::db::notifications::MESSAGE, json!({ "Message": text, "SRAUri": "" }));
                        Ok(json!({ "id": id }))
                    }
                }
                "rival" => match body_xuid(&body, "from").filter(|&f| app.store.user_exists(f) && f != to) {
                    Some(from) => test_rival(app, to, from),
                    None => Err((400, "escolha quem desafia".into())),
                },
                _ => Err((400, "kind: message ou rival".into())),
            },
            None => Err((404, "jogador desconhecido".into())),
        },
        ("DELETE", ["notifications", id]) => match id.parse::<i64>() {
            Ok(id) if app.store.delete_notification(id) => Ok(json!({})),
            _ => Err((404, "notificação desconhecida".into())),
        },
        ("POST", ["clubs"]) => {
            let owner = body_xuid(&body, "owner");
            match owner.filter(|&x| app.store.user_exists(x)) {
                Some(owner) => app
                    .store
                    .create_club(owner, &club_fields(&body, None))
                    .map(|id| json!({ "id": id }))
                    .map_err(|e| (400, e)),
                None => Err((404, "dono desconhecido".into())),
            }
        }
        ("PUT", ["clubs", id]) => match club(id) {
            Some(c) => app.store.update_club(c.id, &club_fields(&body, Some(&c))).map(|_| json!({})).map_err(|e| (400, e)),
            None => Err((404, "clube desconhecido".into())),
        },
        ("DELETE", ["clubs", id]) => match club(id) {
            Some(c) if app.store.delete_club(c.id) => Ok(json!({})),
            _ => Err((404, "clube desconhecido".into())),
        },
        ("POST", ["clubs", id, "members"]) => match (club(id), body_xuid(&body, "xuid").filter(|&x| app.store.user_exists(x))) {
            (Some(c), Some(x)) => {
                app.store.delete_club_request(c.id, x);
                app.store.add_member(c.id, x, crate::db::ROLE_MEMBER);
                Ok(json!({}))
            }
            _ => Err((404, "clube ou jogador desconhecido".into())),
        },
        ("PUT", ["clubs", id, "members", x]) => match (club(id), xuid(x)) {
            (Some(c), Some(x)) if app.store.set_member_role(c.id, x, body["role"].as_i64().unwrap_or(0)) => Ok(json!({})),
            _ => Err((400, "membro ou cargo inválido".into())),
        },
        ("DELETE", ["clubs", id, "members", x]) => match (club(id), xuid(x)) {
            (Some(c), Some(x)) if app.store.membership(x).map(|m| m.0) == Some(c.id) => {
                app.store.leave_club(x, crate::db::clubs::ACTION_REMOVED);
                Ok(json!({}))
            }
            _ => Err((404, "membro desconhecido".into())),
        },
        ("POST", ["clubs", id, "invites"]) => match (club(id), body_xuid(&body, "xuid").filter(|&x| app.store.user_exists(x))) {
            (Some(c), Some(x)) => {
                let sender = body_xuid(&body, "sender").unwrap_or(0);
                app.store.invite_to_club(c.id, sender, x).map(|id| json!({ "id": id })).map_err(|e| (400, e))
            }
            _ => Err((404, "clube ou jogador desconhecido".into())),
        },
        ("DELETE", ["clubs", _, "invites", invite]) => match invite.parse::<i64>() {
            Ok(i) if app.store.delete_club_invite(i) => Ok(json!({})),
            _ => Err((404, "convite desconhecido".into())),
        },
        ("DELETE", ["clubs", id, "requests", x]) => match (club(id), x.parse::<u64>().ok()) {
            (Some(c), Some(x)) if app.store.delete_club_request(c.id, x) => Ok(json!({})),
            _ => Err((404, "pedido desconhecido".into())),
        },
        ("POST", ["clubs", id, "notices"]) => match club(id) {
            Some(c) => {
                let message = body["message"].as_str().unwrap_or("").trim().to_owned();
                if message.is_empty() {
                    Err((400, "mensagem vazia".into()))
                } else {
                    let author = body_xuid(&body, "xuid").unwrap_or(0);
                    Ok(json!({ "id": app.store.add_club_notice(c.id, author, &message) }))
                }
            }
            None => Err((404, "clube desconhecido".into())),
        },
        ("DELETE", ["clubs", _, "notices", n]) => match n.parse::<i64>() {
            Ok(n) if app.store.delete_club_notice(n) => Ok(json!({})),
            _ => Err((404, "aviso desconhecido".into())),
        },
        _ => Err((404, "not found".into())),
    };
    match result {
        Ok(value) => {
            app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
            app.hub.broadcast(&json!({ "type": "clubs" }));
            json_reply(200, &value)
        }
        Err((status, error)) => json_reply(status, &json!({ "error": error })),
    }
}

/// A test "your rival beat your time" on the last real race event the game asked about
/// (GetBestScore before a race): `from` is 1 to 3 % quicker than `to`'s time there, or than a
/// plausible time for the route.
fn test_rival(app: &App, to: u64, from: u64) -> Result<Value, (u16, String)> {
    let Some((board, score_type, event, track, class)) = app.store.last_race_event() else {
        return Err((400, "nenhum evento de corrida conhecido ainda: entre numa corrida no jogo primeiro".into()));
    };
    let mine = app.store.leaderboard(board, score_type, Some(track), Some(event)).into_iter().find(|r| r.xuid == to);
    let lower = crate::db::lower_is_better(score_type);
    let base = match &mine {
        Some(r) => r.score,
        None => app.catalog.tracks.get(&track).map(|t| t.length_m as f64 / 30.5).filter(|s| *s > 5.0).unwrap_or(90.0),
    };
    let k = 0.01 + (crate::unix_ms() % 200) as f64 / 10_000.0;
    let score = ((if lower { base * (1.0 - k) } else { base * (1.0 + k) }) * 1000.0).round() / 1000.0;
    let car = mine.as_ref().and_then(|r| r.data["carId"].as_i64()).unwrap_or(461);
    let track_name = app.catalog.tracks.get(&track).map(|t| t.name.clone()).unwrap_or_default();
    let club = app.store.membership(from).and_then(|(id, role)| app.store.club(id).map(|c| (c.tag, role)));
    let params = json!({
        "senderXuid": from,
        "senderGamertag": app.store.gamertag(from),
        "senderClubTag": club.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
        "senderClubRole": club.as_ref().map(|c| c.1).unwrap_or(0),
        "senderFlags": 0,
        "sentTime": crate::forza::iso_utc(crate::unix_ms()),
        "timeTrialId": crate::forza::challenge_target(board, event).1,
        "carId": car,
        "carClassId": class.max(0),
        "localizedTimeTrialName": track_name,
        "trackName": track_name,
        "trackId": track,
        "score": score,
        "scoreType": score_type,
        "category": crate::forza::challenge_target(board, event).0,
    });
    let id = app.store.notify(to, crate::db::notifications::TIME_BEATEN, params);
    Ok(json!({ "id": id, "score": score, "trackId": track, "event": event }))
}
fn clubs_reply(app: &App) -> Value {
    let keywords: Vec<Value> = crate::forza::club_keywords()
        .iter()
        .map(|&(id, category, text)| json!({ "id": id, "category": category, "text": text }))
        .collect();
    json!({ "clubs": app.store.clubs_json(), "keywords": keywords })
}

/// A XUID sent as a string (JS cannot hold 64-bit numbers) or a number.
fn body_xuid(body: &Value, key: &str) -> Option<u64> {
    body[key].as_str().and_then(|s| s.parse().ok()).or_else(|| body[key].as_u64())
}

/// Club fields from a manager request; `current` fills what an edit leaves out.
fn club_fields(body: &Value, current: Option<&crate::db::clubs::Club>) -> crate::db::ClubFields {
    let text = |key: &str, old: Option<&String>| {
        body[key].as_str().map(str::to_owned).or_else(|| old.cloned()).unwrap_or_default()
    };
    let num = |key: &str, old: Option<i64>| body[key].as_i64().or(old).unwrap_or(0);
    crate::db::ClubFields {
        tag: text("tag", current.map(|c| &c.tag)),
        name: text("name", current.map(|c| &c.name)),
        description: text("description", current.map(|c| &c.description)),
        privacy: num("privacy", current.map(|c| c.privacy)),
        region: num("region", current.map(|c| c.region)),
        attitude: num("attitude", current.map(|c| c.attitude)),
        interest: num("interest", current.map(|c| c.interest)),
    }
}

/// Xbox rules: 1 to 15 characters, letters, digits and single spaces, starting with a letter.
fn valid_gamertag(tag: &str) -> bool {
    let first_letter = tag.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    first_letter
        && tag.len() <= 15
        && !tag.contains("  ")
        && !tag.ends_with(' ')
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')
}

/// `extra`: more header lines, each ending in CRLF; one of them may set Cache-Control (the
/// default is no-store).
fn response(status: u16, content_type: &str, extra: &str, body: &[u8]) -> Vec<u8> {
    let cache = if extra.to_ascii_lowercase().contains("cache-control:") { "" } else { "Cache-Control: no-store\r\n" };
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{cache}{extra}\r\n",
        crate::reason(status),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

thread_local! {
    /// The account endpoint being answered asked for Portuguese (X-Fh2-Lang): json_reply
    /// translates its "error" and "message" (l10n.rs).
    static REPLY_PORTUGUESE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn json_reply(status: u16, value: &Value) -> Vec<u8> {
    let mut value = value.clone();
    if REPLY_PORTUGUESE.with(|p| p.get()) {
        for key in ["error", "message"] {
            if let Some(text) = value[key].as_str() {
                value[key] = Value::from(crate::l10n::tr(true, text));
            }
        }
    }
    response(status, "application/json; charset=utf-8", "", &serde_json::to_vec(&value).unwrap_or_default())
}

fn redirect(to: &str) -> Vec<u8> {
    response(302, "text/plain", &format!("Location: {to}\r\n"), b"")
}

/// The newest release, {"hook": "0.2.74+fh2online.37", "release": "v0.1.3"}, from
/// responses/infinity/latest.json (tools/package_release.sh writes it; deploy it with the
/// release). Null fields when missing.
fn latest_release(root: &Path) -> Value {
    std::fs::read(root.join("responses").join("infinity").join("latest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn static_file(root: &Path, rest: &str) -> Vec<u8> {
    if rest.split('/').any(|p| p == "..") || rest.contains('\\') {
        return response(400, "text/plain", "", b"bad path");
    }
    let wanted = if rest.is_empty() { "index.html" } else { rest };
    let file = root.join(wanted);
    let (file, name) = if file.is_file() { (file, wanted) } else { (root.join("index.html"), "index.html") };
    match fs::read(&file) {
        Ok(bytes) => response(200, content_type(name), "", &bytes),
        Err(_) => response(
            404,
            "text/html; charset=utf-8",
            "",
            format!(
                "<h1>FH2Online manager</h1><p>A interface ainda não foi compilada ({}). \
                 Rode <code>web\\build.bat</code>, ou use <code>web\\dev.bat</code> e abra http://localhost:5173/ui/.</p>",
                root.display()
            )
            .as_bytes(),
        ),
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
