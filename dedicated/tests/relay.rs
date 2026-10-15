//! The relay on a loopback port, with fake consoles that speak NerdCore.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use dedicated_server::{
    bits::{Bits, Reader},
    nerd::{self, Acks, Car, CarEvent, Handle, Header, Msg, ObjectMsg, OpenConnection, OpenResponse},
    relay::{Relay, Settings},
};

struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start(settings: Settings) -> Server {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut relay = Relay::new(sock, settings);
            relay.run(&s).unwrap();
        });
        Server { addr, stop, thread: Some(thread) }
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
        }
    }

    fn send(&mut self, msgs: &[Msg]) {
        let bits: Vec<Bits> = msgs.iter().map(Msg::write).collect();
        let (ack, ackbits) = self.acks.header();
        let id = self.next_id;
        self.next_id += 1;
        self.sock.send_to(&nerd::packet(&bits, id, ack, ackbits), self.server).unwrap();
    }

    fn send_unsequenced(&mut self, msgs: &[Msg]) {
        let bits: Vec<Bits> = msgs.iter().map(Msg::write).collect();
        let (ack, ackbits) = self.acks.header();
        self.sock.send_to(&nerd::packet(&bits, nerd::NO_SEQ, ack, ackbits), self.server).unwrap();
    }

    /// One packet's messages (None on timeout). Sequenced packets are acked right away, as the
    /// game does, so the relay does not resend them.
    fn recv(&mut self) -> Option<(Header, Vec<Msg>)> {
        let mut buf = [0u8; 65536];
        let (n, _) = self.sock.recv_from(&mut buf).ok()?;
        let data = &buf[..n];
        let h = Header::parse(data).expect("header");
        assert_eq!(h.size as usize, n, "size field");
        let mut r = Reader::at(data, nerd::HEADER_LEN * 8);
        let mut msgs = Vec::new();
        for _ in 0..h.count {
            msgs.push(Msg::read(&mut r).expect("the relay sends only known messages"));
        }
        if h.id != nerd::NO_SEQ {
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
            if self.last_ping.elapsed() > Duration::from_millis(200) {
                self.last_ping = Instant::now();
                self.send_unsequenced(&[Msg::Ping(1)]);
            }
            if let Some((_, msgs)) = self.recv() {
                self.inbox.extend(msgs);
            }
        }
        panic!("client {}: no {what}", self.xuid);
    }

    /// Nothing matching for `d` (the client keeps reading and acking).
    fn expect_none(&mut self, d: Duration, what: &str, pred: impl Fn(&Msg) -> bool) {
        assert!(!self.inbox.iter().any(|m| pred(m)), "client {}: unexpected {what}", self.xuid);
        let end = Instant::now() + d;
        while Instant::now() < end {
            if let Some((_, msgs)) = self.recv() {
                assert!(!msgs.iter().any(|m| pred(m)), "client {}: unexpected {what}", self.xuid);
            }
        }
    }

    fn open(&mut self) -> OpenResponse {
        let oc = OpenConnection {
            version: nerd::VERSION.to_vec(),
            game: nerd::GAME.to_vec(),
            flags: 2,
            user: self.xuid,
            a: 0x12,
            b: 4,
            attempt: self.attempt,
        };
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

    /// The whole handshake, up to the SessionHost.
    fn join(&mut self) {
        let r = self.open();
        assert_eq!(r.reason, nerd::REASON_OK);
        self.send(&[Msg::OpenResponse(OpenResponse {
            flags: 2,
            reason: nerd::REASON_RESPONSE_ACK,
            attempt: r.own_attempt,
            own_attempt: self.attempt,
        })]);
        self.wait_for("TimeSync", |m| matches!(m, Msg::TimeSync { .. }));
        // SyncDone shares the TimeSync's packet, so it may have been read with it: accept either
        // order by checking the SessionHost, which comes after both
        self.wait_for("SessionHost", |m| {
            matches!(m, Msg::Object(o) if o.create && o.router == 6 && o.handle == Handle::Own(0) && o.ty == 0 && o.seq == 0)
        });
    }
}

