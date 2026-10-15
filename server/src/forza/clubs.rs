//! Car Clubs for the game: Forza.WebServices.CarClub (read: details, search, recommendations)
//! and Xls.WebServices.Club (actions: create, invite, join, roles, notice board).

use serde_json::{Value, json};

use super::{Ctx, iso_utc, user_data};
use crate::db::{ClubFields, ROLE_COLEADER, ROLE_LEADER, ROLE_MEMBER};
use crate::db::clubs::{Club, PRIVACY_FRIENDS, PRIVACY_INVITE_ONLY, PRIVACY_OPEN};

/// Club ladder reward tiers: (XP to reach, credits).
pub const LADDER_TIERS: &[(f32, i32)] = &[(5_000.0, 5_000), (20_000.0, 10_000), (50_000.0, 25_000), (100_000.0, 50_000), (250_000.0, 100_000)];

/// ForzaCarClubKeyword.Category: which selector a keyword goes in.
pub const CATEGORY_REGION: i64 = 0;
pub const CATEGORY_ATTITUDE: i64 = 1;
pub const CATEGORY_INTEREST: i64 = 2;

/// (id, category, text). The ids are what CreateClub / ChangeClubDetails send back.
pub const KEYWORDS: &[(i64, i64, &str)] = &[
    (1, CATEGORY_REGION, "South America"),
    (2, CATEGORY_REGION, "North America"),
    (3, CATEGORY_REGION, "Europe"),
    (4, CATEGORY_REGION, "Asia"),
    (5, CATEGORY_REGION, "Oceania"),
    (6, CATEGORY_REGION, "Africa"),
    (11, CATEGORY_ATTITUDE, "Casual"),
    (12, CATEGORY_ATTITUDE, "Competitive"),
    (13, CATEGORY_ATTITUDE, "Friendly"),
    (14, CATEGORY_ATTITUDE, "Hardcore"),
    (21, CATEGORY_INTEREST, "Racing"),
    (22, CATEGORY_INTEREST, "Drift"),
    (23, CATEGORY_INTEREST, "Off-road"),
    (24, CATEGORY_INTEREST, "Classics"),
    (25, CATEGORY_INTEREST, "Supercars"),
    (26, CATEGORY_INTEREST, "Liveries"),
    (27, CATEGORY_INTEREST, "Photography"),
    (28, CATEGORY_INTEREST, "Tuning"),
];

pub fn keyword(id: i64) -> Value {
    match KEYWORDS.iter().find(|k| k.0 == id) {
        Some(&(id, category, text)) => json!({ "Keyword": text, "KeywordId": id, "Category": category }),
        None => json!({}),
    }
}

/// UserData for a player, with their club fields.
pub fn member_data(ctx: &Ctx, xuid: u64) -> Value {
    user_data(ctx, xuid)
}

/// The ClubData struct for a club, as seen by `viewer` (friendInClub is one of their friends).
pub fn club_data(ctx: &Ctx, club: &Club, viewer: u64) -> Value {
    let members = ctx.store.club_members(club.id);
    let leader = members.iter().find(|m| m.1 == ROLE_LEADER).map(|m| m.0);
    let friends: Vec<u64> = ctx.store.friends(viewer).into_iter().map(|f| f.0).collect();
    let friend = members.iter().find(|m| friends.contains(&m.0)).map(|m| m.0);
    // The member with the highest career level stands in for the club champion.
    let champion = members.iter().map(|m| m.0).max_by_key(|&x| ctx.store.login(x)["currentCareerLevel"].as_u64().unwrap_or(0));
    let some = |x: Option<u64>| x.map(|x| member_data(ctx, x)).unwrap_or_else(|| json!({}));
    let (rank, xp) = ctx.store.club_xp_rank(club.id);
    json!({
        "clubId": club.id,
        "clubTag": club.tag,
        "clubName": club.name,
        "clubDescription": club.description,
        "isOpenForMembership": club.privacy == PRIVACY_OPEN || club.privacy == PRIVACY_FRIENDS,
        "creationDate": iso_utc(club.created),
        "carCount": 0,
        "memberCount": members.len(),
        "clubLeader": some(leader),
        "friendInClub": some(friend),
        "clubChampion": some(champion),
        "regionKeyword": keyword(club.region),
        "attitudeKeyword": keyword(club.attitude),
        "interestKeyword": keyword(club.interest),
        "clubPrivacy": club.privacy,
        "clubXP": xp,
        // among every club on the server
        "clubXPRank": rank,
    })
}

