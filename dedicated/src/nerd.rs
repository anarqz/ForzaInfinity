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
//!
//! Also read, so that what follows them in a packet is not lost: the car events (20, 21, 35),
//! MsgTagCollision (36), the traffic messages (16, 17, 71, 72, 73, 74), MsgCollObjDetachPart
//! (24), MsgChatManagement (33), MsgInactivePlayersPulled (70), SetGroupSatNavEvent (52), and
//! every other message a console sends (`other_layout`: ownership, car meet, race, road trip,
//! weather replies, leadership, bucket list), and ObjectGrantOwnership / ObjectTransferOwnership
//! (11, 12), which the relay sends. Still unknown, so they end the parse: the other
//! server-to-console messages (a console never sends them), and ObjectNeedsUpdates (13) for
//! objects whose var index width is not known (every router but ConsoleSynced).

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
pub const MSG_SERVER_TRAFFIC_FULL_SYNC: u8 = 16;
pub const MSG_SERVER_TRAFFIC_ROUTE_PROGRESS: u8 = 17;
pub const MSG_BREAK_GLASS: u8 = 20;
pub const MSG_CAR_COLLISION: u8 = 21;
pub const MSG_COLL_OBJ_DETACH_PART: u8 = 24;
pub const MSG_CHAT_MANAGEMENT: u8 = 33;
pub const MSG_FINALISE_SESSION_ENTRY: u8 = 34;
pub const MSG_REPAIR_DAMAGE: u8 = 35;
pub const MSG_TAG_COLLISION: u8 = 36;
pub const MSG_SET_GROUP_SAT_NAV: u8 = 52;
pub const MSG_INACTIVE_PLAYERS_PULLED: u8 = 70;
pub const MSG_SERVER_TRAFFIC_MESSAGE: u8 = 71;
pub const MSG_CLIENT_TRAFFIC_MESSAGE: u8 = 72;
pub const MSG_SERVER_TRAFFIC_FULL_ROUTE: u8 = 73;
pub const MSG_SERVER_TRAFFIC_STATIONARY: u8 = 74;

/// The message table's names (142F4DB20: 77 entries of name, description, and the send channel
/// and id as two u32s just before the next name).
pub fn msg_name(id: u8) -> &'static str {
    const NAMES: [&str; 77] = [
        "OpenConnection",
        "OpenConnectionResponse",
        "CloseConnection",
        "Ping",
        "Pong",
        "TimeSync",
        "ConnectionSyncDone",
        "ObjectCreate",
        "ObjectUpdate",
        "ObjectDelete",
        "ObjectRequestOwnership",
        "ObjectGrantOwnership",
        "ObjectTransferOwnership",
        "ObjectNeedsUpdates",
        "PlayerInput",
        "CarPredictionFull",
        "ServerTrafficFullSync",
        "ServerTrafficRouteProgressUpdate",
        "ClientTrafficFullSync",
        "ClientTrafficRouteProgressUpdate",
        "MsgBreakGlass",
        "MsgCarCollision",
        "MsgCollObjPos",
        "MsgCollObjRest",
        "MsgCollObjDetachPart",
        "MsgCollObjPosVel",
        "MsgCarMeetPlayerInviteRequest",
        "MsgCarMeetPlayerJoinOrCancelRequest",
        "MsgCarMeetLaunchWagerRace",
        "MsgCarMeetNotifyPendingGameMode",
        "MsgCarMeetConfigureGameOptionsForWagerRace",
        "MsgCarMeetConfiguredGameOptionsForWagerRace",
        "MsgCarMeetGameModeSessionReady",
        "MsgChatManagement",
        "FinaliseSessionEntry",
        "MsgRepairDamage",
        "MsgTagCollision",
        "MsgTagBuckets",
        "MsgRequestRace",
        "MsgStartingGrid",
        "MsgStartRace",
        "MsgReadyRequest",
        "MsgReadyRequestResponse",
        "MsgInProgressScoreUpdate",
        "MsgLapScore",
        "MsgScoreKeyframe",
        "MsgVoteToKick",
        "MsgVoteForTrack",
        "SelectNextRoadTripEvent",
        "SelectedNextRoadTripEvent",
        "ConfigureGameOptionsForRoadTripEvent",
        "ConfiguredGameOptionsForRoadTripEvent",
        "SetGroupSatNavEvent",
        "RequestSendFullWeather",
        "FullWeather",
        "RequestSendNewAtmosphericData",
        "NewAtmosphericData",
        "RequestSendNewPrecipitationData",
        "NewPrecipitationData",
        "RequestLeaderChange",
        "DropLeadership",
        "BucketListChallengeStart",
        "BucketListChallengeEnded",
        "RequestSynchronizeBucketListChallenge",
        "RequestDesynchronizeBucketListChallenge",
        "RespondBucketListPartner",
        "BucketListStatusPlayerWaiting",
        "BucketListStatusChallengeStarted",
        "BucketListStatusChallengeEnded",
        "MsgDebugLog",
        "MsgInactivePlayersPulled",
        "ServerTrafficMessage",
        "ClientTrafficMessage",
        "ServerTrafficFullRoute",
        "ServerTrafficStationary",
        "ClientTrafficFullRoute",
        "ClientTrafficStationary",
    ];
    NAMES.get(id as usize).copied().unwrap_or("?")
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

/// SessionHost's vars (SessionHostSynced, 14180FF70: room for 15, so a 4-bit var index; the
/// order is the one 141810FE0 registers them in): 0 LobbyType, 1 PrivacyMode, 2 CountdownType,
/// 3 CountdownEndTime, 4 RaceReady, 5 RaceParticipantsSelected, 6 InRace,
/// 7 LastInactivePlayersPulledTime, 8 RoundGuid, 9 RaceStartTime, 10 NumLapsCompleted,
/// 11 FurthestProgress, 12 GroupSatNavNode. Every one is the server's.
pub const SESSION_HOST_VAR_INDEX_BITS: u32 = 4;
/// GroupSatNavNode (var 12; type vtable 142C5C9D8, pack 140290E80 / unpack 140290F40: 32 bits,
/// read signed): the group's GPS destination, set by the server from SetGroupSatNavEvent.
pub const SESSION_HOST_GROUP_SAT_NAV_NODE: u8 = 12;

/// SessionHost race vars (widths from the var types' pack functions: SyncedBool 1417DE9F0, 1 bit;
/// the times 1417D8030, 32 bits).
pub const SESSION_HOST_COUNTDOWN_TYPE: u8 = 2;
pub const SESSION_HOST_COUNTDOWN_END_TIME: u8 = 3;
pub const SESSION_HOST_RACE_READY: u8 = 4;
pub const SESSION_HOST_RACE_PARTICIPANTS_SELECTED: u8 = 5;
pub const SESSION_HOST_IN_RACE: u8 = 6;
pub const SESSION_HOST_RACE_START_TIME: u8 = 9;

/// SessionHost payload entries: (var, value, bits).
pub fn session_host_payload(entries: &[(u8, u64, u32)]) -> Bits {
    let mut p = Bits::new();
    for &(var, value, bits) in entries {
        p.push(var as u64, SESSION_HOST_VAR_INDEX_BITS).push(value, bits);
    }
    p
}

/// The SessionHost payload entry setting GroupSatNavNode.
pub fn group_sat_nav_payload(node: i32) -> Bits {
    let mut p = Bits::new();
    p.push(SESSION_HOST_GROUP_SAT_NAV_NODE as u64, SESSION_HOST_VAR_INDEX_BITS).push(node as u32 as u64, 32);
    p
}

/// ConsoleSynced (router 3; NerdLib 1417DE550): 9 bool vars, a 4-bit var index. 0 RenderReady,
/// 1 SessionEntryFinalised, 2 ParticipatingInRace, 3 RaceInProgress, 4 Ready,
/// 5 HasLoadedCarsForAllParticipants, 6 HasGrid, 7 AvailableArbitration, 8 MatchTicketArbiter.
/// 1, 2 and 4 are the server's (SyncedBool<0,1>).
pub const CONSOLE_RENDER_READY: u8 = 0;
pub const CONSOLE_SESSION_ENTRY_FINALISED: u8 = 1;
pub const CONSOLE_PARTICIPATING_IN_RACE: u8 = 2;
pub const CONSOLE_READY: u8 = 4;
pub const CONSOLE_HAS_LOADED_CARS: u8 = 5;
pub const CONSOLE_HAS_GRID: u8 = 6;
pub const CONSOLE_VAR_INDEX_BITS: u32 = 4;

