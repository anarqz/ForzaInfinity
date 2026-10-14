//! Car Clubs: clubs, members, invites, join requests, history and the notice board.
//!
//! Values the game uses (from its string tables, FUN_140c197d0 / FUN_140bbbfd0):
//! - roleInClub: 1 = Leader, 2 = CoLeader, 3 = Member (0 = not in a club);
//! - privacy: 0 = Open, 1 = OpenToFriends, 2 = InviteOnly, 3 = Private.
//!
//! A player is in at most one club. When the leader leaves, the oldest co-leader (or member)
//! takes over; a club with no members is deleted.

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::Store;
use crate::unix_ms;

pub const ROLE_LEADER: i64 = 1;
pub const ROLE_COLEADER: i64 = 2;
pub const ROLE_MEMBER: i64 = 3;

pub const PRIVACY_OPEN: i64 = 0;
pub const PRIVACY_FRIENDS: i64 = 1;
pub const PRIVACY_INVITE_ONLY: i64 = 2;

/// ClubHistoryEntryData.action (our own numbering; the game only shows it).
pub const ACTION_JOINED: i64 = 0;
pub const ACTION_LEFT: i64 = 1;
pub const ACTION_REMOVED: i64 = 2;
pub const ACTION_ROLE: i64 = 3;
pub const ACTION_CREATED: i64 = 4;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS clubs (
        id INTEGER PRIMARY KEY AUTOINCREMENT, tag TEXT NOT NULL UNIQUE COLLATE NOCASE,
        name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', privacy INTEGER NOT NULL DEFAULT 0,
        region INTEGER NOT NULL DEFAULT 0, attitude INTEGER NOT NULL DEFAULT 0,
        interest INTEGER NOT NULL DEFAULT 0, xp INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS club_members (
        xuid INTEGER PRIMARY KEY, club INTEGER NOT NULL, role INTEGER NOT NULL,
        joined INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS club_members_club ON club_members (club);
    CREATE TABLE IF NOT EXISTS club_invites (
        id INTEGER PRIMARY KEY AUTOINCREMENT, club INTEGER NOT NULL, xuid INTEGER NOT NULL,
        sender INTEGER NOT NULL, created INTEGER NOT NULL, UNIQUE (club, xuid));
    CREATE TABLE IF NOT EXISTS club_requests (
        club INTEGER NOT NULL, xuid INTEGER NOT NULL, created INTEGER NOT NULL,
        PRIMARY KEY (club, xuid));
    CREATE TABLE IF NOT EXISTS club_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT, xuid INTEGER NOT NULL, club_name TEXT NOT NULL,
        role INTEGER NOT NULL, action INTEGER NOT NULL, time INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS club_notices (
        id INTEGER PRIMARY KEY AUTOINCREMENT, club INTEGER NOT NULL, xuid INTEGER NOT NULL,
        message TEXT NOT NULL, created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS user_settings (
        xuid INTEGER PRIMARY KEY, accepts_club_invites INTEGER NOT NULL DEFAULT 1);";

#[derive(Clone, Debug)]
pub struct Club {
    pub id: i64,
    pub tag: String,
    pub name: String,
    pub description: String,
    pub privacy: i64,
    pub region: i64,
    pub attitude: i64,
    pub interest: i64,
    pub created: i64,
}

/// The fields a club can be created or edited with.
#[derive(Clone, Debug, Default)]
pub struct ClubFields {
    pub tag: String,
    pub name: String,
    pub description: String,
    pub privacy: i64,
    pub region: i64,
    pub attitude: i64,
    pub interest: i64,
}

const CLUB_COLUMNS: &str = "id, tag, name, description, privacy, region, attitude, interest, created";

fn club_row(r: &rusqlite::Row) -> rusqlite::Result<Club> {
    Ok(Club {
        id: r.get(0)?,
        tag: r.get(1)?,
        name: r.get(2)?,
        description: r.get(3)?,
        privacy: r.get(4)?,
        region: r.get(5)?,
        attitude: r.get(6)?,
        interest: r.get(7)?,
        created: r.get(8)?,
    })
}

/// The game's limits: tag 2..4 characters, name up to 24, description up to 200.
pub fn validate(fields: &ClubFields) -> Result<(), String> {
    let tag = fields.tag.trim();
    if !(2..=4).contains(&tag.chars().count()) || !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("tag: 2 a 4 letras ou dígitos".into());
    }
    let name = fields.name.trim();
    if name.is_empty() || name.chars().count() > 24 {
        return Err("nome: 1 a 24 caracteres".into());
    }
    if fields.description.chars().count() > 200 {
        return Err("descrição: até 200 caracteres".into());
    }
    if !(0..=3).contains(&fields.privacy) {
        return Err("privacidade: 0 a 3".into());
    }
    Ok(())
}

impl Store {
    pub fn club(&self, id: i64) -> Option<Club> {
        self.with(|db| {
            db.query_row(&format!("SELECT {CLUB_COLUMNS} FROM clubs WHERE id = ?1"), [id], club_row).optional()
        })
        .flatten()
    }

    pub fn club_by_tag(&self, tag: &str) -> Option<Club> {
        self.with(|db| {
            db.query_row(&format!("SELECT {CLUB_COLUMNS} FROM clubs WHERE tag = ?1 COLLATE NOCASE"), [tag.trim()], club_row)
                .optional()
        })
        .flatten()
    }

    /// Every club, biggest first.
    pub fn clubs(&self) -> Vec<Club> {
        self.with(|db| {
            let mut stmt = db.prepare(&format!(
                "SELECT {CLUB_COLUMNS} FROM clubs c
                 ORDER BY (SELECT COUNT(*) FROM club_members m WHERE m.club = c.id) DESC, c.xp DESC, c.id"
            ))?;
            let rows = stmt.query_map([], club_row)?.flatten().collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// (club id, role) of the player's club.
    pub fn membership(&self, xuid: u64) -> Option<(i64, i64)> {
        self.with(|db| {
            db.query_row("SELECT club, role FROM club_members WHERE xuid = ?1", [xuid as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
        })
        .flatten()
    }

    /// (xuid, role, joined), leaders first.
    pub fn club_members(&self, club: i64) -> Vec<(u64, i64, i64)> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT xuid, role, joined FROM club_members WHERE club = ?1 ORDER BY role, joined")?;
            let rows = stmt
                .query_map([club], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?)))?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    fn club_history_add(&self, xuid: u64, club: &Club, role: i64, action: i64) {
        self.with(|db| {
            db.execute(
                "INSERT INTO club_history (xuid, club_name, role, action, time) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![xuid as i64, club.name, role, action, unix_ms()],
            )
        });
    }

    /// (club name, role, action, time), newest first.
    pub fn club_history(&self, xuid: u64, max: i64) -> Vec<(String, i64, i64, i64)> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT club_name, role, action, time FROM club_history WHERE xuid = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![xuid as i64, max.max(1)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    pub fn create_club(&self, owner: u64, fields: &ClubFields) -> Result<i64, String> {
        validate(fields)?;
        if self.membership(owner).is_some() {
            return Err("o jogador já está em um clube".into());
        }
        if self.club_by_tag(&fields.tag).is_some() {
            return Err("essa tag já existe".into());
        }
        let now = unix_ms();
        let id = self
            .with(|db| {
                db.execute(
                    "INSERT INTO clubs (tag, name, description, privacy, region, attitude, interest, created)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        fields.tag.trim().to_uppercase(),
                        fields.name.trim(),
                        fields.description.trim(),
                        fields.privacy,
                        fields.region,
                        fields.attitude,
                        fields.interest,
                        now
                    ],
                )?;
                Ok(db.last_insert_rowid())
            })
            .ok_or("erro no banco")?;
        self.add_member(id, owner, ROLE_LEADER);
        if let Some(club) = self.club(id) {
            self.club_history_add(owner, &club, ROLE_LEADER, ACTION_CREATED);
        }
        Ok(id)
    }

    /// Edits name, description, privacy and keywords (the tag stays).
    pub fn update_club(&self, id: i64, fields: &ClubFields) -> Result<(), String> {
        let club = self.club(id).ok_or("clube desconhecido")?;
        let checked = ClubFields { tag: club.tag.clone(), ..fields.clone() };
        validate(&checked)?;
        self.with(|db| {
            db.execute(
                "UPDATE clubs SET name = ?2, description = ?3, privacy = ?4, region = ?5, attitude = ?6, interest = ?7
                 WHERE id = ?1",
                params![
                    id,
                    checked.name.trim(),
                    checked.description.trim(),
                    checked.privacy,
                    checked.region,
                    checked.attitude,
                    checked.interest
                ],
            )
        })
        .ok_or_else(|| "erro no banco".to_owned())
        .map(|_| ())
    }

    pub fn delete_club(&self, id: i64) -> bool {
        let Some(club) = self.club(id) else { return false };
        for (xuid, role, _) in self.club_members(id) {
            self.club_history_add(xuid, &club, role, ACTION_REMOVED);
        }
        self.with(|db| {
            for sql in [
                "DELETE FROM club_members WHERE club = ?1",
                "DELETE FROM club_invites WHERE club = ?1",
                "DELETE FROM club_requests WHERE club = ?1",
                "DELETE FROM club_notices WHERE club = ?1",
                "DELETE FROM clubs WHERE id = ?1",
            ] {
                db.execute(sql, [id])?;
            }
            Ok(())
        })
        .is_some()
    }

    /// Puts the player in the club (leaving any other one first).
    pub fn add_member(&self, club: i64, xuid: u64, role: i64) {
        if let Some((current, _)) = self.membership(xuid) {
            if current == club {
                return;
            }
            self.leave_club(xuid, ACTION_LEFT);
        }
        self.with(|db| {
            db.execute(
                "INSERT INTO club_members (xuid, club, role, joined) VALUES (?1, ?2, ?3, ?4)",
                params![xuid as i64, club, role, unix_ms()],
            )?;
            db.execute("DELETE FROM club_invites WHERE xuid = ?1", [xuid as i64])?;
            db.execute("DELETE FROM club_requests WHERE xuid = ?1", [xuid as i64])
        });
        if role != ROLE_LEADER {
            if let Some(c) = self.club(club) {
                self.club_history_add(xuid, &c, role, ACTION_JOINED);
            }
        }
    }

    /// Leaves (or is removed from) the club. A leader hands over to the oldest co-leader or
    /// member; the last member out deletes the club.
    pub fn leave_club(&self, xuid: u64, action: i64) -> bool {
        let Some((club_id, role)) = self.membership(xuid) else { return false };
        let club = self.club(club_id);
        self.with(|db| db.execute("DELETE FROM club_members WHERE xuid = ?1", [xuid as i64]));
        if let Some(c) = &club {
            self.club_history_add(xuid, c, role, action);
        }
        let rest = self.club_members(club_id);
        if rest.is_empty() {
            self.delete_club(club_id);
        } else if role == ROLE_LEADER {
            // club_members comes sorted by role, then join date
            let (heir, _, _) = rest[0];
            self.set_member_role(club_id, heir, ROLE_LEADER);
        }
        true
    }

    /// Changes a member's role; a new leader turns the old one into a co-leader.
    pub fn set_member_role(&self, club: i64, xuid: u64, role: i64) -> bool {
        if !(ROLE_LEADER..=ROLE_MEMBER).contains(&role) || self.membership(xuid).map(|m| m.0) != Some(club) {
            return false;
        }
        self.with(|db| {
            if role == ROLE_LEADER {
                db.execute(
                    "UPDATE club_members SET role = ?2 WHERE club = ?1 AND role = ?3",
                    params![club, ROLE_COLEADER, ROLE_LEADER],
                )?;
            }
            db.execute("UPDATE club_members SET role = ?3 WHERE club = ?1 AND xuid = ?2", params![club, xuid as i64, role])
        });
        if let Some(c) = self.club(club) {
            self.club_history_add(xuid, &c, role, ACTION_ROLE);
        }
        true
    }

    pub fn invite_to_club(&self, club: i64, sender: u64, xuid: u64) -> Result<i64, String> {
        if self.membership(xuid).map(|m| m.0) == Some(club) {
            return Err("o jogador já está no clube".into());
        }
        if !self.accepts_club_invites(xuid) {
            return Err("o jogador não aceita convites de clube".into());
        }
        self.with(|db| {
            db.execute(
                "INSERT INTO club_invites (club, xuid, sender, created) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (club, xuid) DO UPDATE SET sender = ?3, created = ?4",
                params![club, xuid as i64, sender as i64, unix_ms()],
            )?;
            db.query_row("SELECT id FROM club_invites WHERE club = ?1 AND xuid = ?2", params![club, xuid as i64], |r| {
                r.get(0)
            })
        })
        .ok_or_else(|| "erro no banco".into())
    }

    /// (invite id, club, sender, created) for invites sent to the player.
    pub fn club_invites_for(&self, xuid: u64) -> Vec<(i64, i64, u64, i64)> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT id, club, sender, created FROM club_invites WHERE xuid = ?1 ORDER BY id")?;
            let rows = stmt
                .query_map([xuid as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64, r.get(3)?)))?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// (invite id, invited xuid, sender, created) for a club's pending invites.
    pub fn club_invites_of(&self, club: i64) -> Vec<(i64, u64, u64, i64)> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT id, xuid, sender, created FROM club_invites WHERE club = ?1 ORDER BY id")?;
            let rows = stmt
                .query_map([club], |r| {
                    Ok((r.get(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64, r.get(3)?))
                })?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    pub fn delete_club_invite(&self, id: i64) -> bool {
        self.with(|db| db.execute("DELETE FROM club_invites WHERE id = ?1", [id])).is_some_and(|n| n > 0)
    }

    /// Accepts an invite: by its id, or any invite from that club when the id is unknown.
    pub fn accept_club_invite(&self, xuid: u64, club: i64, invitation: i64) -> Result<(), String> {
        let found = self.club_invites_for(xuid).into_iter().any(|(id, c, _, _)| c == club && (id == invitation || invitation <= 0));
        if !found {
            return Err("convite não encontrado".into());
        }
        if self.club(club).is_none() {
            return Err("clube desconhecido".into());
        }
        self.add_member(club, xuid, ROLE_MEMBER);
        Ok(())
    }

    pub fn request_to_join(&self, club: i64, xuid: u64) {
        self.with(|db| {
            db.execute(
                "INSERT OR REPLACE INTO club_requests (club, xuid, created) VALUES (?1, ?2, ?3)",
                params![club, xuid as i64, unix_ms()],
            )
        });
    }

    /// (xuid, created) of players asking to join.
    pub fn club_requests(&self, club: i64) -> Vec<(u64, i64)> {
        self.with(|db| {
            let mut stmt = db.prepare("SELECT xuid, created FROM club_requests WHERE club = ?1 ORDER BY created")?;
            let rows = stmt.query_map([club], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))?.flatten().collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    pub fn delete_club_request(&self, club: i64, xuid: u64) -> bool {
        self.with(|db| db.execute("DELETE FROM club_requests WHERE club = ?1 AND xuid = ?2", params![club, xuid as i64]))
            .is_some_and(|n| n > 0)
    }

    pub fn add_club_notice(&self, club: i64, xuid: u64, message: &str) -> i64 {
        self.with(|db| {
            db.execute(
                "INSERT INTO club_notices (club, xuid, message, created) VALUES (?1, ?2, ?3, ?4)",
                params![club, xuid as i64, message, unix_ms()],
            )?;
            Ok(db.last_insert_rowid())
        })
        .unwrap_or(0)
    }

    pub fn delete_club_notice(&self, id: i64) -> bool {
        self.with(|db| db.execute("DELETE FROM club_notices WHERE id = ?1", [id])).is_some_and(|n| n > 0)
    }

    /// (id, xuid, message, created), newest first.
    pub fn club_notices(&self, club: i64) -> Vec<(i64, u64, String, i64)> {
        self.with(|db| {
            let mut stmt =
                db.prepare("SELECT id, xuid, message, created FROM club_notices WHERE club = ?1 ORDER BY id DESC LIMIT 50")?;
            let rows = stmt
                .query_map([club], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64, r.get(2)?, r.get(3)?)))?
                .flatten()
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// XP the player earned since a time (Scoreboard.SubmitXpScore rows).
    pub fn xp_since(&self, xuid: u64, since: i64) -> f64 {
        self.with(|db| {
            db.query_row("SELECT COALESCE(SUM(xp), 0) FROM xp WHERE xuid = ?1 AND time >= ?2", params![xuid as i64, since], |r| {
                r.get::<_, f64>(0)
            })
        })
        .unwrap_or(0.0)
    }

    /// The club ladder: (xuid, XP earned since joining), best first.
    pub fn club_ladder(&self, club: i64) -> Vec<(u64, f64)> {
        let mut rows: Vec<(u64, f64)> =
            self.club_members(club).into_iter().map(|(x, _, joined)| (x, self.xp_since(x, joined))).collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows
    }

    /// Club XP: what its members earned while in it.
    pub fn club_xp(&self, club: i64) -> i64 {
        self.club_ladder(club).iter().map(|r| r.1).sum::<f64>() as i64
    }

    pub fn accepts_club_invites(&self, xuid: u64) -> bool {
        self.with(|db| {
            db.query_row("SELECT accepts_club_invites FROM user_settings WHERE xuid = ?1", [xuid as i64], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
        })
        .flatten()
        .is_none_or(|v| v != 0)
    }

    pub fn set_accepts_club_invites(&self, xuid: u64, accepts: bool) {
        self.with(|db| {
            db.execute(
                "INSERT INTO user_settings (xuid, accepts_club_invites) VALUES (?1, ?2)
                 ON CONFLICT (xuid) DO UPDATE SET accepts_club_invites = ?2",
                params![xuid as i64, accepts as i64],
            )
        });
    }

    /// A club as the manager shows it: members, pending invites and requests, notices.
    pub fn club_json(&self, club: &Club) -> Value {
        let tag = |x: u64| self.gamertag(x);
        json!({
            "id": club.id,
            "tag": club.tag,
            "name": club.name,
            "description": club.description,
            "privacy": club.privacy,
            "region": club.region,
            "attitude": club.attitude,
            "interest": club.interest,
            "xp": self.club_xp(club.id),
            "created": club.created,
            "members": self.club_members(club.id).into_iter().map(|(x, role, joined)| json!({
                "xuid": x.to_string(), "gamertag": tag(x), "role": role, "joined": joined,
                "presence": self.presence_json(x),
            })).collect::<Vec<_>>(),
            "invites": self.club_invites_of(club.id).into_iter().map(|(id, x, sender, created)| json!({
                "id": id, "xuid": x.to_string(), "gamertag": tag(x),
                "sender": sender.to_string(), "senderGamertag": tag(sender), "created": created,
            })).collect::<Vec<_>>(),
            "requests": self.club_requests(club.id).into_iter().map(|(x, created)| json!({
                "xuid": x.to_string(), "gamertag": tag(x), "created": created,
            })).collect::<Vec<_>>(),
            "notices": self.club_notices(club.id).into_iter().map(|(id, x, message, created)| json!({
                "id": id, "xuid": x.to_string(), "gamertag": tag(x), "message": message, "created": created,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn clubs_json(&self) -> Value {
        Value::Array(self.clubs().iter().map(|c| self.club_json(c)).collect())
    }
}
