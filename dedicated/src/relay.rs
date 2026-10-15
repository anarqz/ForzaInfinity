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
//! wait_for_session_host). Every object a console creates, updates or deletes is forwarded to the
//! other consoles with the relay handle (its owner's peer index), and kept in a history that
//! late joiners get as a snapshot. Cars (CarPredictionFull, ~30 a second, unsequenced) are
//! forwarded the same way and not kept.
//!
//! History (fix: relay.py kept every message forever and replayed all of them). The receiver
//! applies object updates strictly in seq order (1417B1E70: link+1 mod 255; newer ones wait for
//! the gap, older ones are dropped), and a Create sets the link to its own seq. Payloads are
//! (var index, data) lists read to the end (1417B2510), and var data widths are per type and
//! not known here, so updates cannot be merged per var. They can be concatenated, though: a
//! later entry for a var overwrites the earlier one. So each object keeps:
//!   - `base`: the Create's payload with the oldest updates appended, under the seq of the last
//!     one appended; a late joiner gets it as one Create with that seq, so the owner's next live
//!     update (seq+1) applies right after;
//!   - `tail`: the updates that no longer fit in base (u14 payload size), in seq order, replayed
//!     verbatim after the Create (at most TAIL_MAX of them and TAIL_MAX_BITS);
//!   - `pending`: updates that came ahead of a gap, until the owner's resend fills it.
//!
//! When the tail is full its oldest update is folded into base if it fits, else dropped: a late
//! joiner then misses that update's vars until they change again (logged once per object). Every
//! update so far is small, so this should not happen in practice.
//!
//! Duplicates (fix: relay.py's dedup set grew forever): consoles resend unacked messages, and
//! a message is new only if its seq is newer than the object's (or, ahead of a gap, not pending
//! yet). That state is the history itself, so it is bounded with it.
//!
//! Reliability (fix): consoles drop out-of-order updates and wait on gaps, so a lost packet of
//! object messages would stall that object for good. Our sequenced packets are kept until the
//! console acks them (the ack and ack bits of its packet headers) and resent in a new packet
//! otherwise.
//!
//! Peers: by address, plus the XUID from OpenConnection: an OpenConnection from a new address
//! with the XUID of a connected peer replaces that peer (NAT rebinding, a rejoin). A console that
//! starts over (an OpenConnection in packet 0), goes quiet for 15 s or closes the connection is
//! dropped, and the others get an ObjectDelete for each of its objects. Only an OpenConnection
//! makes a peer: other packets from unknown addresses are ignored. Peer indices are 1..=254,
//! handed out round-robin so a freed index is reused as late as possible (receivers remember
//! deletes per handle, 1417AB560, and drop a Create with an older seq for it).
//!
//! Game messages carry no length, so a packet is read only as far as its messages are known; the
//! rest is dropped (the packet is still acked) and each unknown message id is logged once.

use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crate::{
    bits::{Bits, Reader},
    nerd::{self, Car, Handle, Header, Msg, ObjectDelete, ObjectMsg, OpenResponse, ReadError},
    net,
};

/// The attempt value the server gives in OpenConnectionResponse and ConnectionSyncDone.
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

struct Peer {
    addr: SocketAddr,
    index: u8,
    xuid: u64,
    acks: nerd::Acks,
    /// Our next sequenced packet id: 1, 2, ... skipping 0xFFFE (the "no ack" value) and 0xFFFF
    /// (unsequenced).
    next_pid: u16,
    in_flight: VecDeque<InFlight>,
    in_flight_overflow: bool,
    synced: bool,
    sync_index: u16,
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
}

impl Peer {
    fn new(addr: SocketAddr, index: u8, xuid: u64, now: Instant) -> Peer {
        Peer {
            addr,
            index,
            xuid,
            acks: nerd::Acks::default(),
            next_pid: 1,
            in_flight: VecDeque::new(),
            in_flight_overflow: false,
            synced: false,
            sync_index: 0,
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
    /// (due, peer address, message): the mirror's delayed cars.
    mirror_queue: VecDeque<(Instant, SocketAddr, Bits)>,
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
            mirror_queue: VecDeque::new(),
        }
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
            println!("[{:02}:{:02}:{:02}] {line}", s / 3600, s / 60 % 60, s % 60);
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

    /// Our clock for Ping/Pong/TimeSync, ms since start (never 0).
    fn now_ms(&self) -> u32 {
        (self.t0.elapsed().as_millis() as u32).wrapping_add(1)
    }

    /// Runs until `stop` is set: packets in, and the timers (keepalive, resends, quiet peers,
    /// the mirror) between them.
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
            self.tick(Instant::now());
        }
        Ok(())
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

    /// Messages to peer `pi`. Sequenced ones are kept until acked and resent.
    fn send(&mut self, pi: usize, msgs: Vec<Bits>, sequenced: bool) {
        self.send_tries(pi, msgs, sequenced, 0);
    }

