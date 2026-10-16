//! The game server's UDP side: a NerdCore relay (nerd.rs has the formats). Port of fh2-mods
//! `tools/online/server/relay.py`, with the fixes listed in PLANO_FREEROAM.md.
//!
//! Each console (peer) gets the server's side of a connection: OpenConnection is answered Ok,
//! the console's ResponseAck gets TimeSync and ConnectionSyncDone, Pings get Pongs, sequenced
//! packets are acked, and the server pings every half second (consoles drop a server that is
//! quiet for 10 s).
//!
//! Objects: the server owns one object itself, the lobby's SessionHost (router 6, type 0), which
//! consoles wait for before they finish joining (stateflow network_matchmaking.xml
//! wait_for_session_host). Every object a console creates, updates or deletes on a forwarded
//! router is forwarded to the other consoles with the relay handle (its owner's peer index), and
//! kept in a history that late joiners get as a snapshot. Cars (CarPredictionFull, ~30 a second,
//! unsequenced) are forwarded the same way and not kept.
//!
//! History (fix: relay.py kept every message forever and replayed all of them). The receiver
//! applies object updates strictly in seq order (1417B1E70: link+1 mod 255; newer ones wait for
//! the gap, older ones are dropped), and a Create sets the link to its own seq. Payloads are
//! (var index, data) lists read to the end (1417B2510), so updates can be concatenated: a
//! later entry for a var overwrites the earlier one. So each object keeps:
//!   - `base`: the Create's payload with the oldest updates appended, under the seq of the last
//!     one appended; a late joiner gets it as one Create with that seq, so the owner's next live
//!     update (seq+1) applies right after;
//!   - `tail`: the updates that no longer fit in base (u14 payload size), in seq order, replayed
//!     verbatim after the Create (at most TAIL_MAX of them and TAIL_MAX_BITS);
//!   - `pending`: updates that came ahead of a gap, until the owner's resend fills it.
//!
//! When the tail is full its oldest update is folded into base if it fits, else dropped: a late
//! joiner then misses that update's vars until they change again (logged once per object).
//!
//! Duplicates (fix: relay.py's dedup set grew forever): consoles resend unacked messages, and
//! a message is new only if its seq is newer than the object's (or, ahead of a gap, not pending
//! yet). That state is the history itself, so it is bounded with it.
//!
//! Reliability (fix): consoles drop out-of-order updates and wait on gaps, so a lost packet of
//! object messages would stall that object for good. Our sequenced packets are kept until the
//! console acks them (the ack and ack bits of its packet headers) and resent in a new packet
//! otherwise; a console that never acks is dropped.
//!
//! Peers: by address, plus the XUID from OpenConnection. A console that starts over (an
//! OpenConnection in packet 0), goes quiet for 15 s or closes the connection is dropped, and the
//! others get an ObjectDelete for each of its objects. Only an OpenConnection makes a peer: other
//! packets from unknown addresses are ignored. Peer indices are 1..=254, handed out round-robin
//! so a freed index is reused as late as possible (receivers remember deletes per handle,
//! 1417AB560, and drop a Create with an older seq for it).
//!
//! The group's GPS (SetGroupSatNavEvent, 52): a console asks the server to set the group's
//! destination, and consoles do not act on the message themselves (their ILobbyEvents slot for it
//! is `return false`), so it is not relayed: the server sets its SessionHost's GroupSatNavNode
//! var instead, with an ObjectUpdate to every console, and a late joiner gets it in the
//! SessionHost's Create.
//!
//! Game messages carry no length, so a packet is read only as far as its messages are known; the
//! rest is dropped (the packet is still acked) and each unknown message id is logged once.
//!
//! Security. Everything a peer sends ends up in the other players' games, whose deserializers
//! trust their input (MsgCollObjDetachPart's reader copies up to 2047 bytes into a 1024-byte
//! stack buffer; NetworkName's reader keeps a count it did not read). So:
//!   - Forwarded: only cars (15), car events (20, 21, 35) and ObjectCreate / Update / Delete on
//!     routers 1, 2 and 3 (nerd::forwarded_router), each written again from what was read:
//!     payloads var by var (nerd::sanitize_payload: indices, duplicates, widths, finite floats in
//!     range, NetworkName length, ConsoleSynced's server vars stripped, a size cap per router),
//!     car positions and event floats checked (nerd::car_ok, car_event_ok). Everything else is
//!     read only to get past it.
//!   - Handshake: our OpenConnectionResponse carries a random attempt per peer, which the
//!     console's ResponseAck must echo before it counts as connected (a spoofed source address
//!     never sees it). Until then the peer gets only handshake replies, never more bytes than
//!     it sent, is not a player (max_players), and is dropped after `unsynced_timeout`.
//!     Handshake replies are capped per IP per second, peers per IP, and peers waiting.
//!   - XUIDs: a peer cannot change its XUID; a newcomer with the XUID of a connected player
//!     replaces it only once it has proven its address and the old one has been quiet for
//!     `takeover_quiet` (otherwise it is turned away).
//!   - Limits per peer: token buckets on packets, messages and bytes (sustained excess drops the
//!     peer), its history (`history_max_bits`), what waits to be sent to it (snapshot cap, paced).
//!   - Access: commands on stdin (main.rs; never the network): `kick <xuid> [reason]` drops the
//!     player, who may come back at once (a player whose car database changed is kept out by
//!     the server, which no longer sends them to a world); `allow <ip>` and `allowonly 1|0`
//!     restrict who may connect (loopback always may).
//!   - A packet that panics the handling is dropped and logged once per address; log lines are
//!     stripped of control characters, and refusals are logged at most once per IP per minute.

use std::{
    collections::{
        VecDeque,
        hash_map::RandomState,
    },
    hash::{BuildHasher, Hasher},
    io,
    net::{IpAddr, SocketAddr, UdpSocket},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, Sender, TryRecvError},
    },
    time::{Duration, Instant},
};

use crate::{
    bits::{Bits, Reader},
    nerd::{self, Car, CarEvent, Handle, Header, Msg, ObjectDelete, ObjectMsg, OpenResponse, ReadError},
    net,
};

