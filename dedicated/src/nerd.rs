//! NerdCore (Engine\NetV2\NerdCore) packets and messages, as the game reads and writes them.
//! Port of fh2-mods `tools/online/server/nerd.py` (the addresses are the game's, 1.0.0.64).
//!
//! Packet: an 11-byte header (14176D3E0), little-endian: id u16, ack u16, ack bits u32, message
//! count u8, packet size u16 (the whole packet, in bytes). An id of 0xFFFF skips the receiver's
//! sequence check (Connection::ReadPacketHeader, 1417672B0): unsequenced. Then the messages, bit
//! packed (bits.rs): a 7-bit id and the fields, with no length, so a message whose layout is not
//! known ends the parse of its packet.
//!
//! Handshake (client -> server -> client ...):
//!   OpenConnection (0) -> OpenConnectionResponse (1, reason Ok, echoing the client's attempt and
//!   giving ours) -> the client's OpenConnectionResponse with reason 6 (ResponseAck) -> TimeSync (5)
//!   and ConnectionSyncDone (6, our attempt again) -> Ping (3) / Pong (4) both ways.
//!
//! Objects (NerdLib SyncedObject): ObjectCreate (7), ObjectUpdate (8), ObjectDelete (9), and the
//! cars, CarPredictionFull (15). Layouts at each type below.

use crate::bits::{BitError, BitResult, Bits, Reader};

pub const HEADER_LEN: usize = 11;
/// Packet id of an unsequenced packet.
pub const NO_SEQ: u16 = 0xFFFF;
/// The ack field when nothing was received yet.
pub const NO_ACK: u16 = 0xFFFE;

pub const MSG_OPEN_CONNECTION: u8 = 0;
pub const MSG_OPEN_RESPONSE: u8 = 1;
pub const MSG_CLOSE: u8 = 2;
pub const MSG_PING: u8 = 3;
pub const MSG_PONG: u8 = 4;
pub const MSG_TIME_SYNC: u8 = 5;
pub const MSG_SYNC_DONE: u8 = 6;
pub const MSG_OBJECT_CREATE: u8 = 7;
pub const MSG_OBJECT_UPDATE: u8 = 8;
pub const MSG_OBJECT_DELETE: u8 = 9;
pub const MSG_CAR: u8 = 15;
pub const MSG_FINALISE_SESSION_ENTRY: u8 = 34;

/// The message table's names (142F4DB20) for the ids seen so far.
pub fn msg_name(id: u8) -> &'static str {
    match id {
        0 => "OpenConnection",
        1 => "OpenConnectionResponse",
        2 => "CloseConnection",
        3 => "Ping",
        4 => "Pong",
        5 => "TimeSync",
        6 => "ConnectionSyncDone",
        7 => "ObjectCreate",
        8 => "ObjectUpdate",
        9 => "ObjectDelete",
        10 => "ObjectRequestOwnership",
        11 => "ObjectGrantOwnership",
        12 => "ObjectTransferOwnership",
        13 => "ObjectNeedsUpdates",
        14 => "PlayerInput",
        15 => "CarPredictionFull",
        34 => "FinaliseSessionEntry",
        35 => "MsgRepairDamage",
        53 => "RequestSendFullWeather",
        54 => "FullWeather",
        71 => "ServerTrafficMessage",
        72 => "ClientTrafficMessage",
        _ => "?",
    }
}

/// What the client sends as OpenConnection's version and game (the build hash, also in the
/// gameserverds variant `Release_6CC152FD.FreeRoam`).
pub const VERSION: &[u8] = b"6CC152FD";
pub const GAME: &[u8] = b"Flagship";

