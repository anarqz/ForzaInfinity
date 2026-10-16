//! STUN (RFC 5389) on the relay's own socket, and what the host learns from it (F3):
//!   - Binding requests to two public STUN servers every `stun_every`: the world's public address
//!     (IPv4 and IPv6) and the NAT's kind (the same public port for both servers: a cone NAT;
//!     different ports: symmetric, which a cone client cannot punch). They also keep the NAT's
//!     mapping for UDP 5101 open.
//!   - Binding requests from a joining player's proxy (its punch) get a Binding success with the
//!     address they came from: the path works, and the player's NAT now lets us in.
//!   - `punch <ip:port>` (a stdin command, from the hook, from the ForzaInfinity server): a burst
//!     of Binding requests to a joining player, so that our NAT lets their packets in.
//!   - The result goes to `dedicated_server.net` next to the exe (nat.rs), which the hook reads
//!     for the world's announcement.
//! STUN packets are recognised before the NerdCore parser and never reach it.

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

pub const MAGIC: u32 = 0x2112_A442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
/// The pre-RFC 5389 code some servers still send.
const ATTR_XOR_MAPPED_ADDRESS_OLD: u16 = 0x8020;
/// A STUN message is at most this long here (ours are 20 or 32 bytes).
const MAX_LEN: usize = 548;

pub type TxId = [u8; 12];

/// A STUN message as far as we read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub kind: u16,
    pub txid: TxId,
    /// XOR-MAPPED-ADDRESS (or MAPPED-ADDRESS) of a response.
    pub mapped: Option<SocketAddr>,
}

/// Whether `data` is a STUN message: the two top bits clear, the magic cookie, and a length that
/// matches the packet. NerdCore packets never look like that (their header starts with the
/// packet id and flags, and the cookie at bytes 4..8 would be a coincidence of 2^-32 on top).
pub fn is_stun(data: &[u8]) -> bool {
    data.len() >= 20
        && data.len() <= MAX_LEN
        && data[0] & 0xC0 == 0
        && u32::from_be_bytes([data[4], data[5], data[6], data[7]]) == MAGIC
        && u16::from_be_bytes([data[2], data[3]]) as usize + 20 == data.len()
        && data.len() % 4 == 0
}

pub fn parse(data: &[u8]) -> Option<Message> {
    if !is_stun(data) {
        return None;
    }
    let kind = u16::from_be_bytes([data[0], data[1]]);
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&data[8..20]);
    let mut mapped = None;
    let mut at = 20;
    while at + 4 <= data.len() {
        let t = u16::from_be_bytes([data[at], data[at + 1]]);
        let n = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        let value = data.get(at + 4..at + 4 + n)?;
        match t {
            ATTR_XOR_MAPPED_ADDRESS | ATTR_XOR_MAPPED_ADDRESS_OLD => {
                if let Some(a) = address(value, Some(&txid)) {
                    mapped = Some(a);
                }
            }
            ATTR_MAPPED_ADDRESS if mapped.is_none() => mapped = address(value, None),
            _ => {}
        }
        at += 4 + n.div_ceil(4) * 4;
    }
    Some(Message { kind, txid, mapped })
}

/// A (XOR-)MAPPED-ADDRESS value: 0, family (1: IPv4, 2: IPv6), port, address.
fn address(v: &[u8], xor: Option<&TxId>) -> Option<SocketAddr> {
    if v.len() < 8 {
        return None;
    }
    let magic = MAGIC.to_be_bytes();
    let mut port = u16::from_be_bytes([v[2], v[3]]);
    if xor.is_some() {
        port ^= (MAGIC >> 16) as u16;
    }
    match v[1] {
        1 => {
            let mut a = [v[4], v[5], v[6], v[7]];
            if xor.is_some() {
                for i in 0..4 {
                    a[i] ^= magic[i];
                }
            }
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(a)), port))
        }
        2 if v.len() >= 20 => {
            let mut a = [0u8; 16];
            a.copy_from_slice(&v[4..20]);
            if let Some(t) = xor {
                for i in 0..16 {
                    a[i] ^= if i < 4 { magic[i] } else { t[i - 4] };
                }
            }
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(a)), port))
        }
        _ => None,
    }
}