/// The attempt value the server gave before attempts were random (kept for reference: a
/// ResponseAck echoing it is refused unless it happens to be the peer's own).
pub const OWN_ATTEMPT: u32 = 1;
/// The XUID Azimuth gives every player without a ForzaInfinity account (0x0009000000000000).
pub const AZIMUTH_XUID: u64 = 0x0009_0000_0000_0000;
/// The mirror's ghost player (--mirror).
pub const MIRROR_INDEX: u8 = 250;
/// Messages per packet, and body bytes per packet, for what we send (the game's own limit is
/// not known; this stays under a 1280-byte IPv6 MTU).
const PACKET_MSGS_MAX: usize = 8;
const PACKET_BODY_MAX: usize = 1200 - nerd::HEADER_LEN;
/// History bounds, per object and per peer.
const TAIL_MAX: usize = 32;
const TAIL_MAX_BITS: usize = 64 * 1024;
const PENDING_MAX: usize = 16;
const OBJECTS_MAX: usize = 256;
const ORPHANS_MAX: usize = 32;
/// Our sequenced packets waiting for an ack, per peer, and how often one is resent.
const IN_FLIGHT_MAX: usize = 256;
const RESEND_TRIES_MAX: u8 = 20;
/// The mirror's delayed cars.
const MIRROR_QUEUE_MAX: usize = 8192;
/// Bounded tables: bans, allowed IPs, handshake counters, log throttles, panic logs.
const BANS_MAX: usize = 256;
const ALLOW_MAX: usize = 64;
const IP_TABLE_MAX: usize = 1024;
const PANIC_LOGGED_MAX: usize = 64;
/// An `allow` lasts this long unless it is given again.
const ALLOW_TTL: Duration = Duration::from_secs(600);
/// A refusal is logged once per IP in this long.
const REFUSAL_LOG_EVERY: Duration = Duration::from_secs(60);
/// The longest stdin command line (main.rs cuts there) and log line.
pub const COMMAND_LINE_MAX: usize = 256;
const LOG_LINE_MAX: usize = 512;
/// How often what waits in a peer's queue goes out, and how much each time.
const PACE_EVERY: Duration = Duration::from_millis(20);
const PACE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct Settings {
    pub max_players: u8,
    /// Dropped after this long without a packet.
    pub quiet_drop: Duration,
    /// Our ping to every synced peer.
    pub keepalive: Duration,
    /// Unacked sequenced packets are resent after this long (or twice the round trip, if longer).
    pub resend_after: Duration,
    /// Testing alone: each console also gets its own objects back as another player's (peer
    /// 250), and its car this much later.
    pub mirror: Option<Duration>,
    /// Protocol lines (every message), not just joins and drops.
    pub verbose: bool,
    /// Nothing on the console (tests).
    pub quiet: bool,
    /// A peer that has not finished the handshake is dropped after this long.
    pub unsynced_timeout: Duration,
    /// Peers (connected or not) from one IP.
    pub max_peers_per_ip: usize,
    /// Peers in the handshake at once.
    pub max_unsynced: usize,
    /// Handshake replies (OpenConnectionResponse, refusals) per IP per second.
    pub handshake_per_sec: u32,
    /// A newcomer takes over a connected XUID only if the old peer has been quiet this long.
    pub takeover_quiet: Duration,
    /// Per peer, per second: packets, messages and bytes (bursts up to twice that). A console
    /// sends ~30 car packets a second plus a few others, ~100 bytes each.
    pub rate_packets: u32,
    pub rate_msgs: u32,
    pub rate_bytes: u32,
    /// Seconds in a row with packets over the rate before the peer is dropped.
    pub abuse_secs: u32,
    /// What a peer's objects may hold in the history, in bits.
    pub history_max_bits: usize,
    /// What may wait to be sent to one peer (the snapshot for a late joiner, and what comes
    /// while it goes out), in bits.
    pub queue_max_bits: usize,
    /// How long a peer dropped for abuse (its IP) is refused.
    pub abuse_ban: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_players: 8,
            quiet_drop: Duration::from_secs(15),
            keepalive: Duration::from_millis(500),
            resend_after: Duration::from_millis(500),
            mirror: None,
            verbose: false,
            quiet: false,
            unsynced_timeout: Duration::from_secs(3),
            max_peers_per_ip: 2,
            max_unsynced: 32,
            handshake_per_sec: 4,
            takeover_quiet: Duration::from_secs(3),
            rate_packets: 300,
            rate_msgs: 1000,
            rate_bytes: 256 * 1024,
            abuse_secs: 5,
            history_max_bits: 2 * 1024 * 1024,
            queue_max_bits: 8 * 1024 * 1024,
            abuse_ban: Duration::from_secs(60),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// History of one object

#[derive(Debug, Clone)]
struct Obj {
    ty: u8,
    /// The seq a late joiner's Create gets: the last update folded into `base`.
    base_seq: u8,
    base: Bits,
    tail: VecDeque<(u8, Bits)>,
    tail_bits: usize,
    /// Newest seq applied in order (base_seq or the last of the tail).
    last: u8,
    pending: Vec<(u8, Bits)>,
    /// An update was dropped from the history (logged once).
    lossy: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Fresh {
    /// New: forward it.
    New,
    /// Seen already (a resend): drop it.
    Duplicate,
}

impl Obj {
    fn new(ty: u8, seq: u8, payload: Bits) -> Obj {
        Obj {
            ty,
            base_seq: seq,
            base: payload,
            tail: VecDeque::new(),
            tail_bits: 0,
            last: seq,
            pending: Vec::new(),
            lossy: false,
        }
    }

    /// An update from the owner. Returns whether it is new, and true in the second value when
    /// the history had to drop an update.
    fn update(&mut self, seq: u8, payload: Bits) -> (Fresh, bool) {
        if seq == nerd::seq_next(self.last) {
            let mut dropped = self.apply(seq, payload);
            // the gap is filled: what waited behind it follows
            while let Some(i) = self.pending.iter().position(|(s, _)| *s == nerd::seq_next(self.last)) {
                let (s, p) = self.pending.swap_remove(i);
                dropped |= self.apply(s, p);
            }
            (Fresh::New, dropped)
        } else if nerd::seq_newer(seq, self.last) {
            if self.pending.iter().any(|(s, _)| *s == seq) {
                return (Fresh::Duplicate, false);
            }
            let mut dropped = false;
            if self.pending.len() >= PENDING_MAX {
                // the one furthest ahead goes (only a console far out of order gets here)
                let ahead = |s: u8| (s as u16 + 255 - self.last as u16) % 255;
                let far = (0..self.pending.len()).max_by_key(|&i| ahead(self.pending[i].0)).unwrap();
                self.pending.swap_remove(far);
                dropped = true;
            }
            self.pending.push((seq, payload));
            (Fresh::New, dropped)
        } else {
            (Fresh::Duplicate, false)
        }
    }

    /// The in-order update `seq`: into base while it fits and nothing waits in the tail, else
    /// to the tail. Returns true if an update had to be dropped.
    fn apply(&mut self, seq: u8, payload: Bits) -> bool {
        self.last = seq;
        if self.tail.is_empty() && self.base.len() + payload.len() <= nerd::MAX_PAYLOAD_BITS {
            self.base.append(&payload);
            self.base_seq = seq;
            return false;
        }
        self.tail_bits += payload.len();
        self.tail.push_back((seq, payload));
        let mut dropped = false;
        while self.tail.len() > TAIL_MAX || self.tail_bits > TAIL_MAX_BITS {
            let (s, p) = self.tail.pop_front().unwrap();
            self.tail_bits -= p.len();
            if self.base.len() + p.len() <= nerd::MAX_PAYLOAD_BITS {
                self.base.append(&p);
            } else {
                dropped = true;
            }
            // either way the joiner's Create now stands for everything up to s
            self.base_seq = s;
        }
        if dropped && !self.lossy {
            self.lossy = true;
            return true;
        }
        false
    }

    /// The messages that bring a late joiner up to date, with `handle` (the relay form).
    fn snapshot(&self, router: u8, handle: Handle, out: &mut Vec<Bits>) {
        out.push(
            Msg::Object(ObjectMsg { create: true, router, handle, ty: self.ty, seq: self.base_seq, payload: self.base.clone() })
                .write(),
        );
        let upd = |seq: u8, payload: &Bits| {
            Msg::Object(ObjectMsg { create: false, router, handle, ty: 0, seq, payload: payload.clone() }).write()
        };
        for (s, p) in &self.tail {
            out.push(upd(*s, p));
        }
        for (s, p) in &self.pending {
            out.push(upd(*s, p));
        }
    }
}


// ---------------------------------------------------------------------------------------------
// Peers

struct InFlight {
    pid: u16,
    msgs: Vec<Bits>,
    sent: Instant,
    tries: u8,
}

/// Token buckets: packets, messages and bytes a second, bursts up to two seconds' worth.
#[derive(Debug, Clone)]
struct Bucket {
    packets: f64,
    msgs: f64,
    bytes: f64,
    last: Instant,
}

impl Bucket {
    fn new(s: &Settings, now: Instant) -> Bucket {
        Bucket { packets: 2.0 * s.rate_packets as f64, msgs: 2.0 * s.rate_msgs as f64, bytes: 2.0 * s.rate_bytes as f64, last: now }
    }

    /// Takes one packet of `msgs` messages and `bytes` bytes; false (nothing taken) when over.
    fn take(&mut self, s: &Settings, now: Instant, msgs: u32, bytes: usize) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        let fill = |v: f64, rate: u32| (v + rate as f64 * dt).min(2.0 * rate as f64);
        self.packets = fill(self.packets, s.rate_packets);
        self.msgs = fill(self.msgs, s.rate_msgs);
        self.bytes = fill(self.bytes, s.rate_bytes);
        let (m, b) = (msgs as f64, bytes as f64);
        if self.packets < 1.0 || self.msgs < m || self.bytes < b {
            return false;
        }
        self.packets -= 1.0;
        self.msgs -= m;
        self.bytes -= b;
        true
    }
}

struct Peer {
    addr: SocketAddr,
    index: u8,
    xuid: u64,
    /// Our attempt in its OpenConnectionResponse: random, echoed by its ResponseAck.
    own_attempt: u32,
    acks: nerd::Acks,
    /// Our next sequenced packet id: 1, 2, ... skipping 0xFFFE (the "no ack" value) and 0xFFFF
    /// (unsequenced).
    next_pid: u16,
    in_flight: VecDeque<InFlight>,
    in_flight_overflow: bool,
    synced: bool,
    sync_index: u16,
    /// Until synced: bytes from it and to it (we never send more than we got).
    rx_bytes: usize,
    tx_bytes: usize,
    /// Its ConsoleSynced object (router 3), and the seq of our last server update to it.
    console_id: Option<u32>,
    console_seq: Option<u8>,
    /// FinaliseSessionEntry came (answered once the ConsoleSynced object is known).
    finalise_asked: bool,
    finalised: bool,
    /// What it owns: (router, id) -> history.
    objects: Vec<((u8, u32), Obj)>,
    objects_full: bool,
    /// Updates for objects whose Create has not come yet.
    orphans: VecDeque<ObjectMsg>,
    last_seen: Instant,
    joined: Instant,
    /// Round trip from our pings, ms.
    rtt_ms: Option<u32>,
    cars: u64,
    bucket: Option<Bucket>,
    /// Packets over the rate: since when, in a row, and the last one.
    over_since: Option<Instant>,
    over_last: Option<Instant>,
    /// Sequenced messages waiting to go out paced (the snapshot, and what came after it).
    queue: VecDeque<Bits>,
    queue_bits: usize,
    /// It is to be dropped, and why (set where dropping at once would move other peers).
    doom: Option<String>,
}

impl Peer {
    fn new(addr: SocketAddr, index: u8, xuid: u64, now: Instant) -> Peer {
        Peer {
            addr,
            index,
            xuid,
            own_attempt: random_attempt(addr),
            acks: nerd::Acks::default(),
            next_pid: 1,
            in_flight: VecDeque::new(),
            in_flight_overflow: false,
            synced: false,
            sync_index: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            console_id: None,
            console_seq: None,
            finalise_asked: false,
            finalised: false,
            objects: Vec::new(),
            objects_full: false,
            orphans: VecDeque::new(),
            last_seen: now,
            joined: now,
            rtt_ms: None,
            cars: 0,
            bucket: None,
            over_since: None,
            over_last: None,
            queue: VecDeque::new(),
            queue_bits: 0,
            doom: None,
        }
    }

    fn take_pid(&mut self) -> u16 {
        let pid = self.next_pid;
        self.next_pid = match pid.wrapping_add(1) {
            0xFFFE | 0xFFFF => 0,
            n => n,
        };
        pid
    }

    fn object(&mut self, key: (u8, u32)) -> Option<&mut Obj> {
        self.objects.iter_mut().find(|(k, _)| *k == key).map(|(_, o)| o)
    }

    /// What its history holds, in bits.
    fn history_bits(&self) -> usize {
        let objs: usize = self
            .objects
            .iter()
            .map(|(_, o)| o.base.len() + o.tail_bits + o.pending.iter().map(|(_, p)| p.len()).sum::<usize>())
            .sum();
        objs + self.orphans.iter().map(|o| o.payload.len()).sum::<usize>()
    }

    fn ip(&self) -> IpAddr {
        norm_ip(self.addr.ip())
    }
}

/// An IPv4-mapped IPv6 address as the IPv4 one (the dual-stack socket gives those).
pub fn norm_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// A random, nonzero attempt (std only: RandomState's per-process random keys, a counter, the
/// time and the address).
fn random_attempt(addr: SocketAddr) -> u32 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.write(addr.to_string().as_bytes());
    let v = h.finish();
    match (v ^ (v >> 32)) as u32 {
        0 | OWN_ATTEMPT => 0x5EED_0001,
        n => n,
    }
}

