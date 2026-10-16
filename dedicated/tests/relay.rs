//! The relay on a loopback port, with fake consoles that speak NerdCore.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use dedicated_server::{
    bits::{Bits, Reader},
    carstate,
    nerd::{self, Acks, Car, CarEvent, Handle, Header, Msg, ObjectMsg, OpenConnection, OpenResponse},
    relay::{Relay, Settings},
};

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Stdin command lines.
    cmd: mpsc::Sender<String>,
}

impl Server {
    fn start(settings: Settings) -> Server {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let (cmd, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut relay = Relay::new(sock, settings);
            relay.set_commands(rx);
            relay.run(&s).unwrap();
        });
        Server { addr, stop, thread: Some(thread), cmd }
    }

    fn command(&self, line: &str) {
        self.cmd.send(line.to_string()).unwrap();
        // the relay reads commands between packets (every 10 ms at most)
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Client {
    sock: UdpSocket,
    server: SocketAddr,
    next_id: u16,
    acks: Acks,
    xuid: u64,
    attempt: u32,
    last_ping: Instant,
    pings_seen: usize,
    /// Messages read but not asked for yet.
    inbox: std::collections::VecDeque<Msg>,
    /// Bytes sent to and received from the relay.
    bytes_out: usize,
    bytes_in: usize,
    /// Never ack (a console that stopped acking).
    no_acks: bool,
    /// The car states the relay passed to this client (carstate.rs), as they came.
    car_states: Vec<carstate::Down>,
}

const WAIT: Duration = Duration::from_secs(5);

impl Client {
    fn new(server: &Server, xuid: u64) -> Client {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        Client {
            sock,
            server: server.addr,
            next_id: 0,
            acks: Acks::default(),
            xuid,
            attempt: 0x1000 + xuid as u32,
            last_ping: Instant::now(),
            pings_seen: 0,
            inbox: Default::default(),
            bytes_out: 0,
            bytes_in: 0,
            no_acks: false,
            car_states: Vec::new(),
        }
    }

    fn header(&self) -> (u16, u32) {
        if self.no_acks { (nerd::NO_ACK, 0) } else { self.acks.header() }
    }

    fn raw(&mut self, data: &[u8]) {
        self.bytes_out += data.len();
        self.sock.send_to(data, self.server).unwrap();
    }

    fn send(&mut self, msgs: &[Msg]) {
        let bits: Vec<Bits> = msgs.iter().map(Msg::write).collect();
        let (ack, ackbits) = self.header();
        let id = self.next_id;
        self.next_id += 1;
        self.raw(&nerd::packet(&bits, id, ack, ackbits));
    }

    fn send_unsequenced(&mut self, msgs: &[Msg]) {
        let bits: Vec<Bits> = msgs.iter().map(Msg::write).collect();
        let (ack, ackbits) = self.header();
        self.raw(&nerd::packet(&bits, nerd::NO_SEQ, ack, ackbits));
    }

    /// One packet's messages (None on timeout). Sequenced packets are acked right away, as the
    /// game does, so the relay does not resend them.
    fn recv(&mut self) -> Option<(Header, Vec<Msg>)> {
        let mut buf = [0u8; 65536];
        let (n, _) = self.sock.recv_from(&mut buf).ok()?;
        self.bytes_in += n;
        let data = &buf[..n];
        if carstate::is_carstate(data) {
            self.car_states.push(carstate::parse_down(data).expect("a car state the relay passes on"));
            return Some((Header::parse(&nerd::packet(&[], nerd::NO_SEQ, 0, 0)).unwrap(), Vec::new()));
        }
        let h = Header::parse(data).expect("header");
        assert_eq!(h.size as usize, n, "size field");
        let mut r = Reader::at(data, nerd::HEADER_LEN * 8);
        let mut msgs = Vec::new();
        for _ in 0..h.count {
            msgs.push(Msg::read(&mut r).expect("the relay sends only known messages"));
        }
        if h.id != nerd::NO_SEQ && !self.no_acks {
            self.acks.add(h.id);
            self.send_unsequenced(&[]);
        }
        self.pings_seen += msgs.iter().filter(|m| matches!(m, Msg::Ping(_))).count();
        Some((h, msgs))
    }

    /// Reads until a message matches (pinging the relay meanwhile, so this client stays alive).
    fn wait_for(&mut self, what: &str, pred: impl Fn(&Msg) -> bool) -> Msg {
        let end = Instant::now() + WAIT;
        loop {
            if let Some(i) = self.inbox.iter().position(|m| pred(m)) {
                return self.inbox.remove(i).unwrap();
            }
            if Instant::now() >= end {
                break;
            }
            self.keepalive();
            if let Some((_, msgs)) = self.recv() {
                self.inbox.extend(msgs);
            }
        }
        panic!("client {}: no {what}", self.xuid);
    }

    fn keepalive(&mut self) {
        if self.last_ping.elapsed() > Duration::from_millis(200) {
            self.last_ping = Instant::now();
            self.send_unsequenced(&[Msg::Ping(1)]);
        }
    }

    /// Nothing matching for `d` (the client keeps reading, acking and pinging).
    fn expect_none(&mut self, d: Duration, what: &str, pred: impl Fn(&Msg) -> bool) {
        assert!(!self.inbox.iter().any(|m| pred(m)), "client {}: unexpected {what}", self.xuid);
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.keepalive();
            if let Some((_, msgs)) = self.recv() {
                assert!(!msgs.iter().any(|m| pred(m)), "client {}: unexpected {what}", self.xuid);
                self.inbox.extend(msgs);
            }
        }
    }

    fn open_connection(&self) -> OpenConnection {
        OpenConnection {
            version: nerd::VERSION.to_vec(),
            game: nerd::GAME.to_vec(),
            flags: 2,
            user: self.xuid,
            a: 0x12,
            b: 4,
            attempt: self.attempt,
        }
    }

    fn open(&mut self) -> OpenResponse {
        let oc = self.open_connection();
        self.next_id = 0;
        self.send(&[Msg::OpenConnection(oc)]);
        let attempt = self.attempt;
        match self.wait_for("OpenConnectionResponse", |m| matches!(m, Msg::OpenResponse(_))) {
            Msg::OpenResponse(r) => {
                assert_eq!(r.attempt, attempt);
                assert_eq!(r.flags, 2);
                r
            }
            _ => unreachable!(),
        }
    }

    fn ack(&mut self, own_attempt: u32) {
        self.send(&[Msg::OpenResponse(OpenResponse {
            flags: 2,
            reason: nerd::REASON_RESPONSE_ACK,
            attempt: own_attempt,
            own_attempt: self.attempt,
        })]);
    }

    /// The whole handshake, up to the SessionHost (its Create).
    fn join(&mut self) -> ObjectMsg {
        let r = self.open();
        assert_eq!(r.reason, nerd::REASON_OK);
        assert!(r.own_attempt != 0);
        self.ack(r.own_attempt);
        self.wait_for("TimeSync", |m| matches!(m, Msg::TimeSync { .. }));
        let own = r.own_attempt;
        self.wait_for("SyncDone", |m| *m == Msg::SyncDone(own));
        match self.wait_for("SessionHost", |m| {
            matches!(m, Msg::Object(o) if o.create && o.router == 6 && o.handle == Handle::Own(0) && o.ty == 0)
        }) {
            Msg::Object(o) => o,
            _ => unreachable!(),
        }
    }
}