/// OpenConnectionResponse reasons (141765B30 / 141765EC0).
pub const REASON_OK: u8 = 0;
pub const REASON_VERSION_MISMATCH: u8 = 1;
pub const REASON_GAME_MISMATCH: u8 = 2;
pub const REASON_BANNED: u8 = 3;
/// 4 and 5 are both "refused" in nerd.py's notes; which text the game shows for each is not
/// known. 4 is used for a full server.
// TODO(F1): confirm 4 vs 5 in 141765EC0 and what the player sees.
pub const REASON_REFUSED: u8 = 4;
/// Not a refusal: the client's answer to Ok, after which both sides count as connected.
pub const REASON_RESPONSE_ACK: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub id: u16,
    pub ack: u16,
    pub ackbits: u32,
    pub count: u8,
    pub size: u16,
}

impl Header {
    pub fn parse(data: &[u8]) -> Option<Header> {
        if data.len() < HEADER_LEN {
            return None;
        }
        let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]);
        Some(Header {
            id: u16_at(0),
            ack: u16_at(2),
            ackbits: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            count: data[8],
            size: u16_at(9),
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.ack.to_le_bytes());
        out.extend_from_slice(&self.ackbits.to_le_bytes());
        out.push(self.count);
        out.extend_from_slice(&self.size.to_le_bytes());
    }
}

/// A whole packet: header plus the concatenated messages.
pub fn packet(msgs: &[Bits], id: u16, ack: u16, ackbits: u32) -> Vec<u8> {
    let mut body = Bits::new();
    for m in msgs {
        body.append(m);
    }
    let size = HEADER_LEN + body.as_bytes().len();
    let mut out = Vec::with_capacity(size);
    Header { id, ack, ackbits, count: msgs.len() as u8, size: size as u16 }.write(&mut out);
    out.extend_from_slice(body.as_bytes());
    out
}

// ---------------------------------------------------------------------------------------------
// Connection messages

/// OpenConnection (0; 14176D450): version str (max 38), game str (max 16), 2 bits, u64 user id
/// (the XUID), u32, u32, u32 connect attempt (the client's [conn+0x24]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenConnection {
    pub version: Vec<u8>,
    pub game: Vec<u8>,
    pub flags: u8,
    pub user: u64,
    pub a: u32,
    pub b: u32,
    pub attempt: u32,
}

/// OpenConnectionResponse (1; 14176D770): 2 bits (echoed), 3-bit reason, u32 (must equal the
/// client's attempt), u32 (the sender's own attempt; the client keeps it in [conn+0x28]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenResponse {
    pub flags: u8,
    pub reason: u8,
    pub attempt: u32,
    pub own_attempt: u32,
}

// ---------------------------------------------------------------------------------------------
// Objects

/// Routers (manager ids, 4 bits; table at 1422E4410): (id bits, type bits; 0 = one type, no type
/// field on Create). 1, 2, 3: the player's objects (3 is ConsoleSynced); 6: the lobby (type 0
/// SessionHost, 1 GameOptions; only the server makes these); 9: Road Trip (3 types); 12: Bucket
/// List (2 types). 8, 10 (weather), 11 are registered too, widths not known yet: a message for
/// them ends the parse of its packet.
pub fn router_widths(router: u8) -> Option<(u32, u32)> {
    match router {
        1..=3 => Some((7, 0)),
        6 => Some((2, 2)),
        9 | 12 => Some((4, 2)),
        _ => None,
    }
}

pub const ROUTER_CONSOLE_SYNCED: u8 = 3;
pub const ROUTER_LOBBY: u8 = 6;
/// The server's SessionHost: router 6, id 0, type 0.
pub const SESSION_HOST_ID: u32 = 0;

/// ConsoleSynced (router 3; NerdLib 1417DE550): 9 bool vars, a 4-bit var index. 0 RenderReady,
/// 1 SessionEntryFinalised, 2 ParticipatingInRace, 3 RaceInProgress, 4 Ready,
/// 5 HasLoadedCarsForAllParticipants, 6 HasGrid, 7 AvailableArbitration, 8 MatchTicketArbiter.
/// 1, 2 and 4 are the server's (SyncedBool<0,1>).
pub const CONSOLE_SESSION_ENTRY_FINALISED: u8 = 1;
pub const CONSOLE_VAR_INDEX_BITS: u32 = 4;