    fn send_tries(&mut self, pi: usize, msgs: Vec<Bits>, sequenced: bool, tries: u8) {
        let now = Instant::now();
        for chunk in Self::chunks(msgs) {
            let peer = &mut self.peers[pi];
            let (ack, ackbits) = peer.acks.header();
            let pid = if sequenced { peer.take_pid() } else { nerd::NO_SEQ };
            let data = nerd::packet(&chunk, pid, ack, ackbits);
            Self::send_raw(&self.sock, peer.addr, &data);
            if sequenced {
                if peer.in_flight.len() >= IN_FLIGHT_MAX {
                    peer.in_flight.pop_front();
                    peer.in_flight_overflow = true;
                }
                peer.in_flight.push_back(InFlight { pid, msgs: chunk, sent: now, tries });
            }
        }
        let peer = &mut self.peers[pi];
        if std::mem::take(&mut peer.in_flight_overflow) {
            let line = format!("peer {} is not acking: oldest unacked packets dropped", peer.index);
            self.log(&line);
        }
    }

    /// An empty unsequenced packet (or one with `msgs`) carrying our acks.
    fn send_unsequenced(&mut self, pi: usize, msgs: Vec<Bits>) {
        if msgs.is_empty() {
            let peer = &self.peers[pi];
            let (ack, ackbits) = peer.acks.header();
            Self::send_raw(&self.sock, peer.addr, &nerd::packet(&[], nerd::NO_SEQ, ack, ackbits));
        } else {
            self.send(pi, msgs, false);
        }
    }

    /// To every other synced peer.
    fn forward(&mut self, from: usize, msgs: &[Bits], sequenced: bool) {
        for j in 0..self.peers.len() {
            if j != from && self.peers[j].synced {
                self.send(j, msgs.to_vec(), sequenced);
            }
        }
    }

