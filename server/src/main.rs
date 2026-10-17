//! FH2Online test server (std only: no tokio, no windows-sys, builds with any
//! Rust toolchain, including x86_64-pc-windows-gnu without dlltool).
//!
//! The azimuth-tls.dll hook (Mode=Proxy) rewrites every game request to
//! `http://<listen>/<original host>/<original path>` and adds
//! `X-Fh2-Original-Url` / `X-Fh2-Request-Id`. This server:
//!   1. logs every request (and the answer it gave) to `captures/*.jsonl`;
//!   2. answers from files under `responses/` (re-read on every request, so
//!      you can edit them while the game is running).
//!
//! Lookup order for a response spec:
//!   a. the first matching rule in `responses/routes.json`;
//!   b. Forza RPC (`X-ClassName` + `X-Method`):
//!      `responses/<ns>/<Class>/<Method>.json`, then a built-in handler (forza.rs), then
//!      `<ns>/<Class>/_default.json`, then `responses/<ns>/_default.json`
//!      (`<ns>` is `forza` for Forza.WebServices.* and `xls` for Xls.WebServices.*);
//!   c. other hosts: `responses/hosts/<host>/_default.json`;
//!   d. `responses/_default.json`;
//!   e. built-in 404.
//!
//! With --freeroam / --freeroam-world, the game-server and session calls of online free roam
//! (gameserverds, gameserverms, sessiondirectory, momatch) are answered by freeroam.rs before c,
//! and players can host worlds (/infinity/v1/worlds, worlds.rs).
//!
//! Forza RPC bodies are `bin/xtsw` (xtsw.rs). The request is decoded into the capture as JSON,
//! and a spec can answer with `"xtsw": { <_Results as JSON> }` (merged over the built-in
//! handler, missing fields = zero) or `"xtsw_error": { "code": n, "message": "..." }`.
//!
//! Player data lives in SQLite (db.rs, `data/fh2online.sqlite`). Requests that do not come from
//! the hook (no `X-Fh2-Original-Url`) to `/api/*`, `/ws` and `/ui/*` are the manager: a JSON
//! API, a WebSocket feed and the built web UI (api.rs, live.rs, ../web). The manager answers
//! only this machine (loopback, not through a reverse proxy).
//!
//! Abuse limits (guard.rs): connections per address, header and body sizes, request rates.
//! Behind a reverse proxy (TLS) on this machine, the proxy must send X-Forwarded-For (or
//! CF-Connecting-IP, from cloudflared): it names the client. With --behind-proxy, connections
//! from that proxy are not held to the per-address connection limit (every player comes
//! through it); the rate limits still go by the client it names.

mod admission;
mod api;
mod cardb;
mod db;
mod forza;
mod freeroam;
mod gamedb;
mod strtable;
mod gamerpic;
mod l10n;
mod guard;
mod live;
mod moderation_cli;
mod news_cli;
mod sha256;
mod worlds;
mod xtsw;

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Largest request body (the biggest game upload, a ghost, is ~30 KB).
const MAX_BODY: usize = 2 << 20;
/// Largest request line or header line, and how many headers a request may have.
const MAX_LINE: u64 = 8 << 10;
const MAX_HEADERS: usize = 64;

struct App {
    root: PathBuf,
    /// captures/*.jsonl, only with --capture (development: it keeps every body).
    capture: Option<Mutex<File>>,
    guard: guard::Guard,
    counter: AtomicU64,
    schema: xtsw::Schema,
    store: db::Store,
    catalog: gamedb::Catalog,
    hub: live::Hub,
    /// Built web UI (web/dist), served at /ui/.
    web: PathBuf,
    /// The public totals (/infinity/v1/stats) and when they were computed.
    stats: Mutex<Option<(std::time::Instant, serde_json::Value)>>,
    /// A proxy on this machine relays every player (--behind-proxy).
    behind_proxy: bool,
    /// New accounts are taken (--registration closed turns them away).
    registration_open: bool,
    /// Player slots and the waiting line (--max-players).
    admission: admission::Admission,
    /// Online free roam (--freeroam / --freeroam-world): game servers and sessions.
    freeroam: Option<freeroam::Freeroam>,
    /// Only the game's own cars play online (cardb.rs).
    car_db: cardb::Gate,
}