/// A line for the console: control characters out, length bounded.
pub fn sanitize_line(s: &str) -> String {
    s.chars().take(LOG_LINE_MAX).map(|c| if c.is_control() { '?' } else { c }).collect()
}

fn shared_xuid(xuid: u64) -> bool {
    xuid == 0 || xuid == AZIMUTH_XUID
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BanKey {
    Ip(IpAddr),
}

/// What happened to the peer list (tests and the console).
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub peers: usize,
    pub synced: usize,
}

pub struct Relay {
    sock: UdpSocket,
    settings: Settings,
    t0: Instant,
    peers: Vec<Peer>,
    /// Where the round-robin search for a free index starts.
    next_index: u8,
    unknown_logged: [bool; 128],
    other_logged: Vec<String>,
    last_keepalive: Instant,
    last_resend_scan: Instant,
    last_pace: Instant,
    /// (due, peer address, message): the mirror's delayed cars.
    mirror_queue: VecDeque<(Instant, SocketAddr, Bits)>,
    /// Our SessionHost: the seq of its last update (its Create is seq 0) and its
    /// GroupSatNavNode var (0: no group destination).
    host_seq: u8,
    group_sat_nav: i32,
    /// Stdin commands (main.rs).
    commands: Option<Receiver<String>>,
    bans: Vec<(BanKey, Instant)>,
    allow: Vec<(IpAddr, Instant)>,
    allow_only: bool,
    /// Per IP: the start of its current second and the handshake replies in it.
    handshakes: Vec<(IpAddr, Instant, u32)>,
    refusals_logged: Vec<(IpAddr, Instant)>,
    panics_logged: Vec<SocketAddr>,
    /// The largest payload seen per router (verbose log when it grows).
    payload_seen: [usize; 16],
}

impl Relay {
    pub fn new(sock: UdpSocket, settings: Settings) -> Relay {
        let now = Instant::now();
        Relay {
            sock,
            settings,
            t0: now,
            peers: Vec::new(),
            next_index: 1,
            unknown_logged: [false; 128],
            other_logged: Vec::new(),
            last_keepalive: now,
            last_resend_scan: now,
            last_pace: now,
            mirror_queue: VecDeque::new(),
            host_seq: 0,
            group_sat_nav: 0,
            commands: None,
            bans: Vec::new(),
            allow: Vec::new(),
            allow_only: false,
            handshakes: Vec::new(),
            refusals_logged: Vec::new(),
            panics_logged: Vec::new(),
            payload_seen: [0; 16],
        }
    }