fn payload(n: usize, seed: u64) -> Bits {
    let mut b = Bits::new();
    for i in 0..n {
        b.push((seed >> (i % 61)) ^ (i as u64 / 3), 1);
    }
    b
}

/// A car prediction at a real position (the rest any bits).
fn car_body(seed: u64) -> Bits {
    let mut b = Bits::new();
    for v in [1500.0f32 + seed as f32, 30.0, -2200.0] {
        b.push(v.to_bits() as u64, 32);
    }
    b.append(&payload(nerd::CAR_BODY_BITS - 96, seed));
    b
}

fn floats(vals: &[f32]) -> Bits {
    let mut b = Bits::new();
    for v in vals {
        b.push(v.to_bits() as u64, 32);
    }
    b
}

/// ConsoleSynced entries (index, value) of its 1- and 2-bit vars.
fn console(entries: &[(u64, u64)]) -> Bits {
    let mut b = Bits::new();
    for &(i, v) in entries {
        b.push(i, 4).push(v, if i >= 7 { 2 } else { 1 });
    }
    b
}

/// PlayerData: BufChoices, NetworkName, XUID, and `extra` 32-bit points vars (8..=11).
fn player(name: &str, extra: &[u64]) -> Bits {
    let mut b = Bits::new();
    b.push(0, 8).push(0x0102_0304_0506_0708, 64);
    let units: Vec<u16> = name.encode_utf16().collect();
    b.push(3, 8).push(units.len() as u64, 5);
    for u in units {
        b.push(u as u64, 16);
    }
    b.push(4, 8).push(2535412345678901, 64);
    for (k, v) in extra.iter().enumerate() {
        b.push(8 + k as u64, 8).push(*v, 32);
    }
    b
}

/// CarNetworked: Fuel and CarUniverseId.
fn car_vars(fuel: f32) -> Bits {
    let mut b = Bits::new();
    b.push(8, 5).push(fuel.to_bits() as u64, 32).push(21, 5).push(9, 5);
    b
}

fn object(create: bool, router: u8, id: u32, seq: u8, p: &Bits) -> Msg {
    Msg::Object(ObjectMsg { create, router, handle: Handle::Own(id), ty: 0, seq, payload: p.clone() })
}

fn is_obj(m: &Msg, create: bool, router: u8, handle: Handle, seq: u8) -> Option<Bits> {
    match m {
        Msg::Object(o) if o.create == create && o.router == router && o.handle == handle && o.seq == seq => {
            Some(o.payload.clone())
        }
        _ => None,
    }
}

fn is_delete(m: &Msg, router: u8, handle: Handle) -> bool {
    matches!(m, Msg::Delete(d) if d.router == router && d.handle == handle)
}

fn settings() -> Settings {
    Settings {
        max_players: 3,
        quiet_drop: Duration::from_millis(1500),
        keepalive: Duration::from_millis(100),
        resend_after: Duration::from_millis(200),
        quiet: true,
        // every test client is on 127.0.0.1
        max_peers_per_ip: 16,
        handshake_per_sec: 1000,
        takeover_quiet: Duration::from_millis(300),
        ..Default::default()
    }
}

#[test]
fn handshake_sends_timesync_and_syncdone() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let r = a.open();
    assert_eq!(r.reason, nerd::REASON_OK);
    assert!(r.own_attempt != 0 && r.own_attempt != 1, "a random attempt");
    a.ack(r.own_attempt);
    let (h, msgs) = loop {
        let (h, msgs) = a.recv().expect("sync packet");
        if msgs.iter().any(|m| matches!(m, Msg::TimeSync { .. })) {
            break (h, msgs);
        }
    };
    assert_eq!(h.id, nerd::NO_SEQ, "TimeSync and SyncDone go unsequenced, as relay.py sends them");
    assert!(matches!(msgs[0], Msg::TimeSync { index: 0, .. }));
    assert_eq!(msgs[1], Msg::SyncDone(r.own_attempt));
    // the SessionHost comes sequenced
    let (h, msgs) = loop {
        let (h, msgs) = a.recv().expect("world");
        if !msgs.is_empty() {
            break (h, msgs);
        }
    };
    assert_ne!(h.id, nerd::NO_SEQ);
    assert!(is_obj(&msgs[0], true, 6, Handle::Own(0), 0).is_some());
    // ping -> pong with our time echoed
    a.send(&[Msg::Ping(777)]);
    a.wait_for("Pong", |m| matches!(m, Msg::Pong { echo: 777, .. }));
    // and the relay pings us
    a.wait_for("keepalive Ping", |m| matches!(m, Msg::Ping(_)));
    // another console's attempts are different
    let mut b = Client::new(&server, 2);
    assert_ne!(b.open().own_attempt, r.own_attempt);
}