impl App {
    /// Server-wide totals, recomputed at most every 30 s (a public page polls them).
    fn stats(&self) -> serde_json::Value {
        let mut cached = self.stats.lock().unwrap_or_else(|p| p.into_inner());
        let value = match &*cached {
            Some((at, value)) if at.elapsed() < std::time::Duration::from_secs(30) => value.clone(),
            _ => {
                let value = self.store.server_stats();
                *cached = Some((std::time::Instant::now(), value.clone()));
                value
            }
        };
        // who holds a slot and who waits: live, from memory
        let mut value = value;
        let (online, queued) = self.admission.counts();
        let capacity = self.admission.capacity();
        value["online"] = json!(online);
        value["queued"] = json!(queued);
        value["capacity"] = json!(capacity);
        value["full"] = json!(capacity > 0 && online >= capacity);
        // player-hosted worlds alive now and the players in them (free roam off: none)
        let (worlds, world_players) = self.freeroam.as_ref().map(|f| f.worlds.active()).unwrap_or((0, 0));
        value["worlds"] = json!(worlds);
        value["worldPlayers"] = json!(world_players);
        value
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct XtswError {
    code: u32,
    #[serde(default)]
    message: String,
}

/// How to answer one request. Every field is optional.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
struct ResponseSpec {
    status: Option<u16>,
    headers: BTreeMap<String, String>,
    /// Body as UTF-8 text.
    body: Option<String>,
    /// Body as JSON (serialized compactly; sets Content-Type if missing).
    json: Option<Value>,
    /// Body as base64 (for binary payloads copied from a capture).
    body_b64: Option<String>,
    /// Body from a file, relative to the spec file.
    body_file: Option<String>,
    /// Artificial latency, for timeout experiments.
    delay_ms: Option<u64>,
    /// Forza RPC: `_Results` as JSON, encoded with the schema (merged over a built-in handler).
    xtsw: Option<Value>,
    /// Forza RPC: answer with an Xls error (client sees HRESULT 0x80510000 | code).
    xtsw_error: Option<XtswError>,
}

/// One rule in responses/routes.json. Patterns use `*` and `?`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Rule {
    host: Option<String>,
    method: Option<String>,
    path: Option<String>,
    class: Option<String>,
    xmethod: Option<String>,
    /// Spec file relative to responses/ ...
    file: Option<String>,
    /// ... or an inline spec.
    response: Option<ResponseSpec>,
}

struct Request {
    method: String,
    path_and_query: String,
    /// Header names lower-cased, in arrival order.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The address the connection came from.
    peer: IpAddr,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Came through a reverse proxy on this machine.
    fn proxied(&self) -> bool {
        self.headers
            .iter()
            .any(|(n, _)| matches!(n.as_str(), "x-forwarded-for" | "forwarded" | "x-real-ip" | "cf-connecting-ip"))
    }

    /// The client: the peer, or what a local proxy names: CF-Connecting-IP (cloudflared), else
    /// the last X-Forwarded-For entry.
    fn client_ip(&self) -> IpAddr {
        if !self.peer.is_loopback() || !self.proxied() {
            return self.peer;
        }
        let named = |v: &str| v.trim().parse::<IpAddr>().ok();
        self.header("cf-connecting-ip")
            .and_then(named)
            .or_else(|| self.header("x-forwarded-for").and_then(|v| v.rsplit(',').next()).and_then(named))
            // a proxied request without a usable address still must not count as loopback
            .unwrap_or(IpAddr::from([0, 0, 0, 0]))
    }