/// Every club is visible to everyone, except hidden ones (privacy 3): those only to their
/// members and the members' friends.
fn visible(ctx: &Ctx, club: &Club) -> bool {
    if club.privacy != 3 {
        return true;
    }
    let friends = ctx.friend_ids();
    ctx.store.club_members(club.id).iter().any(|m| m.0 == ctx.xuid || friends.contains(&m.0))
}

fn visible_club(ctx: &Ctx, id: i64) -> Option<Club> {
    ctx.store.club(id).filter(|c| visible(ctx, c))
}

/// Club name and description as typed in game, cleaned.
fn club_text(params: &Value, key: &str, fallback: &str, max: usize) -> String {
    super::clean_text(params[key].as_str().unwrap_or(fallback), max)
}

fn error(code: u32, message: &str) -> Value {
    json!({ "__xtsw_error": { "code": code, "message": message } })
}

fn param_i64(params: &Value, name: &str) -> i64 {
    params[name].as_i64().unwrap_or(0)
}

fn param_xuid(params: &Value, name: &str) -> u64 {
    params[name].as_u64().unwrap_or(0)
}

fn max_results(params: &Value, name: &str) -> usize {
    params[name].as_i64().filter(|&n| n > 0).unwrap_or(25).min(200) as usize
}

/// Can `xuid` join without approval?
fn joins_directly(ctx: &Ctx, club: &Club, xuid: u64) -> bool {
    match club.privacy {
        PRIVACY_OPEN => true,
        PRIVACY_FRIENDS => {
            let friends: Vec<u64> = ctx.store.friends(xuid).into_iter().map(|f| f.0).collect();
            ctx.store.club_members(club.id).iter().any(|m| friends.contains(&m.0))
        }
        _ => false,
    }
}

/// The caller's club and role, or an error reply.
fn my_club(ctx: &Ctx) -> Result<(Club, i64), Value> {
    let (id, role) = ctx.store.membership(ctx.xuid).ok_or_else(|| error(1, "not in a club"))?;
    let club = ctx.store.club(id).ok_or_else(|| error(1, "unknown club"))?;
    Ok((club, role))
}

fn is_officer(role: i64) -> bool {
    role == ROLE_LEADER || role == ROLE_COLEADER
}