#[test]
fn relay_snapshot_drops_and_limits() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1001);
    let mut b = Client::new(&server, 1002);
    a.join();
    b.join();
    let (a_idx, b_idx) = (1u8, 2u8); // round-robin from 1

    // A's objects: ConsoleSynced, and a PlayerData with a Create and an update
    let p3 = console(&[(0, 1), (3, 0), (8, 2)]);
    let (pc, pu) = (player("Alex", &[]), player("Alex2", &[7, 8]));
    a.send(&[object(true, 3, 5, 0, &p3), object(true, 1, 2, 10, &pc), object(false, 1, 2, 11, &pu)]);
    let body = car_body(99);
    a.send_unsequenced(&[Msg::Car(Car { handle: Handle::Own(2), body: body.clone() })]);

    // B gets them with the relay handle and A's index
    let got = b.wait_for("A's ConsoleSynced", |m| is_obj(m, true, 3, Handle::Relay(5, a_idx), 0).is_some());
    assert_eq!(is_obj(&got, true, 3, Handle::Relay(5, a_idx), 0).unwrap(), p3);
    let got = b.wait_for("A's object", |m| is_obj(m, true, 1, Handle::Relay(2, a_idx), 10).is_some());
    assert_eq!(is_obj(&got, true, 1, Handle::Relay(2, a_idx), 10).unwrap(), pc);
    let got = b.wait_for("A's update", |m| is_obj(m, false, 1, Handle::Relay(2, a_idx), 11).is_some());
    assert_eq!(is_obj(&got, false, 1, Handle::Relay(2, a_idx), 11).unwrap(), pu);
    match b.wait_for("A's car", |m| matches!(m, Msg::Car(_))) {
        Msg::Car(c) => {
            assert_eq!(c.handle, Handle::Relay(2, a_idx));
            assert_eq!(c.body, body);
        }
        _ => unreachable!(),
    }

    // a resent update is not relayed again
    a.send(&[object(false, 1, 2, 11, &pu)]);
    b.expect_none(Duration::from_millis(300), "duplicate update", |m| {
        is_obj(m, false, 1, Handle::Relay(2, a_idx), 11).is_some()
    });

    // FinaliseSessionEntry -> server update of A's ConsoleSynced, var 1 = true, seq 0
    a.send(&[Msg::FinaliseSessionEntry]);
    let got = a.wait_for("SessionEntryFinalised", |m| is_obj(m, false, 3, Handle::Server(5), 0).is_some());
    let p = is_obj(&got, false, 3, Handle::Server(5), 0).unwrap();
    assert_eq!(p.len(), 5);
    let mut pr = Reader::new(p.as_bytes());
    assert_eq!((pr.bits(4).unwrap(), pr.bits(1).unwrap()), (1, 1));

    // C joins late: the snapshot has A's object as one Create with the update folded in
    let mut c = Client::new(&server, 1003);
    c.join();
    let c_idx = 3u8;
    let got = c.wait_for("snapshot of A's object", |m| is_obj(m, true, 1, Handle::Relay(2, a_idx), 11).is_some());
    let mut merged = pc.clone();
    merged.append(&pu);
    assert_eq!(is_obj(&got, true, 1, Handle::Relay(2, a_idx), 11).unwrap(), merged);
    c.wait_for("snapshot of A's ConsoleSynced", |m| is_obj(m, true, 3, Handle::Relay(5, a_idx), 0).is_some());

    // the world is full (3)
    let mut d = Client::new(&server, 1004);
    assert_eq!(d.open().reason, nerd::REASON_REFUSED);

    // A comes back from another address (same XUID) once the old A is quiet: the old A goes,
    // B and C delete its objects
    std::thread::sleep(Duration::from_millis(400));
    let mut a2 = Client::new(&server, 1001);
    a2.join();
    for cl in [&mut b, &mut c] {
        cl.wait_for("delete of A's object", |m| {
            matches!(m, Msg::Delete(d) if d.router == 1 && d.handle == Handle::Relay(2, a_idx) && d.seq == 12)
        });
        cl.wait_for("delete of A's ConsoleSynced", |m| {
            matches!(m, Msg::Delete(d) if d.router == 3 && d.handle == Handle::Relay(5, a_idx) && d.seq == 1)
        });
    }
    // the next index, not A's old one (D was refused before it got one)
    let a2_idx = 4u8;

    // A2 makes an object and closes: B gets the delete
    let p = car_vars(0.5);
    a2.send(&[object(true, 2, 9, 0, &p)]);
    b.wait_for("A2's object", |m| is_obj(m, true, 2, Handle::Relay(9, a2_idx), 0).is_some());
    a2.send(&[Msg::Close]);
    b.wait_for("delete of A2's object", |m| {
        matches!(m, Msg::Delete(d) if d.router == 2 && d.handle == Handle::Relay(9, a2_idx) && d.seq == 1)
    });

    // C makes an object, then goes quiet: dropped, B gets the delete (B pings while it waits)
    c.send(&[object(true, 2, 7, 3, &p)]);
    b.wait_for("C's object", |m| is_obj(m, true, 2, Handle::Relay(7, c_idx), 3).is_some());
    let t = Instant::now();
    b.wait_for("delete of quiet C's object", |m| {
        matches!(m, Msg::Delete(d) if d.router == 2 && d.handle == Handle::Relay(7, c_idx) && d.seq == 4)
    });
    assert!(t.elapsed() >= Duration::from_millis(1000), "dropped only after the quiet time");
    assert!(b.pings_seen > 0, "keepalive pings");
    let _ = b_idx;
}

#[test]
fn strangers_and_bad_versions() {
    let server = Server::start(settings());
    // a packet that is not an OpenConnection, from an unknown address: no answer
    let mut x = Client::new(&server, 5);
    x.send(&[Msg::Ping(1)]);
    assert!(x.recv().is_none());
    // another build: refused with "version mismatch"
    let oc = OpenConnection {
        version: b"DEADBEEF".to_vec(),
        game: nerd::GAME.to_vec(),
        flags: 0,
        user: 5,
        a: 0,
        b: 0,
        attempt: 9,
    };
    x.send(&[Msg::OpenConnection(oc)]);
    match x.wait_for("refusal", |m| matches!(m, Msg::OpenResponse(_))) {
        Msg::OpenResponse(r) => assert_eq!((r.reason, r.attempt), (nerd::REASON_VERSION_MISMATCH, 9)),
        _ => unreachable!(),
    }
    // garbage
    x.sock.send_to(&[1, 2, 3], server.addr).unwrap();
    x.sock.send_to(&[0u8; 11], server.addr).unwrap();
    x.sock.send_to(&[0xFFu8; 1400], server.addr).unwrap();
    // still serving
    let mut y = Client::new(&server, 6);
    y.join();
}

#[test]
fn unacked_object_messages_are_resent() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[object(true, 1, 3, 0, &player("x", &[]))]);
    // B reads the first copy without acking it: the relay sends it again in a new packet
    let mut buf = [0u8; 4096];
    b.sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut ids = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    while ids.len() < 2 && Instant::now() < end {
        let Ok((n, _)) = b.sock.recv_from(&mut buf) else { continue };
        let h = Header::parse(&buf[..n]).unwrap();
        let mut r = Reader::at(&buf[..n], nerd::HEADER_LEN * 8);
        if let Ok(m) = Msg::read(&mut r)
            && is_obj(&m, true, 1, Handle::Relay(3, 1), 0).is_some()
        {
            ids.push(h.id);
        }
    }
    assert_eq!(ids.len(), 2, "resent");
    assert_ne!(ids[0], ids[1], "in a new packet");
}