    /// From this machine directly (not relayed): the only callers the manager answers.
    fn local_admin(&self) -> bool {
        self.peer.is_loopback() && !self.proxied()
    }
}

struct Target {
    original_url: String,
    host: String,
    path: String,
    query: String,
    /// X-ClassName without the assembly: "Forza.WebServices.User".
    forza_class: Option<String>,
    forza_method: Option<String>,
}

impl Target {
    /// Directory under responses/: ("forza", "User") or ("xls", "User").
    fn forza_dir(&self) -> Option<(&'static str, &str)> {
        let class = self.forza_class.as_deref()?;
        let short = class.rsplit('.').next().unwrap_or(class);
        Some((if class.starts_with("Xls.") { "xls" } else { "forza" }, short))
    }
}

fn main() {
    // admin news, straight into the database (never over HTTP): news_cli.rs
    if std::env::args().nth(1).as_deref() == Some("news") {
        news_cli::run(std::env::args().skip(2).collect());
        return;
    }
    // reported accounts: list, review, clear or reset (never over HTTP): moderation_cli.rs
    if std::env::args().nth(1).as_deref() == Some("moderation") {
        moderation_cli::run(std::env::args().skip(2).collect());
        return;
    }
    let mut listen: SocketAddr = "127.0.0.1:8081".parse().unwrap();
    let mut root = std::env::current_dir().unwrap();
    let mut gamedb_path = None;
    let mut web = None;
    let mut db_path = None;
    let mut capture_on = false;
    let mut behind_proxy = false;
    let mut registration_open = true;
    let mut max_players = 150;
    let mut freeroam_on = false;
    let mut freeroam_world = None;
    let mut cardb_allow: Vec<String> = Vec::new();
    let mut cardb_gate = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--capture" => capture_on = true,
            "--cardb-allow" => {
                let h = args.next().expect("--cardb-allow SHA-256");
                if !cardb::is_sha256(&h) {
                    eprintln!("--cardb-allow: a SHA-256 in hex (64 digits)");
                    std::process::exit(2);
                }
                cardb_allow.push(h);
            }
            "--cardb-gate" => {
                cardb_gate = match args.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => {
                        eprintln!("--cardb-gate on|off");
                        std::process::exit(2);
                    }
                }
            }
            "--behind-proxy" => behind_proxy = true,
            "--max-players" => max_players = args.next().expect("--max-players N").parse().expect("bad number"),
            "--registration" => {
                registration_open = match args.next().as_deref() {
                    Some("open") => true,
                    Some("closed") => false,
                    _ => panic!("--registration open|closed"),
                }
            }
            "--freeroam" => freeroam_on = true,
            "--freeroam-world" => {
                use std::net::ToSocketAddrs;
                let world = args.next().expect("--freeroam-world HOST:PORT");
                let addr = world.to_socket_addrs().ok().and_then(|mut a| a.next()).unwrap_or_else(|| {
                    eprintln!("--freeroam-world: cannot resolve {world}");
                    std::process::exit(2);
                });
                freeroam_world = Some(addr);
            }
            "--listen" => listen = args.next().expect("--listen ADDR").parse().expect("bad address"),
            "--root" => root = PathBuf::from(args.next().expect("--root DIR")),
            "--gamedb" => gamedb_path = Some(PathBuf::from(args.next().expect("--gamedb FILE"))),
            "--web" => web = Some(PathBuf::from(args.next().expect("--web DIR"))),
            "--db" => db_path = Some(PathBuf::from(args.next().expect("--db FILE"))),
            "-h" | "--help" => {
                println!(
                    "fh2-server [--listen 127.0.0.1:8081] [--root DIR] [--db FILE] [--gamedb gamedbRC.slt] [--web web/dist] [--capture] [--behind-proxy] [--registration open|closed] [--max-players 150 (0: no limit)] [--freeroam] [--freeroam-world HOST:PORT] [--cardb-gate on|off] [--cardb-allow SHA-256]..."
                );
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    fs::create_dir_all(root.join("responses")).unwrap();
    let capture_path = capture_on.then(|| {
        fs::create_dir_all(root.join("captures")).unwrap();
        root.join("captures").join(format!("server-{}.jsonl", unix_ms()))
    });
    let capture = capture_path
        .as_ref()
        .map(|p| Mutex::new(OpenOptions::new().create(true).append(true).open(p).unwrap()));
    let listener = TcpListener::bind(listen).unwrap_or_else(|e| {
        eprintln!("cannot listen on {listen}: {e}");
        std::process::exit(1);
    });

    println!("ForzaInfinity server on http://{listen}");
    println!("  responses: {}", root.join("responses").display());
    match &capture_path {
        Some(p) => println!("  capture:   {}", p.display()),
        None => println!("  capture:   off (--capture to record requests)"),
    }

    let schema = xtsw::Schema::load_embedded();
    println!("  schema:    {} classes, {} types", schema.services.len(), schema.types.len());

    let db_path = db_path.unwrap_or_else(|| root.join("data").join("fh2online.sqlite"));
    let store = db::Store::open(&db_path).unwrap_or_else(|e| {
        eprintln!("cannot open the database: {e}");
        std::process::exit(1);
    });
    println!("  database:  {}", db_path.display());
    let bests = store.import_all_save_bests();
    let granted = store.grant_expansions();
    if granted > 0 {
        println!("  accounts:  {granted} expansion grant(s) added (Storm Island)");
    }
    let filled = store.fill_all_save_cars();
    if filled > 0 {
        println!("  saves:     {filled} save best(s) given the player's car");
    }
    if bests > 0 {
        println!("  saves:     {bests} speed trap / zone best(s) taken from cloud saves");
    }

    // Default: the game folder next to FH2Online (FH2\ForzaHorizon2).
    let gamedb_path = gamedb_path
        .unwrap_or_else(|| root.join("../../ForzaHorizon2/Mount/media/stripped/gamedbRC.slt"));
    let catalog = gamedb::Catalog::load(&gamedb_path).unwrap_or_else(|e| {
        eprintln!("  gamedb:    not loaded ({e}); cars will show as ids");
        gamedb::Catalog::default()
    });
    if !catalog.cars.is_empty() {
        println!("  gamedb:    {} cars, {} tracks", catalog.cars.len(), catalog.tracks.len());
    }
    // the game's own car database: only it plays online (cardb.rs)
    let car_db = if cardb_gate {
        let mut allowed = cardb_allow;
        match std::fs::read(&gamedb_path) {
            Ok(bytes) => allowed.insert(0, sha256::hex(&bytes)),
            Err(e) if allowed.is_empty() => eprintln!("  cars:      the car database gate is off ({e}; --cardb-allow names one)"),
            Err(_) => {}
        }
        let gate = cardb::Gate::new(allowed);
        if gate.on() {
            let shown: Vec<String> = gate.allowed().iter().map(|h| format!("{}...", &h[..16])).collect();
            println!("  cars:      only the game's own car database plays online ({})", shown.join(", "));
        }
        gate
    } else {
        println!("  cars:      car database gate off (--cardb-gate off)");
        cardb::Gate::default()
    };
    let web = web.unwrap_or_else(|| root.join("../web/dist"));
    println!("  manager:   http://{listen}/ui/  (api /api, live /ws)");

    let app = Arc::new(App {
        root,
        capture,
        guard: guard::Guard::default(),
        counter: AtomicU64::new(0),
        schema,
        store,
        catalog,
        hub: live::Hub::default(),
        web,
        stats: Mutex::new(None),
        behind_proxy,
        registration_open,
        admission: admission::Admission::new(max_players),
        freeroam: (freeroam_on || freeroam_world.is_some()).then(|| freeroam::Freeroam::new(freeroam_world)),
        car_db,
    });
    match max_players {
        0 => println!("  players:   no limit"),
        n => println!("  players:   {n} at once, then a waiting line"),
    }
    if behind_proxy {
        println!("  proxy:     players come through a proxy on this machine");
    }
    if !registration_open {
        println!("  accounts:  registration closed");
    }
    match (&app.freeroam, freeroam_world) {
        (None, _) => {}
        (Some(_), Some(world)) => println!("  freeroam:  on, player-hosted worlds, else world {world}"),
        (Some(_), None) => println!("  freeroam:  on, player-hosted worlds only"),
    }
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let app = Arc::clone(&app);
                thread::spawn(move || {
                    if let Err(e) = serve_connection(&app, stream) {
                        // idle keep-alive connections timing out or being dropped are normal
                        use std::io::ErrorKind::*;
                        if !matches!(e.kind(), TimedOut | WouldBlock | ConnectionAborted | ConnectionReset) {
                            eprintln!("connection: {e}");
                        }
                    }
                });
            }
            Err(e) => eprintln!("accept: {e}"),
        }
    }
}