/// The u14 payload size, in bits.
pub const MAX_PAYLOAD_BITS: usize = (1 << 14) - 1;

// ---------------------------------------------------------------------------------------------
// Payloads of the objects the relay forwards
//
// Only routers 1 (PlayerData), 2 (CarNetworked) and 3 (ConsoleSynced) are forwarded (the
// player's own objects in free roam; 6 is the server's, the others are not mapped). A payload is
// a list of (var index, var data) (1417B2510); the index width is the bit length of the class's
// var room (141789BA0), the vars are numbered in the order the class registers them (the
// 1417B5020 / 1417B50E0 calls), and each var's width is its type's pack function (vtable slot
// 0x28). The relay reads every entry it can, checks it, and writes the payload again from what
// it read (`sanitize_payload`); anything else drops the message.

/// A var's data, as its type packs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VarKind {
    /// n plain bits.
    Bits(u32),
    /// A f32 (140290EA0 / 1409A49A0): must be finite and within FLOAT_MAX.
    F32,
    /// NetworkName (142F4CE18; pack 1417D8050, read 1417F40E0): u5 count, then count UTF-16
    /// units. The reader takes at most 16: with more it stops reading (the rest of the payload is
    /// then misread) and keeps the count, while the buffer holds 18 units.
    Name,
}

use VarKind::{Bits as VB, F32 as VF, Name as VN};

/// The most UTF-16 units a NetworkName may have (1417F40E0: `count < 0x11`).
pub const NETWORK_NAME_MAX: u32 = 16;

/// The largest magnitude a forwarded float may have (damage, wear, fuel, collision values): no
/// legitimate value comes near it, and NaN / infinity are refused outright.
pub const FLOAT_MAX: f32 = 1.0e6;
/// The largest coordinate of a car's position (the first vec3 of CarPredictionFull, 14181FFA0):
/// the map is a few km across, so 100 km is far outside it.
pub const POSITION_MAX: f32 = 1.0e5;

fn float_ok(bits: u32, max: f32) -> bool {
    let f = f32::from_bits(bits);
    f.is_finite() && f.abs() <= max
}

/// ConsoleSynced vars that only the server sets (SyncedBool<0,1>, 142F48CA8): 1
/// SessionEntryFinalised, 2 ParticipatingInRace, 4 Ready. A console's entries for them are
/// stripped before forwarding, so no console can set them on its object as the others see it.
pub const CONSOLE_SERVER_VARS: [u32; 3] = [1, 2, 4];

/// ConsoleSynced (1417DE550: room 9, so 4-bit indices; registered by 1417DEC00): 0-6 bools
/// (142F44980 / 142F48CA8: pack 1417DE9F0, 1 bit), 7 AvailableArbitration and
/// 8 MatchTicketArbiter (142F48CF0 / 142F48D38: pack 141821F40, 2 bits).
const CONSOLE_VARS: [VarKind; 9] = [VB(1), VB(1), VB(1), VB(1), VB(1), VB(1), VB(1), VB(2), VB(2)];

/// CarNetworked (1417DBDD0: room 22 (0x16), 5-bit indices; registered by 1417DC590): 0-20 the
/// car's damage, wear and fuel (struct 14181ED70, registered by 14181F2D0: CurRearDownforceDamage
/// ... CurWheel3SuspensionDmg, all 142C5C798: a f32), 21 CarUniverseId (142F48458: pack
/// 1417DC370, 5 bits).
const CAR_VARS: [VarKind; 22] = [
    VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF, VF,
    VB(5),
];
pub const CAR_VAR_INDEX_BITS: u32 = 5;

/// PlayerData (1417F3230: room 163 (0xA3), 8-bit indices; registered by 1417F4620). Its own
/// vars come first: 0 BufChoices (8 u8, 1417F3FD0), 1 HasLivery, 2 IsNewLivery, 3 NetworkName,
/// 4 XUID (64, 140290EB0), 5 IsLeader, 6 NumReceivedKickVotes (32), 7 HasFinalScore,
/// 8 HeatsRacePoints, 9 HeatsTotalPoints, 10 SeriesRacePoints, 11 SeriesTotalPoints (32 each).
/// Then the vars of three structs it points to (14180D2D0, 1417E97C0, 141823DA0: about 150, with
/// nested arrays), which are not mapped: from the first entry with an index of 12 or more, the
/// rest of the payload is kept as it is, if the whole payload is within PLAYER_DATA_MAX_BITS.
const PLAYER_VARS: [VarKind; 12] = [VB(64), VB(1), VB(1), VN, VB(64), VB(1), VB(32), VB(1), VB(32), VB(32), VB(32), VB(32)];
pub const PLAYER_VAR_INDEX_BITS: u32 = 8;
const PLAYER_VAR_ROOM: u32 = 163;
/// PlayerData's payload limit. Not observed (no capture with sizes yet): an estimate from its
/// layout, 163 vars of an 8-bit index and at most ~40 bits each on average (the mapped ones are
/// 1 to 64 bits, NetworkName at most 5 + 16 x 16), about 8000 bits, rounded up. The relay logs
/// the largest payload it sees per router (verbose), so this can be set from real games.
pub const PLAYER_DATA_MAX_BITS: usize = 8192;

/// Why a payload is not forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadError {
    /// Not a router the relay forwards.
    Router(u8),
    /// A var index past the class's vars.
    Index(u32),
    /// A var given twice (the game writes each dirty var once).
    Duplicate(u32),
    /// A f32 that is NaN, infinite or over FLOAT_MAX.
    NotFinite(u32),
    /// A NetworkName longer than the reader takes.
    NameTooLong(u32),
    /// The payload ends inside an entry.
    Truncated,
    /// Over the router's size limit.
    TooBig(usize),
}

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadError::Router(r) => write!(f, "router {r} is not forwarded"),
            PayloadError::Index(i) => write!(f, "var index {i} out of range"),
            PayloadError::Duplicate(i) => write!(f, "var {i} given twice"),
            PayloadError::NotFinite(i) => write!(f, "var {i} is not a sane float"),
            PayloadError::NameTooLong(n) => write!(f, "NetworkName of {n} units"),
            PayloadError::Truncated => f.write_str("payload ends inside a var"),
            PayloadError::TooBig(n) => write!(f, "payload of {n} bits over the limit"),
        }
    }
}

/// Whether the relay forwards objects of this router.
pub fn forwarded_router(router: u8) -> bool {
    matches!(router, 1..=3)
}

/// The most bits a payload of this router can have. Mapped classes: every var once with its
/// largest data (ConsoleSynced 9 x 4 + 7 + 4 = 47; CarNetworked 22 x 5 + 21 x 32 + 5 = 787).
pub fn payload_max_bits(router: u8) -> Option<usize> {
    fn max_of(index_bits: u32, vars: &[VarKind]) -> usize {
        vars.iter()
            .map(|v| {
                index_bits as usize
                    + match v {
                        VarKind::Bits(n) => *n as usize,
                        VarKind::F32 => 32,
                        VarKind::Name => 5 + 16 * NETWORK_NAME_MAX as usize,
                    }
            })
            .sum()
    }
    match router {
        1 => Some(PLAYER_DATA_MAX_BITS),
        2 => Some(max_of(CAR_VAR_INDEX_BITS, &CAR_VARS)),
        3 => Some(max_of(CONSOLE_VAR_INDEX_BITS, &CONSOLE_VARS)),
        _ => None,
    }
}