/// What the Steam Deck sent on joining: RepairDamage and ServerTrafficMessage ahead of a car in
/// one packet. The car still gets through, the repair is relayed with the relay handle, and the
/// traffic message is not relayed.
#[test]
fn repair_and_traffic_do_not_lose_the_car() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    let a_idx = 1u8;
    let body = car_body(7);
    let repair = Msg::CarEvent(CarEvent { id: nerd::MSG_REPAIR_DAMAGE, handle: Handle::Own(2), body: Bits::new() });
    let car = Msg::Car(Car { handle: Handle::Own(2), body: body.clone() });
    a.send_unsequenced(&[repair, car.clone()]);
    match b.wait_for("A's car after the repair", |m| matches!(m, Msg::Car(_))) {
        Msg::Car(c) => assert_eq!((c.handle, c.body), (Handle::Relay(2, a_idx), body.clone())),
        _ => unreachable!(),
    }
    b.wait_for("A's repair", |m| {
        matches!(m, Msg::CarEvent(e) if e.id == nerd::MSG_REPAIR_DAMAGE && e.handle == Handle::Relay(2, a_idx))
    });

    // traffic first, then a glass break and the car: all read
    let traffic = Msg::Traffic { id: nerd::MSG_SERVER_TRAFFIC_MESSAGE, body: payload(72, 3) };
    let mut glass_body = Bits::new();
    glass_body.push(4, 32).append(&floats(&[1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0])).push(1, 32);
    let glass = Msg::CarEvent(CarEvent { id: nerd::MSG_BREAK_GLASS, handle: Handle::Own(2), body: glass_body.clone() });
    a.send_unsequenced(&[traffic, glass, car]);
    b.wait_for("A's glass", |m| {
        matches!(m, Msg::CarEvent(e) if e.id == nerd::MSG_BREAK_GLASS && e.handle == Handle::Relay(2, a_idx) && e.body == glass_body)
    });
    b.wait_for("A's second car", |m| matches!(m, Msg::Car(_)));
    b.expect_none(Duration::from_millis(300), "relayed traffic", |m| matches!(m, Msg::Traffic { .. }));
}

/// What the Deck sent in free roam: SetGroupSatNavEvent ahead of a car in one packet. The car
/// gets through; the 52 itself is not relayed (consoles do not act on it), the server answers it
/// with GroupSatNavNode on its SessionHost, to everyone, and a late joiner gets it in the Create.
#[test]
fn group_sat_nav_is_answered_and_does_not_lose_the_car() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    assert_eq!(a.join().seq, 0);
    b.join();
    let a_idx = 1u8;
    let body = car_body(11);
    let car = Msg::Car(Car { handle: Handle::Own(2), body: body.clone() });
    let sat_nav = |node| Msg::SetGroupSatNav { handle: Handle::Own(nerd::SESSION_HOST_ID), node };
    a.send(&[sat_nav(4242), car.clone()]);
    match b.wait_for("A's car after the sat nav", |m| matches!(m, Msg::Car(_))) {
        Msg::Car(c) => assert_eq!((c.handle, c.body), (Handle::Relay(2, a_idx), body.clone())),
        _ => unreachable!(),
    }
    let node_4242 = nerd::group_sat_nav_payload(4242);
    for c in [&mut a, &mut b] {
        let p = c.wait_for("the SessionHost update", |m| is_obj(m, false, 6, Handle::Own(0), 1).is_some());
        assert_eq!(is_obj(&p, false, 6, Handle::Own(0), 1), Some(node_4242.clone()));
    }
    b.expect_none(Duration::from_millis(300), "a relayed SetGroupSatNavEvent", |m| matches!(m, Msg::SetGroupSatNav { .. }));

    // the same node again, a node out of range, then RequestLeaderChange and the car: no new
    // update, the car arrives
    let mut leader = Bits::new();
    Handle::Own(2).write(&mut leader, 7);
    a.send_unsequenced(&[sat_nav(4242), sat_nav(-5), sat_nav(nerd::SAT_NAV_NODE_MAX + 1), Msg::Other { id: 59, body: leader }, car.clone()]);
    b.wait_for("A's second car", |m| matches!(m, Msg::Car(_)));
    b.expect_none(Duration::from_millis(300), "an update for an unchanged or bad node", |m| {
        matches!(m, Msg::Object(o) if o.router == 6) || matches!(m, Msg::Other { .. })
    });

    // a console still in the handshake cannot set it
    let mut x = Client::new(&server, 9);
    x.open();
    x.send(&[sat_nav(77)]);
    b.expect_none(Duration::from_millis(300), "a node set by a half-connected peer", |m| matches!(m, Msg::Object(o) if o.router == 6));

    // a late joiner: the Create has the node, under the last update's seq
    let mut c = Client::new(&server, 3);
    let host = c.join();
    assert_eq!((host.seq, host.payload), (1, node_4242));

    // cleared (node 0): seq 2, to all three
    b.send(&[sat_nav(0)]);
    for cl in [&mut a, &mut b, &mut c] {
        let p = cl.wait_for("the cleared node", |m| is_obj(m, false, 6, Handle::Own(0), 2).is_some());
        assert_eq!(is_obj(&p, false, 6, Handle::Own(0), 2), Some(nerd::group_sat_nav_payload(0)));
    }
}