fn serve_connection(app: &App, stream: TcpStream) -> std::io::Result<()> {
    let peer = stream.peer_addr()?.ip();
    let mut writer = stream.try_clone()?;
    // the local proxy carries everyone: only the total limit applies to it
    let shared = app.behind_proxy && peer.is_loopback();
    let Some(_slot) = app.guard.connect(peer, !shared) else {
        let _ = writer.write_all(&plain_reply(503, "too many connections"));
        return Ok(());
    };
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream);
    // Keep-alive: serve requests until the client closes the connection.
    while let Some(request) = read_request(&mut reader, peer)? {
        let keep_alive = !request.header("connection").is_some_and(|v| v.eq_ignore_ascii_case("close"));
        if api::is_local(&request) {
            if !api::allowed(app, &request) {
                writer.write_all(&plain_reply(429, "slow down"))?;
                break;
            }
            match api::handle(app, &request) {
                api::Outcome::Reply(bytes) => {
                    writer.write_all(&bytes)?;
                    writer.flush()?;
                }
                api::Outcome::Upgrade(key) => {
                    let hello = api::hello(app);
                    return app.hub.serve(writer, &key, &hello);
                }
            }
            if !keep_alive {
                break;
            }
            continue;
        }
        if !app.guard.allow(request.client_ip(), guard::Bucket::Game) {
            let path = request.path_and_query.split('?').next().unwrap_or("");
            api::log_refusal(app, &request, request.client_ip(), guard::Bucket::Game, path);
            writer.write_all(&plain_reply(429, "slow down"))?;
            break;
        }
        let bytes = handle(app, &request);
        writer.write_all(&bytes)?;
        writer.flush()?;
        if !keep_alive {
            break;
        }
    }
    Ok(())
}

/// One line of at most MAX_LINE bytes; Ok(None) at end of stream.
fn read_line(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    if reader.by_ref().take(MAX_LINE).read_until(b'\n', &mut buf)? == 0 {
        return Ok(None);
    }
    if !buf.ends_with(b"\n") {
        return Err(std::io::Error::other("line too long"));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn read_request(reader: &mut BufReader<TcpStream>, peer: IpAddr) -> std::io::Result<Option<Request>> {
    let Some(line) = read_line(reader)? else { return Ok(None) };
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_owned();
    let path_and_query = parts.next().unwrap_or("/").to_owned();
    if method.is_empty() || method.len() > 8 || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(None);
    }
    let mut headers = Vec::new();
    loop {
        let Some(h) = read_line(reader)? else { return Ok(None) };
        let h = h.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(std::io::Error::other("too many headers"));
        }
        if let Some((name, value)) = h.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let mut request = Request { method, path_and_query, headers, body: Vec::new(), peer };
    if request.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        request.body = read_chunked(reader)?;
    } else if let Some(length) = request.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if length > MAX_BODY {
            return Err(std::io::Error::other("request body too large"));
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        request.body = body;
    }
    Ok(Some(request))
}