fn payload(n: usize, seed: u64) -> Bits {
    let mut b = Bits::new();
    for i in 0..n {
        b.push((seed >> (i % 61)) ^ (i as u64 / 3), 1);
    }
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

fn settings() -> Settings {
    Settings {
        max_players: 3,
        quiet_drop: Duration::from_millis(1500),
        keepalive: Duration::from_millis(100),
        resend_after: Duration::from_millis(200),
        quiet: true,
        ..Default::default()
    }
}

#[test]
fn handshake_sends_timesync_and_syncdone() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1);
    let r = a.open();
    assert_eq!(r.reason, nerd::REASON_OK);
    assert_eq!(r.own_attempt, 1);
    a.send(&[Msg::OpenResponse(OpenResponse { flags: 2, reason: 6, attempt: 1, own_attempt: a.attempt })]);
    let (h, msgs) = loop {
        let (h, msgs) = a.recv().expect("sync packet");
        if msgs.iter().any(|m| matches!(m, Msg::TimeSync { .. })) {
            break (h, msgs);
        }
    };
    assert_eq!(h.id, nerd::NO_SEQ, "TimeSync and SyncDone go unsequenced, as relay.py sends them");
    assert!(matches!(msgs[0], Msg::TimeSync { index: 0, .. }));
    assert_eq!(msgs[1], Msg::SyncDone(1));
    // the SessionHost comes sequenced
    let (h, msgs) = a.recv().expect("world");
    assert_ne!(h.id, nerd::NO_SEQ);
    assert!(is_obj(&msgs[0], true, 6, Handle::Own(0), 0).is_some());
    // ping -> pong with our time echoed
    a.send(&[Msg::Ping(777)]);
    a.wait_for("Pong", |m| matches!(m, Msg::Pong { echo: 777, .. }));
    // and the relay pings us
    a.wait_for("keepalive Ping", |m| matches!(m, Msg::Ping(_)));
}

#[test]
fn relay_snapshot_drops_and_limits() {
    let server = Server::start(settings());
    let mut a = Client::new(&server, 1001);
    let mut b = Client::new(&server, 1002);
    a.join();
    b.join();
    let (a_idx, b_idx) = (1u8, 2u8); // round-robin from 1

    // A's objects: ConsoleSynced, and an object with a Create and an update
    let p3 = payload(20, 3);
    let (pc, pu) = (payload(300, 11), payload(45, 12));
    a.send(&[object(true, 3, 5, 0, &p3), object(true, 1, 2, 10, &pc), object(false, 1, 2, 11, &pu)]);
    let car_body = payload(nerd::CAR_BODY_BITS, 99);
    a.send_unsequenced(&[Msg::Car(Car { handle: Handle::Own(2), body: car_body.clone() })]);

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
            assert_eq!(c.body, car_body);
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

    // A comes back from another address (same XUID): the old A goes, B and C delete its objects
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
    let a2_idx = 4u8; // the next index, not A's old one

    // A2 makes an object and closes: B gets the delete
    let p = payload(8, 1);
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
    a.send(&[object(true, 1, 3, 0, &payload(10, 1))]);
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
    let car_body = payload(nerd::CAR_BODY_BITS, 7);
    let repair = Msg::CarEvent(CarEvent { id: nerd::MSG_REPAIR_DAMAGE, handle: Handle::Own(2), body: Bits::new() });
    let car = Msg::Car(Car { handle: Handle::Own(2), body: car_body.clone() });
    a.send_unsequenced(&[repair, car.clone()]);
    match b.wait_for("A's car after the repair", |m| matches!(m, Msg::Car(_))) {
        Msg::Car(c) => assert_eq!((c.handle, c.body), (Handle::Relay(2, a_idx), car_body.clone())),
        _ => unreachable!(),
    }
    b.wait_for("A's repair", |m| {
        matches!(m, Msg::CarEvent(e) if e.id == nerd::MSG_REPAIR_DAMAGE && e.handle == Handle::Relay(2, a_idx))
    });

    // traffic first, then a glass break and the car: all read
    let traffic = Msg::Traffic { id: nerd::MSG_SERVER_TRAFFIC_MESSAGE, body: payload(72, 3) };
    let glass_body = payload(352, 4);
    let glass = Msg::CarEvent(CarEvent { id: nerd::MSG_BREAK_GLASS, handle: Handle::Own(2), body: glass_body.clone() });
    a.send_unsequenced(&[traffic, glass, car]);
    b.wait_for("A's glass", |m| {
        matches!(m, Msg::CarEvent(e) if e.id == nerd::MSG_BREAK_GLASS && e.handle == Handle::Relay(2, a_idx) && e.body == glass_body)
    });
    b.wait_for("A's second car", |m| matches!(m, Msg::Car(_)));
    b.expect_none(Duration::from_millis(300), "relayed traffic", |m| matches!(m, Msg::Traffic { .. }));
}