/// Events: a console that enters one asks for its GameOptions (ObjectRequestOwnership, router 6
/// type 1, a Transfer handle). With races on, the first one gets it (ObjectGrantOwnership, seq 0:
/// the object's first message from the server) and a resend of the same request is not answered
/// again; while it has the event the others are denied (ObjectTransferOwnership), until it
/// leaves. With races off nothing is answered (the hook sends the player back to free roam).
#[test]
fn events_go_to_the_first_console_that_enters_one() {
    fn request(id: u32) -> Msg {
        let mut b = Bits::new();
        b.push(6, 4).push(nerd::GAME_OPTIONS_TYPE as u64, 2);
        Handle::Transfer(id).write(&mut b, 2);
        Msg::Other { id: nerd::MSG_OBJECT_REQUEST_OWNERSHIP, body: b }
    }
    fn answer(m: &Msg) -> Option<(u8, Msg)> {
        match m {
            Msg::Other { id, .. } if *id == nerd::MSG_OBJECT_GRANT_OWNERSHIP || *id == nerd::MSG_OBJECT_TRANSFER_OWNERSHIP => Some((*id, m.clone())),
            _ => None,
        }
    }
    let is_answer = |m: &Msg| answer(m).is_some();

    let off = Server::start(Settings { races: false, ..settings() });
    let mut x = Client::new(&off, 7);
    x.join();
    x.send(&[request(0)]);
    x.expect_none(Duration::from_millis(400), "an answer with races off", is_answer);
    drop(x);
    drop(off);

    let server = Server::start(Settings { races: true, ..settings() });
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[request(0)]);
    let g = a.wait_for("the grant", is_answer);
    assert_eq!(g.write(), nerd::grant_ownership(6, Handle::Transfer(0), 0, &Bits::new()));
    a.send(&[request(0)]);
    a.expect_none(Duration::from_millis(400), "a second answer to a resent request", is_answer);

    b.send(&[request(0)]);
    let d = b.wait_for("the denial", is_answer);
    assert_eq!(d.write(), nerd::transfer_ownership(6, Handle::Transfer(0), 0));

    // A leaves: B's next request (after the resend window) gets the event, as its seq 1
    a.send(&[Msg::Close]);
    b.expect_none(Duration::from_millis(2100), "an answer while waiting", is_answer);
    b.send(&[request(0)]);
    let g = b.wait_for("the grant once A left", is_answer);
    assert_eq!(g.write(), nerd::grant_ownership(6, Handle::Transfer(0), 1, &Bits::new()));
}

/// A race with one console, the relay as its server: MsgRequestRace from the console that has
/// the event -> RaceReady on the SessionHost and ParticipatingInRace on its console; its cars
/// loaded (HasLoadedCarsForAllParticipants) -> MsgStartingGrid; its race route loaded
/// (RenderReady) -> MsgStartRace and InRace.
#[test]
fn a_race_with_one_console_starts() {
    let server = Server::start(Settings { races: true, ..settings() });
    let mut a = Client::new(&server, 1);
    a.join();
    a.send(&[object(true, 3, 5, 0, &console(&[(0, 0)]))]);
    let mut req = Bits::new();
    req.push(6, 4).push(nerd::GAME_OPTIONS_TYPE as u64, 2);
    Handle::Transfer(0).write(&mut req, 2);
    a.send(&[Msg::Other { id: nerd::MSG_OBJECT_REQUEST_OWNERSHIP, body: req }]);
    a.wait_for("the grant", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_OBJECT_GRANT_OWNERSHIP));
    a.send(&[Msg::Other { id: nerd::MSG_REQUEST_RACE, body: Bits::new() }]);
    // alone in the world: no launch countdown, selected at once
    let ready = nerd::session_host_payload(&[(4, 1, 1), (6, 0, 1)]);
    let p = a.wait_for("RaceReady", |m| is_obj(m, false, 6, Handle::Own(0), 1).is_some());
    assert_eq!(is_obj(&p, false, 6, Handle::Own(0), 1), Some(ready));
    let selected = nerd::session_host_payload(&[(5, 1, 1), (2, 0, 2)]);
    let p = a.wait_for("RaceParticipantsSelected", |m| is_obj(m, false, 6, Handle::Own(0), 2).is_some());
    assert_eq!(is_obj(&p, false, 6, Handle::Own(0), 2), Some(selected));
    let p = a.wait_for("ParticipatingInRace", |m| is_obj(m, false, 3, Handle::Server(5), 0).is_some());
    assert_eq!(is_obj(&p, false, 3, Handle::Server(5), 0), Some(console(&[(2, 1)])));
    // ready, then its cars loaded: the grid, with its own console first
    let mut ready_req = Bits::new();
    Handle::Own(0).write(&mut ready_req, 2);
    ready_req.bool(true);
    a.send(&[Msg::Other { id: nerd::MSG_READY_REQUEST, body: ready_req }]);
    a.wait_for("Ready", |m| is_obj(m, false, 3, Handle::Server(5), 1).is_some());
    a.send(&[object(false, 3, 5, 1, &console(&[(5, 1)]))]);
    let g = a.wait_for("the grid", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_STARTING_GRID));
    assert_eq!(g.write(), nerd::starting_grid(&[Handle::Own(5)]));
    // its route loaded: the start
    a.send(&[object(false, 3, 5, 2, &console(&[(0, 1)]))]);
    a.wait_for("the start", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_START_RACE));
    a.wait_for("InRace", |m| is_obj(m, false, 6, Handle::Own(0), 3).is_some());
    // it crosses the line (MsgScoreKeyframe): the race is over, InRace off
    let mut kf = Bits::new();
    Handle::Own(2).write(&mut kf, 7);
    for _ in 0..5 {
        kf.string(&[0; 8]);
    }
    kf.push(0, 32 + 3 * 8 + 3);
    a.send(&[Msg::Other { id: nerd::MSG_SCORE_KEYFRAME, body: kf }]);
    let off = nerd::session_host_payload(&[(4, 0, 1), (5, 0, 1), (6, 0, 1)]);
    let p = a.wait_for("the end", |m| is_obj(m, false, 6, Handle::Own(0), 4).is_some());
    assert_eq!(is_obj(&p, false, 6, Handle::Own(0), 4), Some(off));
}