fn read_chunked(reader: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let size_line = read_line(reader)?.unwrap_or_default();
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
            .map_err(|_| std::io::Error::other("bad chunk size"))?;
        if size == 0 {
            // Trailers until the blank line.
            for _ in 0..MAX_HEADERS {
                match read_line(reader)? {
                    Some(t) if !t.trim().is_empty() => continue,
                    _ => break,
                }
            }
            return Ok(body);
        }
        if body.len() + size > MAX_BODY {
            return Err(std::io::Error::other("request body too large"));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
}

fn handle(app: &App, request: &Request) -> Vec<u8> {
    let seq = app.counter.fetch_add(1, Ordering::Relaxed) + 1;
    let target = resolve_target(request);
    let (spec, mut source, specific) = find_spec(&app.root, &request.method, &target);
    let delay = spec.as_ref().and_then(|s| s.delay_ms);
    let (xuid, banned) = request_identity(&app.store, request);
    if banned {
        println!("#{seq:<4} {:<5} refused: account suspended", request.method);
        let body = b"account suspended";
        let mut out = format!("HTTP/1.1 403 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n", reason(403), body.len())
            .into_bytes();
        out.extend_from_slice(body);
        return out;
    }
    // only the game's own cars play online (cardb.rs): the game gets what a suspended account
    // gets and plays offline; the hook's GET /infinity/v1/me says why
    if xuid != 0
        && let Some(why) = app.car_db.refusal(request.header("x-fh2-cardb"))
    {
        if app.car_db.first_time(xuid) {
            println!("#{seq:<4} {:<5} refused: xuid {xuid}'s car database is {}", request.method, why.name());
        }
        return plain_reply(403, "car database not accepted");
    }
    // Without an FH2Online account (or with Online=0 in the hook) the Forza services are down:
    // the game plays offline and nothing is written under a shared or claimed identity.
    if xuid == 0 && target.forza_class.is_some() {
        println!("#{seq:<4} {:<5} {} -> 503 (no account)", request.method, target.forza_method.as_deref().unwrap_or("?"));
        return plain_reply(503, "no FH2Online account");
    }
    // a slot, or the line: without a slot the game gets the same answer as without an account
    // and plays offline; the hook shows the place in line (GET /infinity/v1/queue)
    if xuid != 0 {
        let status = app.admission.arrive(xuid);
        if status.place != admission::Place::Playing {
            println!("#{seq:<4} {:<5} -> 503 (waiting: {:?})", request.method, status.place);
            return plain_reply(503, "server full");
        }
    }
    let ctx = forza::Ctx {
        store: &app.store,
        catalog: &app.catalog,
        xuid,
        events: Default::default(),
        mode: Default::default(),
    };

    // Online free roam (freeroam.rs): only for an account, and not over a routes.json rule.
    let mut freeroam_reply = match &app.freeroam {
        Some(f) if !specific && ctx.xuid != 0 && target.forza_class.is_none() => {
            let gamertag = |x| app.store.gamertag(x);
            let friends = |x| mutual_friends(&app.store, x);
            let caller = freeroam::Caller::new(ctx.xuid, request.client_ip(), &gamertag, &friends)
                .with_car_db(request.header("x-fh2-cardb"))
                .with_mode(app.store.account_mode(ctx.xuid));
            f.handle(&request.method, &target.host, &target.path, &request.body, &caller)
        }
        _ => None,
    };

    let mut request_xtsw = None;
    let mut response_xtsw = None;
    let (status, out_headers, out_body) = match forza_answer(app, &ctx, &target, request, spec.as_ref(), specific) {
        Some(answer) => {
            request_xtsw = Some(answer.request);
            response_xtsw = answer.response;
            if let Some(s) = answer.source {
                source = Some(s);
            }
            match answer.reply {
                Some(reply) => reply,
                None => build_body(spec, &source),
            }
        }
        None if freeroam_reply.is_some() => {
            let reply = freeroam_reply.take().unwrap_or_else(|| freeroam::Reply { status: 500, body: None });
            source = Some(format!("builtin:freeroam:{}", target.host));
            let mut headers = BTreeMap::new();
            let body = match reply.body {
                Some(body) => {
                    headers.insert("Content-Type".into(), "application/json; charset=utf-8".into());
                    // fh2srv.py 305: every JSON answer has an ETag
                    headers.insert("ETag".into(), format!("\"{seq}\""));
                    serde_json::to_vec(&body).unwrap_or_default()
                }
                None => Vec::new(),
            };
            (reply.status, headers, body)
        }
        None if !specific && target.host == gamerpic::HOST => {
            // the picture URL we put in the Xbox profile settings
            let xuid = target
                .query
                .split('&')
                .find_map(|kv| kv.strip_prefix("xuid="))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(ctx.xuid);
            let (png, from) = gamerpic::get(&app.root, xuid);
            source = Some(format!("builtin:gamerpic:{}", gamerpic_source(&from)));
            let mut headers = BTreeMap::new();
            headers.insert("Content-Type".into(), "image/png".into());
            (200, headers, png)
        }
        None => match (specific, forza::live_service(&ctx, &forza::LiveRequest {
            method: &request.method,
            host: &target.host,
            path: &target.path,
            query: &target.query,
            body: &request.body,
        })) {
            (false, Some(reply)) => {
                source = Some(format!("builtin:{}", target.host));
                let mut headers = BTreeMap::new();
                headers.insert("Content-Type".into(), "application/json; charset=utf-8".into());
                (200, headers, serde_json::to_vec(&reply).unwrap_or_default())
            }
            _ => build_body(spec, &source),
        },
    };

    let label = match (&target.forza_dir(), &target.forza_method) {
        (Some((_, c)), Some(m)) => format!("{c}.{m}"),
        _ => format!("{}{}", target.host, target.path),
    };
    println!(
        "#{seq:<4} {:<5} {:<60} {:>6} bytes -> {} via {}",
        request.method,
        truncate(&label, 60),
        request.body.len(),
        status,
        source.as_deref().unwrap_or("builtin")
    );

    if let Some(capture) = &app.capture {
        capture_request(capture, seq, request, &target, &request_xtsw, &source, status, &out_headers, &out_body, &response_xtsw);
    }

    // Transactional requests: keep the exact reply for Transaction.Reconcile.
    if let (Some(tx), Some(_)) = (request.header("x-transactionid"), &response_xtsw) {
        if (200..300).contains(&status) && ctx.xuid != 0 {
            app.store.save_transaction(tx, ctx.xuid, &label, &out_body);
        }
    }

    // Live feed for the manager UI: every game request (not stored), then the handler's events.
    app.hub.broadcast(&json!({
        "type": "request",
        "seq": seq,
        "time": unix_ms(),
        "xuid": ctx.xuid.to_string(),
        "label": label,
        "status": status,
        "source": source,
    }));
    let mut presence_changed = false;
    for event in ctx.events.take() {
        presence_changed |= event["kind"] == "presence";
        app.hub.broadcast(&json!({ "type": "event", "event": event }));
    }
    if presence_changed {
        app.hub.broadcast(&json!({ "type": "users", "users": app.store.users() }));
    }

    if let Some(ms) = delay {
        thread::sleep(Duration::from_millis(ms));
    }

    let mut response = format!("HTTP/1.1 {status} {}\r\n", reason(status)).into_bytes();
    for (name, value) in &out_headers {
        let lower = name.to_ascii_lowercase();
        if lower == "content-length" || lower == "connection" || lower == "transfer-encoding" {
            continue;
        }
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            continue;
        }
        response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    response.extend_from_slice(
        format!(
            "X-Fh2-Response-Source: {}\r\nContent-Length: {}\r\n\r\n",
            source.as_deref().unwrap_or("builtin"),
            out_body.len()
        )
        .as_bytes(),
    );
    response.extend_from_slice(&out_body);
    response
}

/// One line of captures/*.jsonl (the account token is never written).
#[allow(clippy::too_many_arguments)]
fn capture_request(
    capture: &Mutex<File>,
    seq: u64,
    request: &Request,
    target: &Target,
    request_xtsw: &Option<Value>,
    source: &Option<String>,
    status: u16,
    out_headers: &BTreeMap<String, String>,
    out_body: &[u8],
    response_xtsw: &Option<Value>,
) {
    let headers: Vec<(String, String)> = request
        .headers
        .iter()
        .map(|(n, v)| (n.clone(), if n == "x-fh2-token" { "(hidden)".into() } else { v.clone() }))
        .collect();
    let record = json!({
        "seq": seq,
        "time_ms": unix_ms(),
        "client_request_id": request.header("x-fh2-request-id"),
        "method": request.method,
        "original_url": target.original_url,
        "host": target.host,
        "path": target.path,
        "query": target.query,
        "forza_class": target.forza_class,
        "forza_method": target.forza_method,
        "request_headers": headers_json(&headers),
        "request_body_b64": B64.encode(&request.body),
        "request_body_text": std::str::from_utf8(&request.body).ok().filter(|t| !t.is_empty()),
        "request_xtsw": request_xtsw,
        "response_source": source,
        "response_status": status,
        "response_headers": out_headers,
        "response_body_b64": B64.encode(out_body),
        "response_xtsw": response_xtsw,
    });
    if let Ok(mut line) = serde_json::to_vec(&record) {
        line.push(b'\n');
        if let Ok(mut file) = capture.lock() {
            let _ = file.write_all(&line);
            let _ = file.flush();
        }
    }
}

fn plain_reply(status: u16, text: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{text}",
        reason(status),
        text.len()
    )
    .into_bytes()
}