    /// Where commands come from (stdin lines, main.rs).
    pub fn set_commands(&mut self, rx: Receiver<String>) {
        self.commands = Some(rx);
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    pub fn counts(&self) -> Counts {
        Counts { peers: self.peers.len(), synced: self.peers.iter().filter(|p| p.synced).count() }
    }

    fn log(&self, line: &str) {
        if !self.settings.quiet {
            let s = self.t0.elapsed().as_secs();
            println!("[{:02}:{:02}:{:02}] {}", s / 3600, s / 60 % 60, s % 60, sanitize_line(line));
        }
    }

    fn debug(&self, line: &str) {
        if self.settings.verbose {
            self.log(line);
        }
    }

    /// Logs a line the first time only (bounded: a few distinct texts).
    fn log_once(&mut self, line: String) {
        if self.other_logged.len() < 256 && !self.other_logged.contains(&line) {
            self.log(&line);
            self.other_logged.push(line);
        }
    }

    /// A refusal: logged at most once per IP per REFUSAL_LOG_EVERY.
    fn log_refusal(&mut self, ip: IpAddr, line: &str) {
        let now = Instant::now();
        self.refusals_logged.retain(|(_, t)| now.duration_since(*t) < REFUSAL_LOG_EVERY);
        if self.refusals_logged.iter().any(|(i, _)| *i == ip) {
            return;
        }
        if self.refusals_logged.len() >= IP_TABLE_MAX {
            self.refusals_logged.remove(0);
        }
        self.refusals_logged.push((ip, now));
        self.log(line);
    }

    /// Our clock for Ping/Pong/TimeSync, ms since start (never 0).
    fn now_ms(&self) -> u32 {
        (self.t0.elapsed().as_millis() as u32).wrapping_add(1)
    }

    /// Runs until `stop` is set: packets in, and the timers (keepalive, resends, quiet peers,
    /// the mirror, paced queues, commands) between them.
    pub fn run(&mut self, stop: &AtomicBool) -> io::Result<()> {
        self.sock.set_read_timeout(Some(Duration::from_millis(10)))?;
        let mut buf = vec![0u8; 65536];
        while !stop.load(Ordering::Relaxed) {
            match self.sock.recv_from(&mut buf) {
                Ok((n, addr)) => self.handle(&buf[..n], addr),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                // Windows: an earlier send reached a closed port (a console left)
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
                Err(e) => {
                    self.log_once(format!("recv: {e}"));
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            self.poll_commands();
            self.tick(Instant::now());
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Commands (stdin)

    fn poll_commands(&mut self) {
        let Some(rx) = &self.commands else { return };
        let mut lines = Vec::new();
        for _ in 0..64 {
            match rx.try_recv() {
                Ok(l) => lines.push(l),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.commands = None;
                    break;
                }
            }
        }
        for l in lines {
            self.command(&l);
        }
    }

    /// One command line: `kick <xuid> [reason]`, `allow <ip>`, `allowonly 1|0`. Anything else
    /// is ignored.
    pub fn command(&mut self, line: &str) {
        let line: String = line.chars().take(COMMAND_LINE_MAX).collect();
        let mut words = line.split_whitespace();
        match words.next() {
            Some("kick") => {
                let Some(xuid) = words.next().and_then(parse_xuid) else {
                    self.log("comando kick: XUID inválido");
                    return;
                };
                let reason: String = words.collect::<Vec<_>>().join(" ").chars().take(64).collect();
                let reason = if reason.is_empty() { "pelo anfitrião".to_string() } else { reason };
                self.kick(xuid, &reason);
            }
            Some("allow") => {
                let Some(ip) = words.next().and_then(|w| w.parse::<IpAddr>().ok()) else {
                    self.log("comando allow: IP inválido");
                    return;
                };
                self.allow_ip(norm_ip(ip), Instant::now());
            }
            Some("allowonly") => match words.next() {
                Some("1" | "on" | "true") => {
                    self.allow_only = true;
                    self.log("só IPs liberados podem entrar");
                }
                Some("0" | "off" | "false") => {
                    self.allow_only = false;
                    self.log("qualquer IP pode entrar");
                }
                _ => self.log("comando allowonly: use 1 ou 0"),
            },
            _ => self.debug("comando desconhecido ignorado"),
        }
    }

    fn allow_ip(&mut self, ip: IpAddr, now: Instant) {
        self.allow.retain(|(_, until)| *until > now);
        let until = now + ALLOW_TTL;
        if let Some(e) = self.allow.iter_mut().find(|(i, _)| *i == ip) {
            e.1 = until;
            return;
        }
        if self.allow.len() >= ALLOW_MAX {
            // the one that would expire first goes
            if let Some(k) = (0..self.allow.len()).min_by_key(|&k| self.allow[k].1) {
                self.allow.swap_remove(k);
            }
        }
        self.allow.push((ip, until));
        self.debug(&format!("IP liberado: {ip}"));
    }

    fn allowed(&self, ip: IpAddr, now: Instant) -> bool {
        ip.is_loopback() || !self.allow_only || self.allow.iter().any(|(i, until)| *i == ip && *until > now)
    }

    fn ban(&mut self, key: BanKey, dur: Duration) {
        let now = Instant::now();
        self.bans.retain(|(_, until)| *until > now);
        let until = now + dur;
        if let Some(b) = self.bans.iter_mut().find(|(k, _)| *k == key) {
            b.1 = b.1.max(until);
            return;
        }
        if self.bans.len() >= BANS_MAX
            && let Some(k) = (0..self.bans.len()).min_by_key(|&k| self.bans[k].1)
        {
            self.bans.swap_remove(k);
        }
        self.bans.push((key, until));
    }

    fn banned(&self, ip: IpAddr, now: Instant) -> bool {
        self.bans.iter().any(|(BanKey::Ip(i), until)| *until > now && *i == ip)
    }

    /// Drops every peer with `xuid` (the others delete its objects). It is not refused: leaving
    /// and coming back is fine, and a player whose car database changed is kept out by the
    /// server, which sends them to no world.
    fn kick(&mut self, xuid: u64, reason: &str) {
        let mut n = 0;
        while let Some(i) = self.peers.iter().position(|p| p.xuid == xuid) {
            let index = self.peers[i].index;
            self.drop_peer(i, &format!("expulso: {reason}"));
            self.log(&format!("jogador {index} expulso ({reason})"));
            n += 1;
        }
        if n == 0 {
            self.log("kick: ninguém com esse XUID");
        }
    }

    // -----------------------------------------------------------------------------------------
    // Sending

    fn send_raw(sock: &UdpSocket, addr: SocketAddr, data: &[u8]) {
        // a failed send is like a lost packet: resends and the quiet drop deal with it
        let _ = sock.send_to(data, addr);
    }

    /// Splits `msgs` into packets (PACKET_MSGS_MAX messages, PACKET_BODY_MAX bytes; a bigger
    /// message goes alone).
    fn chunks(msgs: Vec<Bits>) -> Vec<Vec<Bits>> {
        let mut out: Vec<Vec<Bits>> = Vec::new();
        let mut cur: Vec<Bits> = Vec::new();
        let mut bits = 0;
        for m in msgs {
            if !cur.is_empty() && (cur.len() >= PACKET_MSGS_MAX || (bits + m.len()).div_ceil(8) > PACKET_BODY_MAX) {
                out.push(std::mem::take(&mut cur));
                bits = 0;
            }
            bits += m.len();
            cur.push(m);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    /// Messages to peer `pi`. Sequenced ones are kept until acked and resent. Before the peer
    /// is synced, a packet that would take what we sent past what it sent is not sent.
    fn send(&mut self, pi: usize, msgs: Vec<Bits>, sequenced: bool) {
        self.send_tries(pi, msgs, sequenced, 0);
    }

    fn send_tries(&mut self, pi: usize, msgs: Vec<Bits>, sequenced: bool, tries: u8) {
        let now = Instant::now();
        for chunk in Self::chunks(msgs) {
            let Some(peer) = self.peers.get_mut(pi) else { return };
            let (ack, ackbits) = peer.acks.header();
            let pid = if sequenced { peer.take_pid() } else { nerd::NO_SEQ };
            let data = nerd::packet(&chunk, pid, ack, ackbits);
            if !peer.synced {
                if peer.tx_bytes + data.len() > peer.rx_bytes {
                    continue;
                }
                peer.tx_bytes += data.len();
            }
            Self::send_raw(&self.sock, peer.addr, &data);
            if sequenced {
                if peer.in_flight.len() >= IN_FLIGHT_MAX {
                    peer.in_flight.pop_front();
                    peer.in_flight_overflow = true;
                }
                peer.in_flight.push_back(InFlight { pid, msgs: chunk, sent: now, tries });
            }
        }
        let Some(peer) = self.peers.get_mut(pi) else { return };
        if std::mem::take(&mut peer.in_flight_overflow) {
            let line = format!("peer {} is not acking: oldest unacked packets dropped (logged once)", peer.index);
            self.log_once(line);
        }
    }

    /// Sequenced messages to peer `pi`, behind what already waits in its queue (so a late
    /// joiner gets the snapshot before anything newer).
    fn send_seq(&mut self, pi: usize, msgs: Vec<Bits>) {
        let Some(peer) = self.peers.get_mut(pi) else { return };
        if peer.queue.is_empty() {
            self.send(pi, msgs, true);
            return;
        }
        for m in msgs {
            if peer.queue_bits + m.len() > self.settings.queue_max_bits {
                peer.doom.get_or_insert_with(|| "fila de envio cheia".to_string());
                return;
            }
            peer.queue_bits += m.len();
            peer.queue.push_back(m);
        }
    }

    /// An empty unsequenced packet (or one with `msgs`) carrying our acks.
    fn send_unsequenced(&mut self, pi: usize, msgs: Vec<Bits>) {
        if msgs.is_empty() {
            let Some(peer) = self.peers.get_mut(pi) else { return };
            let (ack, ackbits) = peer.acks.header();
            let data = nerd::packet(&[], nerd::NO_SEQ, ack, ackbits);
            if !peer.synced {
                if peer.tx_bytes + data.len() > peer.rx_bytes {
                    return;
                }
                peer.tx_bytes += data.len();
            }
            Self::send_raw(&self.sock, peer.addr, &data);
        } else {
            self.send(pi, msgs, false);
        }
    }

    /// To every other synced peer.
    fn forward(&mut self, from: usize, msgs: &[Bits], sequenced: bool) {
        for j in 0..self.peers.len() {
            if j != from && self.peers[j].synced {
                if sequenced {
                    self.send_seq(j, msgs.to_vec());
                } else {
                    self.send(j, msgs.to_vec(), false);
                }
            }
        }
    }

    /// An answer to an address that is not a peer (a refusal), acking its packet: only if it is
    /// no bigger than what came from there.
    fn send_to_stranger(&self, addr: SocketAddr, pid: u16, msg: Msg, request_len: usize) {
        let (ack, ackbits) = if pid == nerd::NO_SEQ { (nerd::NO_ACK, 0) } else { (pid, 0) };
        let data = nerd::packet(&[msg.write()], nerd::NO_SEQ, ack, ackbits);
        if data.len() <= request_len {
            Self::send_raw(&self.sock, addr, &data);
        }
    }

    /// Whether one more handshake reply to `ip` fits this second.
    fn handshake_ok(&mut self, ip: IpAddr, now: Instant) -> bool {
        let limit = self.settings.handshake_per_sec;
        if let Some(e) = self.handshakes.iter_mut().find(|(i, _, _)| *i == ip) {
            if now.duration_since(e.1) >= Duration::from_secs(1) {
                e.1 = now;
                e.2 = 0;
            }
            e.2 += 1;
            return e.2 <= limit;
        }
        self.handshakes.retain(|(_, t, _)| now.duration_since(*t) < Duration::from_secs(1));
        if self.handshakes.len() >= IP_TABLE_MAX {
            // a flood of sources: the oldest entry goes
            self.handshakes.remove(0);
        }
        self.handshakes.push((ip, now, 1));
        limit >= 1
    }

    // -----------------------------------------------------------------------------------------
    // Peers coming and going

    fn find(&self, addr: SocketAddr) -> Option<usize> {
        self.peers.iter().position(|p| p.addr == addr)
    }

    /// The next free index after the last one handed out (1..=254; 250 is the mirror's).
    fn free_index(&mut self) -> Option<u8> {
        for k in 0..254u16 {
            let cand = ((self.next_index as u16 - 1 + k) % 254 + 1) as u8;
            if cand == MIRROR_INDEX && self.settings.mirror.is_some() {
                continue;
            }
            if self.peers.iter().any(|p| p.index == cand) {
                continue;
            }
            self.next_index = cand % 254 + 1;
            return Some(cand);
        }
        None
    }

    fn drop_peer(&mut self, pi: usize, why: &str) {
        if pi >= self.peers.len() {
            return;
        }
        let peer = self.peers.remove(pi);
        let deletes: Vec<Bits> = peer
            .objects
            .iter()
            .map(|((router, id), o)| {
                Msg::Delete(ObjectDelete {
                    router: *router,
                    handle: Handle::Relay(*id, peer.index),
                    seq: nerd::seq_next(o.last),
                })
                .write()
            })
            .collect();
        if !deletes.is_empty() {
            for j in 0..self.peers.len() {
                if self.peers[j].synced {
                    self.send_seq(j, deletes.clone());
                }
            }
        }
        self.mirror_queue.retain(|(_, addr, _)| *addr != peer.addr);
        if self.peers.is_empty() {
            // a new group starts without the old destination (the seq goes on: harmless)
            self.group_sat_nav = 0;
        }
        if !peer.synced {
            self.debug(&format!("conexão {} ({}) desfeita: {why}", peer.index, net::show(&peer.addr)));
            return;
        }
        let secs = peer.joined.elapsed().as_secs();
        self.log(&format!(
            "jogador {} saiu ({why}): {} objeto(s) apagado(s) para os outros; ficou {}m{:02}s, {} pacotes de carro",
            peer.index,
            deletes.len(),
            secs / 60,
            secs % 60,
            peer.cars
        ));
        let c = self.counts();
        self.log(&format!("jogadores: {}/{}", c.synced, self.settings.max_players));
    }

    /// Drops the peers marked for it.
    fn drop_doomed(&mut self) {
        while let Some(i) = self.peers.iter().position(|p| p.doom.is_some()) {
            let why = self.peers[i].doom.clone().unwrap_or_default();
            self.drop_peer(i, &why);
        }
    }

    /// An OpenConnection from an address that is not a peer: a new peer (not a player until
    /// its ResponseAck echoes our attempt), or a refusal.
    fn admit(&mut self, addr: SocketAddr, hdr: &Header, oc: &nerd::OpenConnection, len: usize, now: Instant) -> Option<usize> {
        let ip = norm_ip(addr.ip());
        if !self.handshake_ok(ip, now) {
            return None;
        }
        let refuse = |relay: &mut Relay, reason: u8, why: &str| {
            relay.send_to_stranger(
                addr,
                hdr.id,
                Msg::OpenResponse(OpenResponse { flags: oc.flags, reason, attempt: oc.attempt, own_attempt: 0 }),
                len,
            );
            relay.log_refusal(ip, &format!("recusado {} (xuid {}): {why}", net::show(&addr), oc.user));
        };
        if !self.allowed(ip, now) {
            refuse(self, nerd::REASON_REFUSED, "IP não liberado");
            return None;
        }
        if self.banned(ip, now) {
            refuse(self, nerd::REASON_BANNED, "desconectado há pouco por excesso de tráfego");
            return None;
        }
        if oc.version != nerd::VERSION {
            refuse(self, nerd::REASON_VERSION_MISMATCH, &format!("versão {}", String::from_utf8_lossy(&oc.version)));
            return None;
        }
        if oc.game != nerd::GAME {
            refuse(self, nerd::REASON_GAME_MISMATCH, &format!("jogo {}", String::from_utf8_lossy(&oc.game)));
            return None;
        }
        if self.peers.iter().filter(|p| p.ip() == ip).count() >= self.settings.max_peers_per_ip {
            refuse(self, nerd::REASON_REFUSED, &format!("mais de {} conexões deste IP", self.settings.max_peers_per_ip));
            return None;
        }
        if self.peers.iter().filter(|p| !p.synced).count() >= self.settings.max_unsynced {
            refuse(self, nerd::REASON_REFUSED, "conexões demais esperando");
            return None;
        }
        let replaces = !shared_xuid(oc.user) && self.peers.iter().any(|p| p.synced && p.xuid == oc.user);
        if self.counts().synced >= self.settings.max_players as usize && !replaces {
            refuse(self, nerd::REASON_REFUSED, &format!("mundo cheio ({} jogadores)", self.settings.max_players));
            return None;
        }
        let Some(index) = self.free_index() else {
            refuse(self, nerd::REASON_REFUSED, "sem índice livre");
            return None;
        };
        let mut peer = Peer::new(addr, index, oc.user, now);
        peer.bucket = Some(Bucket::new(&self.settings, now));
        self.peers.push(peer);
        self.debug(&format!("conexão {index} de {} (xuid {})", net::show(&addr), oc.user));
        Some(self.peers.len() - 1)
    }

    /// The ResponseAck echoed our attempt: the address is proven. A peer with the same XUID
    /// gives way only if it has been quiet for `takeover_quiet`; the world must have room.
    /// Returns the peer's index now (other peers may have gone), or None if it was turned away.
    fn complete_handshake(&mut self, pi: usize) -> Option<usize> {
        let now = Instant::now();
        let (addr, xuid) = (self.peers[pi].addr, self.peers[pi].xuid);
        if !shared_xuid(xuid)
            && let Some(old) = self.peers.iter().position(|p| p.synced && p.xuid == xuid && p.addr != addr)
        {
            let quiet = now.duration_since(self.peers[old].last_seen);
            if quiet >= self.settings.takeover_quiet {
                let from = net::show(&self.peers[old].addr);
                self.drop_peer(old, &format!("voltou por {}, antes {from}", net::show(&addr)));
            } else {
                let ip = norm_ip(addr.ip());
                self.log_refusal(ip, &format!("recusado {} (xuid {xuid}): esse XUID está jogando", net::show(&addr)));
                let i = self.find(addr)?;
                self.drop_peer(i, "XUID já conectado");
                return None;
            }
        }
        let pi = self.find(addr)?;
        if self.counts().synced >= self.settings.max_players as usize {
            self.drop_peer(pi, "mundo cheio");
            return None;
        }
        self.on_synced(pi);
        Some(pi)
    }

    /// The handshake is done: TimeSync and ConnectionSyncDone, then the server's SessionHost and
    /// everything the others own (queued, sent paced).
    fn on_synced(&mut self, pi: usize) {
        let now_ms = self.now_ms();
        let peer = &mut self.peers[pi];
        peer.synced = true;
        peer.joined = Instant::now();
        let ts = Msg::TimeSync { index: peer.sync_index, time: now_ms }.write();
        peer.sync_index = peer.sync_index.wrapping_add(1);
        let own = peer.own_attempt;
        self.send(pi, vec![ts, Msg::SyncDone(own).write()], false);
        // the SessionHost as it is now: the seq of its last update, and the group's destination
        let payload = if self.group_sat_nav != 0 { nerd::group_sat_nav_payload(self.group_sat_nav) } else { Bits::new() };
        let mut world = vec![
            Msg::Object(ObjectMsg {
                create: true,
                router: nerd::ROUTER_LOBBY,
                handle: Handle::Own(nerd::SESSION_HOST_ID),
                ty: 0,
                seq: self.host_seq,
                payload,
            })
            .write(),
        ];
        for (j, owner) in self.peers.iter().enumerate() {
            if j == pi || !owner.synced {
                continue;
            }
            for ((router, id), o) in &owner.objects {
                o.snapshot(*router, Handle::Relay(*id, owner.index), &mut world);
            }
        }
        let n = world.len();
        let max = self.settings.queue_max_bits;
        let peer = &mut self.peers[pi];
        let mut cut = false;
        for m in world {
            if peer.queue_bits + m.len() > max {
                cut = true;
                break;
            }
            peer.queue_bits += m.len();
            peer.queue.push_back(m);
        }
        let index = peer.index;
        if cut {
            self.log(&format!("jogador {index}: estado do mundo maior que o limite; cortado"));
        }
        self.log(&format!("jogador {index} entrou no mundo ({n} mensagens de estado)"));
        let c = self.counts();
        self.log(&format!("jogadores: {}/{}", c.synced, self.settings.max_players));
        self.pace(pi);
    }

    /// Sends the next PACE_BYTES of what waits in peer `pi`'s queue (fewer while many of our
    /// packets to it are not acked yet).
    fn pace(&mut self, pi: usize) {
        let Some(peer) = self.peers.get_mut(pi) else { return };
        if peer.queue.is_empty() || peer.in_flight.len() > IN_FLIGHT_MAX / 2 {
            return;
        }
        let mut batch = Vec::new();
        let mut bytes = 0;
        while let Some(m) = peer.queue.front() {
            let b = m.len().div_ceil(8);
            if !batch.is_empty() && bytes + b > PACE_BYTES {
                break;
            }
            bytes += b;
            peer.queue_bits -= m.len();
            if let Some(m) = peer.queue.pop_front() {
                batch.push(m);
            }
        }
        self.send(pi, batch, true);
    }

    // -----------------------------------------------------------------------------------------
    // Packets in

    /// One packet. A packet whose handling panics is dropped (logged once per address); the
    /// relay goes on.
    pub fn handle(&mut self, data: &[u8], addr: SocketAddr) {
        if catch_unwind(AssertUnwindSafe(|| self.handle_packet(data, addr))).is_err() {
            if self.panics_logged.len() < PANIC_LOGGED_MAX && !self.panics_logged.contains(&addr) {
                self.panics_logged.push(addr);
                self.log(&format!("pacote de {} descartado: erro ao tratar (registrado uma vez)", net::show(&addr)));
            }
        }
        self.drop_doomed();
    }

    fn handle_packet(&mut self, data: &[u8], addr: SocketAddr) {
        let Some(hdr) = Header::parse(data) else { return };
        let now = Instant::now();
        let body = Reader::at(data, nerd::HEADER_LEN * 8);
        let first = if hdr.count > 0 { body.peek(7).ok().map(|v| v as u8) } else { None };
        let mut pi = self.find(addr);
        // a new launch from the same address starts over at packet 0 with an OpenConnection
        if let Some(i) = pi
            && self.peers[i].synced
            && hdr.id == 0
            && first == Some(nerd::MSG_OPEN_CONNECTION)
        {
            self.drop_peer(i, "nova conexão");
            pi = None;
        }
        let mut admitted_now = false;
        let pi = match pi {
            Some(i) => i,
            None => {
                admitted_now = true;
                // only an OpenConnection makes a peer
                if first != Some(nerd::MSG_OPEN_CONNECTION) {
                    return;
                }
                let Ok(Msg::OpenConnection(oc)) = Msg::read(&mut body.clone()) else { return };
                match self.admit(addr, &hdr, &oc, data.len(), now) {
                    Some(i) => i,
                    None => return,
                }
            }
        };
        // the rate
        {
            let s = &self.settings;
            let peer = &mut self.peers[pi];
            let ok = peer.bucket.get_or_insert_with(|| Bucket::new(s, now)).take(s, now, hdr.count as u32, data.len());
            if !ok {
                let since = match (peer.over_since, peer.over_last) {
                    (Some(t), Some(l)) if now.duration_since(l) < Duration::from_millis(1500) => t,
                    _ => now,
                };
                peer.over_since = Some(since);
                peer.over_last = Some(now);
                let (index, ip) = (peer.index, peer.ip());
                if now.duration_since(since) >= Duration::from_secs(self.settings.abuse_secs as u64) {
                    let ban = self.settings.abuse_ban;
                    self.peers[pi].doom = Some("tráfego acima do limite".to_string());
                    self.ban(BanKey::Ip(ip), ban);
                    self.log(&format!("jogador {index}: tráfego acima do limite por {} s; desconectado", self.settings.abuse_secs));
                } else {
                    self.log_once(format!("peer {index}: traffic over the rate; packets dropped (logged once)"));
                }
                return;
            }
        }
        {
            let peer = &mut self.peers[pi];
            peer.last_seen = now;
            if !peer.synced {
                peer.rx_bytes = peer.rx_bytes.saturating_add(data.len());
            }
            peer.acks.add(hdr.id);
            // our packets it acks
            peer.in_flight.retain(|f| !nerd::acks_packet(hdr.ack, hdr.ackbits, f.pid));
        }
        let index = self.peers[pi].index;
        self.debug(&format!("peer {index} packet id {} ack {} count {} ({} bytes)", hdr.id, hdr.ack, hdr.count, data.len()));

        let mut r = body;
        let mut out: Vec<Bits> = Vec::new(); // unsequenced answers
        let mut out_seq: Vec<Bits> = Vec::new(); // sequenced answers
        let mut fwd: Vec<Bits> = Vec::new(); // object messages for the others
        let mut ghost: Vec<Bits> = Vec::new(); // the same, back to the sender (mirror)
        let mut cars: Vec<Bits> = Vec::new();
        let mut synced_now = false;
        let synced = self.peers[pi].synced;
        for _ in 0..hdr.count {
            if r.left() < 7 {
                break;
            }
            let msg = match Msg::read(&mut r) {
                Ok(m) => m,
                Err(ReadError::UnknownMsg(id)) => {
                    if !self.unknown_logged[id as usize & 127] {
                        self.unknown_logged[id as usize & 127] = true;
                        self.log(&format!(
                            "peer {index}: message {id} ({}) not known; the rest of its packets is dropped (logged once)",
                            nerd::msg_name(id)
                        ));
                    }
                    break;
                }
                Err(e) => {
                    self.log_once(format!("peer {index}: {e}; rest of the packet dropped (logged once per kind)"));
                    break;
                }
            };
            match msg {
                Msg::OpenConnection(oc) => {
                    let peer = &self.peers[pi];
                    if oc.user != peer.xuid {
                        self.log_once(format!("peer {index}: OpenConnection with another XUID ignored"));
                        continue;
                    }
                    let (own, ip) = (peer.own_attempt, peer.ip());
                    // the one that made the peer was counted in admit
                    if !std::mem::take(&mut admitted_now) && !self.handshake_ok(ip, now) {
                        continue;
                    }
                    self.debug(&format!("peer {index} OpenConnection xuid {} attempt {}", oc.user, oc.attempt));
                    out.push(
                        Msg::OpenResponse(OpenResponse { flags: oc.flags, reason: nerd::REASON_OK, attempt: oc.attempt, own_attempt: own })
                            .write(),
                    );
                }
                Msg::OpenResponse(resp) => {
                    self.debug(&format!("peer {index} OpenConnectionResponse reason {}", resp.reason));
                    if resp.reason == nerd::REASON_RESPONSE_ACK && !synced {
                        if resp.attempt == self.peers[pi].own_attempt {
                            synced_now = true;
                        } else {
                            self.log_once(format!("peer {index}: ResponseAck with the wrong attempt ignored"));
                        }
                    }
                }
                Msg::Close => {
                    self.drop_peer(pi, "fechou a conexão");
                    return;
                }
                Msg::Ping(t) => out.push(Msg::Pong { echo: t, time: self.now_ms() }.write()),
                Msg::Pong { echo, time } => {
                    // the answer to our ping: Pong(0, our time)
                    let now_ms = self.now_ms();
                    if echo == 0 && time <= now_ms && now_ms - time < 60_000 {
                        self.peers[pi].rtt_ms = Some(now_ms - time);
                    }
                }
                // nothing else until the handshake is done
                m if !synced => self.debug(&format!("peer {index}: {} before the handshake ignored", nerd::msg_name(m.id()))),
                Msg::TimeSync { .. } | Msg::SyncDone(_) => self.debug(&format!("peer {index}: server-only message ignored")),
                Msg::Object(o) => {
                    if let Some(m) = self.on_object(pi, o) {
                        if self.settings.mirror.is_some() {
                            ghost.push(m.ghost.write());
                        }
                        fwd.push(m.fwd.write());
                    }
                }
                Msg::Delete(d) => {
                    let Handle::Own(id) = d.handle else {
                        self.log_once(format!("peer {index}: ObjectDelete with handle {:?} not relayed", d.handle));
                        continue;
                    };
                    if !nerd::forwarded_router(d.router) {
                        self.log_once(format!("peer {index}: ObjectDelete on router {} not relayed", d.router));
                        continue;
                    }
                    let peer = &mut self.peers[pi];
                    peer.objects.retain(|(k, _)| *k != (d.router, id));
                    peer.orphans.retain(|o| (o.router, o.handle.id()) != (d.router, id));
                    self.debug(&format!("peer {index} ObjectDelete router {} id {id} seq {}", d.router, d.seq));
                    fwd.push(Msg::Delete(ObjectDelete { handle: Handle::Relay(id, index), ..d }).write());
                    if self.settings.mirror.is_some() {
                        ghost.push(Msg::Delete(ObjectDelete { handle: Handle::Relay(id, MIRROR_INDEX), ..d }).write());
                    }
                }
                Msg::Car(c) => {
                    let Handle::Own(id) = c.handle else {
                        self.log_once(format!("peer {index}: car with handle {:?} not relayed", c.handle));
                        continue;
                    };
                    if !nerd::car_ok(&c.body) {
                        self.log_once(format!("peer {index}: car with an impossible position not relayed (logged once)"));
                        continue;
                    }
                    self.peers[pi].cars += 1;
                    if let Some(delay) = self.settings.mirror {
                        if self.mirror_queue.len() >= MIRROR_QUEUE_MAX {
                            self.mirror_queue.pop_front();
                        }
                        let m = Msg::Car(Car { handle: Handle::Relay(id, MIRROR_INDEX), body: c.body.clone() }).write();
                        self.mirror_queue.push_back((now + delay, addr, m));
                    }
                    cars.push(Msg::Car(Car { handle: Handle::Relay(id, index), body: c.body }).write());
                }
                Msg::CarEvent(e) => {
                    // the sender's car: relayed like the car itself, with the relay handle
                    let Handle::Own(id) = e.handle else {
                        self.log_once(format!("peer {index}: {} with handle {:?} not relayed", nerd::msg_name(e.id), e.handle));
                        continue;
                    };
                    if !nerd::car_event_ok(e.id, &e.body) {
                        self.log_once(format!("peer {index}: {} with impossible values not relayed (logged once)", nerd::msg_name(e.id)));
                        continue;
                    }
                    self.debug(&format!("peer {index} {} car {id}", nerd::msg_name(e.id)));
                    let relayed = |peer| Msg::CarEvent(CarEvent { handle: Handle::Relay(id, peer), ..e.clone() }).write();
                    if self.settings.mirror.is_some() {
                        ghost.push(relayed(MIRROR_INDEX));
                    }
                    // RepairDamage goes on the console's reliable channel (2), so it is resent
                    // until acked; glass and collisions on channel 5, unreliable, like the cars
                    if e.id == nerd::MSG_REPAIR_DAMAGE {
                        fwd.push(relayed(index));
                    } else {
                        cars.push(relayed(index));
                    }
                }
                Msg::SetGroupSatNav { handle, node } => {
                    self.debug(&format!("peer {index} SetGroupSatNavEvent {handle:?} node {node}"));
                    if !(0..=nerd::SAT_NAV_NODE_MAX).contains(&node) {
                        self.log_once(format!("peer {index}: SetGroupSatNavEvent with node {node} out of range ignored"));
                        continue;
                    }
                    self.set_group_sat_nav(index, node);
                }
                // Read only so the rest of the packet is: the traffic and the collidable objects
                // were the Xbox server's own simulation, which this relay does not run; Tag's
                // second car would need a handle per receiver (Playground games, not free roam);
                // chat management and InactivePlayersPulled are for the server's matchmaking.
                // The `Other` ones are all requests to the server (ownership, car meet, race,
                // road trip, leadership, votes, bucket list) or weather replies it never asked
                // for; none of them is meant for the other consoles.
                Msg::TagCollision(_)
                | Msg::Traffic { .. }
                | Msg::CollObjDetachPart { .. }
                | Msg::ChatManagement { .. }
                | Msg::InactivePlayersPulled(_)
                | Msg::Other { .. } => {
                    self.debug(&format!("peer {index} {} (not relayed)", nerd::msg_name(msg.id())));
                }
                Msg::FinaliseSessionEntry => {
                    self.debug(&format!("peer {index} FinaliseSessionEntry"));
                    self.peers[pi].finalise_asked = true;
                    if let Some(m) = self.finalise(pi) {
                        out_seq.push(m);
                    }
                }
            }
        }
        // its history within bounds
        if self.peers[pi].history_bits() > self.settings.history_max_bits {
            self.peers[pi].doom = Some("histórico grande demais".to_string());
            return;
        }
        if !fwd.is_empty() {
            self.forward(pi, &fwd, true);
        }
        if !ghost.is_empty() {
            self.send_seq(pi, ghost);
        }
        if !cars.is_empty() {
            self.forward(pi, &cars, false);
        }
        // the ConsoleSynced Create may have come after FinaliseSessionEntry
        if let Some(m) = self.finalise(pi) {
            out_seq.push(m);
        }
        if !out_seq.is_empty() {
            self.send_seq(pi, out_seq);
        }
        if !out.is_empty() || hdr.id != nerd::NO_SEQ {
            self.send_unsequenced(pi, out);
        }
        if synced_now {
            self.complete_handshake(pi);
        }
    }

    /// SetGroupSatNavEvent: the server's answer, GroupSatNavNode on its SessionHost, to every
    /// synced console (the sender too: the var is what the consoles go by). Any console may set
    /// it: the relay keeps no group leader.
    fn set_group_sat_nav(&mut self, from_index: u8, node: i32) {
        if node == self.group_sat_nav {
            return;
        }
        self.group_sat_nav = node;
        self.host_seq = nerd::seq_next(self.host_seq);
        self.debug(&format!("jogador {from_index}: GPS do grupo -> nó {node}"));
        let update = Msg::Object(ObjectMsg {
            create: false,
            router: nerd::ROUTER_LOBBY,
            handle: Handle::Own(nerd::SESSION_HOST_ID),
            ty: 0,
            seq: self.host_seq,
            payload: nerd::group_sat_nav_payload(node),
        })
        .write();
        for j in 0..self.peers.len() {
            if self.peers[j].synced {
                self.send_seq(j, vec![update.clone()]);
            }
        }
    }

    /// SessionEntryFinalised = true on the peer's own ConsoleSynced object, once it asked and the
    /// object is known. Server updates are taken strictly in order and the first one expected
    /// from us is seq 0 (1417B1E70).
    fn finalise(&mut self, pi: usize) -> Option<Bits> {
        let peer = self.peers.get_mut(pi)?;
        if !peer.finalise_asked || peer.finalised {
            return None;
        }
        let id = peer.console_id?;
        peer.finalised = true;
        let seq = peer.console_seq.map_or(0, nerd::seq_next);
        peer.console_seq = Some(seq);
        let index = peer.index;
        self.debug(&format!("peer {index} -> SessionEntryFinalised (ConsoleSynced {id}, seq {seq})"));
        Some(nerd::console_bool_update(id, seq, nerd::CONSOLE_SESSION_ENTRY_FINALISED, true).write())
    }

    /// An object message from peer `pi`: checked and written again (nerd::sanitize_payload),
    /// the history, and what to forward (None: a duplicate, refused, or not the sender's own).
    fn on_object(&mut self, pi: usize, o: ObjectMsg) -> Option<Relayed> {
        let index = self.peers[pi].index;
        let Handle::Own(id) = o.handle else {
            self.log_once(format!(
                "peer {index}: {} router {} with handle {:?} not relayed (logged once)",
                if o.create { "ObjectCreate" } else { "ObjectUpdate" },
                o.router,
                o.handle
            ));
            return None;
        };
        if !nerd::forwarded_router(o.router) || o.ty != 0 {
            self.log_once(format!("peer {index}: objects of router {} type {} are not relayed (logged once)", o.router, o.ty));
            return None;
        }
        let raw_len = o.payload.len();
        let payload = match nerd::sanitize_payload(o.router, &o.payload) {
            Ok(p) => p,
            Err(e) => {
                self.log_once(format!("peer {index}: router {} payload refused: {e} (logged once)", o.router));
                return None;
            }
        };
        if let Some(seen) = self.payload_seen.get_mut(o.router as usize & 15)
            && raw_len > *seen
        {
            *seen = raw_len;
            self.debug(&format!("router {}: largest payload so far {raw_len} bits", o.router));
        }
        let o = ObjectMsg { payload, ..o };
        let key = (o.router, id);
        let verbose = self.settings.verbose;
        let peer = &mut self.peers[pi];
        let mut lossy = false;
        let fresh = if o.create {
            let fresh = match peer.object(key) {
                Some(obj) if !nerd::seq_newer(o.seq, obj.last) => Fresh::Duplicate,
                _ => Fresh::New,
            };
            if fresh == Fresh::New {
                peer.objects.retain(|(k, _)| *k != key);
                if peer.objects.len() < OBJECTS_MAX {
                    let mut obj = Obj::new(o.ty, o.seq, o.payload.clone());
                    // updates that came before their Create
                    let orphans: Vec<ObjectMsg> = {
                        let (mine, rest): (Vec<_>, Vec<_>) =
                            peer.orphans.drain(..).partition(|u| (u.router, u.handle.id()) == key);
                        peer.orphans = rest.into();
                        mine
                    };
                    for u in orphans {
                        lossy |= obj.update(u.seq, u.payload).1;
                    }
                    peer.objects.push((key, obj));
                } else {
                    peer.objects_full = true;
                }
                if o.router == nerd::ROUTER_CONSOLE_SYNCED && peer.console_id != Some(id) {
                    peer.console_id = Some(id);
                    peer.console_seq = None;
                }
            }
            fresh
        } else {
            match peer.object(key) {
                Some(obj) => {
                    let (fresh, dropped) = obj.update(o.seq, o.payload.clone());
                    lossy |= dropped;
                    fresh
                }
                None => {
                    if peer.orphans.iter().any(|u| (u.router, u.handle.id(), u.seq) == (o.router, id, o.seq)) {
                        Fresh::Duplicate
                    } else {
                        if peer.orphans.len() >= ORPHANS_MAX {
                            peer.orphans.pop_front();
                        }
                        peer.orphans.push_back(o.clone());
                        Fresh::New
                    }
                }
            }
        };
        let full = std::mem::take(&mut peer.objects_full);
        if verbose {
            self.debug(&format!(
                "peer {index} {} router {} id {id}{} seq {} {} bits{}",
                if o.create { "ObjectCreate" } else { "ObjectUpdate" },
                o.router,
                if o.create { format!(" type {}", o.ty) } else { String::new() },
                o.seq,
                o.payload.len(),
                if fresh == Fresh::Duplicate { " (resent)" } else { "" }
            ));
        }
        if lossy {
            self.log(&format!("peer {index}: history of router {} id {id} is full; an old update was dropped", o.router));
        }
        if full {
            self.log_once(format!("peer {index}: more than {OBJECTS_MAX} objects; new ones are relayed but not kept"));
        }
        if fresh == Fresh::Duplicate {
            return None;
        }
        let relayed = |peer_index| Msg::Object(ObjectMsg { handle: Handle::Relay(id, peer_index), ..o.clone() });
        Some(Relayed { fwd: relayed(index), ghost: relayed(MIRROR_INDEX) })
    }

    // -----------------------------------------------------------------------------------------
    // Timers

    /// Keepalive pings, quiet and half-connected peers, resends, paced queues and the mirror's
    /// cars.
    pub fn tick(&mut self, now: Instant) {
        // the mirror's cars that are due
        while let Some((due, _, _)) = self.mirror_queue.front() {
            if *due > now {
                break;
            }
            let Some((_, addr, m)) = self.mirror_queue.pop_front() else { break };
            if let Some(pi) = self.find(addr) {
                self.send(pi, vec![m], false);
            }
        }
        if now.duration_since(self.last_resend_scan) >= Duration::from_millis(50) {
            self.last_resend_scan = now;
            self.resend(now);
        }
        if now.duration_since(self.last_pace) >= PACE_EVERY {
            self.last_pace = now;
            for pi in 0..self.peers.len() {
                self.pace(pi);
            }
        }
        if now.duration_since(self.last_keepalive) >= self.settings.keepalive {
            self.last_keepalive = now;
            let quiet = self.settings.quiet_drop;
            let unsynced = self.settings.unsynced_timeout;
            let mut i = 0;
            while i < self.peers.len() {
                if now.duration_since(self.peers[i].last_seen) > quiet {
                    let why = format!("calado por {} s", quiet.as_secs());
                    self.drop_peer(i, &why);
                    continue;
                }
                if !self.peers[i].synced && now.duration_since(self.peers[i].joined) > unsynced {
                    self.drop_peer(i, "não terminou a conexão a tempo");
                    continue;
                }
                if self.peers[i].synced {
                    let ping = Msg::Ping(self.now_ms()).write();
                    self.send(i, vec![ping], false);
                }
                i += 1;
            }
        }
        self.drop_doomed();
    }

    /// Sequenced packets not acked in time go again, in new packets; a peer that never acks one
    /// is dropped.
    fn resend(&mut self, now: Instant) {
        for pi in 0..self.peers.len() {
            let peer = &mut self.peers[pi];
            let wait = self.settings.resend_after.max(Duration::from_millis(2 * peer.rtt_ms.unwrap_or(0) as u64));
            let mut again: Vec<(Vec<Bits>, u8)> = Vec::new();
            let mut gave_up = 0;
            peer.in_flight.retain_mut(|f| {
                if now.duration_since(f.sent) < wait {
                    return true;
                }
                if f.tries >= RESEND_TRIES_MAX {
                    gave_up += f.msgs.len();
                } else {
                    again.push((std::mem::take(&mut f.msgs), f.tries + 1));
                }
                false
            });
            if gave_up > 0 {
                peer.doom = Some(format!("{gave_up} mensagem(ns) nunca confirmada(s)"));
            }
            for (msgs, tries) in again {
                self.send_tries(pi, msgs, true, tries);
            }
        }
    }
}

/// Reads command lines from `input` into `tx` until its end (or until nobody listens). A line
/// longer than COMMAND_LINE_MAX bytes is dropped whole; empty lines are skipped.
pub fn read_commands(mut input: impl io::BufRead, tx: &Sender<String>) {
    let mut line: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let (consumed, done) = match input.fill_buf() {
            Ok([]) => return,
            Ok(buf) => {
                let (part, consumed, done) = match buf.iter().position(|&b| b == b'\n') {
                    Some(i) => (&buf[..i], i + 1, true),
                    None => (buf, buf.len(), false),
                };
                if !too_long {
                    line.extend_from_slice(part);
                    if line.len() > COMMAND_LINE_MAX {
                        too_long = true;
                        line.clear();
                    }
                }
                (consumed, done)
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        input.consume(consumed);
        if done {
            if !too_long {
                let text = String::from_utf8_lossy(&line).trim().to_string();
                if !text.is_empty() && tx.send(text).is_err() {
                    return;
                }
            }
            line.clear();
            too_long = false;
        }
    }
}

/// A XUID in a command: decimal, or hex with 0x.
fn parse_xuid(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

struct Relayed {
    fwd: Msg,
    ghost: Msg,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(n: usize, v: u64) -> Bits {
        let mut b = Bits::new();
        for i in 0..n {
            b.push(v >> (i % 64), 1);
        }
        b
    }

    fn cat(parts: &[&Bits]) -> Bits {
        let mut b = Bits::new();
        for p in parts {
            b.append(p);
        }
        b
    }

    #[test]
    fn updates_fold_into_the_create() {
        let (p0, p1, p2) = (bits(10, 1), bits(5, 2), bits(7, 3));
        let mut o = Obj::new(0, 10, p0.clone());
        assert_eq!(o.update(11, p1.clone()), (Fresh::New, false));
        assert_eq!(o.update(12, p2.clone()), (Fresh::New, false));
        assert_eq!(o.update(11, p1.clone()).0, Fresh::Duplicate);
        assert_eq!(o.update(10, p1.clone()).0, Fresh::Duplicate);
        assert_eq!((o.base_seq, o.last), (12, 12));
        assert_eq!(o.base, cat(&[&p0, &p1, &p2]));
        let mut out = Vec::new();
        o.snapshot(1, Handle::Relay(4, 9), &mut out);
        assert_eq!(out.len(), 1);
        let m = Msg::read(&mut Reader::new(out[0].as_bytes())).unwrap();
        assert_eq!(
            m,
            Msg::Object(ObjectMsg { create: true, router: 1, handle: Handle::Relay(4, 9), ty: 0, seq: 12, payload: cat(&[&p0, &p1, &p2]) })
        );
    }

    #[test]
    fn gap_waits_then_drains() {
        let mut o = Obj::new(0, 254, Bits::new());
        // 1 before 0 (the seq wraps at 255)
        assert_eq!(o.update(1, bits(3, 1)).0, Fresh::New);
        assert_eq!(o.update(1, bits(3, 1)).0, Fresh::Duplicate);
        assert_eq!(o.last, 254);
        assert_eq!(o.pending.len(), 1);
        // a joiner now gets the Create (254) and the pending 1
        let mut out = Vec::new();
        o.snapshot(2, Handle::Relay(0, 1), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(o.update(0, bits(2, 0)).0, Fresh::New);
        assert_eq!((o.last, o.base_seq, o.pending.len()), (1, 1, 0));
        assert_eq!(o.base.len(), 5);
    }

    #[test]
    fn full_base_goes_to_the_tail_and_the_tail_is_bounded() {
        let big = nerd::MAX_PAYLOAD_BITS - 100;
        let mut o = Obj::new(0, 0, bits(big, 7));
        let mut seq = 0u8;
        let mut lossy = false;
        for _ in 0..TAIL_MAX + 10 {
            seq = nerd::seq_next(seq);
            let (f, d) = o.update(seq, bits(200, seq as u64));
            assert_eq!(f, Fresh::New);
            lossy |= d;
        }
        assert!(lossy, "updates had to go");
        assert!(o.tail.len() <= TAIL_MAX);
        // the Create stands right before the first tail update, so the chain has no gap
        assert_eq!(nerd::seq_next(o.base_seq), o.tail.front().unwrap().0);
        assert_eq!(o.last, seq);
        let mut out = Vec::new();
        o.snapshot(1, Handle::Relay(0, 1), &mut out);
        assert_eq!(out.len(), 1 + o.tail.len());
        let seqs: Vec<u8> = out
            .iter()
            .map(|b| match Msg::read(&mut Reader::new(b.as_bytes())).unwrap() {
                Msg::Object(m) => m.seq,
                _ => panic!(),
            })
            .collect();
        for w in seqs.windows(2) {
            assert_eq!(nerd::seq_next(w[0]), w[1]);
        }
    }

    #[test]
    fn pending_is_bounded() {
        let mut o = Obj::new(0, 0, Bits::new());
        for s in 2..(2 + PENDING_MAX as u8 + 5) {
            o.update(s, bits(1, 1));
        }
        assert_eq!(o.pending.len(), PENDING_MAX);
    }

    #[test]
    fn chunks_respect_limits() {
        let small: Vec<Bits> = (0..20).map(|i| bits(50, i)).collect();
        let c = Relay::chunks(small);
        assert_eq!(c.iter().map(|c| c.len()).collect::<Vec<_>>(), [8, 8, 4]);
        let big: Vec<Bits> = (0..3).map(|i| bits(9000, i)).collect();
        let c = Relay::chunks(big);
        assert_eq!(c.len(), 3, "two 1125-byte messages do not share a packet");
    }

    #[test]
    fn pids_skip_the_special_values() {
        let mut p = Peer::new("127.0.0.1:1".parse().unwrap(), 1, 0, Instant::now());
        p.next_pid = 0xFFFD;
        assert_eq!(p.take_pid(), 0xFFFD);
        assert_eq!(p.take_pid(), 0);
        assert_eq!(p.take_pid(), 1);
    }

    fn relay() -> Relay {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        Relay::new(sock, Settings { quiet: true, ..Default::default() })
    }

    #[test]
    fn allow_list_and_loopback() {
        let mut r = relay();
        let now = Instant::now();
        let ip = |s: &str| norm_ip(s.parse::<IpAddr>().unwrap());
        // off: everyone
        assert!(r.allowed(ip("10.0.0.1"), now));
        r.command("allowonly 1");
        assert!(!r.allowed(ip("10.0.0.1"), now));
        for lo in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            assert!(r.allowed(ip(lo), now), "{lo}");
        }
        r.command("allow 10.0.0.1");
        r.command("allow ::ffff:10.0.0.2");
        r.command("allow 2001:db8::7");
        assert!(r.allowed(ip("10.0.0.1"), now));
        assert!(r.allowed(ip("10.0.0.2"), now), "an IPv4-mapped address is the IPv4 one");
        assert!(r.allowed(ip("::ffff:10.0.0.1"), now));
        assert!(r.allowed(ip("2001:db8::7"), now));
        assert!(!r.allowed(ip("10.0.0.3"), now));
        // expiry
        assert!(!r.allowed(ip("10.0.0.1"), now + ALLOW_TTL + Duration::from_secs(1)));
        // bounded
        for k in 0..(ALLOW_MAX as u32 + 10) {
            r.command(&format!("allow 10.1.{}.{}", k / 256, k % 256));
        }
        assert_eq!(r.allow.len(), ALLOW_MAX);
        // junk is ignored
        r.command("allow notanip");
        r.command("allowonly maybe");
        r.command("rm -rf /");
        assert_eq!(r.allow.len(), ALLOW_MAX);
        r.command("allowonly 0");
        assert!(r.allowed(ip("10.9.9.9"), now));
    }

    #[test]
    fn kick_does_not_ban_abuse_does() {
        let mut r = relay();
        let now = Instant::now();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        r.command("kick 2535412345678901 griefing");
        r.command("kick 0x10");
        r.command(&format!("kick {AZIMUTH_XUID}"));
        assert!(r.bans.is_empty() && !r.banned(ip, now));
        r.ban(BanKey::Ip(ip), Duration::from_secs(5));
        assert!(r.banned(ip, now));
        assert!(!r.banned("10.0.0.2".parse().unwrap(), now));
        assert!(!r.banned(ip, now + Duration::from_secs(6)));
        // bounded
        for x in 0..(BANS_MAX as u32 + 20) {
            r.ban(BanKey::Ip(IpAddr::from((0x0a00_0000 + x).to_be_bytes())), Duration::from_secs(60));
        }
        assert_eq!(r.bans.len(), BANS_MAX);
        r.command("kick");
        r.command("kick abc");
        r.command("kick -5");
        assert_eq!(parse_xuid("18446744073709551615"), Some(u64::MAX));
        assert_eq!(parse_xuid("18446744073709551616"), None);
    }

    #[test]
    fn command_lines_are_bounded() {
        let (tx, rx) = std::sync::mpsc::channel();
        let long = "x".repeat(COMMAND_LINE_MAX + 1);
        let input = format!("kick 1\n\n{long}\nallow 10.0.0.1\r\n{}\nallowonly 1", "y".repeat(COMMAND_LINE_MAX));
        read_commands(std::io::Cursor::new(input.into_bytes()), &tx);
        let got: Vec<String> = rx.try_iter().collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], "kick 1");
        assert_eq!(got[1], "allow 10.0.0.1");
        assert_eq!(got[2].len(), COMMAND_LINE_MAX, "exactly the limit is kept");
        // the last line has no newline: at the end of input it is not a command
        // and nobody listening ends the reader
        drop(rx);
        read_commands(std::io::Cursor::new(b"kick 1\nkick 2\n".to_vec()), &tx);
        // a reader that ends (EOF) just returns
        read_commands(std::io::Cursor::new(Vec::new()), &tx);
    }

    #[test]
    fn log_lines_have_no_control_characters() {
        let s = sanitize_line("jogador\x1b[31m vermelho\r\nfalso\x07 ok ção");
        assert!(!s.chars().any(|c| c.is_control()));
        assert!(s.ends_with("ok ção"));
        assert_eq!(sanitize_line(&"a".repeat(5000)).chars().count(), LOG_LINE_MAX);
    }

    #[test]
    fn attempts_are_random_and_never_the_old_constant() {
        let a: SocketAddr = "127.0.0.1:5".parse().unwrap();
        let v: Vec<u32> = (0..64).map(|_| random_attempt(a)).collect();
        assert!(v.iter().all(|&x| x != 0 && x != OWN_ATTEMPT));
        let mut d = v.clone();
        d.sort();
        d.dedup();
        assert!(d.len() > 60, "practically all different");
    }

    #[test]
    fn bucket_allows_cars_and_stops_floods() {
        let s = Settings::default();
        let t0 = Instant::now();
        let mut b = Bucket::new(&s, t0);
        // 30 car packets a second for 10 s, plus 10 other packets: all pass
        for k in 0..300u64 {
            let t = t0 + Duration::from_millis(k * 33);
            assert!(b.take(&s, t, 2, 120), "{k}");
        }
        // a flood in one instant: stopped within two seconds' worth
        let t = t0 + Duration::from_secs(10);
        let passed = (0..10_000).filter(|_| b.take(&s, t, 1, 100)).count();
        assert!(passed <= 2 * s.rate_packets as usize, "{passed}");
        assert!(passed > 0);
    }
}