pub fn handle(ctx: &Ctx, xls: bool, class: &str, method: &str, params: &Value) -> Option<Value> {
    let me = ctx.xuid;
    Some(match (xls, class, method) {
        // ------------------------------------------------------------ Forza.WebServices.CarClub
        (false, "CarClub", "GetClubKeywords") => {
            let max = params["maxKeywordResults"].as_u64().unwrap_or(128) as usize;
            json!({ "keywords": KEYWORDS.iter().take(max).map(|k| keyword(k.0)).collect::<Vec<_>>() })
        }
        (false, "CarClub", "GetRecommendedClubs") => {
            let mine = ctx.store.membership(me).map(|m| m.0);
            let list: Vec<Value> = ctx
                .store
                .clubs()
                .iter()
                .filter(|c| Some(c.id) != mine && c.privacy != 3 && visible(ctx, c))
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, c, me))
                .collect();
            json!({ "totalClubs": list.len(), "clubDetailsList": list })
        }
        (false, "CarClub", "GetFriendsClubs") => {
            let mut seen = Vec::new();
            for friend in ctx.friend_ids() {
                if let Some((club, _)) = ctx.store.membership(friend) {
                    if !seen.contains(&club) {
                        seen.push(club);
                    }
                }
            }
            let list: Vec<Value> = seen
                .into_iter()
                .filter_map(|id| ctx.store.club(id))
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, &c, me))
                .collect();
            json!({ "totalClubs": list.len(), "friendsClubDetailsList": list })
        }
        (false, "CarClub", "GetClubDetails") => match visible_club(ctx, param_i64(params, "clubId")) {
            Some(c) => json!({ "clubDetails": club_data(ctx, &c, me) }),
            None => json!({ "clubDetails": {} }),
        },
        (false, "CarClub", "GetClubFullInfo") => match visible_club(ctx, param_i64(params, "clubId")) {
            Some(c) => {
                let max = max_results(params, "maxLeaders");
                let leaders: Vec<Value> = ctx
                    .store
                    .club_members(c.id)
                    .into_iter()
                    .filter(|m| is_officer(m.1))
                    .take(max)
                    .map(|m| member_data(ctx, m.0))
                    .collect();
                json!({ "clubDetails": club_data(ctx, &c, me), "leaders": leaders })
            }
            None => json!({ "clubDetails": {}, "leaders": [] }),
        },
        (false, "CarClub", "SearchForClubByTag") => {
            match ctx.store.club_by_tag(params["clubTag"].as_str().unwrap_or("")).filter(|c| visible(ctx, c)) {
                Some(c) => json!({ "clubDetails": club_data(ctx, &c, me) }),
                // an error, not an empty club: the game lists empty details as a blank club (id 0)
                // and crashes when it is opened
                None => error(1, "no club with that tag"),
            }
        }
        (false, "CarClub", "SearchForClubsByName") => {
            let wanted = params["clubName"].as_str().unwrap_or("").to_lowercase();
            let list: Vec<Value> = ctx
                .store
                .clubs()
                .iter()
                .filter(|c| c.privacy != 3 && visible(ctx, c))
                .filter(|c| c.name.to_lowercase().contains(&wanted) || c.tag.to_lowercase() == wanted)
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, c, me))
                .collect();
            json!({ "totalClubs": list.len(), "clubDetailsList": list })
        }
        (false, "CarClub", "SearchForClubs") => {
            let p = &params["clubSearchParams"];
            let privacy = p["Privacy"].as_i64().unwrap_or(-1);
            let (region, attitude, interest) =
                (param_i64(p, "RegionKeywordId"), param_i64(p, "AttitudeKeywordId"), param_i64(p, "InterestKeywordId"));
            // every search is "friends only": other clubs do not exist for the player
            let list: Vec<Value> = ctx
                .store
                .clubs()
                .iter()
                .filter(|c| c.privacy != 3 && visible(ctx, c))
                // "WithNone" enums: anything outside the four privacy values means "any"
                .filter(|c| !(0..=3).contains(&privacy) || c.privacy == privacy)
                .filter(|c| region == 0 || c.region == region)
                .filter(|c| attitude == 0 || c.attitude == attitude)
                .filter(|c| interest == 0 || c.interest == interest)
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, c, me))
                .collect();
            json!({ "totalClubs": list.len(), "clubDetailsList": list })
        }

        // The club ladder: members ranked by the XP they earned while in the club, with the
        // reward tiers (topTierCount = tiers reached).
        (false, "Scoreboard", "GetLadderRows") => {
            let club = param_i64(params, "clubId");
            let ladder = if visible_club(ctx, club).is_some() { ctx.store.club_ladder(club) } else { Vec::new() };
            let tiers: Vec<Value> = LADDER_TIERS
                .iter()
                .take(params["maxTiers"].as_i64().filter(|&n| n > 0).unwrap_or(5) as usize)
                .enumerate()
                .map(|(i, &(xp, credits))| json!({ "xpThreshold": xp, "creditRewardAmount": credits, "tierNumber": i + 1 }))
                .collect();
            let row = |rank: usize, xuid: u64, score: f64| {
                json!({
                    "rank": rank,
                    "user": member_data(ctx, xuid),
                    "score": score,
                    "topTierCount": LADDER_TIERS.iter().filter(|t| score >= t.0 as f64).count(),
                })
            };
            let start = params["startAt"].as_i64().unwrap_or(1).max(1) as usize;
            let max = params["maxResults"].as_i64().filter(|&n| n > 0).unwrap_or(100).min(200) as usize;
            let rows: Vec<Value> =
                ladder.iter().enumerate().skip(start - 1).take(max).map(|(i, &(x, s))| row(i + 1, x, s)).collect();
            let user_row = ladder
                .iter()
                .position(|r| r.0 == me)
                .map(|i| row(i + 1, me, ladder[i].1))
                .unwrap_or_else(|| json!({ "rank": 0, "user": member_data(ctx, me), "score": 0.0, "topTierCount": 0 }));
            json!({ "rows": rows, "ladderTiers": tiers, "userRow": user_row, "available": ladder.len() })
        }

        // ------------------------------------------------------------ Xls.WebServices.Club
        (true, "Club", "CreateClub") => {
            let fields = ClubFields {
                tag: params["clubTag"].as_str().unwrap_or("").into(),
                name: club_text(params, "clubName", "", 24),
                description: club_text(params, "clubDescription", "", 200),
                privacy: param_i64(params, "privacy"),
                region: param_i64(params, "regionKeywordId"),
                attitude: param_i64(params, "attitudeKeywordId"),
                interest: param_i64(params, "interestKeywordId"),
            };
            if ctx.store.events_since(me, "club", crate::unix_ms() - 3_600_000) >= 10 {
                return Some(error(1, "too many club changes"));
            }
            match ctx.store.create_club(me, &fields) {
                Ok(id) => {
                    ctx.event("club", format!("Criou o clube [{}] {}", fields.tag.to_uppercase(), fields.name), json!({ "clubId": id }));
                    json!({ "clubId": id })
                }
                Err(e) => error(1, &e),
            }
        }
        (true, "Club", "ChangeClubDetails") => match my_club(ctx) {
            Ok((club, role)) if is_officer(role) => {
                let fields = ClubFields {
                    tag: club.tag.clone(),
                    name: club_text(params, "newClubName", &club.name, 24),
                    description: club_text(params, "newClubDescription", &club.description, 200),
                    privacy: param_i64(params, "privacyLevel"),
                    region: param_i64(params, "regionKeyword"),
                    attitude: param_i64(params, "attitudeKeyword"),
                    interest: param_i64(params, "interestKeyword"),
                };
                match ctx.store.update_club(club.id, &fields) {
                    Ok(()) => json!({}),
                    Err(e) => error(1, &e),
                }
            }
            Ok(_) => error(1, "not a club leader"),
            Err(e) => e,
        },
        (true, "Club", "ChangeMemberRole") => match my_club(ctx) {
            Ok((club, ROLE_LEADER)) => {
                let who = param_xuid(params, "userToChange");
                // the leader hands over leadership by promoting someone else, never demotes self
                if who != me && ctx.store.set_member_role(club.id, who, param_i64(params, "newRole")) {
                    json!({})
                } else {
                    error(1, "not a member")
                }
            }
            Ok(_) => error(1, "only the leader changes roles"),
            Err(e) => e,
        },
        (true, "Club", "GetClubMembers") => {
            let id = param_i64(params, "clubId");
            let members = if visible_club(ctx, id).is_some() { ctx.store.club_members(id) } else { Vec::new() };
            let recent = crate::unix_ms() - 7 * 24 * 3600 * 1000;
            let list: Vec<Value> = members
                .iter()
                .take(params["maxMembers"].as_u64().filter(|&n| n > 0).unwrap_or(100).min(500) as usize)
                .map(|&(x, _, joined)| json!({ "user": member_data(ctx, x), "recentClubStatusChange": joined > recent }))
                .collect();
            json!({ "totalMembers": members.len(), "clubMembers": list })
        }
        (true, "Club", "GetClubStatus") => {
            let who = match param_xuid(params, "xuid") {
                x if x != 0 && ctx.can_see(x) => x,
                _ => me,
            };
            json!({ "clubStatus": {
                "acceptsClubInvites": ctx.store.accepts_club_invites(who),
                "clubId": ctx.store.membership(who).map(|m| m.0).unwrap_or(0),
            }})
        }
        (true, "Club", "GetClubHistory") => {
            let who = match param_xuid(params, "userXuid") {
                x if x != 0 && ctx.can_see(x) => x,
                _ => me,
            };
            let entries: Vec<Value> = ctx
                .store
                .club_history(who, params["maxEntries"].as_i64().unwrap_or(20).clamp(0, 100))
                .into_iter()
                .map(|(club_name, role, action, time)| {
                    json!({ "clubName": club_name, "date": iso_utc(time), "user": member_data(ctx, who), "role": role, "action": action })
                })
                .collect();
            json!({ "userClubHistory": entries })
        }
        // Friends who could be invited (not in the caller's club).
        (true, "Club", "GetUsersToInvite") => {
            let mine = ctx.store.membership(me).map(|m| m.0);
            let list: Vec<Value> = ctx
                .friend_ids()
                .into_iter()
                .filter(|&f| ctx.store.membership(f).map(|m| m.0) != mine || mine.is_none())
                .take(params["maxResults"].as_u64().filter(|&n| n > 0).unwrap_or(50).min(200) as usize)
                .map(|x| {
                    let club_name = ctx.store.membership(x).and_then(|m| ctx.store.club(m.0)).map(|c| c.name).unwrap_or_default();
                    json!({
                        "user": member_data(ctx, x),
                        "acceptsClubInvites": ctx.store.accepts_club_invites(x),
                        "hasEverLoggedIn": !ctx.store.login(x).is_null(),
                        "clubName": club_name,
                    })
                })
                .collect();
            json!({ "candidatesForInvite": list })
        }
        // Friends who are in a club the caller could join.
        (true, "Club", "GetUsersToJoin") => {
            let list: Vec<Value> = ctx
                .friend_ids()
                .into_iter()
                .filter_map(|x| {
                    let (club_id, _) = ctx.store.membership(x)?;
                    let club = ctx.store.club(club_id)?;
                    Some(json!({
                        "user": member_data(ctx, x),
                        "clubMemberCount": ctx.store.club_members(club_id).len(),
                        "clubName": club.name,
                        "clubIsOpenForMembership": club.privacy == PRIVACY_OPEN || club.privacy == PRIVACY_FRIENDS,
                    }))
                })
                .take(params["maxResults"].as_u64().filter(|&n| n > 0).unwrap_or(50).min(200) as usize)
                .collect();
            json!({ "usersInClubs": list })
        }
        (true, "Club", "InviteToClub") | (true, "Club", "InviteToClubByName") => match my_club(ctx) {
            Ok((club, role)) if is_officer(role) || club.privacy == PRIVACY_OPEN => {
                let who = match params["usernameToInvite"].as_str() {
                    Some(name) => ctx.store.xuid_by_gamertag(name).unwrap_or(0),
                    None => param_xuid(params, "userToInvite"),
                };
                // anyone on the server can be invited (acceptsClubInvites still says no for them)
                if who == 0 || who == me || !ctx.can_see(who) {
                    return Some(error(1, "unknown player"));
                }
                match ctx.store.invite_to_club(club.id, me, who) {
                    Ok(_) => {
                        ctx.event_for(who, "club", format!("Convite para o clube [{}] {}", club.tag, club.name), json!({ "clubId": club.id }));
                        json!({})
                    }
                    Err(e) => error(1, &e),
                }
            }
            Ok(_) => error(1, "not allowed to invite"),
            Err(e) => e,
        },
        (true, "Club", "AcceptClubInvite") => {
            let club = param_i64(params, "clubId");
            match ctx.store.accept_club_invite(me, club, params["invitationId"].as_i64().unwrap_or(0)) {
                Ok(()) => {
                    joined_event(ctx, club);
                    json!({})
                }
                Err(e) => error(1, &e),
            }
        }
        (true, "Club", "RequestJoiningClub") | (true, "Club", "RequestJoiningClubByName") => {
            let club = match params["clubTag"].as_str() {
                Some(tag) => ctx.store.club_by_tag(tag),
                None => ctx.store.club(param_i64(params, "clubId")),
            };
            // a club with one of the player's friends in it (or one that invited them)
            let Some(club) = club.filter(|c| visible(ctx, c) || has_invite(ctx, c.id)) else {
                return Some(error(1, "unknown club"));
            };
            if club.privacy == 3 || (club.privacy == PRIVACY_INVITE_ONLY && !has_invite(ctx, club.id)) {
                ctx.store.request_to_join(club.id, me);
                return Some(json!({ "requestApproved": false }));
            }
            if joins_directly(ctx, &club, me) || has_invite(ctx, club.id) {
                ctx.store.add_member(club.id, me, ROLE_MEMBER);
                joined_event(ctx, club.id);
                json!({ "requestApproved": true })
            } else {
                ctx.store.request_to_join(club.id, me);
                json!({ "requestApproved": false })
            }
        }
        (true, "Club", "ApproveRequestToJoin") => match my_club(ctx) {
            Ok((club, role)) if is_officer(role) => {
                let who = param_xuid(params, "approvedUser");
                // only someone who asked to join
                if ctx.store.delete_club_request(club.id, who) {
                    ctx.store.add_member(club.id, who, ROLE_MEMBER);
                    json!({})
                } else {
                    error(1, "no such request")
                }
            }
            Ok(_) => error(1, "not a club leader"),
            Err(e) => e,
        },
        (true, "Club", "LeaveClub") => {
            if let Ok((club, _)) = my_club(ctx) {
                ctx.store.leave_club(me, crate::db::clubs::ACTION_LEFT);
                ctx.event("club", format!("Saiu do clube [{}] {}", club.tag, club.name), json!({ "clubId": club.id }));
            }
            json!({})
        }
        (true, "Club", "RemoveFromClub") => match my_club(ctx) {
            Ok((club, role)) if is_officer(role) => {
                let who = param_xuid(params, "userToRemove");
                match ctx.store.membership(who) {
                    Some((c, their_role)) if c == club.id && (role == ROLE_LEADER || their_role == ROLE_MEMBER) => {
                        ctx.store.leave_club(who, crate::db::clubs::ACTION_REMOVED);
                        json!({})
                    }
                    _ => error(1, "cannot remove that member"),
                }
            }
            Ok(_) => error(1, "not a club leader"),
            Err(e) => e,
        },
        // A member suggests a kick: the leaders get a notification.
        (true, "Club", "RecommendMemberForRemoval") => match my_club(ctx) {
            Ok((club, _)) => {
                let who = param_xuid(params, "userToRemove");
                if who == me || ctx.store.membership(who).map(|m| m.0) != Some(club.id) {
                    return Some(error(1, "not a member"));
                }
                if ctx.store.events_since(me, "club-report", crate::unix_ms() - 3_600_000) >= 5 {
                    return Some(error(1, "too many reports"));
                }
                ctx.event("club-report", "Sugeriu remover um membro do clube", json!({ "clubId": club.id }));
                let note = json!({
                    "recommendingRemovalXuid": me,
                    "recommendingRemovalGamertag": ctx.store.game_tag(me),
                    "recommendedForRemovalXuid": who,
                    "recommendedForRemovalGamertag": ctx.store.game_tag(who),
                    "clubId": club.id,
                });
                for (member, role, _) in ctx.store.club_members(club.id) {
                    if is_officer(role) && member != me {
                        ctx.store.notify(member, crate::db::notifications::CLUB_REMOVE_RECOMMENDATION, note.clone());
                    }
                }
                json!({})
            }
            Err(e) => e,
        },
        (true, "Club", "CreateNoticeBoardNotification") => match my_club(ctx) {
            Ok((club, _)) => {
                let text = super::clean_text(params["noticeBoardMessage"].as_str().unwrap_or(""), 200);
                if text.is_empty() {
                    return Some(error(1, "empty notice"));
                }
                if ctx.store.events_since(me, "notice", crate::unix_ms() - 3_600_000) >= 20 {
                    return Some(error(1, "too many notices"));
                }
                ctx.event("notice", format!("Aviso no mural do clube [{}]", club.tag), json!({ "clubId": club.id }));
                json!({ "notificationId": ctx.store.add_club_notice(club.id, me, &text) })
            }
            Err(e) => e,
        },
        // the author, or an officer of the club, takes a notice down
        (true, "Club", "DeleteNoticeBoardNotification") => match my_club(ctx) {
            Ok((club, role)) => {
                let id = params["notificationId"].as_i64().unwrap_or(0);
                let mine = ctx.store.club_notices(club.id).iter().any(|n| n.0 == id && (n.1 == me || is_officer(role)));
                if mine {
                    ctx.store.delete_club_notice(id);
                }
                json!({})
            }
            Err(e) => e,
        },
        (true, "User", "AcceptClubInvites") => {
            ctx.store.set_accepts_club_invites(me, params["acceptingClubInvites"].as_bool().unwrap_or(true));
            json!({})
        }
        _ => return None,
    })
}

fn has_invite(ctx: &Ctx, club: i64) -> bool {
    ctx.store.club_invites_for(ctx.xuid).iter().any(|i| i.1 == club)
}

fn joined_event(ctx: &Ctx, club: i64) {
    if let Some(c) = ctx.store.club(club) {
        ctx.event("club", format!("Entrou no clube [{}] {}", c.tag, c.name), json!({ "clubId": club }));
    }
}
