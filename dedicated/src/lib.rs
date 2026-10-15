//! Forza Horizon 2 free roam game server, run by a player (PLANO_FREEROAM.md). The game connects
//! to it over NerdCore (UDP) and it relays each console's objects and cars to the others, as
//! Turn 10's Xbox Live Compute servers did. std only.

pub mod bits;
pub mod config;
pub mod nerd;
pub mod net;
pub mod relay;