/// Two consoles: A starts its event, B is invited (the launch countdown on the SessionHost),
/// gets A's GameOptions, joins by asking Ready; at the countdown's end both take part, and the
/// grid waits for both.
#[test]
fn a_second_console_joins_the_event() {
    let server = Server::start(Settings { races: true, race_invite: Duration::from_millis(600), ..settings() });
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[object(true, 3, 5, 0, &console(&[(0, 0)]))]);
    b.send(&[object(true, 3, 6, 0, &console(&[(0, 0)]))]);
    let mut req = Bits::new();
    req.push(6, 4).push(nerd::GAME_OPTIONS_TYPE as u64, 2);
    Handle::Transfer(0).write(&mut req, 2);
    a.send(&[Msg::Other { id: nerd::MSG_OBJECT_REQUEST_OWNERSHIP, body: req }]);
    a.wait_for("the grant", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_OBJECT_GRANT_OWNERSHIP));
    // A's GameOptions: B gets it as the server's update of its own (seq 0 from us)
    let opts = payload(40, 3);
    a.send(&[Msg::Object(ObjectMsg { create: false, router: 6, handle: Handle::Transfer(0), ty: 0, seq: 0, payload: opts.clone() })]);
    let g = b.wait_for("A's GameOptions", |m| is_obj(m, false, 6, Handle::Transfer(0), 0).is_some());
    assert_eq!(is_obj(&g, false, 6, Handle::Transfer(0), 0), Some(opts));
    a.send(&[Msg::Other { id: nerd::MSG_REQUEST_RACE, body: Bits::new() }]);
    // B is invited (MsgRequestRace from the server), A is not
    b.wait_for("the invite", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_REQUEST_RACE));
    // the launch countdown, to both
    for c in [&mut a, &mut b] {
        let p = c.wait_for("the launch countdown", |m| is_obj(m, false, 6, Handle::Own(0), 1).is_some());
        let body = is_obj(&p, false, 6, Handle::Own(0), 1).unwrap();
        assert_eq!(body.len(), 4 * 4 + 1 + 1 + 2 + 32);
    }
    let mut ready_req = Bits::new();
    Handle::Own(0).write(&mut ready_req, 2);
    ready_req.bool(true);
    b.send(&[Msg::Other { id: nerd::MSG_READY_REQUEST, body: ready_req }]);
    // the countdown's end: both selected, both taking part
    a.wait_for("A taking part", |m| is_obj(m, false, 3, Handle::Server(5), 0).is_some());
    let p = b.wait_for("B taking part", |m| {
        is_obj(m, false, 3, Handle::Server(6), 1).is_some_and(|p| p == console(&[(2, 1)]))
    });
    let _ = p;
    // the grid only once both have their cars loaded
    a.send(&[object(false, 3, 5, 1, &console(&[(5, 1)]))]);
    a.expect_none(Duration::from_millis(300), "a grid before B is loaded", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_STARTING_GRID));
    b.send(&[object(false, 3, 6, 1, &console(&[(5, 1)]))]);
    let g = a.wait_for("A's grid", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_STARTING_GRID));
    assert_eq!(g.write(), nerd::starting_grid(&[Handle::Own(5), Handle::Relay(6, 2)]));
    let g = b.wait_for("B's grid", |m| matches!(m, Msg::Other { id, .. } if *id == nerd::MSG_STARTING_GRID));
    assert_eq!(g.write(), nerd::starting_grid(&[Handle::Relay(5, 1), Handle::Own(6)]));
}

/// `kick <xuid>` on stdin: the player goes at once (the others delete its objects) and may
/// come back right away. Unknown and over-long lines do nothing.
#[test]
fn kick_drops_and_may_come_back() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 2535412345678901);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[object(true, 2, 4, 0, &car_vars(1.0))]);
    b.wait_for("A's object", |m| is_obj(m, true, 2, Handle::Relay(4, 1), 0).is_some());
    server.command("bogus command");
    server.command(&"kick 2535412345678901 ".repeat(40));
    server.command("kick 2535412345678901 trapaça");
    b.wait_for("delete of the kicked player's object", |m| is_delete(m, 2, Handle::Relay(4, 1)));
    // its endpoint is forgotten: its packets get nothing
    a.inbox.clear();
    a.send(&[Msg::Ping(5)]);
    a.expect_none(Duration::from_millis(300), "an answer after the kick", |m| matches!(m, Msg::Pong { .. }));
    // joining again is not refused
    let mut a2 = Client::new(&server, 2535412345678901);
    assert_eq!(a2.open().reason, nerd::REASON_OK);
    // B is fine
    b.send(&[Msg::Ping(6)]);
    b.wait_for("B's pong", |m| matches!(m, Msg::Pong { echo: 6, .. }));
}

/// What a hostile console could send to crash the others: none of it reaches them.
#[test]
fn hostile_objects_and_events_are_not_forwarded() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    // a lobby object (only the server makes those), a road trip one
    a.send(&[
        Msg::Object(ObjectMsg { create: true, router: 6, handle: Handle::Own(1), ty: 1, seq: 0, payload: payload(40, 1) }),
        Msg::Object(ObjectMsg { create: true, router: 9, handle: Handle::Own(1), ty: 0, seq: 0, payload: payload(40, 1) }),
    ]);
    // a NetworkName longer than the reader takes; a NaN fuel; an index past ConsoleSynced's
    let mut long_name = Bits::new();
    long_name.push(3, 8).push(31, 5);
    for _ in 0..31 {
        long_name.push(0x41, 16);
    }
    let mut nan = Bits::new();
    nan.push(8, 5).push(f32::NAN.to_bits() as u64, 32);
    a.send(&[object(true, 1, 1, 0, &long_name), object(true, 2, 2, 0, &nan), object(true, 3, 3, 0, &console(&[(9, 1)]))]);
    // MsgCollObjDetachPart (its reader overflows a 1024-byte buffer), a car far off the map,
    // a collision with a NaN
    let detach = Msg::CollObjDetachPart { kind: 1, data: vec![0x41; 2000] };
    let far = Msg::Car(Car { handle: Handle::Own(2), body: {
        let mut b = Bits::new();
        b.append(&floats(&[1.0e9, 0.0, 0.0])).append(&payload(nerd::CAR_BODY_BITS - 96, 1));
        b
    } });
    let coll = Msg::CarEvent(CarEvent { id: nerd::MSG_CAR_COLLISION, handle: Handle::Own(2), body: floats(&[0.0, 0.0, 0.0, f32::NAN, 1.0, 1.0]) });
    a.send_unsequenced(&[detach, far, coll]);
    // a ConsoleSynced that claims the server's vars: they are stripped
    a.send(&[object(true, 3, 5, 0, &console(&[(0, 1), (1, 1), (2, 1), (4, 1), (7, 3)]))]);
    let got = b.wait_for("A's ConsoleSynced", |m| is_obj(m, true, 3, Handle::Relay(5, 1), 0).is_some());
    assert_eq!(is_obj(&got, true, 3, Handle::Relay(5, 1), 0).unwrap(), console(&[(0, 1), (7, 3)]));
    b.expect_none(Duration::from_millis(400), "a hostile message", |m| match m {
        Msg::Object(o) => o.router != 3 || o.handle != Handle::Relay(5, 1),
        Msg::Car(_) | Msg::CarEvent(_) | Msg::CollObjDetachPart { .. } => true,
        _ => false,
    });
}