struct ForzaAnswer {
    /// Decoded `_Parameters` (or `{"error": ...}`), for the capture.
    request: Value,
    /// `_Results` we encoded, or the Xls error we sent.
    response: Option<Value>,
    source: Option<String>,
    /// None: fall back to the spec's raw body.
    reply: Option<(u16, BTreeMap<String, String>, Vec<u8>)>,
}

/// Decodes a Forza RPC and, when there is something to encode (a built-in handler, or a spec
/// with `xtsw` / `xtsw_error`), builds the `bin/xtsw` reply.
fn forza_answer(
    app: &App,
    ctx: &forza::Ctx,
    target: &Target,
    request: &Request,
    spec: Option<&ResponseSpec>,
    specific: bool,
) -> Option<ForzaAnswer> {
    let class = target.forza_class.as_deref()?;
    let method = target.forza_method.as_deref()?;
    let def = app.schema.method(class, method);
    let params = match def.and_then(|d| d.params.as_deref()) {
        Some(ty) => app.schema.decode(ty, &request.body).unwrap_or_else(|e| json!({ "error": e })),
        None if def.is_none() => json!({ "error": "method not in schema.json" }),
        None => Value::Null,
    };
    let mut answer = ForzaAnswer { request: params, response: None, source: None, reply: None };

    let spec_encodes = spec.is_some_and(|s| s.xtsw.is_some() || s.xtsw_error.is_some());
    // A method-specific spec file wins over the built-in handler; a fallback _default does not.
    // Handlers write to the database: never feed them a body that failed to decode.
    let decoded = answer.request.get("error").is_none();
    let builtin = if (specific && !spec_encodes) || !decoded {
        None
    } else {
        forza::handle(ctx, class, method, &answer.request)
    };
    if builtin.is_none() && !spec_encodes {
        return Some(answer);
    }
    let spec = if specific || builtin.is_none() { spec } else { None };
    if builtin.is_some() && spec.is_none() {
        answer.source = Some(format!("builtin:{class}.{method}"));
    }

    let mut headers = spec.map(|s| s.headers.clone()).unwrap_or_default();
    headers.entry("Content-Type".into()).or_insert_with(|| "bin/xtsw".into());
    let status = spec.and_then(|s| s.status).unwrap_or(200);

    if let Some(err) = spec.and_then(|s| s.xtsw_error.as_ref()) {
        answer.response = Some(json!({ "xtsw_error": err }));
        answer.reply = Some((status, headers, xtsw::error_envelope(err.code, &err.message)));
        return Some(answer);
    }

    // A handler can answer with an Xls error: {"__xtsw_error": {"code": n, "message": "..."}}.
    if let Some(err) = builtin.as_ref().and_then(|b| b.get("__xtsw_error")) {
        let code = err["code"].as_u64().unwrap_or(1) as u32;
        let message = err["message"].as_str().unwrap_or("");
        answer.response = Some(json!({ "xtsw_error": err }));
        answer.reply = Some((status, headers, xtsw::error_envelope(code, message)));
        return Some(answer);
    }
    let mut results = builtin.unwrap_or_else(|| json!({}));
    if let Some(over) = spec.and_then(|s| s.xtsw.as_ref()) {
        forza::merge(&mut results, over);
    }
    let encoded = match def.and_then(|d| d.results.as_deref()) {
        Some(ty) => app.schema.encode(ty, &results),
        None => Ok(Vec::new()), // no _Results type: the header alone
    };
    match encoded {
        Ok(bytes) => {
            answer.response = Some(results);
            answer.reply = Some((status, headers, xtsw::ok_envelope(&bytes)));
        }
        Err(e) => {
            eprintln!("{class}.{method}: cannot encode reply: {e}");
            answer.response = Some(json!({ "encode_error": e, "results": results }));
            let mut h = BTreeMap::new();
            h.insert("Content-Type".into(), "text/plain".into());
            answer.reply = Some((500, h, format!("fh2-server: {e}").into_bytes()));
        }
    }
    Some(answer)
}

