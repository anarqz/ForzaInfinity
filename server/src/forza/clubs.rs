//! Car Clubs for the game: Forza.WebServices.CarClub (read: details, search, recommendations)
//! and Xls.WebServices.Club (actions: create, invite, join, roles, notice board).

use serde_json::{Value, json};

use super::{Ctx, iso_utc, user_data};
use crate::db::{ClubFields, ROLE_COLEADER, ROLE_LEADER, ROLE_MEMBER};
use crate::db::clubs::{Club, PRIVACY_FRIENDS, PRIVACY_INVITE_ONLY, PRIVACY_OPEN};

/// ForzaCarClubKeyword.Category: which selector a keyword goes in.
pub const CATEGORY_REGION: i64 = 0;
pub const CATEGORY_ATTITUDE: i64 = 1;
pub const CATEGORY_INTEREST: i64 = 2;

/// (id, category, text). The ids are what CreateClub / ChangeClubDetails send back.
pub const KEYWORDS: &[(i64, i64, &str)] = &[
    (1, CATEGORY_REGION, "América do Sul"),
    (2, CATEGORY_REGION, "América do Norte"),
    (3, CATEGORY_REGION, "Europa"),
    (4, CATEGORY_REGION, "Ásia"),
    (5, CATEGORY_REGION, "Oceania"),
    (6, CATEGORY_REGION, "África"),
    (11, CATEGORY_ATTITUDE, "Casual"),
    (12, CATEGORY_ATTITUDE, "Competitivo"),
    (13, CATEGORY_ATTITUDE, "Amigável"),
    (14, CATEGORY_ATTITUDE, "Hardcore"),
    (21, CATEGORY_INTEREST, "Corridas"),
    (22, CATEGORY_INTEREST, "Drift"),
    (23, CATEGORY_INTEREST, "Off-road"),
    (24, CATEGORY_INTEREST, "Clássicos"),
    (25, CATEGORY_INTEREST, "Supercarros"),
    (26, CATEGORY_INTEREST, "Pinturas"),
    (27, CATEGORY_INTEREST, "Fotografia"),
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
        "clubXP": club.xp,
        "clubXPRank": rank(ctx, club),
    })
}