/// The u14 payload size, in bits.
pub const MAX_PAYLOAD_BITS: usize = (1 << 14) - 1;

/// Object handle (write 1417B0E40, read in 1417AB160): two flag bits a, b, the id (the router's
/// width), and for the relay form an 8-bit peer index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    /// (0, 0): the object belongs to the connection it came from.
    Own(u32),
    /// (0, 1) + peer index (flag 30): the object belongs to that peer (the relay's form).
    Relay(u32, u8),
    /// (1, 1) (flag 29): a server update of the receiver's own object
    /// (SyncedObject::PackServerUpdateObjectId, 1417B1A20).
    Server(u32),
    /// (1, 0) (flag 31): an ownership transfer.
    Transfer(u32),
}

impl Handle {
    pub fn id(&self) -> u32 {
        match *self {
            Handle::Own(id) | Handle::Relay(id, _) | Handle::Server(id) | Handle::Transfer(id) => id,
        }
    }

    pub fn read(r: &mut Reader, id_bits: u32) -> BitResult<Handle> {
        let a = r.bool()?;
        let b = r.bool()?;
        let id = r.u32(id_bits)?;
        Ok(match (a, b) {
            (false, false) => Handle::Own(id),
            (false, true) => Handle::Relay(id, r.u8(8)?),
            (true, true) => Handle::Server(id),
            (true, false) => Handle::Transfer(id),
        })
    }

    pub fn write(&self, w: &mut Bits, id_bits: u32) {
        let (a, b) = match self {
            Handle::Own(_) => (false, false),
            Handle::Relay(..) => (false, true),
            Handle::Server(_) => (true, true),
            Handle::Transfer(_) => (true, false),
        };
        w.bool(a).bool(b).push(self.id() as u64, id_bits);
        if let Handle::Relay(_, peer) = self {
            w.push(*peer as u64, 8);
        }
    }
}

/// ObjectCreate (7; receive 1417AB160) and ObjectUpdate (8; 1417B1E70): router (4 bits), handle,
/// [Create, when the router has more than one type: type id], seq u8, payload size u14 (bits),
/// payload. The payload (1417B2510) is a list of (var index, var data) read until its size is
/// used up, so payloads can be concatenated: a later entry for a var overwrites an earlier one.
///
/// Seqs count mod 255. A Create sets the object's link seq to seq-1 and applies itself as the
/// update `seq`; an update applies only when it is link+1, a newer one waits for the gap to fill,
/// an older one is dropped (1417B1E70, 1417B2160). A Create for an existing object replaces it
/// unless the object's seq is newer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMsg {
    pub create: bool,
    pub router: u8,
    pub handle: Handle,
    /// Create only, for routers with a type field; 0 otherwise.
    pub ty: u8,
    pub seq: u8,
    pub payload: Bits,
}

/// ObjectDelete (9; 1417AB560): router, handle, seq u8. The receiver deletes the object unless
/// its seq is newer, and remembers the delete (a later Create with an older seq is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectDelete {
    pub router: u8,
    pub handle: Handle,
    pub seq: u8,
}

/// CarPredictionFull (15; send 141808170, receive 1417CF0A0 / 141808240): the car's handle (a
/// 7-bit id: the player's car object), the prediction (678 bits, fixed: 14181FFE0 96 + 141822400
/// 326 + 4 x 141825F00 64), then a u32. Sent unsequenced, about 30 a second.
pub const CAR_BODY_BITS: usize = 678 + 32;
pub const CAR_ID_BITS: u32 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Car {
    pub handle: Handle,
    pub body: Bits,
}

/// Seq arithmetic mod 255, as the receiver does it: `a` is newer than `b` when it is ahead by
/// less than 128.
pub fn seq_next(s: u8) -> u8 {
    ((s as u16 + 1) % 255) as u8
}