fn resolve_target(request: &Request) -> Target {
    let original_url = request
        .header("x-fh2-original-url")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https:/{}", request.path_and_query)); // path is /<host>/<rest>
    let without_scheme = original_url.split_once("://").map(|(_, r)| r).unwrap_or(&original_url);
    let (host_part, rest) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i..]),
        None => (without_scheme, "/"),
    };
    let host = host_part.split(':').next().unwrap_or("").to_ascii_lowercase();
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (path, query) = (path.to_owned(), query.to_owned());

    // "Forza.WebServices.Gifting, Forza.WebServices" -> "Forza.WebServices.Gifting"
    let forza_class = request.header("x-classname").map(|c| c.split(',').next().unwrap_or(c).trim().to_owned());
    let forza_method = request.header("x-method").map(|m| m.trim().to_owned());
    Target { original_url, host, path, query, forza_class, forza_method }
}

/// Returns (spec, its path under responses/, whether it is specific to this request:
/// a routes.json rule or responses/<ns>/<Class>/<Method>.json).
fn find_spec(root: &Path, method: &str, t: &Target) -> (Option<ResponseSpec>, Option<String>, bool) {
    let responses = root.join("responses");

    if let Ok(text) = fs::read_to_string(responses.join("routes.json")) {
        match serde_json::from_str::<Vec<Rule>>(&text) {
            Ok(rules) => {
                for (index, rule) in rules.into_iter().enumerate() {
                    if !rule_matches(&rule, method, t) {
                        continue;
                    }
                    if let Some(spec) = rule.response {
                        return (Some(spec), Some(format!("routes.json#{index}")), true);
                    }
                    if let Some(file) = rule.file {
                        if let Some(spec) = load_spec(&responses.join(&file)) {
                            return (Some(spec), Some(file), true);
                        }
                    }
                }
            }
            Err(e) => eprintln!("routes.json: {e}"),
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut specific = 0;
    if let (Some((ns, class)), Some(xmethod)) = (t.forza_dir(), &t.forza_method) {
        let dir = responses.join(ns).join(safe(class));
        candidates.push(dir.join(format!("{}.json", safe(xmethod))));
        specific = 1;
        candidates.push(dir.join("_default.json"));
        candidates.push(responses.join(ns).join("_default.json"));
    }
    if !t.host.is_empty() {
        candidates.push(responses.join("hosts").join(safe(&t.host)).join("_default.json"));
    }
    candidates.push(responses.join("_default.json"));

    for (index, path) in candidates.into_iter().enumerate() {
        if let Some(spec) = load_spec(&path) {
            let rel = path.strip_prefix(&responses).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            return (Some(spec), Some(rel), index < specific);
        }
    }
    (None, None, false)
}

fn rule_matches(rule: &Rule, method: &str, t: &Target) -> bool {
    let eq = |pattern: &Option<String>, value: Option<&str>| match pattern {
        None => true,
        Some(p) => value.is_some_and(|v| glob(&p.to_ascii_lowercase(), &v.to_ascii_lowercase())),
    };
    eq(&rule.host, Some(&t.host))
        && eq(&rule.method, Some(method))
        && eq(&rule.path, Some(&t.path))
        && eq(&rule.class, t.forza_class.as_deref())
        && eq(&rule.xmethod, t.forza_method.as_deref())
}

/// `*` matches any run of characters (including '/'), `?` one character.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn load_spec(path: &Path) -> Option<ResponseSpec> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<ResponseSpec>(&text) {
        Ok(mut spec) => {
            if let Some(file) = &spec.body_file {
                let base = path.parent().unwrap_or(Path::new("."));
                spec.body_file = Some(base.join(file).to_string_lossy().into_owned());
            }
            Some(spec)
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            None
        }
    }
}