/// Reads a payload of a forwarded router entry by entry, checks every var, and writes it again
/// from what was read (PlayerData's unmapped tail, see PLAYER_VARS, is copied within its limit).
pub fn sanitize_payload(router: u8, payload: &Bits) -> Result<Bits, PayloadError> {
    let (index_bits, vars, room): (u32, &[VarKind], u32) = match router {
        1 => (PLAYER_VAR_INDEX_BITS, &PLAYER_VARS, PLAYER_VAR_ROOM),
        2 => (CAR_VAR_INDEX_BITS, &CAR_VARS, CAR_VARS.len() as u32),
        3 => (CONSOLE_VAR_INDEX_BITS, &CONSOLE_VARS, CONSOLE_VARS.len() as u32),
        r => return Err(PayloadError::Router(r)),
    };
    let max = payload_max_bits(router).unwrap_or(0);
    let len = payload.len();
    if len > max {
        return Err(PayloadError::TooBig(len));
    }
    let bytes = payload.as_bytes();
    let mut r = Reader::new(bytes);
    let mut out = Bits::new();
    let mut seen = [false; 32];
    // reads n bits of the payload (not of the padding after it)
    fn get(r: &mut Reader, len: usize, n: u32) -> Result<u64, PayloadError> {
        if r.pos() + n as usize > len {
            return Err(PayloadError::Truncated);
        }
        r.bits(n).map_err(|_| PayloadError::Truncated)
    }
    while r.pos() < len {
        let start = r.pos();
        let index = get(&mut r, len, index_bits)? as u32;
        let Some(kind) = vars.get(index as usize) else {
            if index < room {
                // PlayerData's unmapped vars: the rest as it is
                let mut tail = Reader::at(bytes, start);
                let rest = tail.take(len - start).map_err(|_| PayloadError::Truncated)?;
                out.append(&rest);
                return Ok(out);
            }
            return Err(PayloadError::Index(index));
        };
        if std::mem::replace(&mut seen[index as usize], true) {
            return Err(PayloadError::Duplicate(index));
        }
        let keep = !(router == ROUTER_CONSOLE_SYNCED && CONSOLE_SERVER_VARS.contains(&index));
        let mut entry = Bits::new();
        entry.push(index as u64, index_bits);
        match *kind {
            VarKind::Bits(n) => {
                let v = get(&mut r, len, n)?;
                entry.push(v, n);
            }
            VarKind::F32 => {
                let v = get(&mut r, len, 32)? as u32;
                if !float_ok(v, FLOAT_MAX) {
                    return Err(PayloadError::NotFinite(index));
                }
                entry.push(v as u64, 32);
            }
            VarKind::Name => {
                let n = get(&mut r, len, 5)? as u32;
                if n > NETWORK_NAME_MAX {
                    return Err(PayloadError::NameTooLong(n));
                }
                entry.push(n as u64, 5);
                for _ in 0..n {
                    let c = get(&mut r, len, 16)?;
                    entry.push(c, 16);
                }
            }
        }
        if keep {
            out.append(&entry);
        }
    }
    Ok(out)
}

/// The f32s of a car event's body (after the handle) must be finite and within FLOAT_MAX:
/// MsgBreakGlass (u32, 3 vec3, s32) and MsgCarCollision (vec3, 3 floats).
pub fn car_event_ok(id: u8, body: &Bits) -> bool {
    let (skip, n) = match id {
        MSG_BREAK_GLASS => (32, 9),
        MSG_CAR_COLLISION => (0, 6),
        MSG_REPAIR_DAMAGE => (0, 0),
        _ => return false,
    };
    if car_event_bits(id) != Some(body.len()) {
        return false;
    }
    let mut r = Reader::new(body.as_bytes());
    if r.take(skip).is_err() {
        return false;
    }
    (0..n).all(|_| r.bits(32).is_ok_and(|v| float_ok(v as u32, FLOAT_MAX)))
}

/// The position of a car (CarPredictionFull's first field), when its body is one.
pub fn car_position(body: &Bits) -> Option<[f32; 3]> {
    if !car_ok(body) {
        return None;
    }
    let mut r = Reader::new(body.as_bytes());
    let mut p = [0f32; 3];
    for c in &mut p {
        *c = f32::from_bits(r.bits(32).ok()? as u32);
    }
    Some(p)
}

/// A CarPredictionFull body: its first field is the position, a vec3 (14181FFA0, from
/// 141822100), whose coordinates must be finite and within POSITION_MAX. The rest are quantized
/// fixed-width fields (u16s and u64s), any value of which is in range.
pub fn car_ok(body: &Bits) -> bool {
    if body.len() != CAR_BODY_BITS {
        return false;
    }
    let mut r = Reader::new(body.as_bytes());
    (0..3).all(|_| r.bits(32).is_ok_and(|v| float_ok(v as u32, POSITION_MAX)))
}

/// The SatNav nodes a console may set (SetGroupSatNavEvent): 0 (none) up to this. Nodes are
/// road-graph node indices of the map; 2^24 is far over any map's node count, and a value outside
/// is dropped rather than clamped (a clamped node would be another, wrong place).
pub const SAT_NAV_NODE_MAX: i32 = 0x00FF_FFFF;

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

// The other messages a console sends. Each one's writer builds it right after the 7-bit id
// (14176B670 with [1422E4408] = 7 bits), from the message's own sender function (the callers of
// 141760C70, which takes the send channel and the id); the console's reader, where it has one,
// is the callback registered for the id with 141768170. Primitives: 14176B380 / 14176B670 /
// 14176AD40 / 14176ABD0 write n bits (u32, u8, signed, u16), 14176AB90 a 32-bit float,
// 14176A8A0 one bit, 14176AEC0 64 bits, 14181FFA0 a vec3 (3 floats, 96 bits).

/// A vec3 (14181FFA0, read 141820020): three 32-bit floats.
const VEC3_BITS: usize = 96;

/// Car events: the car's handle (7-bit id, as CarPredictionFull), then a fixed body. The console
/// registers a reader for each in the car router (1417CF210), so the server relays them.
///   - MsgBreakGlass (20; send 141804B10, read 141804C10): u32, 3 vec3, s32 (352 bits).
///   - MsgCarCollision (21; send 141804D30, read 141804E20): vec3, 3 floats (192 bits).
///   - MsgRepairDamage (35; send 141804F30, read 141804FD0): nothing after the handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarEvent {
    pub id: u8,
    pub handle: Handle,
    pub body: Bits,
}

/// The body of a car event after its handle, in bits.
pub fn car_event_bits(id: u8) -> Option<usize> {
    match id {
        MSG_BREAK_GLASS => Some(32 + 3 * VEC3_BITS + 32),
        MSG_CAR_COLLISION => Some(VEC3_BITS + 3 * 32),
        MSG_REPAIR_DAMAGE => Some(0),
        _ => None,
    }
}

/// MsgTagCollision (36; send 1418048B0, read 1418049C0): two car handles (7-bit ids: the
/// sender's car and the car it touched), then four u3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCollision {
    pub a: Handle,
    pub b: Handle,
    pub vals: [u8; 4],
}

/// Traffic. The Xbox server ran the AI traffic; consoles send it ServerTraffic* and get
/// ClientTraffic* back (the console's traffic router, 1417DD790, reads only 18, 19, 72, 75, 76).
/// Fixed sizes, kept as raw bits:
///   - ServerTrafficFullSync (16; send 1417F6E70): u8, then 141821450: u32, vec3, 9 u16, u32,
///     2 bools (314 bits).
///   - ServerTrafficRouteProgressUpdate (17; send 1417F6C00): u8, 141821150 (float, u32, u32),
///     vec3 (200 bits).
///   - ServerTrafficMessage (71; send 1417F5210) and ClientTrafficMessage (72; read 14181FD70):
///     u4, then 1418208E0 / 141820950: u8, s14, u5, u8, bool; then s32 (72 bits).
///   - ServerTrafficFullRoute (73; send 1417F6CD0): u8, 1418211F0 (8 u16, u2), 141821150, vec3
///     (330 bits).
///   - ServerTrafficStationary (74; send 1417F6DB0): u8, 141821360: vec3, 3 u16, u32, bool
///     (185 bits).
pub fn traffic_bits(id: u8) -> Option<usize> {
    match id {
        MSG_SERVER_TRAFFIC_FULL_SYNC => Some(8 + 32 + VEC3_BITS + 9 * 16 + 32 + 2),
        MSG_SERVER_TRAFFIC_ROUTE_PROGRESS => Some(8 + 96 + VEC3_BITS),
        MSG_SERVER_TRAFFIC_MESSAGE | MSG_CLIENT_TRAFFIC_MESSAGE => Some(4 + (8 + 14 + 5 + 8 + 1) + 32),
        MSG_SERVER_TRAFFIC_FULL_ROUTE => Some(8 + (8 * 16 + 2) + 96 + VEC3_BITS),
        MSG_SERVER_TRAFFIC_STATIONARY => Some(8 + VEC3_BITS + 3 * 16 + 32 + 1),
        _ => None,
    }
}