pub fn seq_newer(a: u8, b: u8) -> bool {
    (b < a && a - b < 0x80) || (a < b && b - a > 0x80)
}

// ---------------------------------------------------------------------------------------------
// Messages

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    OpenConnection(OpenConnection),
    OpenResponse(OpenResponse),
    /// CloseConnection (2): its fields are not known; the connection ends either way.
    Close,
    /// Ping (3; 14176DA90): the sender's time, ms.
    Ping(u32),
    /// Pong (4; 14176DBA0): the ping's time echoed, the sender's time. The client pings with its
    /// time and gets Pong(its time, ours); our pings get Pong(0, ours) back (1417666C0).
    Pong { echo: u32, time: u32 },
    /// TimeSync (5; 14176DCF0, server to client): u16 index, u32 server time (ms).
    TimeSync { index: u16, time: u32 },
    /// ConnectionSyncDone (6; 14176DD80, server to client): u32, equal to the attempt the server
    /// gave in its OpenConnectionResponse.
    SyncDone(u32),
    Object(ObjectMsg),
    Delete(ObjectDelete),
    Car(Car),
    /// FinaliseSessionEntry (34): no fields; the console has loaded the session.
    FinaliseSessionEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// A message id whose layout is not known: the rest of the packet cannot be read.
    UnknownMsg(u8),
    /// An object message for a router whose widths are not known.
    UnknownRouter(u8),
    Bits(BitError),
}

impl From<BitError> for ReadError {
    fn from(e: BitError) -> Self {
        ReadError::Bits(e)
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::UnknownMsg(id) => write!(f, "unknown message {id} ({})", msg_name(*id)),
            ReadError::UnknownRouter(r) => write!(f, "unknown router {r}"),
            ReadError::Bits(e) => write!(f, "{e}"),
        }
    }
}

impl Msg {
    pub fn id(&self) -> u8 {
        match self {
            Msg::OpenConnection(_) => MSG_OPEN_CONNECTION,
            Msg::OpenResponse(_) => MSG_OPEN_RESPONSE,
            Msg::Close => MSG_CLOSE,
            Msg::Ping(_) => MSG_PING,
            Msg::Pong { .. } => MSG_PONG,
            Msg::TimeSync { .. } => MSG_TIME_SYNC,
            Msg::SyncDone(_) => MSG_SYNC_DONE,
            Msg::Object(o) if o.create => MSG_OBJECT_CREATE,
            Msg::Object(_) => MSG_OBJECT_UPDATE,
            Msg::Delete(_) => MSG_OBJECT_DELETE,
            Msg::Car(_) => MSG_CAR,
            Msg::FinaliseSessionEntry => MSG_FINALISE_SESSION_ENTRY,
        }
    }

