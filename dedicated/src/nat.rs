//! What the host's network looks like from the Internet, and the punches:
//! STUN rounds on the relay's socket (stun.rs), the router's port mapping (upnp.rs, its own
//! thread) and the joining players' punches. The relay owns one Nat and feeds it every STUN
//! packet and its timer; the result is written to `dedicated_server.net` for the hook:
//!
//!   nat cone                      cone | symmetric | unknown (no two STUN answers yet)
//!   public 203.0.113.5:5101       one line per public address STUN saw (IPv4, IPv6)
//!   upnp 203.0.113.5:5101         upnp <ip:port> | upnp none | upnp off | upnp pending |
//!                                 upnp failed <why>
//!   othernat 1                    the router's own external address is not public (double NAT
//!                                 or CGNAT): its mapping alone does not open the world
//!
//! Nothing here sends anything but STUN Binding requests (to the STUN servers and, for a punch,
//! to the player) and Binding successes (to whoever sent us a request).

use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use crate::{
    stun::{self, TxId},
    upnp,
};

/// Public STUN servers (no server of ours). Two, for the NAT's kind.
pub const STUN_SERVERS: [&str; 2] = ["stun.l.google.com:19302", "stun.cloudflare.com:3478"];
/// A punch: this many Binding requests, this far apart.
const PUNCH_SENDS: u8 = 10;
const PUNCH_GAP: Duration = Duration::from_millis(200);
const PUNCHES_MAX: usize = 32;
const PENDING_MAX: usize = 16;
/// Binding requests answered per second (a flood of them gets no answers).
const ANSWERS_PER_SECOND: u32 = 50;

#[derive(Debug, Clone)]
pub struct NatSettings {
    pub stun_servers: Vec<String>,
    pub stun_every: Duration,
    pub upnp: bool,
    pub port: u16,
    /// Where the summary goes (None: nowhere, as in the tests).
    pub state_file: Option<PathBuf>,
}