/// MsgCollObjDetachPart (24; send 1417DDAF0, which logs "[MsgCollObjDetachPart] Send"; read
/// 1417DDBD0): u4, u11 count, count bytes. The console's reader copies the bytes into a
/// 1024-byte stack buffer without a check, so this must never be sent on with more than 1024.
pub const COLL_OBJ_DETACH_PART_MAX: usize = 1024;
/// MsgChatManagement (33; send 1417F5040, read 1417F5120): u64, u7 count, count bytes. The
/// reader's buffer holds 64.
pub const CHAT_MANAGEMENT_MAX: usize = 64;

/// MsgInactivePlayersPulled (70; send 1417F7000): a lobby object handle (2-bit id).
pub const INACTIVE_PLAYERS_ID_BITS: u32 = 2;

/// SetGroupSatNavEvent (52; send 141815730, from 140A4DD00 (node 0: clear) and 140A52200 (the
/// chosen event's node)): a lobby object handle (2-bit id: the SessionHost, [netmgr+0xD0]), then
/// the node, s32. The lobby router (141809420) reads it with 1418092B0, which casts the object
/// to Nerd::ILobbyEvents and calls slot 0x38 (1418157F0) with the node: on a console that slot
/// is 140A41740 (`return false`) for both SessionHost and CGameOptions, so only the server acts
/// on it; what consoles see is the SessionHost var GroupSatNavNode.
pub const SAT_NAV_ID_BITS: u32 = 2;

/// A field of the messages read only to get past them (`other_layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// n bits: the plain writers (14176B380, 14176B670, 14176AD40, 14176B230, 14176AEC0), a
    /// bit (14176A8A0), a f32 (14176AB90, 32).
    Bits(usize),
    /// An object handle (1417B0E40) with an id of this many bits.
    Handle(u32),
    /// A compact length and that many bytes (14176A900), at most this many: a std::string
    /// (14176B0C0, max 64) or a f64 (14176ABB0: always 8 bytes, so 8 + 64 bits).
    Bytes(usize),
}

use Field::{Bits as B, Bytes as Y, Handle as H};

/// A f64 (14176ABB0).
const F64: Field = Y(8);
/// Atmospheric data (141820C40): 3 f32, 3 s32, u32 (224 bits).
const ATMOSPHERIC_BITS: usize = 3 * 32 + 3 * 32 + 32;
/// Precipitation data (141821050): 3 x 32 bits (96).
const PRECIPITATION_BITS: usize = 96;

/// The layout of a message a console sends that the relay reads only to get past it (each from
/// its sender, the caller of 141760C70 for the id, checked against the disassembly):
///   - PlayerInput (14; no sender; read 1418200D0 / skipped by 1418201D0, router 1): handle
///     (7-bit id), u16, s12, s12, 4 bits.
///   - MsgCarMeetPlayerInviteRequest (26; 1417F75D0): u2, u32.
///   - MsgCarMeetPlayerJoinOrCancelRequest (27; 1417F70F0): bit, u2, u32.
///   - MsgCarMeetConfiguredGameOptionsForWagerRace (31; 1417F7270): u32.
///   - MsgCarMeetGameModeSessionReady (32; 1417F7380): two strings (max 64).
///   - MsgRequestRace (38; 1417F6F30): nothing.
///   - MsgReadyRequest (41; 1417F7B20): lobby handle (2-bit id), bit.
///   - MsgInProgressScoreUpdate (43; 1418040E0): handle (7-bit id), f32, bit.
///   - MsgLapScore (44; 141804270): handle (7), 3 f64, u32, 2 bits.
///   - MsgScoreKeyframe (45; 1418044E0): handle (7), 5 f64, u32, 3 u8, 3 bits.
///   - MsgVoteToKick (46; 141815870): lobby handle (2), u64 (ILobbyEvents 0x28).
///   - MsgVoteForTrack (47; 141811E60): lobby handle (2), s32 (ILobbyEvents 0x30).
///   - SelectedNextRoadTripEvent (49; 1417F7720): bit.
///   - ConfiguredGameOptionsForRoadTripEvent (51; 1417F7830): s32, u16.
///   - FullWeather (54; 1417F6840): atmospheric, precipitation, s32, s32 (384 bits).
///   - NewAtmosphericData (56; 1417F69E0): atmospheric (224).
///   - NewPrecipitationData (58; 1417F6AF0): precipitation (96).
///   - RequestLeaderChange (59; 1418159B0): handle (7-bit id); read 141815A50.
///   - DropLeadership (60; 1417F7A50): nothing; read 1417F7AD0.
///   - BucketListChallengeEnded (62; 141805070): bit.
///   - RequestSynchronizeBucketListChallenge (63; 141805180): s32, s32.
///   - RequestDesynchronizeBucketListChallenge (64; 1418052D0): nothing.
pub fn other_layout(id: u8) -> Option<&'static [Field]> {
    Some(match id {
        14 => &[H(7), B(16 + 12 + 12 + 4)],
        26 => &[B(2 + 32)],
        27 => &[B(1 + 2 + 32)],
        31 => &[B(32)],
        32 => &[Y(64), Y(64)],
        38 | 60 | 64 => &[],
        40 => &[H(2), B(32)],
        41 => &[H(2), B(1)],
        43 => &[H(7), B(32 + 1)],
        44 => &[H(7), F64, F64, F64, B(32 + 2)],
        45 => &[H(7), F64, F64, F64, F64, F64, B(32 + 3 * 8 + 3)],
        46 => &[H(2), B(64)],
        47 => &[H(2), B(32)],
        49 | 62 => &[B(1)],
        51 => &[B(32 + 16)],
        54 => &[B(ATMOSPHERIC_BITS + PRECIPITATION_BITS + 64)],
        56 => &[B(ATMOSPHERIC_BITS)],
        58 => &[B(PRECIPITATION_BITS)],
        59 => &[H(7)],
        63 => &[B(32 + 32)],
        _ => return None,
    })
}

pub const MSG_OBJECT_REQUEST_OWNERSHIP: u8 = 10;
pub const MSG_OBJECT_GRANT_OWNERSHIP: u8 = 11;
pub const MSG_OBJECT_TRANSFER_OWNERSHIP: u8 = 12;
pub const MSG_OBJECT_NEEDS_UPDATES: u8 = 13;
/// The lobby's GameOptions type (router 6): the event a console wants to start.
pub const GAME_OPTIONS_TYPE: u32 = 1;

/// ObjectRequestOwnership's body: what a console asks the server to own. A console asks for an
/// object it made itself (its GameOptions, when it enters an event: stateflow
/// NetworkGetGameOptionsOwnership) with a Transfer handle, takes it as its own at once with the
/// request pending (SyncedObject +0x38 = 0xFFFFFFFE, +0x70 set; 1417B6A80), and waits until the
/// server answers with ObjectGrantOwnership or ObjectTransferOwnership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipRequest {
    pub router: u8,
    pub ty: u32,
    pub handle: Handle,
}

pub fn ownership_request(body: &Bits) -> Option<OwnershipRequest> {
    let mut r = Reader::new(body.as_bytes());
    let router = r.u8(4).ok()?;
    let (id_bits, type_bits) = router_widths(router)?;
    let ty = if type_bits > 0 { r.u32(type_bits).ok()? } else { 0 };
    let handle = Handle::read(&mut r, id_bits).ok()?;
    Some(OwnershipRequest { router, ty, handle })
}

/// ObjectGrantOwnership (11; the console reads it with 1417AB720 -> 1417AD1B0 -> 1417B1E70 ->
/// 1417B1C10): router u4, the handle of the request (a Transfer handle: the reader asserts flag
/// 31), seq u8 (the object's next seq from the server, as an update: 1417B1E70), payload size u14
/// and vars as in an update (the server's values; the console keeps those it changed while
/// pending). The console then owns the object, its pending request is dropped (slot 0x78,
/// 1417AE450), and it sends the object to the server.
pub fn grant_ownership(router: u8, handle: Handle, seq: u8, payload: &Bits) -> Bits {
    let (id_bits, _) = router_widths(router).unwrap_or((0, 0));
    let mut w = Bits::new();
    w.push(MSG_OBJECT_GRANT_OWNERSHIP as u64, 7).push(router as u64, 4);
    handle.write(&mut w, id_bits);
    w.push(seq as u64, 8).push(payload.len() as u64, 14).append(payload);
    w
}