fn build_body(spec: Option<ResponseSpec>, source: &Option<String>) -> (u16, BTreeMap<String, String>, Vec<u8>) {
    let spec = spec.unwrap_or_else(|| ResponseSpec { status: Some(404), ..Default::default() });
    let mut headers = spec.headers.clone();
    let body: Vec<u8> = if let Some(v) = &spec.json {
        if !headers.keys().any(|k| k.eq_ignore_ascii_case("content-type")) {
            headers.insert("Content-Type".into(), "application/json".into());
        }
        serde_json::to_vec(v).unwrap_or_default()
    } else if let Some(b) = &spec.body_b64 {
        B64.decode(b.trim()).unwrap_or_else(|e| {
            eprintln!("{}: bad body_b64: {e}", source.as_deref().unwrap_or("?"));
            Vec::new()
        })
    } else if let Some(f) = &spec.body_file {
        fs::read(f).unwrap_or_else(|e| {
            eprintln!("{f}: {e}");
            Vec::new()
        })
    } else {
        spec.body.clone().unwrap_or_default().into_bytes()
    };
    (spec.status.unwrap_or(200), headers, body)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn headers_json(headers: &[(String, String)]) -> Value {
    let mut map = serde_json::Map::new();
    for (name, value) in headers {
        let v = Value::String(value.clone());
        match map.get_mut(name) {
            Some(Value::Array(list)) => list.push(v),
            Some(existing) => *existing = Value::Array(vec![existing.take(), v]),
            None => {
                map.insert(name.clone(), v);
            }
        }
    }
    Value::Object(map)
}

/// Keep file names portable: anything outside [A-Za-z0-9._-] becomes '_'.
fn safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() || s.chars().all(|c| c == '.') { "_".into() } else { s }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

fn gamerpic_source(from: &gamerpic::Source) -> &'static str {
    match from {
        gamerpic::Source::Manual => "manual",
        gamerpic::Source::Placeholder => "placeholder",
    }
}

fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The players `xuid` is friends with (both added each other).
fn mutual_friends(store: &db::Store, xuid: u64) -> Vec<u64> {
    let mode = store.account_mode(xuid);
    store.friends(xuid).into_iter().filter(|f| f.2 && store.account_mode(f.0) == mode).map(|f| f.0).collect()
}

/// Who is playing: the account of X-Fh2-Token (sent by the hook once the player has an
/// account on this server and is online), else nobody (0). The game's own X-UserId is never
/// trusted: every Azimuth install reports the same one. Returns (xuid, banned).
fn request_identity(store: &db::Store, request: &Request) -> (u64, bool) {
    match request.header("x-fh2-token").and_then(|t| store.account_by_token(t)) {
        Some(account) => (account.xuid, account.banned),
        None => (0, false),
    }
}