    /// One message: the 7-bit id, then its fields.
    pub fn read(r: &mut Reader) -> Result<Msg, ReadError> {
        let id = r.u8(7)?;
        Ok(match id {
            MSG_OPEN_CONNECTION => Msg::OpenConnection(OpenConnection {
                version: r.string(38)?,
                game: r.string(16)?,
                flags: r.u8(2)?,
                user: r.bits(64)?,
                a: r.u32(32)?,
                b: r.u32(32)?,
                attempt: r.u32(32)?,
            }),
            MSG_OPEN_RESPONSE => Msg::OpenResponse(OpenResponse {
                flags: r.u8(2)?,
                reason: r.u8(3)?,
                attempt: r.u32(32)?,
                own_attempt: r.u32(32)?,
            }),
            MSG_CLOSE => Msg::Close,
            MSG_PING => Msg::Ping(r.u32(32)?),
            MSG_PONG => Msg::Pong { echo: r.u32(32)?, time: r.u32(32)? },
            MSG_TIME_SYNC => Msg::TimeSync { index: r.bits(16)? as u16, time: r.u32(32)? },
            MSG_SYNC_DONE => Msg::SyncDone(r.u32(32)?),
            MSG_OBJECT_CREATE | MSG_OBJECT_UPDATE => {
                let create = id == MSG_OBJECT_CREATE;
                let router = r.u8(4)?;
                let (id_bits, type_bits) = router_widths(router).ok_or(ReadError::UnknownRouter(router))?;
                let handle = Handle::read(r, id_bits)?;
                let ty = if create && type_bits > 0 { r.u8(type_bits)? } else { 0 };
                let seq = r.u8(8)?;
                let n = r.bits(14)? as usize;
                let payload = r.take(n)?;
                Msg::Object(ObjectMsg { create, router, handle, ty, seq, payload })
            }
            MSG_OBJECT_DELETE => {
                let router = r.u8(4)?;
                let (id_bits, _) = router_widths(router).ok_or(ReadError::UnknownRouter(router))?;
                let handle = Handle::read(r, id_bits)?;
                Msg::Delete(ObjectDelete { router, handle, seq: r.u8(8)? })
            }
            MSG_CAR => {
                let handle = Handle::read(r, CAR_ID_BITS)?;
                Msg::Car(Car { handle, body: r.take(CAR_BODY_BITS)? })
            }
            MSG_FINALISE_SESSION_ENTRY => Msg::FinaliseSessionEntry,
            other => return Err(ReadError::UnknownMsg(other)),
        })
    }

    pub fn write(&self) -> Bits {
        let mut w = Bits::new();
        w.push(self.id() as u64, 7);
        match self {
            Msg::OpenConnection(oc) => {
                w.string(&oc.version).string(&oc.game).push(oc.flags as u64, 2).push(oc.user, 64);
                w.push(oc.a as u64, 32).push(oc.b as u64, 32).push(oc.attempt as u64, 32);
            }
            Msg::OpenResponse(o) => {
                w.push(o.flags as u64, 2).push(o.reason as u64, 3);
                w.push(o.attempt as u64, 32).push(o.own_attempt as u64, 32);
            }
            Msg::Close | Msg::FinaliseSessionEntry => {}
            Msg::Ping(t) => {
                w.push(*t as u64, 32);
            }
            Msg::Pong { echo, time } => {
                w.push(*echo as u64, 32).push(*time as u64, 32);
            }
            Msg::TimeSync { index, time } => {
                w.push(*index as u64, 16).push(*time as u64, 32);
            }
            Msg::SyncDone(a) => {
                w.push(*a as u64, 32);
            }
            Msg::Object(o) => {
                let (id_bits, type_bits) = router_widths(o.router).expect("known router");
                w.push(o.router as u64, 4);
                o.handle.write(&mut w, id_bits);
                if o.create && type_bits > 0 {
                    w.push(o.ty as u64, type_bits);
                }
                debug_assert!(o.payload.len() <= MAX_PAYLOAD_BITS);
                w.push(o.seq as u64, 8).push(o.payload.len() as u64, 14);
                w.append(&o.payload);
            }
            Msg::Delete(d) => {
                let (id_bits, _) = router_widths(d.router).expect("known router");
                w.push(d.router as u64, 4);
                d.handle.write(&mut w, id_bits);
                w.push(d.seq as u64, 8);
            }
            Msg::Car(c) => {
                c.handle.write(&mut w, CAR_ID_BITS);
                w.append(&c.body);
            }
        }
        w
    }
}

/// A server update setting one of the server's bools on a console's own ConsoleSynced object.
pub fn console_bool_update(console_id: u32, seq: u8, index: u8, value: bool) -> Msg {
    let mut payload = Bits::new();
    payload.push(index as u64, CONSOLE_VAR_INDEX_BITS).bool(value);
    Msg::Object(ObjectMsg {
        create: false,
        router: ROUTER_CONSOLE_SYNCED,
        handle: Handle::Server(console_id),
        ty: 0,
        seq,
        payload,
    })
}

// ---------------------------------------------------------------------------------------------
// Acks