/// ObjectTransferOwnership (12; 1417AB9B0 -> 1417B01D0 -> 1417B1E70 -> 1417B24C0): router u4, the
/// handle, seq u8. The sender (the server) owns the object from then on; a console whose request
/// was pending drops it (1417A7290), so its NetworkGetGameOptionsOwnership leaves by
/// OwnershipDenied: someone else has the event.
pub fn transfer_ownership(router: u8, handle: Handle, seq: u8) -> Bits {
    let (id_bits, _) = router_widths(router).unwrap_or((0, 0));
    let mut w = Bits::new();
    w.push(MSG_OBJECT_TRANSFER_OWNERSHIP as u64, 7).push(router as u64, 4);
    handle.write(&mut w, id_bits);
    w.push(seq as u64, 8);
    w
}

/// Var index widths, where known: ConsoleSynced (router 3; one type).
pub fn var_index_bits(router: u8) -> Option<u32> {
    (router == ROUTER_CONSOLE_SYNCED).then_some(CONSOLE_VAR_INDEX_BITS)
}

/// Walks a message body after its id without keeping it: where it ends.
///   - ObjectRequestOwnership (10; send 1417B18C0, from 1417B6960): router u4, the type id when
///     the router has more than one type (1417B1840), the handle.
///   - ObjectNeedsUpdates (13; send 1417B1550, from 1417B6C60): router u4, the handle, u16
///     count, then per var asked for: two 0 bits and the var index (the object's index width).
///   - ObjectGrantOwnership / ObjectTransferOwnership (11, 12; server to console, read so the
///     tests' consoles see them): `grant_ownership`, `transfer_ownership`.
///   - the `other_layout` ones.
fn skip_other(r: &mut Reader, id: u8) -> Result<(), ReadError> {
    match id {
        MSG_STARTING_GRID => {
            Handle::read(r, 2)?;
            let n = r.bits(4)? as u32;
            for _ in 0..n {
                Handle::read(r, 7)?;
            }
        }
        MSG_OBJECT_GRANT_OWNERSHIP | MSG_OBJECT_TRANSFER_OWNERSHIP => {
            let router = r.u8(4)?;
            let (id_bits, _) = router_widths(router).ok_or(ReadError::UnknownRouter(router))?;
            Handle::read(r, id_bits)?;
            r.take(8)?;
            if id == MSG_OBJECT_GRANT_OWNERSHIP {
                let n = r.bits(14)? as usize;
                r.take(n)?;
            }
        }
        MSG_OBJECT_REQUEST_OWNERSHIP | MSG_OBJECT_NEEDS_UPDATES => {
            let router = r.u8(4)?;
            let (id_bits, type_bits) = router_widths(router).ok_or(ReadError::UnknownRouter(router))?;
            if id == MSG_OBJECT_REQUEST_OWNERSHIP {
                r.take(type_bits as usize)?;
                Handle::read(r, id_bits)?;
            } else {
                let index_bits = var_index_bits(router).ok_or(ReadError::UnknownMsg(id))?;
                Handle::read(r, id_bits)?;
                let n = r.bits(16)? as usize;
                r.take(n * (2 + index_bits as usize))?;
            }
        }
        _ => {
            for f in other_layout(id).ok_or(ReadError::UnknownMsg(id))? {
                match *f {
                    Field::Bits(n) => {
                        r.take(n)?;
                    }
                    Field::Handle(bits) => {
                        Handle::read(r, bits)?;
                    }
                    Field::Bytes(max) => {
                        r.string(max)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// A ConsoleSynced payload (as sanitize_payload wrote it) as (var, value) pairs; None if it
/// does not read.
pub fn console_vars(payload: &Bits) -> Option<Vec<(u8, u64)>> {
    let len = payload.len();
    let mut r = Reader::new(payload.as_bytes());
    let mut out = Vec::new();
    while r.pos() < len {
        let index = r.u8(CONSOLE_VAR_INDEX_BITS).ok()?;
        let VarKind::Bits(n) = *CONSOLE_VARS.get(index as usize)? else { return None };
        out.push((index, r.bits(n).ok()?));
    }
    (r.pos() == len).then_some(out)
}

/// MsgReadyRequest (41)'s bit: the console is ready (or not) for the race.
pub fn ready_request(body: &Bits) -> Option<bool> {
    let mut r = Reader::new(body.as_bytes());
    Handle::read(&mut r, 2).ok()?;
    r.bool().ok()
}

pub const MSG_REQUEST_RACE: u8 = 38;
pub const MSG_STARTING_GRID: u8 = 39;
pub const MSG_START_RACE: u8 = 40;
pub const MSG_READY_REQUEST: u8 = 41;
pub const MSG_SCORE_KEYFRAME: u8 = 45;

/// MsgStartingGrid (39; the lobby router 141809420 reads the SessionHost handle with 1418092B0,
/// then 141824040): u4 count and the participants' handles (7-bit ids) in grid order. The
/// console's SessionHost (ILobbyEvents slot 1, 140A2E130) places each car by its handle
/// (140A31140; one not in the list gets a slot from the back) and, if it takes part, fires
/// NetworkSwitchRoute: the race route loads.
pub fn starting_grid(grid: &[Handle]) -> Bits {
    let mut w = Bits::new();
    w.push(MSG_STARTING_GRID as u64, 7);
    Handle::Own(SESSION_HOST_ID).write(&mut w, 2);
    w.push(grid.len().min(12) as u64, 4);
    for h in grid.iter().take(12) {
        h.write(&mut w, 7);
    }
    w
}

/// MsgStartRace (40; 1418241E0): the SessionHost handle, u32 the start time (server ms, as
/// TimeSync). The SessionHost (slot 2, 140A2E020) fires its start event with it.
pub fn start_race(time: u32) -> Bits {
    let mut w = Bits::new();
    w.push(MSG_START_RACE as u64, 7);
    Handle::Own(SESSION_HOST_ID).write(&mut w, 2);
    w.push(time as u64, 32);
    w
}

/// Whether `skip_other` knows the message.
pub fn is_other(id: u8) -> bool {
    matches!(
        id,
        MSG_OBJECT_REQUEST_OWNERSHIP
            | MSG_OBJECT_GRANT_OWNERSHIP
            | MSG_OBJECT_TRANSFER_OWNERSHIP
            | MSG_OBJECT_NEEDS_UPDATES
            | MSG_STARTING_GRID
    ) || other_layout(id).is_some()
}

fn read_bytes(r: &mut Reader, n: usize) -> BitResult<Vec<u8>> {
    if r.left() < n * 8 {
        return Err(BitError::Short);
    }
    (0..n).map(|_| r.u8(8)).collect()
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
    /// FinaliseSessionEntry (34; send 1417F7980): no fields; the console has loaded the session.
    FinaliseSessionEntry,
    /// MsgBreakGlass (20), MsgCarCollision (21), MsgRepairDamage (35).
    CarEvent(CarEvent),
    TagCollision(TagCollision),
    /// The ServerTraffic* messages and ClientTrafficMessage: the body after the id, raw.
    Traffic { id: u8, body: Bits },
    CollObjDetachPart { kind: u8, data: Vec<u8> },
    ChatManagement { xuid: u64, data: Vec<u8> },
    InactivePlayersPulled(Handle),
    /// SetGroupSatNavEvent (52): the lobby handle and the node (0: none).
    SetGroupSatNav { handle: Handle, node: i32 },
    /// A message read only to get past it (`skip_other`): the body after the id, raw.
    Other { id: u8, body: Bits },
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
            Msg::CarEvent(e) => e.id,
            Msg::TagCollision(_) => MSG_TAG_COLLISION,
            Msg::Traffic { id, .. } => *id,
            Msg::CollObjDetachPart { .. } => MSG_COLL_OBJ_DETACH_PART,
            Msg::ChatManagement { .. } => MSG_CHAT_MANAGEMENT,
            Msg::InactivePlayersPulled(_) => MSG_INACTIVE_PLAYERS_PULLED,
            Msg::SetGroupSatNav { .. } => MSG_SET_GROUP_SAT_NAV,
            Msg::Other { id, .. } => *id,
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
            MSG_BREAK_GLASS | MSG_CAR_COLLISION | MSG_REPAIR_DAMAGE => {
                let handle = Handle::read(r, CAR_ID_BITS)?;
                let n = car_event_bits(id).unwrap_or(0);
                Msg::CarEvent(CarEvent { id, handle, body: r.take(n)? })
            }
            MSG_TAG_COLLISION => Msg::TagCollision(TagCollision {
                a: Handle::read(r, CAR_ID_BITS)?,
                b: Handle::read(r, CAR_ID_BITS)?,
                vals: [r.u8(3)?, r.u8(3)?, r.u8(3)?, r.u8(3)?],
            }),
            MSG_SERVER_TRAFFIC_FULL_SYNC
            | MSG_SERVER_TRAFFIC_ROUTE_PROGRESS
            | MSG_SERVER_TRAFFIC_MESSAGE
            | MSG_CLIENT_TRAFFIC_MESSAGE
            | MSG_SERVER_TRAFFIC_FULL_ROUTE
            | MSG_SERVER_TRAFFIC_STATIONARY => {
                let n = traffic_bits(id).unwrap_or(0);
                Msg::Traffic { id, body: r.take(n)? }
            }
            MSG_COLL_OBJ_DETACH_PART => {
                let kind = r.u8(4)?;
                let n = r.bits(11)? as usize;
                Msg::CollObjDetachPart { kind, data: read_bytes(r, n)? }
            }
            MSG_CHAT_MANAGEMENT => {
                let xuid = r.bits(64)?;
                let n = r.bits(7)? as usize;
                Msg::ChatManagement { xuid, data: read_bytes(r, n)? }
            }
            MSG_INACTIVE_PLAYERS_PULLED => Msg::InactivePlayersPulled(Handle::read(r, INACTIVE_PLAYERS_ID_BITS)?),
            MSG_SET_GROUP_SAT_NAV => Msg::SetGroupSatNav {
                handle: Handle::read(r, SAT_NAV_ID_BITS)?,
                node: r.u32(32)? as i32,
            },
            other if is_other(other) => {
                let mut probe = r.clone();
                skip_other(&mut probe, other)?;
                let n = probe.pos() - r.pos();
                Msg::Other { id: other, body: r.take(n)? }
            }
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
            Msg::CarEvent(e) => {
                debug_assert_eq!(Some(e.body.len()), car_event_bits(e.id));
                e.handle.write(&mut w, CAR_ID_BITS);
                w.append(&e.body);
            }
            Msg::TagCollision(t) => {
                t.a.write(&mut w, CAR_ID_BITS);
                t.b.write(&mut w, CAR_ID_BITS);
                for v in t.vals {
                    w.push(v as u64, 3);
                }
            }
            Msg::Traffic { id, body } => {
                debug_assert_eq!(Some(body.len()), traffic_bits(*id));
                w.append(body);
            }
            Msg::CollObjDetachPart { kind, data } => {
                debug_assert!(data.len() < 1 << 11);
                w.push(*kind as u64, 4).push(data.len() as u64, 11);
                for &b in data {
                    w.push(b as u64, 8);
                }
            }
            Msg::ChatManagement { xuid, data } => {
                debug_assert!(data.len() < 1 << 7);
                w.push(*xuid, 64).push(data.len() as u64, 7);
                for &b in data {
                    w.push(b as u64, 8);
                }
            }
            Msg::InactivePlayersPulled(h) => h.write(&mut w, INACTIVE_PLAYERS_ID_BITS),
            Msg::SetGroupSatNav { handle, node } => {
                handle.write(&mut w, SAT_NAV_ID_BITS);
                w.push(*node as u32 as u64, 32);
            }
            Msg::Other { body, .. } => {
                w.append(body);
            }
        }
        w
    }
}

/// A server update setting one of the server's bools on a console's own ConsoleSynced object.
pub fn console_bool_update(console_id: u32, seq: u8, index: u8, value: bool) -> Msg {
    console_update(console_id, seq, &[(index, value)])
}

/// A server update of several of a console's bool vars.
pub fn console_update(console_id: u32, seq: u8, vars: &[(u8, bool)]) -> Msg {
    let mut payload = Bits::new();
    for &(index, value) in vars {
        payload.push(index as u64, CONSOLE_VAR_INDEX_BITS).bool(value);
    }
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
        w.push(18, 7).push(0, 30);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownMsg(18)));
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
    fn other_messages_roundtrip() {
        let handles = |id| [Handle::Own(id), Handle::Relay(id, 3), Handle::Server(id), Handle::Transfer(id)];
        for id in [MSG_BREAK_GLASS, MSG_CAR_COLLISION, MSG_REPAIR_DAMAGE] {
            let n = car_event_bits(id).unwrap();
            for h in handles(127) {
                roundtrip(Msg::CarEvent(CarEvent { id, handle: h, body: payload(n, id as u64) }));
            }
        }
        roundtrip(Msg::TagCollision(TagCollision { a: Handle::Own(1), b: Handle::Relay(2, 9), vals: [7, 0, 5, 1] }));
        for id in [16, 17, 71, 72, 73, 74] {
            roundtrip(Msg::Traffic { id, body: payload(traffic_bits(id).unwrap(), id as u64) });
        }
        roundtrip(Msg::CollObjDetachPart { kind: 15, data: vec![] });
        roundtrip(Msg::CollObjDetachPart { kind: 3, data: (0..2047).map(|i| i as u8).collect() });
        roundtrip(Msg::ChatManagement { xuid: u64::MAX - 5, data: (0..127).collect() });
        roundtrip(Msg::InactivePlayersPulled(Handle::Own(3)));
        roundtrip(Msg::InactivePlayersPulled(Handle::Relay(1, 200)));
    }

    #[test]
    fn other_message_sizes() {
        // id + flags + 7-bit id (+ the body)
        let repair = Msg::CarEvent(CarEvent { id: MSG_REPAIR_DAMAGE, handle: Handle::Own(2), body: Bits::new() });
        assert_eq!(repair.write().len(), 7 + 2 + 7);
        assert_eq!(car_event_bits(MSG_BREAK_GLASS), Some(352));
        assert_eq!(car_event_bits(MSG_CAR_COLLISION), Some(192));
        assert_eq!(car_event_bits(MSG_CAR), None);
        let sizes: Vec<_> = [16, 17, 71, 72, 73, 74].iter().map(|&id| traffic_bits(id).unwrap()).collect();
        assert_eq!(sizes, [314, 200, 72, 72, 330, 185]);
        assert_eq!(traffic_bits(18), None);
        let t = Msg::TagCollision(TagCollision { a: Handle::Own(1), b: Handle::Own(2), vals: [0; 4] });
        assert_eq!(t.write().len(), 7 + 9 + 9 + 12);
        assert_eq!(Msg::InactivePlayersPulled(Handle::Own(1)).write().len(), 7 + 2 + 2);
        // a byte count over what is there: truncated, not a huge allocation
        let mut w = Bits::new();
        w.push(MSG_COLL_OBJ_DETACH_PART as u64, 7).push(0, 4).push(2047, 11).push(0, 16);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::Bits(BitError::Short)));
        assert_eq!(msg_name(76), "ClientTrafficStationary");
        assert_eq!(msg_name(35), "MsgRepairDamage");
        assert_eq!(msg_name(77), "?");
    }

    /// The parse goes on after them: [RepairDamage, ServerTrafficMessage, Car] in one packet.
    #[test]
    fn known_messages_do_not_end_the_packet() {
        let car = Msg::Car(Car { handle: Handle::Own(4), body: payload(CAR_BODY_BITS, 1) });
        let msgs = [
            Msg::CarEvent(CarEvent { id: MSG_REPAIR_DAMAGE, handle: Handle::Own(4), body: Bits::new() }),
            Msg::Traffic { id: MSG_SERVER_TRAFFIC_MESSAGE, body: payload(72, 2) },
            car,
        ];
        let bits: Vec<Bits> = msgs.iter().map(|m| m.write()).collect();
        let data = packet(&bits, NO_SEQ, NO_ACK, 0);
        let mut r = Reader::at(&data, HEADER_LEN * 8);
        for m in &msgs {
            assert_eq!(&Msg::read(&mut r).unwrap(), m);
        }
    }

    /// A body for `other_layout(id)`, with every handle form and string length used somewhere.
    fn other_body(id: u8, seed: u64) -> Bits {
        let mut w = Bits::new();
        for (k, f) in other_layout(id).unwrap().iter().enumerate() {
            match *f {
                Field::Bits(n) => {
                    w.append(&payload(n, seed + k as u64));
                }
                Field::Handle(bits) => {
                    let h = match (seed as usize + k) % 4 {
                        0 => Handle::Own(1),
                        1 => Handle::Relay((1 << bits) - 1, 200),
                        2 => Handle::Server(0),
                        _ => Handle::Transfer(1),
                    };
                    h.write(&mut w, bits);
                }
                Field::Bytes(max) => {
                    let n = if max == 8 { 8 } else { (seed as usize + k) % (max + 1) };
                    w.string(&(0..n as u8).collect::<Vec<_>>());
                }
            }
        }
        w
    }

    fn raw(id: u8, body: &Bits) -> Bits {
        let mut w = Bits::new();
        w.push(id as u64, 7).append(body);
        w
    }

    #[test]
    fn set_group_sat_nav_roundtrip_and_layout() {
        for h in [Handle::Own(0), Handle::Relay(3, 9), Handle::Server(1), Handle::Transfer(2)] {
            for node in [0, 1, -1, i32::MAX, i32::MIN, 123456] {
                roundtrip(Msg::SetGroupSatNav { handle: h, node });
            }
        }
        // 7 id + 2 flags + 2-bit id + s32
        let m = Msg::SetGroupSatNav { handle: Handle::Own(0), node: -2 };
        let w = m.write();
        assert_eq!(w.len(), 43);
        let mut r = Reader::new(w.as_bytes());
        assert_eq!(r.bits(7).unwrap(), 52);
        assert_eq!(r.bits(4).unwrap(), 0);
        assert_eq!(r.bits(32).unwrap(), 0xFFFF_FFFE);
        assert_eq!(msg_name(MSG_SET_GROUP_SAT_NAV), "SetGroupSatNavEvent");
        // the server's answer: var 12, 32 bits
        let p = group_sat_nav_payload(-2);
        assert_eq!(p.len(), 36);
        let mut r = Reader::new(p.as_bytes());
        assert_eq!(r.bits(4).unwrap(), 12);
        assert_eq!(r.bits(32).unwrap() as u32 as i32, -2);
    }

    #[test]
    fn other_messages_read_exactly() {
        let ids = [14u8, 26, 27, 31, 32, 38, 41, 43, 44, 45, 46, 47, 49, 51, 54, 56, 58, 59, 60, 62, 63, 64];
        for id in ids {
            assert!(is_other(id), "{id}");
            for seed in 0..4 {
                let body = other_body(id, seed);
                let m = Msg::Other { id, body: body.clone() };
                assert_eq!(m.write(), raw(id, &body));
                roundtrip(m);
            }
        }
        // the fixed sizes, after the id (handles in their own form: 2 flag bits + the id)
        let size = |id| other_body(id, 0).len();
        assert_eq!(size(14), 2 + 7 + 44);
        assert_eq!(size(26), 34);
        assert_eq!(size(27), 35);
        assert_eq!(size(31), 32);
        assert_eq!(size(38), 0);
        assert_eq!(size(43), 2 + 7 + 33);
        assert_eq!(size(44), 2 + 7 + 3 * 72 + 34);
        assert_eq!(size(45), 2 + 7 + 5 * 72 + 59);
        assert_eq!(size(46), 2 + 2 + 64);
        assert_eq!(size(47), 2 + 2 + 32);
        assert_eq!(size(51), 48);
        assert_eq!(size(54), 384);
        assert_eq!(size(56), 224);
        assert_eq!(size(58), 96);
        assert_eq!(size(59), 2 + 7);
        assert_eq!(size(63), 64);
        // still unknown: server-to-console ones, and the ownership messages with no sender
        for id in [18u8, 19, 22, 25, 37, 42, 48, 50, 53, 55, 57, 61, 65, 69, 75, 76] {
            assert!(!is_other(id), "{id}");
            let mut w = Bits::new();
            w.push(id as u64, 7).push(0, 40);
            assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownMsg(id)), "{id}");
        }
        // a string over its limit, a f64 that is not 8 bytes
        let mut w = Bits::new();
        w.push(32, 7).string(&[0; 65]).string(&[]);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::Bits(BitError::TooLong)));
        let mut w = Bits::new();
        w.push(44, 7);
        Handle::Own(1).write(&mut w, 7);
        w.string(&[0; 9]).push(0, 64);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::Bits(BitError::TooLong)));
    }

    #[test]
    fn ownership_answers() {
        // a request as the PC sent it: GameOptions, Transfer handle id 0
        let mut b = Bits::new();
        b.push(6, 4).push(GAME_OPTIONS_TYPE as u64, 2);
        Handle::Transfer(0).write(&mut b, 2);
        assert_eq!(
            ownership_request(&b),
            Some(OwnershipRequest { router: 6, ty: GAME_OPTIONS_TYPE, handle: Handle::Transfer(0) })
        );
        // grant: 7 id + 4 router + 2 flags + 2 id + 8 seq + 14 size, no vars
        let g = grant_ownership(6, Handle::Transfer(0), 0, &Bits::new());
        assert_eq!(g.len(), 7 + 4 + 4 + 8 + 14);
        let mut r = Reader::new(g.as_bytes());
        assert_eq!(r.u8(7), Ok(MSG_OBJECT_GRANT_OWNERSHIP));
        assert_eq!(r.u8(4), Ok(6));
        assert_eq!(Handle::read(&mut r, 2), Ok(Handle::Transfer(0)));
        assert_eq!((r.u8(8), r.bits(14)), (Ok(0), Ok(0)));
        let t = transfer_ownership(6, Handle::Transfer(1), 7);
        assert_eq!(t.len(), 7 + 4 + 4 + 8);
        let mut r = Reader::new(t.as_bytes());
        assert_eq!((r.u8(7), r.u8(4)), (Ok(MSG_OBJECT_TRANSFER_OWNERSHIP), Ok(6)));
        assert_eq!((Handle::read(&mut r, 2), r.u8(8)), (Ok(Handle::Transfer(1)), Ok(7)));
        assert_eq!(ownership_request(&Bits::new()), None);
        // read back as messages (what the tests' consoles do)
        for m in [g, t] {
            let mut r = Reader::new(m.as_bytes());
            let read = Msg::read(&mut r).unwrap();
            assert_eq!(read.write(), m);
        }
    }

    #[test]
    fn ownership_messages() {
        // ObjectRequestOwnership: router 6 has a 2-bit type field, router 1 none
        let mut b = Bits::new();
        b.push(6, 4).push(1, 2);
        Handle::Relay(3, 7).write(&mut b, 2);
        roundtrip(Msg::Other { id: MSG_OBJECT_REQUEST_OWNERSHIP, body: b.clone() });
        assert_eq!(b.len(), 4 + 2 + 2 + 2 + 8);
        let mut b = Bits::new();
        b.push(1, 4);
        Handle::Own(127).write(&mut b, 7);
        roundtrip(Msg::Other { id: MSG_OBJECT_REQUEST_OWNERSHIP, body: b });
        // ObjectNeedsUpdates on a ConsoleSynced object: 2 vars
        let mut b = Bits::new();
        b.push(3, 4);
        Handle::Relay(5, 2).write(&mut b, 7);
        b.push(2, 16).push(0, 2).push(1, 4).push(0, 2).push(4, 4);
        roundtrip(Msg::Other { id: MSG_OBJECT_NEEDS_UPDATES, body: b.clone() });
        assert_eq!(b.len(), 4 + 17 + 16 + 12);
        // ... and on a router whose var index width is not known: the parse ends
        let mut w = Bits::new();
        w.push(13, 7).push(1, 4);
        Handle::Own(1).write(&mut w, 7);
        w.push(1, 16).push(0, 16);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownMsg(13)));
        let mut w = Bits::new();
        w.push(10, 7).push(10, 4).push(0, 30);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::UnknownRouter(10)));
        // a huge count is a truncation, not an allocation
        let mut w = Bits::new();
        w.push(13, 7).push(3, 4);
        Handle::Own(1).write(&mut w, 7);
        w.push(0xFFFF, 16).push(0, 16);
        assert_eq!(Msg::read(&mut Reader::new(w.as_bytes())), Err(ReadError::Bits(BitError::Short)));
    }

    /// What the Deck sent in free roam: SetGroupSatNavEvent ahead of a car, and others.
    #[test]
    fn sat_nav_and_others_do_not_end_the_packet() {
        let msgs = [
            Msg::SetGroupSatNav { handle: Handle::Own(0), node: 77 },
            Msg::Other { id: 59, body: other_body(59, 1) },
            Msg::Other { id: 45, body: other_body(45, 2) },
            Msg::Other { id: 32, body: other_body(32, 3) },
            Msg::Car(Car { handle: Handle::Own(4), body: payload(CAR_BODY_BITS, 1) }),
        ];
        let bits: Vec<Bits> = msgs.iter().map(|m| m.write()).collect();
        let data = packet(&bits, NO_SEQ, NO_ACK, 0);
        let mut r = Reader::at(&data, HEADER_LEN * 8);
        for m in &msgs {
            assert_eq!(&Msg::read(&mut r).unwrap(), m);
        }
    }

    fn entry(w: &mut Bits, index: u32, index_bits: u32, data: &[(u64, u32)]) {
        w.push(index as u64, index_bits);
        for &(v, n) in data {
            w.push(v, n);
        }
    }

    fn f(x: f32) -> (u64, u32) {
        (x.to_bits() as u64, 32)
    }

    #[test]
    fn console_payloads() {
        // RenderReady, HasLoadedCars, AvailableArbitration (2 bits): kept as they are
        let mut p = Bits::new();
        entry(&mut p, 0, 4, &[(1, 1)]);
        entry(&mut p, 5, 4, &[(1, 1)]);
        entry(&mut p, 7, 4, &[(3, 2)]);
        assert_eq!(sanitize_payload(3, &p), Ok(p.clone()));
        // the server's vars (1, 2, 4) are stripped
        let mut with = p.clone();
        entry(&mut with, 1, 4, &[(1, 1)]);
        entry(&mut with, 4, 4, &[(1, 1)]);
        assert_eq!(sanitize_payload(3, &with), Ok(p.clone()));
        // index 9 and up, a duplicate, a cut entry
        let mut bad = Bits::new();
        entry(&mut bad, 9, 4, &[(1, 1)]);
        assert_eq!(sanitize_payload(3, &bad), Err(PayloadError::Index(9)));
        let mut dup = p.clone();
        entry(&mut dup, 0, 4, &[(0, 1)]);
        assert_eq!(sanitize_payload(3, &dup), Err(PayloadError::Duplicate(0)));
        let mut cut = Bits::new();
        cut.push(7, 4).push(1, 1);
        assert_eq!(sanitize_payload(3, &cut), Err(PayloadError::Truncated));
        // the largest one possible, and one bit more
        assert_eq!(payload_max_bits(3), Some(47));
        let mut big = Bits::new();
        big.push(0, 48);
        assert_eq!(sanitize_payload(3, &big), Err(PayloadError::TooBig(48)));
        assert_eq!(sanitize_payload(3, &Bits::new()), Ok(Bits::new()));
    }

    #[test]
    fn car_payloads() {
        let mut p = Bits::new();
        entry(&mut p, 8, 5, &[f(0.75)]); // Fuel
        entry(&mut p, 0, 5, &[f(0.0)]);
        entry(&mut p, 21, 5, &[(17, 5)]); // CarUniverseId
        assert_eq!(sanitize_payload(2, &p), Ok(p.clone()));
        for bad in [f32::NAN, f32::INFINITY, -f32::INFINITY, 2.0e6, -1.0e7] {
            let mut q = Bits::new();
            entry(&mut q, 3, 5, &[f(bad)]);
            assert_eq!(sanitize_payload(2, &q), Err(PayloadError::NotFinite(3)), "{bad}");
        }
        let mut q = Bits::new();
        entry(&mut q, 22, 5, &[(0, 32)]);
        assert_eq!(sanitize_payload(2, &q), Err(PayloadError::Index(22)));
        assert_eq!(payload_max_bits(2), Some(787));
    }

    #[test]
    fn player_payloads() {
        let name: Vec<(u64, u32)> = "Alex".encode_utf16().map(|c| (c as u64, 16)).collect();
        let mut p = Bits::new();
        entry(&mut p, 0, 8, &[(0x0102_0304_0506_0708, 64)]);
        let mut n = vec![(name.len() as u64, 5)];
        n.extend(&name);
        entry(&mut p, 3, 8, &n);
        entry(&mut p, 4, 8, &[(2535412345678901, 64)]);
        assert_eq!(sanitize_payload(1, &p), Ok(p.clone()));
        // 16 units: the most the reader takes; 17: refused
        for (len, ok) in [(16u64, true), (17, false), (31, false)] {
            let mut q = Bits::new();
            q.push(3, 8).push(len, 5);
            for _ in 0..len {
                q.push(0x41, 16);
            }
            assert_eq!(sanitize_payload(1, &q).is_ok(), ok, "{len}");
        }
        // the unmapped vars (12..163): the rest is kept as it is, within the cap
        let mut t = p.clone();
        t.push(40, 8).push(0xABCDEF, 24).push(3, 8).push(0, 5);
        assert_eq!(sanitize_payload(1, &t), Ok(t.clone()));
        let mut q = Bits::new();
        q.push(163, 8).push(0, 8);
        assert_eq!(sanitize_payload(1, &q), Err(PayloadError::Index(163)));
        let mut huge = Bits::new();
        huge.push(12, 8);
        while huge.len() <= PLAYER_DATA_MAX_BITS {
            huge.push(0, 64);
        }
        assert!(matches!(sanitize_payload(1, &huge), Err(PayloadError::TooBig(_))));
        // the other routers are not forwarded at all
        for r in [0u8, 4, 6, 9, 12, 15] {
            assert_eq!(sanitize_payload(r, &Bits::new()), Err(PayloadError::Router(r)));
            assert!(!forwarded_router(r));
        }
    }

    pub(crate) fn car_body_at(x: f32, y: f32, z: f32, seed: u64) -> Bits {
        let mut b = Bits::new();
        b.push(x.to_bits() as u64, 32).push(y.to_bits() as u64, 32).push(z.to_bits() as u64, 32);
        b.append(&payload(CAR_BODY_BITS - 96, seed));
        b
    }

    #[test]
    fn car_and_event_values() {
        assert!(car_ok(&car_body_at(1234.5, -20.0, 3000.0, 1)));
        for bad in [f32::NAN, f32::INFINITY, 1.5e5, -2.0e5] {
            assert!(!car_ok(&car_body_at(0.0, bad, 0.0, 1)), "{bad}");
        }
        assert!(!car_ok(&payload(CAR_BODY_BITS - 1, 1)));
        let mut glass = Bits::new();
        glass.push(7, 32);
        for k in 0..9 {
            glass.push((k as f32 * 1.5).to_bits() as u64, 32);
        }
        glass.push(3, 32);
        assert!(car_event_ok(MSG_BREAK_GLASS, &glass));
        let mut bad = Bits::new();
        bad.push(7, 32);
        for k in 0..9 {
            let v = if k == 4 { f32::NAN } else { 1.0 };
            bad.push(v.to_bits() as u64, 32);
        }
        bad.push(3, 32);
        assert!(!car_event_ok(MSG_BREAK_GLASS, &bad));
        let mut coll = Bits::new();
        for v in [1.0f32, 2.0, 3.0, 0.5, 1.0e5, -9.0] {
            coll.push(v.to_bits() as u64, 32);
        }
        assert!(car_event_ok(MSG_CAR_COLLISION, &coll));
        let mut coll2 = Bits::new();
        for v in [1.0f32, 2.0, 3.0, 0.5, 1.0e9, -9.0] {
            coll2.push(v.to_bits() as u64, 32);
        }
        assert!(!car_event_ok(MSG_CAR_COLLISION, &coll2));
        assert!(car_event_ok(MSG_REPAIR_DAMAGE, &Bits::new()));
        assert!(!car_event_ok(MSG_CAR, &Bits::new()));
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