/// Before the ResponseAck echoes our random attempt, a peer gets only handshake replies, never
/// more bytes than it sent, and nothing of the world.
#[test]
fn the_handshake_proves_the_address_and_does_not_amplify() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    a.join();
    let mut x = Client::new(&server, 7);
    let r = x.open();
    // a ResponseAck with a wrong attempt (a spoofer cannot know ours) does nothing
    x.ack(r.own_attempt ^ 0x5555);
    x.ack(1);
    x.send(&[Msg::Ping(1), Msg::FinaliseSessionEntry]);
    // A drives meanwhile: X gets none of it
    for k in 0..10 {
        a.send_unsequenced(&[Msg::Car(Car { handle: Handle::Own(2), body: car_body(k) })]);
    }
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        if let Some((_, msgs)) = x.recv() {
            for m in msgs {
                assert!(matches!(m, Msg::OpenResponse(_) | Msg::Pong { .. }), "unexpected {m:?} before the handshake");
            }
        }
    }
    assert!(x.bytes_in <= x.bytes_out, "{} bytes in for {} out", x.bytes_in, x.bytes_out);
    // OpenConnections alone (a reflection attempt): every reply is smaller than its request
    let mut y = Client::new(&server, 8);
    for _ in 0..5 {
        let oc = y.open_connection();
        y.send(&[Msg::OpenConnection(oc)]);
    }
    std::thread::sleep(Duration::from_millis(200));
    while y.recv().is_some() {}
    assert!(y.bytes_in <= y.bytes_out, "{} bytes in for {} out", y.bytes_in, y.bytes_out);
    // the right attempt: connected
    x.ack(r.own_attempt);
    x.wait_for("TimeSync", |m| matches!(m, Msg::TimeSync { .. }));
}

#[test]
fn handshake_replies_and_peers_per_ip_are_capped() {
    let server = Server::start(Settings { handshake_per_sec: 2, max_peers_per_ip: 2, ..settings() });
    let mut x = Client::new(&server, 1);
    for _ in 0..8 {
        let oc = x.open_connection();
        x.send(&[Msg::OpenConnection(oc)]);
    }
    std::thread::sleep(Duration::from_millis(200));
    let mut replies = 0;
    while let Some((_, msgs)) = x.recv() {
        replies += msgs.iter().filter(|m| matches!(m, Msg::OpenResponse(_))).count();
    }
    assert!((1..=2).contains(&replies), "{replies} replies in a second");
    std::thread::sleep(Duration::from_millis(1100));
    // two peers from 127.0.0.1 (x, still in its handshake, and y), the third is refused
    let mut y = Client::new(&server, 2);
    y.join();
    let mut z = Client::new(&server, 3);
    assert_eq!(z.open().reason, nerd::REASON_REFUSED);
}

/// Half-open connections do not take a player's place, and go after `unsynced_timeout`.
#[test]
fn half_open_connections_time_out_and_do_not_fill_the_world() {
    let server = Server::start(Settings { max_players: 1, unsynced_timeout: Duration::from_millis(300), ..settings() });
    let mut x = Client::new(&server, 1);
    let r = x.open();
    // the world has room for one player: x is not one yet
    let mut y = Client::new(&server, 2);
    y.join();
    // x's handshake has timed out: its late ResponseAck does nothing
    std::thread::sleep(Duration::from_millis(700));
    x.ack(r.own_attempt);
    x.expect_none(Duration::from_millis(300), "TimeSync after the timeout", |m| matches!(m, Msg::TimeSync { .. }));
    // and now the world is full for a newcomer
    let mut z = Client::new(&server, 3);
    assert_eq!(z.open().reason, nerd::REASON_REFUSED);
}

/// A newcomer with a connected player's XUID: turned away while that player is active, takes
/// its place once it has been quiet. A peer cannot change its XUID.
#[test]
fn xuid_takeover_needs_a_quiet_owner() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 2535412345678901);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[object(true, 2, 4, 0, &car_vars(1.0))]);
    b.wait_for("A's object", |m| is_obj(m, true, 2, Handle::Relay(4, 1), 0).is_some());
    // A is active (pinging): the impostor is turned away after its handshake
    let mut imp = Client::new(&server, 2535412345678901);
    let r = imp.open();
    a.send(&[Msg::Ping(1)]);
    imp.ack(r.own_attempt);
    imp.expect_none(Duration::from_millis(300), "TimeSync for the impostor", |m| matches!(m, Msg::TimeSync { .. }));
    a.wait_for("A still served", |m| matches!(m, Msg::Pong { echo: 1, .. }));
    // A changing its XUID: ignored
    a.inbox.clear();
    let mut oc = a.open_connection();
    oc.user = 2;
    a.send(&[Msg::OpenConnection(oc)]);
    a.expect_none(Duration::from_millis(200), "an answer to another XUID", |m| matches!(m, Msg::OpenResponse(_)));
    // A goes quiet: now the same player from elsewhere takes over
    std::thread::sleep(Duration::from_millis(400));
    let mut a2 = Client::new(&server, 2535412345678901);
    a2.join();
    b.wait_for("delete of the old A's object", |m| is_delete(m, 2, Handle::Relay(4, 1)));
}

/// A flood: packets over the rate are dropped, and a peer that keeps it up is dropped and
/// refused for a while.
#[test]
fn flooders_are_dropped() {
    let server = Server::start(Settings { rate_packets: 50, rate_msgs: 100, abuse_secs: 1, ..settings() });
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    a.send(&[object(true, 2, 4, 0, &car_vars(1.0))]);
    b.wait_for("A's object", |m| is_obj(m, true, 2, Handle::Relay(4, 1), 0).is_some());
    // 30 cars a second for a second: all fine
    for k in 0..30 {
        a.send_unsequenced(&[Msg::Car(Car { handle: Handle::Own(2), body: car_body(k) })]);
        std::thread::sleep(Duration::from_millis(33));
        b.keepalive();
        while let Some((_, m)) = b.recv() {
            b.inbox.extend(m);
        }
    }
    let cars = b.inbox.iter().filter(|m| matches!(m, Msg::Car(_))).count();
    assert!(cars >= 28, "{cars} of 30 cars");
    // then a flood for 1.5 s
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        a.send_unsequenced(&[Msg::Ping(1)]);
        std::thread::sleep(Duration::from_micros(500));
        b.keepalive();
    }
    b.wait_for("delete of the flooder's object", |m| is_delete(m, 2, Handle::Relay(4, 1)));
    let mut a2 = Client::new(&server, 1);
    assert_eq!(a2.open().reason, nerd::REASON_BANNED);
}