impl NatSettings {
    pub fn new(port: u16, upnp: bool, state_file: Option<PathBuf>) -> NatSettings {
        NatSettings {
            stun_servers: STUN_SERVERS.iter().map(|s| s.to_string()).collect(),
            stun_every: Duration::from_secs(25),
            upnp,
            port,
            state_file,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatKind {
    Unknown,
    Cone,
    Symmetric,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Upnp {
    Off,
    Pending,
    Done(upnp::Outcome),
}

pub struct Nat {
    settings: NatSettings,
    socket_is_v6: bool,
    resolved: Option<Receiver<Vec<SocketAddr>>>,
    servers: Vec<SocketAddr>,
    last_round: Option<Instant>,
    /// (transaction, server, sent)
    pending: Vec<(TxId, SocketAddr, Instant)>,
    /// (server, our address as it saw it): the last answer of each server.
    seen: Vec<(SocketAddr, SocketAddr)>,
    /// (player, sends left, next send, transaction)
    punches: Vec<(SocketAddr, u8, Instant, TxId)>,
    upnp_rx: Option<Receiver<upnp::Outcome>>,
    upnp: Upnp,
    answers: (Instant, u32),
    written: String,
    logs: Vec<String>,
}

impl Nat {
    /// Starts the STUN servers' lookup and the router's mapping (each in a thread).
    pub fn new(settings: NatSettings, socket_is_v6: bool) -> Nat {
        let (tx, rx) = mpsc::channel();
        let names = settings.stun_servers.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            for n in names {
                // one IPv4 address per server (the NAT's kind compares IPv4 ports), plus one IPv6
                if let Ok(addrs) = n.to_socket_addrs() {
                    let addrs: Vec<SocketAddr> = addrs.collect();
                    out.extend(addrs.iter().find(|a| a.is_ipv4()).copied());
                    out.extend(addrs.iter().find(|a| a.is_ipv6()).copied());
                }
            }
            let _ = tx.send(out);
        });
        let (upnp_rx, state) = if settings.upnp {
            let (tx, rx) = mpsc::channel();
            let port = settings.port;
            std::thread::spawn(move || upnp::run(port, |o| drop(tx.send(o))));
            (Some(rx), Upnp::Pending)
        } else {
            (None, Upnp::Off)
        };
        let mut nat = Nat {
            settings,
            socket_is_v6,
            resolved: Some(rx),
            servers: Vec::new(),
            last_round: None,
            pending: Vec::new(),
            seen: Vec::new(),
            punches: Vec::new(),
            upnp_rx,
            upnp: state,
            answers: (Instant::now(), 0),
            written: String::new(),
            logs: Vec::new(),
        };
        nat.write_state();
        nat
    }

    /// A packet the relay received. True when it was STUN (handled here; never NerdCore).
    pub fn on_packet(&mut self, sock: &UdpSocket, data: &[u8], from: SocketAddr, now: Instant) -> bool {
        if !stun::is_stun(data) {
            return false;
        }
        let Some(m) = stun::parse(data) else { return true };
        match m.kind {
            stun::BINDING_SUCCESS => {
                let from = stun::canonical(from);
                if let Some(i) = self.pending.iter().position(|(t, s, _)| *t == m.txid && *s == from) {
                    self.pending.swap_remove(i);
                    if let Some(mapped) = m.mapped {
                        self.seen.retain(|(s, _)| *s != from);
                        self.seen.push((from, stun::canonical(mapped)));
                        self.write_state();
                    }
                } else if let Some(p) = self.punches.iter_mut().find(|p| p.3 == m.txid && stun::canonical(p.0) == from) {
                    // the player's proxy answered our punch: the path is open both ways
                    p.1 = 0;
                    self.logs.push(format!("punch: {} respondeu", crate::net::show(&from)));
                }
            }
            stun::BINDING_REQUEST => {
                if now.duration_since(self.answers.0) >= Duration::from_secs(1) {
                    self.answers = (now, 0);
                }
                if self.answers.1 < ANSWERS_PER_SECOND {
                    self.answers.1 += 1;
                    let _ = sock.send_to(&stun::binding_success(&m.txid, from), from);
                }
            }
            _ => {}
        }
        true
    }

    /// The timer: STUN rounds, punches, the router's news.
    pub fn tick(&mut self, sock: &UdpSocket, now: Instant) {
        if let Some(rx) = &self.resolved
            && let Ok(servers) = rx.try_recv()
        {
            if servers.is_empty() {
                self.logs.push("STUN: nenhum servidor STUN encontrado (sem DNS ou sem internet?)".into());
            }
            self.servers = servers;
            self.resolved = None;
        }
        if !self.servers.is_empty() && self.last_round.is_none_or(|t| now.duration_since(t) >= self.settings.stun_every) {
            self.last_round = Some(now);
            self.pending.retain(|(_, _, sent)| now.duration_since(*sent) < Duration::from_secs(10));
            for &server in &self.servers {
                if server.is_ipv6() && !self.socket_is_v6 {
                    continue;
                }
                let t = stun::new_txid();
                if self.pending.len() < PENDING_MAX {
                    self.pending.push((t, server, now));
                }
                let _ = sock.send_to(&stun::binding_request(&t), stun::for_socket(server, self.socket_is_v6));
            }
        }
        for p in self.punches.iter_mut() {
            if p.1 > 0 && now >= p.2 {
                let _ = sock.send_to(&stun::binding_request(&p.3), stun::for_socket(p.0, self.socket_is_v6));
                p.1 -= 1;
                p.2 = now + PUNCH_GAP;
            }
        }
        self.punches.retain(|p| p.1 > 0);
        if let Some(rx) = &self.upnp_rx {
            let mut news = None;
            while let Ok(o) = rx.try_recv() {
                news = Some(o);
            }
            if let Some(o) = news {
                let changed = self.upnp != Upnp::Done(o.clone());
                if changed {
                    self.logs.push(match &o {
                        upnp::Outcome::Mapped { external_ip, external_port, .. } => {
                            format!("UPnP: o roteador abriu a porta (mundo em {})", crate::net::show(&SocketAddr::new(*external_ip, *external_port)))
                        }
                        upnp::Outcome::NotFound => "UPnP: nenhum roteador com UPnP respondeu".into(),
                        upnp::Outcome::Failed(why) => format!("UPnP: {why}"),
                    });
                }
                self.upnp = Upnp::Done(o);
                if changed {
                    self.write_state();
                }
            }
        }
    }

    /// `punch <ip:port>`: Binding requests to a joining player for a couple of seconds.
    pub fn punch(&mut self, to: SocketAddr, now: Instant) {
        let to = stun::canonical(to);
        if to.port() == 0 || to.ip().is_unspecified() || to.ip().is_multicast() {
            return;
        }
        if let Some(p) = self.punches.iter_mut().find(|p| p.0 == to) {
            p.1 = PUNCH_SENDS;
            return;
        }
        if self.punches.len() >= PUNCHES_MAX {
            self.punches.remove(0);
        }
        self.punches.push((to, PUNCH_SENDS, now, stun::new_txid()));
    }

    pub fn kind(&self) -> NatKind {
        let v4: Vec<&SocketAddr> = self.seen.iter().filter(|(s, _)| s.is_ipv4()).map(|(_, m)| m).collect();
        match v4.as_slice() {
            [a, b, ..] if a.port() == b.port() && a.ip() == b.ip() => NatKind::Cone,
            [_, _, ..] => NatKind::Symmetric,
            _ => NatKind::Unknown,
        }
    }

    /// The public addresses STUN saw (one per family).
    pub fn public(&self) -> Vec<SocketAddr> {
        let mut out: Vec<SocketAddr> = Vec::new();
        for (_, m) in &self.seen {
            if !out.iter().any(|o| o.is_ipv4() == m.is_ipv4()) {
                out.push(*m);
            }
        }
        out
    }

    pub fn state_text(&self) -> String {
        let mut t = format!(
            "nat {}\n",
            match self.kind() {
                NatKind::Cone => "cone",
                NatKind::Symmetric => "symmetric",
                NatKind::Unknown => "unknown",
            }
        );
        for p in self.public() {
            t.push_str(&format!("public {}\n", crate::net::show(&p)));
        }
        match &self.upnp {
            Upnp::Off => t.push_str("upnp off\n"),
            Upnp::Pending => t.push_str("upnp pending\n"),
            Upnp::Done(upnp::Outcome::NotFound) => t.push_str("upnp none\n"),
            Upnp::Done(upnp::Outcome::Failed(why)) => t.push_str(&format!("upnp failed {}\n", why.replace('\n', " "))),
            Upnp::Done(upnp::Outcome::Mapped { external_ip, external_port, .. }) => {
                t.push_str(&format!("upnp {}\n", crate::net::show(&SocketAddr::new(*external_ip, *external_port))));
                if upnp::behind_another_nat(*external_ip) {
                    t.push_str("othernat 1\n");
                }
            }
        }
        t
    }

    fn write_state(&mut self) {
        let text = self.state_text();
        if text == self.written {
            return;
        }
        let first = self.written.is_empty();
        self.written = text.clone();
        if !first {
            let summary = text.trim_end().replace('\n', ", ");
            self.logs.push(format!("rede: {summary}"));
        }
        if let Some(path) = &self.settings.state_file {
            let tmp = path.with_extension("net.tmp");
            if std::fs::write(&tmp, text.as_bytes()).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(path);
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Lines for the relay's log.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.logs)
    }

    /// The router's external address, when it forwarded the world.
    pub fn upnp_address(&self) -> Option<IpAddr> {
        match &self.upnp {
            Upnp::Done(upnp::Outcome::Mapped { external_ip, .. }) => Some(*external_ip),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn quiet(servers: Vec<String>) -> NatSettings {
        NatSettings { stun_servers: servers, stun_every: Duration::from_millis(50), upnp: false, port: 5101, state_file: None }
    }

    /// Two fake STUN servers on loopback: the first sees one port, the second `second_port`
    /// (Some: a symmetric NAT) or the same one.
    fn fake_stun(second_port: Option<u16>) -> (Vec<String>, Vec<std::thread::JoinHandle<()>>) {
        let mut names = Vec::new();
        let mut threads = Vec::new();
        for k in 0..2 {
            let s = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            names.push(s.local_addr().unwrap().to_string());
            threads.push(std::thread::spawn(move || {
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0u8; 600];
                if let Ok((n, from)) = s.recv_from(&mut buf) {
                    let m = stun::parse(&buf[..n]).unwrap();
                    let mut seen = from;
                    if k == 1 && let Some(p) = second_port {
                        seen.set_port(p);
                    }
                    s.send_to(&stun::binding_success(&m.txid, seen), from).unwrap();
                }
            }));
        }
        (names, threads)
    }

    fn run_until(nat: &mut Nat, sock: &UdpSocket, done: impl Fn(&Nat) -> bool) {
        sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 2048];
        while Instant::now() < end && !done(nat) {
            if let Ok((n, from)) = sock.recv_from(&mut buf) {
                assert!(nat.on_packet(sock, &buf[..n], from, Instant::now()));
            }
            nat.tick(sock, Instant::now());
        }
    }

    #[test]
    fn cone_nat_and_public_address() {
        let (names, threads) = fake_stun(None);
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut nat = Nat::new(quiet(names), false);
        assert_eq!(nat.kind(), NatKind::Unknown);
        run_until(&mut nat, &sock, |n| n.kind() != NatKind::Unknown);
        assert_eq!(nat.kind(), NatKind::Cone);
        assert_eq!(nat.public(), vec![sock.local_addr().unwrap()]);
        let text = nat.state_text();
        assert!(text.starts_with("nat cone\npublic 127.0.0.1:"), "{text}");
        assert!(text.ends_with("upnp off\n"));
        threads.into_iter().for_each(|t| t.join().unwrap());
    }

    #[test]
    fn symmetric_nat() {
        let (names, threads) = fake_stun(Some(40000));
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut nat = Nat::new(quiet(names), false);
        run_until(&mut nat, &sock, |n| n.kind() != NatKind::Unknown);
        assert_eq!(nat.kind(), NatKind::Symmetric);
        threads.into_iter().for_each(|t| t.join().unwrap());
    }

    #[test]
    fn answers_requests_and_ignores_strangers() {
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut nat = Nat::new(quiet(vec![]), false);
        let t = stun::new_txid();
        assert!(nat.on_packet(&sock, &stun::binding_request(&t), peer.local_addr().unwrap(), Instant::now()));
        let mut buf = [0u8; 600];
        let (n, _) = peer.recv_from(&mut buf).unwrap();
        let m = stun::parse(&buf[..n]).unwrap();
        assert_eq!((m.kind, m.txid, m.mapped), (stun::BINDING_SUCCESS, t, Some(peer.local_addr().unwrap())));
        // a success nobody asked for changes nothing
        let fake = stun::binding_success(&stun::new_txid(), "203.0.113.1:9".parse().unwrap());
        assert!(nat.on_packet(&sock, &fake, peer.local_addr().unwrap(), Instant::now()));
        assert!(nat.public().is_empty());
        // NerdCore is not ours
        assert!(!nat.on_packet(&sock, &[0x00, 0x01, 0x02, 0x03], peer.local_addr().unwrap(), Instant::now()));
        // a flood of requests: at most ANSWERS_PER_SECOND answers
        let now = Instant::now();
        for _ in 0..(ANSWERS_PER_SECOND * 3) {
            nat.on_packet(&sock, &stun::binding_request(&stun::new_txid()), peer.local_addr().unwrap(), now);
        }
        peer.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let mut answers = 0;
        while peer.recv_from(&mut buf).is_ok() {
            answers += 1;
        }
        assert!(answers <= ANSWERS_PER_SECOND, "{answers}");
    }

    #[test]
    fn punches_until_answered() {
        let sock = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let player = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        player.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut nat = Nat::new(quiet(vec![]), false);
        let at = player.local_addr().unwrap();
        nat.punch(at, Instant::now());
        nat.punch("0.0.0.0:5".parse().unwrap(), Instant::now());
        nat.punch("1.2.3.4:0".parse().unwrap(), Instant::now());
        nat.tick(&sock, Instant::now());
        let mut buf = [0u8; 600];
        let (n, from) = player.recv_from(&mut buf).unwrap();
        let m = stun::parse(&buf[..n]).unwrap();
        assert_eq!(m.kind, stun::BINDING_REQUEST);
        // the player answers: no more punches
        player.send_to(&stun::binding_success(&m.txid, from), from).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (n, from) = sock.recv_from(&mut buf).unwrap();
        assert!(nat.on_packet(&sock, &buf[..n], from, Instant::now()));
        nat.tick(&sock, Instant::now() + Duration::from_secs(1));
        assert!(nat.punches.is_empty());
        assert_eq!(nat.take_logs().iter().filter(|l| l.starts_with("punch:")).count(), 1);
    }

    #[test]
    fn punches_are_bounded() {
        let mut nat = Nat::new(quiet(vec![]), false);
        for k in 0..(PUNCHES_MAX as u16 + 10) {
            nat.punch(SocketAddr::new("198.51.100.1".parse().unwrap(), 1000 + k), Instant::now());
        }
        assert_eq!(nat.punches.len(), PUNCHES_MAX);
    }

    #[test]
    fn state_file_is_written() {
        let dir = std::env::temp_dir().join(format!("fh2nat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dedicated_server.net");
        let mut s = quiet(vec![]);
        s.state_file = Some(path.clone());
        let nat = Nat::new(s, false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nat unknown\nupnp off\n");
        drop(nat);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upnp_lines() {
        let mut nat = Nat::new(quiet(vec![]), false);
        nat.upnp = Upnp::Done(upnp::Outcome::Mapped { external_ip: "100.70.1.2".parse().unwrap(), external_port: 5101, lease: 1800 });
        assert!(nat.state_text().ends_with("upnp 100.70.1.2:5101\nothernat 1\n"));
        nat.upnp = Upnp::Done(upnp::Outcome::NotFound);
        assert!(nat.state_text().ends_with("upnp none\n"));
        nat.upnp = Upnp::Done(upnp::Outcome::Failed("x\ny".into()));
        assert!(nat.state_text().ends_with("upnp failed x y\n"));
    }
}
