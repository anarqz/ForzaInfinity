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
//! Every change is announced on /ws as {"type":"users", "users": [...]}.
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
    path == "/" || path == "/ws" || path.starts_with("/api/") || path == "/ui" || path.starts_with("/ui/")
}

pub fn hello(app: &App) -> Value {
    json!({ "type": "hello", "time": unix_ms(), "users": app.store.users() })
}

pub fn handle(app: &App, request: &Request) -> Outcome {
    let path = request.path_and_query.split('?').next().unwrap_or("").to_owned();
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
        _ => Err((404, "not found".into())),
    };
    match result {
        Ok(value) => {
            app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
            json_reply(200, &value)
        }
        Err((status, error)) => json_reply(status, &json!({ "error": error })),
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