fn rank(ctx: &Ctx, club: &Club) -> usize {
    ctx.store.clubs().iter().position(|c| c.id == club.id).map(|i| i + 1).unwrap_or(0)
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
                .filter(|c| Some(c.id) != mine && c.privacy != 3)
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, c, me))
                .collect();
            json!({ "totalClubs": list.len(), "clubDetailsList": list })
        }
        (false, "CarClub", "GetFriendsClubs") => {
            let mut seen = Vec::new();
            for (friend, _, _) in ctx.store.friends(me) {
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
        (false, "CarClub", "GetClubDetails") => match ctx.store.club(param_i64(params, "clubId")) {
            Some(c) => json!({ "clubDetails": club_data(ctx, &c, me) }),
            None => json!({ "clubDetails": {} }),
        },
        (false, "CarClub", "GetClubFullInfo") => match ctx.store.club(param_i64(params, "clubId")) {
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
            match ctx.store.club_by_tag(params["clubTag"].as_str().unwrap_or("")) {
                Some(c) => json!({ "clubDetails": club_data(ctx, &c, me) }),
                None => json!({ "clubDetails": {} }),
            }
        }
        (false, "CarClub", "SearchForClubsByName") => {
            let wanted = params["clubName"].as_str().unwrap_or("").to_lowercase();
            let list: Vec<Value> = ctx
                .store
                .clubs()
                .iter()
                .filter(|c| c.privacy != 3)
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
            let friends_only = p["FriendsOnly"].as_bool().unwrap_or(false);
            let friends: Vec<u64> = ctx.store.friends(me).into_iter().map(|f| f.0).collect();
            let list: Vec<Value> = ctx
                .store
                .clubs()
                .iter()
                .filter(|c| c.privacy != 3)
                // "WithNone" enums: anything outside the four privacy values means "any"
                .filter(|c| !(0..=3).contains(&privacy) || c.privacy == privacy)
                .filter(|c| region == 0 || c.region == region)
                .filter(|c| attitude == 0 || c.attitude == attitude)
                .filter(|c| interest == 0 || c.interest == interest)
                .filter(|c| !friends_only || ctx.store.club_members(c.id).iter().any(|m| friends.contains(&m.0)))
                .take(max_results(params, "maxResults"))
                .map(|c| club_data(ctx, c, me))
                .collect();
            json!({ "totalClubs": list.len(), "clubDetailsList": list })
        }

        // ------------------------------------------------------------ Xls.WebServices.Club
        (true, "Club", "CreateClub") => {
            let fields = ClubFields {
                tag: params["clubTag"].as_str().unwrap_or("").into(),
                name: params["clubName"].as_str().unwrap_or("").into(),
                description: params["clubDescription"].as_str().unwrap_or("").into(),
                privacy: param_i64(params, "privacy"),
                region: param_i64(params, "regionKeywordId"),
                attitude: param_i64(params, "attitudeKeywordId"),
                interest: param_i64(params, "interestKeywordId"),
            };
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
                    name: params["newClubName"].as_str().unwrap_or(&club.name).into(),
                    description: params["newClubDescription"].as_str().unwrap_or(&club.description).into(),
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
                if ctx.store.set_member_role(club.id, who, param_i64(params, "newRole")) {
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
            let members = ctx.store.club_members(id);
            let recent = crate::unix_ms() - 7 * 24 * 3600 * 1000;
            let list: Vec<Value> = members
                .iter()
                .take(params["maxMembers"].as_u64().filter(|&n| n > 0).unwrap_or(100) as usize)
                .map(|&(x, _, joined)| json!({ "user": member_data(ctx, x), "recentClubStatusChange": joined > recent }))
                .collect();
            json!({ "totalMembers": members.len(), "clubMembers": list })
        }
        (true, "Club", "GetClubStatus") => {
            let who = match param_xuid(params, "xuid") {
                0 => me,
                x => x,
            };
            json!({ "clubStatus": {
                "acceptsClubInvites": ctx.store.accepts_club_invites(who),
                "clubId": ctx.store.membership(who).map(|m| m.0).unwrap_or(0),
            }})
        }
        (true, "Club", "GetClubHistory") => {
            let who = match param_xuid(params, "userXuid") {
                0 => me,
                x => x,
            };
            let entries: Vec<Value> = ctx
                .store
                .club_history(who, params["maxEntries"].as_i64().unwrap_or(20))
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
                .store
                .friends(me)
                .into_iter()
                .filter(|f| ctx.store.membership(f.0).map(|m| m.0) != mine || mine.is_none())
                .take(params["maxResults"].as_u64().filter(|&n| n > 0).unwrap_or(50) as usize)
                .map(|(x, _, _)| {
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
                .store
                .friends(me)
                .into_iter()
                .filter_map(|(x, _, _)| {
                    let (club_id, _) = ctx.store.membership(x)?;
                    let club = ctx.store.club(club_id)?;
                    Some(json!({
                        "user": member_data(ctx, x),
                        "clubMemberCount": ctx.store.club_members(club_id).len(),
                        "clubName": club.name,
                        "clubIsOpenForMembership": club.privacy == PRIVACY_OPEN || club.privacy == PRIVACY_FRIENDS,
                    }))
                })
                .take(params["maxResults"].as_u64().filter(|&n| n > 0).unwrap_or(50) as usize)
                .collect();
            json!({ "usersInClubs": list })
        }
        (true, "Club", "InviteToClub") | (true, "Club", "InviteToClubByName") => match my_club(ctx) {
            Ok((club, role)) if is_officer(role) || club.privacy == PRIVACY_OPEN => {
                let who = match params["usernameToInvite"].as_str() {
                    Some(name) => ctx.store.xuid_by_gamertag(name).unwrap_or(0),
                    None => param_xuid(params, "userToInvite"),
                };
                if who == 0 {
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
            let Some(club) = club else { return Some(error(1, "unknown club")) };
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
                if ctx.store.delete_club_request(club.id, who) || ctx.store.user_exists(who) {
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
        // A member suggests a kick: posted on the notice board for the leaders to see.
        (true, "Club", "RecommendMemberForRemoval") => match my_club(ctx) {
            Ok((club, _)) => {
                let who = param_xuid(params, "userToRemove");
                let text = format!("Sugestão de remoção: {}", ctx.store.gamertag(who));
                ctx.store.add_club_notice(club.id, me, &text);
                json!({})
            }
            Err(e) => e,
        },
        (true, "Club", "CreateNoticeBoardNotification") => match my_club(ctx) {
            Ok((club, _)) => {
                let text = params["noticeBoardMessage"].as_str().unwrap_or("");
                json!({ "notificationId": ctx.store.add_club_notice(club.id, me, text) })
            }
            Err(e) => e,
        },
        (true, "Club", "DeleteNoticeBoardNotification") => {
            ctx.store.delete_club_notice(params["notificationId"].as_i64().unwrap_or(0));
            json!({})
        }
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