/// A console that never acks our sequenced packets is dropped (it would hold them forever).
#[test]
fn a_peer_that_never_acks_is_dropped() {
    let server = Server::start(Settings { resend_after: Duration::from_millis(30), quiet_drop: Duration::from_secs(10), ..settings() });
    let mut a = Client::new(&server, 1);
    let mut c = Client::new(&server, 3);
    a.join();
    c.join();
    c.send(&[object(true, 2, 6, 0, &car_vars(1.0))]);
    a.wait_for("C's object", |m| is_obj(m, true, 2, Handle::Relay(6, 2), 0).is_some());
    // C stops acking; A's object comes to it and is never acked
    c.no_acks = true;
    a.send(&[object(true, 2, 4, 0, &car_vars(1.0))]);
    let end = Instant::now() + Duration::from_secs(3);
    let mut dropped = false;
    while Instant::now() < end && !dropped {
        c.keepalive();
        let _ = c.recv();
        if let Some((_, msgs)) = a.recv() {
            dropped = msgs.iter().any(|m| is_delete(m, 2, Handle::Relay(6, 2)));
        }
        a.keepalive();
    }
    assert!(dropped, "the non-acking peer was dropped");
}

/// A peer whose objects hold too much is dropped; a late joiner's snapshot is capped.
#[test]
fn history_and_snapshot_are_capped() {
    let server = Server::start(Settings { history_max_bits: 2000, queue_max_bits: 600, ..settings() });
    let mut a = Client::new(&server, 1);
    let mut b = Client::new(&server, 2);
    a.join();
    b.join();
    // A's first object (~330 bits) fits in the history, but not in a snapshot with another
    let p = player("Alex", &[1, 2, 3, 4]);
    a.send(&[object(true, 1, 1, 0, &p), object(true, 1, 2, 0, &p)]);
    b.wait_for("A's object 2", |m| is_obj(m, true, 1, Handle::Relay(2, 1), 0).is_some());
    let mut c = Client::new(&server, 3);
    c.join();
    c.wait_for("A's object 1 in the snapshot", |m| is_obj(m, true, 1, Handle::Relay(1, 1), 0).is_some());
    c.expect_none(Duration::from_millis(300), "A's object 2 past the snapshot cap", |m| {
        is_obj(m, true, 1, Handle::Relay(2, 1), 0).is_some()
    });
    // more objects than the history may hold: A is dropped
    for id in 3..10 {
        a.send(&[object(true, 1, id, 0, &p)]);
    }
    b.wait_for("delete of A's objects", |m| is_delete(m, 1, Handle::Relay(1, 1)));
}

/// Reads (and pings) for `d`, for the car states to come in; `other` is kept alive meanwhile.
fn pump2(c: &mut Client, other: &mut Client, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        c.keepalive();
        other.keepalive();
        if let Some((_, msgs)) = c.recv() {
            c.inbox.extend(msgs);
        }
    }
}

#[test]
fn car_states_reach_the_hooks_that_speak_them() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 0x0009_0191_ae81_7001);
    let mut b = Client::new(&server, 0x0009_0191_ae81_7002);
    a.join();
    b.join();
    // A drives: the relay keeps its car's position
    a.send_unsequenced(&[Msg::Car(Car { handle: Handle::Own(3), body: car_body(10) })]);
    b.wait_for("A's car", |m| matches!(m, Msg::Car(_)));
    // A switches its headlights off; B has not spoken yet, so it hears nothing
    let off = carstate::State { lights: carstate::LIGHTS_OFF, roof: 0, seq: 1 };
    a.raw(&carstate::up(off, a.xuid));
    pump2(&mut b, &mut a, Duration::from_millis(300));
    assert!(b.car_states.is_empty(), "a peer whose hook never spoke gets nothing");
    // B's hook speaks (the game's own lights): from then on it hears A
    b.raw(&carstate::up(carstate::State::default(), b.xuid));
    pump2(&mut b, &mut a, Duration::from_millis(1300));
    let from_a = b.car_states.iter().find(|d| d.xuid == a.xuid).expect("A's state, resent within a second");
    assert_eq!((from_a.state.lights, from_a.state.roof), (carstate::LIGHTS_OFF, 0));
    assert_eq!(from_a.position, Some([1510.0, 30.0, -2200.0]), "with A's car position");
    // a change goes at once; one in the same 50 ms is dropped
    b.car_states.clear();
    std::thread::sleep(Duration::from_millis(60));
    a.raw(&carstate::up(carstate::State { lights: carstate::LIGHTS_ON, roof: 1, seq: 2 }, a.xuid));
    a.raw(&carstate::up(carstate::State { lights: carstate::LIGHTS_GAME, roof: 0, seq: 3 }, a.xuid));
    pump2(&mut b, &mut a, Duration::from_millis(200));
    assert_eq!(b.car_states.first().map(|d| (d.state.lights, d.state.roof, d.state.seq)), Some((carstate::LIGHTS_ON, 1, 2)));
    // someone else's XUID from A's address is not taken (a sender that is no peer's address is
    // taken only from loopback, as the host's hook: the_hosts_hook_speaks_from_its_own_loopback_socket)
    b.car_states.clear();
    std::thread::sleep(Duration::from_millis(60));
    a.raw(&carstate::up(off, 0x0009_0000_0000_0042));
    pump2(&mut b, &mut a, Duration::from_millis(200));
    assert!(b.car_states.iter().all(|d| d.state.lights == carstate::LIGHTS_ON), "{:?}", b.car_states);
}

#[test]
fn the_hosts_hook_speaks_from_its_own_loopback_socket() {
    let server = Server::start(settings());
    // the host's game is the loopback peer A; its hook has a socket of its own
    let mut a = Client::new(&server, 0x0009_0191_ae81_7003);
    let mut b = Client::new(&server, 0x0009_0191_ae81_7004);
    a.join();
    b.join();
    let hook = UdpSocket::bind("127.0.0.1:0").unwrap();
    hook.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    hook.send_to(&carstate::up(carstate::State::default(), a.xuid), server.addr).unwrap();
    // B speaks and switches its lights on: the host's hook socket hears it, A's game socket does not
    b.raw(&carstate::up(carstate::State { lights: carstate::LIGHTS_ON, roof: 0, seq: 1 }, b.xuid));
    let mut heard = None;
    let end = Instant::now() + Duration::from_secs(2);
    let mut buf = [0u8; 256];
    while heard.is_none() && Instant::now() < end {
        a.keepalive();
        if let Ok((n, _)) = hook.recv_from(&mut buf) {
            heard = carstate::parse_down(&buf[..n]);
        }
    }
    let d = heard.expect("the host's hook hears B");
    assert_eq!((d.xuid, d.state.lights), (b.xuid, carstate::LIGHTS_ON));
    pump2(&mut a, &mut b, Duration::from_millis(300));
    assert!(a.car_states.is_empty(), "the host's game socket never gets one");
}

