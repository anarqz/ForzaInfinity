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

use std::{fs, path::Path};

use serde_json::{Value, json};

use crate::{App, Request, unix_ms};

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
        || path.starts_with(ACCOUNT_PREFIX)
}

/// Account endpoints the hook calls directly (not proxied game traffic).
const ACCOUNT_PREFIX: &str = "/fh2online/v1/";

/// POST register {name, discriminator, device} -> {xuid, gamertag, token} (one account per
/// device; `device` = SHA-256 hex of the full fingerprint + this server's host);
/// GET me (X-Fh2-Token) -> {xuid, gamertag}; GET info -> what the server is.
fn account_api(app: &App, request: &Request, path: &str) -> Vec<u8> {
    let token_account = || request.header("x-fh2-token").and_then(|t| app.store.account_by_token(t));
    match (request.method.as_str(), &path[ACCOUNT_PREFIX.len()..]) {
        ("GET", "info") => json_reply(200, &json!({ "name": "FH2Online", "api": 1, "registration": "open" })),
        ("POST", "register") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let name = body["name"].as_str().unwrap_or("").trim();
            let discriminator = body["discriminator"].as_str().unwrap_or("").trim();
            let device = body["device"].as_str().unwrap_or("").trim();
            match app.store.register_account(name, discriminator, device) {
                Ok((account, token)) => {
                    let gamertag = account.gamertag();
                    let web_key = app.store.rotate_web_key(account.xuid).unwrap_or_default();
                    app.store.add_event(account.xuid, "account", &format!("Conta {gamertag} criada"), json!({}));
                    app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
                    json_reply(
                        200,
                        &json!({ "xuid": account.xuid.to_string(), "gamertag": gamertag, "token": token, "webKey": web_key }),
                    )
                }
                Err(e) => json_reply(400, &json!({ "error": e })),
            }
        }
        // a new site key, asked from the game's options (the old one stops working)
        ("POST", "webkey") => match token_account() {
            Some(a) if !a.banned => match app.store.rotate_web_key(a.xuid) {
                Some(key) => json_reply(200, &json!({ "webKey": key })),
                None => json_reply(500, &json!({ "error": "Erro ao gerar a chave." })),
            },
            Some(_) => json_reply(403, &json!({ "error": "Conta suspensa neste servidor." })),
            None => json_reply(401, &json!({ "error": "Token desconhecido." })),
        },
        // site login check: name#12345 + web key
        ("POST", "weblogin") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let gamertag = body["gamertag"].as_str().unwrap_or("");
            let key = body["webKey"].as_str().unwrap_or("");
            match app.store.account_by_web_key(gamertag, key) {
                Some(a) if !a.banned => json_reply(200, &json!({ "xuid": a.xuid.to_string(), "gamertag": a.gamertag() })),
                Some(_) => json_reply(403, &json!({ "error": "Conta suspensa neste servidor." })),
                None => json_reply(401, &json!({ "error": "Nome ou chave incorretos." })),
            }
        }
        ("GET", "me") => match token_account() {
            Some(a) if a.banned => json_reply(403, &json!({ "error": "Conta suspensa neste servidor." })),
            Some(a) => json_reply(200, &json!({ "xuid": a.xuid.to_string(), "gamertag": a.gamertag() })),
            None => json_reply(401, &json!({ "error": "Token desconhecido." })),
        },
        _ => json_reply(404, &json!({ "error": "not found" })),
    }
}

pub fn hello(app: &App) -> Value {
    json!({ "type": "hello", "time": unix_ms(), "users": app.store.users() })
}

pub fn handle(app: &App, request: &Request) -> Outcome {
    let path = request.path_and_query.split('?').next().unwrap_or("").to_owned();
    if path.starts_with(ACCOUNT_PREFIX) {
        return Outcome::Reply(account_api(app, request, &path));
    }
    if request.method != "GET" && request.method != "HEAD" {
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
            let (png, from) = crate::gamerpic::get(&app.root, xuid, &app.store.gamertag(xuid));
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

fn response(status: u16, content_type: &str, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra}\r\n",
        crate::reason(status),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn json_reply(status: u16, value: &Value) -> Vec<u8> {
    response(status, "application/json; charset=utf-8", "", &serde_json::to_vec(value).unwrap_or_default())
}

fn redirect(to: &str) -> Vec<u8> {
    response(302, "text/plain", &format!("Location: {to}\r\n"), b"")
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