pub fn binding_request(txid: &TxId) -> [u8; 20] {
    let mut m = [0u8; 20];
    m[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    m[4..8].copy_from_slice(&MAGIC.to_be_bytes());
    m[8..20].copy_from_slice(txid);
    m
}

/// A Binding success with XOR-MAPPED-ADDRESS = `seen` (how the request reached us).
pub fn binding_success(txid: &TxId, seen: SocketAddr) -> Vec<u8> {
    let seen = canonical(seen);
    let magic = MAGIC.to_be_bytes();
    let mut value = vec![0u8, 0];
    value.extend_from_slice(&(seen.port() ^ (MAGIC >> 16) as u16).to_be_bytes());
    match seen.ip() {
        IpAddr::V4(ip) => {
            value[1] = 1;
            value.extend(ip.octets().iter().zip(magic.iter()).map(|(a, b)| a ^ b));
        }
        IpAddr::V6(ip) => {
            value[1] = 2;
            let key: Vec<u8> = magic.iter().chain(txid.iter()).copied().collect();
            value.extend(ip.octets().iter().zip(key.iter()).map(|(a, b)| a ^ b));
        }
    }
    let mut m = Vec::with_capacity(20 + 4 + value.len());
    m.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    m.extend_from_slice(&((4 + value.len()) as u16).to_be_bytes());
    m.extend_from_slice(&magic);
    m.extend_from_slice(txid);
    m.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    m.extend_from_slice(&(value.len() as u16).to_be_bytes());
    m.extend_from_slice(&value);
    m
}

/// A fresh transaction id (std only: RandomState's per-process random keys, a counter, the time).
pub fn new_txid() -> TxId {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut out = [0u8; 12];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(n);
        h.write_u128(t);
        h.write_usize(i);
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    out
}

/// IPv4-mapped IPv6 (`::ffff:a.b.c.d`, how the dual-stack socket sees IPv4) as plain IPv4.
pub fn canonical(a: SocketAddr) -> SocketAddr {
    match a {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), v6.port()),
            None => a,
        },
        _ => a,
    }
}

/// `a` in the socket's family: IPv4 as `::ffff:a.b.c.d` for the dual-stack (IPv6) socket.
pub fn for_socket(a: SocketAddr, socket_is_v6: bool) -> SocketAddr {
    match (a, socket_is_v6) {
        (SocketAddr::V4(v4), true) => SocketAddr::V6(SocketAddrV6::new(v4.ip().to_ipv6_mapped(), v4.port(), 0, 0)),
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc5769_ipv4_response() {
        // RFC 5769 2.2 (without the SOFTWARE, MESSAGE-INTEGRITY and FINGERPRINT attributes):
        // XOR-MAPPED-ADDRESS 192.0.2.1:32853
        let txid: TxId = [0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae];
        let mut m = vec![0x01, 0x01, 0x00, 0x0c, 0x21, 0x12, 0xa4, 0x42];
        m.extend_from_slice(&txid);
        m.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43]);
        let p = parse(&m).unwrap();
        assert_eq!(p.kind, BINDING_SUCCESS);
        assert_eq!(p.txid, txid);
        assert_eq!(p.mapped, Some("192.0.2.1:32853".parse().unwrap()));
    }

    #[test]
    fn rfc5769_ipv6_response() {
        // RFC 5769 2.3: XOR-MAPPED-ADDRESS [2001:db8:1234:5678:11:2233:4455:6677]:32853
        let txid: TxId = [0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae];
        let mut m = vec![0x01, 0x01, 0x00, 0x18, 0x21, 0x12, 0xa4, 0x42];
        m.extend_from_slice(&txid);
        m.extend_from_slice(&[
            0x00, 0x20, 0x00, 0x14, 0x00, 0x02, 0xa1, 0x47, 0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79, 0xbc, 0x25,
            0xf4, 0xb5, 0xbe, 0xd2, 0xb9, 0xd9,
        ]);
        let p = parse(&m).unwrap();
        assert_eq!(p.mapped, Some("[2001:db8:1234:5678:11:2233:4455:6677]:32853".parse().unwrap()));
    }

    #[test]
    fn our_messages_round_trip() {
        let t = new_txid();
        assert_ne!(t, new_txid());
        let r = binding_request(&t);
        assert!(is_stun(&r));
        assert_eq!(parse(&r).unwrap(), Message { kind: BINDING_REQUEST, txid: t, mapped: None });
        for seen in ["203.0.113.5:5100", "[::ffff:203.0.113.5]:5100", "[2001:db8::7]:61000"] {
            let seen: SocketAddr = seen.parse().unwrap();
            let s = binding_success(&t, seen);
            assert!(is_stun(&s));
            let p = parse(&s).unwrap();
            assert_eq!((p.kind, p.txid, p.mapped), (BINDING_SUCCESS, t, Some(canonical(seen))));
        }
    }

    #[test]
    fn not_stun() {
        assert!(!is_stun(&[0u8; 19]));
        let mut r = binding_request(&new_txid()).to_vec();
        r[4] ^= 1;
        assert!(!is_stun(&r));
        let mut r = binding_request(&new_txid()).to_vec();
        r.push(0);
        assert!(!is_stun(&r));
        // a truncated attribute: not read past the end
        let mut m = binding_request(&new_txid()).to_vec();
        m[3] = 8;
        m.extend_from_slice(&[0x00, 0x20, 0x00, 0x40, 0, 0, 0, 0]);
        assert!(is_stun(&m));
        assert_eq!(parse(&m), None);
        // garbage of every length never panics
        for n in 0..64 {
            let junk: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let _ = parse(&junk);
        }
    }

    #[test]
    fn families() {
        let v4: SocketAddr = "198.51.100.2:3478".parse().unwrap();
        let mapped = for_socket(v4, true);
        assert_eq!(mapped, "[::ffff:198.51.100.2]:3478".parse().unwrap());
        assert_eq!(canonical(mapped), v4);
        assert_eq!(for_socket(v4, false), v4);
    }
}