/// The acks a receiver owes (141766400): ack = the newest packet id seen; bit n-1 of the ack bits
/// = id ack-n seen. Kept as the window itself (33 ids), not a set of every id ever seen.
#[derive(Debug, Clone, Default)]
pub struct Acks {
    top: Option<u16>,
    bits: u32,
}

impl Acks {
    pub fn add(&mut self, pid: u16) {
        if pid == NO_SEQ {
            return;
        }
        let Some(top) = self.top else {
            self.top = Some(pid);
            self.bits = 0;
            return;
        };
        let ahead = pid.wrapping_sub(top);
        if ahead == 0 {
            return;
        }
        if ahead < 0x8000 {
            // newer: the old top becomes bit ahead-1
            let d = ahead as u32;
            self.bits = if d > 32 { 0 } else { self.bits.checked_shl(d).unwrap_or(0) | 1 << (d - 1) };
            self.top = Some(pid);
        } else {
            let back = top.wrapping_sub(pid) as u32;
            if (1..=32).contains(&back) {
                self.bits |= 1 << (back - 1);
            }
        }
    }

    pub fn header(&self) -> (u16, u32) {
        match self.top {
            None => (NO_ACK, 0),
            Some(top) => (top, self.bits),
        }
    }
}

/// Whether the header (ack, ackbits) of a packet we received acknowledges our packet `pid`.
pub fn acks_packet(ack: u16, ackbits: u32, pid: u16) -> bool {
    if ack == pid {
        return ack != NO_ACK || ackbits != 0;
    }
    let back = ack.wrapping_sub(pid) as u32;
    (1..=32).contains(&back) && ackbits & (1 << (back - 1)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(m: Msg) {
        let w = m.write();
        let mut r = Reader::new(w.as_bytes());
        let back = Msg::read(&mut r).unwrap();
        assert_eq!(back, m);
        assert_eq!(r.pos(), w.len(), "{} read exactly its bits", msg_name(m.id()));
    }

    fn payload(n: usize, seed: u64) -> Bits {
        let mut b = Bits::new();
        let mut x = seed | 1;
        for _ in 0..n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b.push(x, 1);
        }
        b
    }

    #[test]
    fn header_roundtrip() {
        let h = Header { id: 0x1234, ack: 0xFFFE, ackbits: 0x8000_0001, count: 3, size: 51 };
        let mut out = Vec::new();
        h.write(&mut out);
        assert_eq!(out, [0x34, 0x12, 0xFE, 0xFF, 0x01, 0, 0, 0x80, 3, 51, 0]);
        assert_eq!(Header::parse(&out), Some(h));
        assert_eq!(Header::parse(&out[..10]), None);
    }

    /// The game's first packet, as captured in fh2-mods tools/online/README.md ("The game's first
    /// packets to the server"): the header and the start of the OpenConnection body.
    #[test]
    fn open_connection_matches_capture() {
        let oc = OpenConnection {
            version: VERSION.to_vec(),
            game: GAME.to_vec(),
            flags: 0,
            user: 0,
            a: 0,
            b: 0,
            attempt: 0,
        };
        let data = packet(&[Msg::OpenConnection(oc).write()], 0, NO_ACK, 0);
        assert_eq!(data.len(), 51);
        let captured: [u8; 30] = [
            0x00, 0x00, 0xfe, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x33, 0x00, // header
            0x00, 0x10, 0x9b, 0xa1, 0xa1, 0x98, 0x1a, 0x19, 0x23, 0x22, 0x10, 0x23, 0xb6, 0xb0,
            0xb3, 0x39, 0xb4, 0x34, 0x38,
        ];
        assert_eq!(&data[..30], &captured);
    }

    #[test]
    fn connection_messages_roundtrip() {
        roundtrip(Msg::OpenConnection(OpenConnection {
            version: VERSION.to_vec(),
            game: GAME.to_vec(),
            flags: 2,
            user: 2533274790395904,
            a: 0x12,
            b: 0x4,
            attempt: 0xDEADBEEF,
        }));
        roundtrip(Msg::OpenResponse(OpenResponse { flags: 3, reason: REASON_RESPONSE_ACK, attempt: 7, own_attempt: 1 }));
        roundtrip(Msg::Close);
        roundtrip(Msg::Ping(0xFFFF_FFFF));
        roundtrip(Msg::Pong { echo: 0, time: 123456 });
        roundtrip(Msg::TimeSync { index: 0xABCD, time: 99 });
        roundtrip(Msg::SyncDone(1));
        roundtrip(Msg::FinaliseSessionEntry);
    }

    #[test]
    fn object_roundtrip_every_handle() {
        let handles = |id| [Handle::Own(id), Handle::Relay(id, 254), Handle::Server(id), Handle::Transfer(id)];
        for (router, id) in [(1u8, 127u32), (3, 5), (6, 3), (9, 15), (12, 1)] {
            for h in handles(id) {
                for create in [true, false] {
                    let (_, type_bits) = router_widths(router).unwrap();
                    let ty = if create && type_bits > 0 { 2 } else { 0 };
                    roundtrip(Msg::Object(ObjectMsg {
                        create,
                        router,
                        handle: h,
                        ty,
                        seq: 254,
                        payload: payload(37 + id as usize, id as u64),
                    }));
                }
                roundtrip(Msg::Delete(ObjectDelete { router, handle: h, seq: 0 }));
            }
        }
        // empty payload, max payload
        roundtrip(Msg::Object(ObjectMsg {
            create: true,
            router: 6,
            handle: Handle::Own(0),
            ty: 0,
            seq: 0,
            payload: Bits::new(),
        }));
        roundtrip(Msg::Object(ObjectMsg {
            create: false,
            router: 2,
            handle: Handle::Own(1),
            ty: 0,
            seq: 9,
            payload: payload(MAX_PAYLOAD_BITS, 5),
        }));
    }

    #[test]
    fn object_layout() {
        // ObjectCreate, router 6, own handle id 0, type 0, seq 0, no payload:
        // 7 + 4 + 2 + 2 + 2 + 8 + 14 = 39 bits
        let m = Msg::Object(ObjectMsg {
            create: true,
            router: 6,
            handle: Handle::Own(0),
            ty: 0,
            seq: 0,
            payload: Bits::new(),
        });
        let w = m.write();
        assert_eq!(w.len(), 39);
        // id 7, then router 6 starting at bit 7: byte 0 = 0x07 | (0 << 7), byte 1 low 3 bits = 6>>1
        assert_eq!(w.as_bytes()[0], 0x07);
        assert_eq!(w.as_bytes()[1] & 0x07, 0x03);
        // relay handle: 8 more bits for the peer
        let mut relayed = m.clone();
        if let Msg::Object(o) = &mut relayed {
            o.handle = Handle::Relay(0, 1);
        }
        assert_eq!(relayed.write().len(), 47);
    }

    #[test]
    fn unknown_router_and_message() {
        let mut w = Bits::new();
        w.push(7, 7).push(10, 4).push(0, 30);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownRouter(10)));
        let mut w = Bits::new();
        w.push(35, 7).push(0, 30);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownMsg(35)));
        // truncated object
        let full = Msg::Object(ObjectMsg {
            create: false,
            router: 1,
            handle: Handle::Own(1),
            ty: 0,
            seq: 1,
            payload: payload(100, 3),
        })
        .write();
        let cut = &full.as_bytes()[..full.as_bytes().len() - 4];
        assert_eq!(Msg::read(&mut Reader::new(cut)), Err(ReadError::Bits(BitError::Short)));
    }

    #[test]
    fn car_roundtrip_and_relay_rewrite() {
        let body = payload(CAR_BODY_BITS, 99);
        let own = Msg::Car(Car { handle: Handle::Own(42), body: body.clone() });
        let w = own.write();
        assert_eq!(w.len(), 7 + 2 + 7 + 710);
        roundtrip(own);
        // what the relay does: same body, the relay handle with the owner's index
        let mut r = Reader::new(w.as_bytes());
        let Msg::Car(car) = Msg::read(&mut r).unwrap() else { panic!() };
        let fwd = Msg::Car(Car { handle: Handle::Relay(car.handle.id(), 7), body: car.body });
        let fw = fwd.write();
        assert_eq!(fw.len(), 7 + 2 + 7 + 8 + 710);
        let Msg::Car(back) = Msg::read(&mut Reader::new(fw.as_bytes())).unwrap() else { panic!() };
        assert_eq!(back.handle, Handle::Relay(42, 7));
        assert_eq!(back.body, body);
    }

    #[test]
    fn console_bool_update_layout() {
        let m = console_bool_update(5, 0, CONSOLE_SESSION_ENTRY_FINALISED, true);
        let w = m.write();
        // 7 id + 4 router + 2 flags + 7 id + 8 seq + 14 size + 5 payload
        assert_eq!(w.len(), 47);
        let Msg::Object(o) = Msg::read(&mut Reader::new(w.as_bytes())).unwrap() else { panic!() };
        assert_eq!(o.handle, Handle::Server(5));
        assert_eq!(o.payload.len(), 5);
        let mut p = Reader::new(o.payload.as_bytes());
        assert_eq!(p.bits(4).unwrap(), 1);
        assert_eq!(p.bits(1).unwrap(), 1);
    }

    #[test]
    fn several_messages_in_one_packet() {
        let msgs = [Msg::Ping(5), Msg::FinaliseSessionEntry, Msg::Pong { echo: 1, time: 2 }];
        let bits: Vec<Bits> = msgs.iter().map(|m| m.write()).collect();
        let data = packet(&bits, 9, 3, 1);
        let h = Header::parse(&data).unwrap();
        assert_eq!((h.id, h.ack, h.ackbits, h.count, h.size as usize), (9, 3, 1, 3, data.len()));
        let mut r = Reader::at(&data, HEADER_LEN * 8);
        for m in &msgs {
            assert_eq!(&Msg::read(&mut r).unwrap(), m);
        }
    }

    #[test]
    fn seq_math() {
        assert_eq!(seq_next(253), 254);
        assert_eq!(seq_next(254), 0);
        assert!(seq_newer(1, 0));
        assert!(seq_newer(0, 254));
        assert!(!seq_newer(0, 0));
        assert!(!seq_newer(0, 1));
        assert!(seq_newer(127, 0));
        assert!(!seq_newer(200, 0));
    }

    #[test]
    fn acks_window() {
        let mut a = Acks::default();
        assert_eq!(a.header(), (NO_ACK, 0));
        a.add(NO_SEQ);
        assert_eq!(a.header(), (NO_ACK, 0));
        a.add(0);
        assert_eq!(a.header(), (0, 0));
        a.add(2);
        assert_eq!(a.header(), (2, 0b10));
        a.add(1);
        assert_eq!(a.header(), (2, 0b11));
        a.add(1); // duplicate
        assert_eq!(a.header(), (2, 0b11));
        a.add(40); // jump: everything falls out of the window
        assert_eq!(a.header(), (40, 0));
        a.add(8); // 32 back: the last bit
        assert_eq!(a.header(), (40, 1 << 31));
        a.add(7); // 33 back: outside
        assert_eq!(a.header(), (40, 1 << 31));
        // wrap
        let mut a = Acks::default();
        a.add(0xFFFD);
        a.add(1);
        assert_eq!(a.header(), (1, 1 << 3));
        assert!(acks_packet(1, 1 << 3, 0xFFFD));
        assert!(acks_packet(1, 0, 1));
        assert!(!acks_packet(1, 0, 0));
        assert!(!acks_packet(NO_ACK, 0, NO_ACK));
    }
}