    /// An answer to an address that is not a peer (a refusal), acking its packet.
    fn send_to_stranger(&self, addr: SocketAddr, pid: u16, msg: Msg) {
        let (ack, ackbits) = if pid == nerd::NO_SEQ { (nerd::NO_ACK, 0) } else { (pid, 0) };
        Self::send_raw(&self.sock, addr, &nerd::packet(&[msg.write()], nerd::NO_SEQ, ack, ackbits));
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

    /// Forgets a peer: the others delete its objects.
    fn drop_peer(&mut self, pi: usize, why: &str) {
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
                    self.send(j, deletes.clone(), true);
                }
            }
        }
        self.mirror_queue.retain(|(_, addr, _)| *addr != peer.addr);
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
        self.log(&format!("jogadores: {}/{}", c.peers, self.settings.max_players));
    }

    /// An OpenConnection from an address that is not a peer: a new peer, or a refusal.
    fn admit(&mut self, addr: SocketAddr, hdr: &Header, oc: &nerd::OpenConnection) -> Option<usize> {
        let refuse = |relay: &Relay, reason: u8, why: &str| {
            relay.send_to_stranger(
                addr,
                hdr.id,
                Msg::OpenResponse(OpenResponse { flags: oc.flags, reason, attempt: oc.attempt, own_attempt: OWN_ATTEMPT }),
            );
            relay.log(&format!("recusado {} (xuid {}): {why}", net::show(&addr), oc.user));
        };
        if oc.version != nerd::VERSION {
            refuse(self, nerd::REASON_VERSION_MISMATCH, &format!("versão {}", String::from_utf8_lossy(&oc.version)));
            return None;
        }
        if oc.game != nerd::GAME {
            refuse(self, nerd::REASON_GAME_MISMATCH, &format!("jogo {}", String::from_utf8_lossy(&oc.game)));
            return None;
        }
        // the same player from a new address (its NAT changed the port, or it came back). Not
        // for Azimuth's own fixed XUID (and 0): players without an account all share it.
        let shared = oc.user == 0 || oc.user == AZIMUTH_XUID;
        if !shared && let Some(old) = self.peers.iter().position(|p| p.xuid == oc.user) {
            let from = net::show(&self.peers[old].addr);
            self.drop_peer(old, &format!("voltou por {}, antes {from}", net::show(&addr)));
        }
        if self.peers.len() >= self.settings.max_players as usize {
            refuse(self, nerd::REASON_REFUSED, &format!("mundo cheio ({} jogadores)", self.settings.max_players));
            return None;
        }
        let Some(index) = self.free_index() else {
            refuse(self, nerd::REASON_REFUSED, "sem índice livre");
            return None;
        };
        self.peers.push(Peer::new(addr, index, oc.user, Instant::now()));
        self.log(&format!("jogador {index} conectando de {} (xuid {})", net::show(&addr), oc.user));
        Some(self.peers.len() - 1)
    }

    /// The handshake is done: TimeSync and ConnectionSyncDone, then the server's SessionHost and
    /// everything the others own.
    fn on_synced(&mut self, pi: usize) {
        let now_ms = self.now_ms();
        let peer = &mut self.peers[pi];
        peer.synced = true;
        let ts = Msg::TimeSync { index: peer.sync_index, time: now_ms }.write();
        peer.sync_index = peer.sync_index.wrapping_add(1);
        self.send(pi, vec![ts, Msg::SyncDone(OWN_ATTEMPT).write()], false);
        let mut world = vec![
            Msg::Object(ObjectMsg {
                create: true,
                router: nerd::ROUTER_LOBBY,
                handle: Handle::Own(nerd::SESSION_HOST_ID),
                ty: 0,
                seq: 0,
                payload: Bits::new(),
            })
            .write(),
        ];
        for (j, owner) in self.peers.iter().enumerate() {
            if j == pi {
                continue;
            }
            for ((router, id), o) in &owner.objects {
                o.snapshot(*router, Handle::Relay(*id, owner.index), &mut world);
            }
        }
        let n = world.len();
        self.send(pi, world, true);
        let index = self.peers[pi].index;
        self.log(&format!("jogador {index} entrou no mundo ({} mensagens de estado)", n));
        let c = self.counts();
        self.log(&format!("jogadores: {}/{}", c.peers, self.settings.max_players));
    }

    // -----------------------------------------------------------------------------------------
    // Packets in

    pub fn handle(&mut self, data: &[u8], addr: SocketAddr) {
        let Some(hdr) = Header::parse(data) else { return };
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
        let pi = match pi {
            Some(i) => i,
            None => {
                // only an OpenConnection makes a peer
                if first != Some(nerd::MSG_OPEN_CONNECTION) {
                    return;
                }
                let Ok(Msg::OpenConnection(oc)) = Msg::read(&mut body.clone()) else { return };
                match self.admit(addr, &hdr, &oc) {
                    Some(i) => i,
                    None => return,
                }
            }
        };
        let now = Instant::now();
        {
            let peer = &mut self.peers[pi];
            peer.last_seen = now;
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
                    self.debug(&format!("peer {index} OpenConnection xuid {} attempt {}", oc.user, oc.attempt));
                    self.peers[pi].xuid = oc.user;
                    out.push(
                        Msg::OpenResponse(OpenResponse {
                            flags: oc.flags,
                            reason: nerd::REASON_OK,
                            attempt: oc.attempt,
                            own_attempt: OWN_ATTEMPT,
                        })
                        .write(),
                    );
                }
                Msg::OpenResponse(resp) => {
                    self.debug(&format!("peer {index} OpenConnectionResponse reason {}", resp.reason));
                    if resp.reason == nerd::REASON_RESPONSE_ACK && !self.peers[pi].synced {
                        synced_now = true;
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
                Msg::FinaliseSessionEntry => {
                    self.debug(&format!("peer {index} FinaliseSessionEntry"));
                    self.peers[pi].finalise_asked = true;
                    if let Some(m) = self.finalise(pi) {
                        out_seq.push(m);
                    }
                }
            }
        }
        if synced_now {
            self.on_synced(pi);
        }
        if !fwd.is_empty() {
            self.forward(pi, &fwd, true);
        }
        if !ghost.is_empty() {
            self.send(pi, ghost, true);
        }
        if !cars.is_empty() {
            self.forward(pi, &cars, false);
        }
        // the ConsoleSynced Create may have come after FinaliseSessionEntry
        if let Some(m) = self.finalise(pi) {
            out_seq.push(m);
        }
        if !out_seq.is_empty() {
            self.send(pi, out_seq, true);
        }
        if !out.is_empty() || hdr.id != nerd::NO_SEQ {
            self.send_unsequenced(pi, out);
        }
    }

    /// SessionEntryFinalised = true on the peer's own ConsoleSynced object, once it asked and the
    /// object is known. Server updates are taken strictly in order and the first one expected
    /// from us is seq 0 (1417B1E70).
    fn finalise(&mut self, pi: usize) -> Option<Bits> {
        let peer = &mut self.peers[pi];
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

    /// An object message from peer `pi`: the history, and what to forward (None: a duplicate,
    /// or not the sender's own object).
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

    /// Keepalive pings, quiet peers, resends and the mirror's cars.
    pub fn tick(&mut self, now: Instant) {
        // the mirror's cars that are due
        while let Some((due, _, _)) = self.mirror_queue.front() {
            if *due > now {
                break;
            }
            let (_, addr, m) = self.mirror_queue.pop_front().unwrap();
            if let Some(pi) = self.find(addr) {
                self.send(pi, vec![m], false);
            }
        }
        if now.duration_since(self.last_resend_scan) >= Duration::from_millis(50) {
            self.last_resend_scan = now;
            self.resend(now);
        }
        if now.duration_since(self.last_keepalive) >= self.settings.keepalive {
            self.last_keepalive = now;
            let quiet = self.settings.quiet_drop;
            let mut i = 0;
            while i < self.peers.len() {
                if now.duration_since(self.peers[i].last_seen) > quiet {
                    let why = format!("calado por {} s", quiet.as_secs());
                    self.drop_peer(i, &why);
                    continue;
                }
                if self.peers[i].synced {
                    let ping = Msg::Ping(self.now_ms()).write();
                    self.send(i, vec![ping], false);
                }
                i += 1;
            }
        }
    }

    /// Sequenced packets not acked in time go again, in new packets.
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
            let index = peer.index;
            for (msgs, tries) in again {
                self.send_tries(pi, msgs, true, tries);
            }
            if gave_up > 0 {
                self.log(&format!("peer {index}: {gave_up} message(s) never acked, given up"));
            }
        }
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
}
