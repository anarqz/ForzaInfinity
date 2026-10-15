//! Built-in handlers for Forza / Xls web service methods, backed by the SQLite store.
//!
//! A handler gets the decoded `_Parameters` (JSON, see xtsw.rs) and returns the `_Results` as
//! JSON. Fields it leaves out are sent as zero / empty. A spec file
//! `responses/forza/<Class>/<Method>.json` with an `xtsw` object is merged over the result, so
//! any field can be tweaked without rebuilding.
//!
//! Methods left out on purpose (they keep answering 503, which the game treats as "offline"
//! and falls back to its own data): User.GetEconomySettings (zeros would zero the rewards),
//! Drivatar.GetDrivatarInformationForRace, Scoreboard.GetRival.

use std::cell::RefCell;

use serde_json::{Value, json};

use crate::{db::Store, gamedb::Catalog};

mod clubs;
mod gates;
mod notifications;
mod storefront;
pub use notifications::challenge_target;

/// The Car Club keywords (id, category, text) the server offers.
pub fn club_keywords() -> &'static [(i64, i64, &'static str)] {
    clubs::KEYWORDS
}

pub struct Ctx<'a> {
    pub store: &'a Store,
    pub catalog: &'a Catalog,
    /// The requesting player (X-UserId), 0 when unknown.
    pub xuid: u64,
    /// Feed events created while handling the request, published by the caller.
    pub events: RefCell<Vec<Value>>,
}

impl Ctx<'_> {
    fn event(&self, kind: &str, title: impl AsRef<str>, detail: Value) {
        self.event_for(self.xuid, kind, title, detail);
    }

    fn event_for(&self, xuid: u64, kind: &str, title: impl AsRef<str>, detail: Value) {
        let e = self.store.add_event(xuid, kind, title.as_ref(), detail);
        self.events.borrow_mut().push(e);
    }

    /// The player's friends: players who added each other. Friends are the social side (the
    /// friends list, presence, messages, Meus Rivais and challenges) and come first in drivatar
    /// grids and rival picks; everything else is open to the whole server.
    fn friend_ids(&self) -> Vec<u64> {
        self.store.friends(self.xuid).into_iter().filter(|f| f.2).map(|f| f.0).collect()
    }

    /// Whether the player sees `xuid`'s times, ghosts, drivatar, liveries and shared files:
    /// themselves or any player on the server (an account not banned).
    fn can_see(&self, xuid: u64) -> bool {
        xuid == self.xuid || (xuid != 0 && self.store.is_public_player(xuid))
    }

    /// Other players' drivatars, in the order they are offered: friends first, then everyone
    /// else on the server (the most recently trained, shuffled per request so grids vary).
    fn drivatar_candidates(&self) -> Vec<u64> {
        let friends: Vec<u64> = self.friend_ids().into_iter().filter(|x| self.store.is_public_player(*x)).collect();
        let mut others: Vec<u64> =
            self.store.drivatar_owners(400).into_iter().filter(|x| *x != self.xuid && !friends.contains(x)).collect();
        let mut seed = (crate::unix_ms() as u64) ^ self.xuid.rotate_left(17) | 1;
        for i in (1..others.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            others.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        friends.into_iter().chain(others).collect()
    }

    fn car_name(&self, id: i64) -> String {
        self.catalog.car(id).map(|c| c.name.clone()).unwrap_or_else(|| format!("carro #{id}"))
    }

    fn track_name(&self, id: i64) -> String {
        self.catalog.tracks.get(&id).map(|t| t.name.clone()).unwrap_or_else(|| format!("pista #{id}"))
    }
}

/// `class` is the X-ClassName without the assembly, e.g. "Forza.WebServices.User".
pub fn handle(ctx: &Ctx, class: &str, method: &str, params: &Value) -> Option<Value> {
    let short = class.rsplit('.').next().unwrap_or(class);
    let xls = class.starts_with("Xls.");
    if let Some(reply) = clubs::handle(ctx, xls, short, method, params) {
        return Some(reply);
    }
    if let Some(reply) = storefront::handle(ctx, xls, short, method, params) {
        return Some(reply);
    }
    if xls && short == "Notifications" {
        if let Some(reply) = notifications::handle(ctx, method, params) {
            return Some(reply);
        }
    }
    Some(match (xls, short, method) {
        (false, "User", "Logon") => user_logon(ctx, params),
        (true, "User", "GetUserData") => json!({ "userData": user_data(ctx, ctx.xuid) }),
        (false, "OnlineProfile", "UpdateAll") => profile_update_all(ctx, params),
        (false, "OnlineProfile", "UpdateAllTables") => profile_update_tables(ctx, params),
        (false, "Scoreboard", "SubmitScore") => submit_score(ctx, params),
        (false, "Scoreboard", "SubmitXpScore") => submit_xp(ctx, params),
        (false, "Scoreboard", "GetBestScore") => best_score(ctx, params),
        (false, "Scoreboard", "GetRowsAnthem") => rows_anthem(ctx, params),
        (false, "Scoreboard", "GetRows") => score_rows(ctx, params),
        (false, "ContentVersioning", "CreateVersionedTuneFromStructs") => create_tune(ctx, params),
        (false, "ContentVersioning", "CreateVersionedLivery") => {
            let car = params["carId"].as_i64().unwrap_or(-1);
            let id = ctx.store.add_livery(ctx.xuid, params["name"].as_str().unwrap_or(""), car, params);
            // keep the design's own copy (from the UpdateLivery of that name just before), so the
            // versioned livery outlives library edits and pruning
            if let Some((_, full)) = ctx.store.livery(id) {
                ctx.store.set_livery_data(id, &full);
            }
            json!({ "versionedLiveryId": id })
        }
        // A player's car paint (drivatars in races and car meets carry its id as LiveryId).
        (false, "ContentVersioning", "RetrieveVersionedLiveryById") => {
            let id = params["versionedLiveryId"].as_i64().unwrap_or(0);
            if id >= BASE_LIVERY_IDS {
                let (car, colour) = ((id - BASE_LIVERY_IDS) / 256, (id - BASE_LIVERY_IDS) % 256);
                return Some(match base_livery(car, colour) {
                    Some(data) => json!({ "versionedLiveryData": {
                        "profileId": 0, "carId": car, "numberOfLayers": 0, "creatorXuid": 0,
                        "data": data, "liveryThumbnail": "", "liveryThumbnailSmall": "",
                    } }),
                    None => json!({ "__xtsw_error": { "code": 1, "message": "no such livery" } }),
                });
            }
            match ctx.store.livery(id).filter(|(owner, _)| ctx.can_see(*owner)) {
                Some((owner, p)) => json!({ "versionedLiveryData": {
                    "profileId": 0,
                    "carId": p["carId"],
                    "numberOfLayers": p["numberOfLayers"],
                    "creatorXuid": if p["creatorXuid"].as_u64().unwrap_or(0) != 0 { p["creatorXuid"].clone() } else { json!(owner) },
                    "data": p["data"],
                    "liveryThumbnail": "",
                    "liveryThumbnailSmall": "",
                } }),
                None => json!({ "__xtsw_error": { "code": 1, "message": "no such livery" } }),
            }
        }
        (false, "OnlineProfile", "UpdateLivery") => {
            let car = params["carId"].as_i64().unwrap_or(-1);
            ctx.store.add_livery(ctx.xuid, params["name"].as_str().unwrap_or(""), car, params);
            ctx.event("livery", format!("Pintura salva para {}", ctx.car_name(car)), json!({ "name": params["name"], "carId": car }));
            json!({})
        }
        // a friend's time, else anyone's; without one the method stays down and the game shows no rival
        (false, "Scoreboard", "GetRival") => return challenge_rival(ctx, params).or_else(|| pick_rival(ctx, params)),
        (false, "Scoreboard", "UploadFile") => upload_file(ctx, params),
        (false, "Scoreboard", "DownloadFile") => download_file(ctx, params),
        (false, "Drivatar", "SaveDrivatarInformation") => save_drivatar(ctx, params),
        (false, "Drivatar", "GetDrivatarInformationForRace") => drivatars_for_race(ctx, params),
        (false, "Drivatar", "GetDrivatarsForCache") => {
            let n = params["numberOfDrivatars"].as_i64().unwrap_or(0).clamp(0, 200) as usize;
            // the drivatars the game keeps for free roam and car meets: the whole server
            let infos: Vec<Value> =
                ctx.drivatar_candidates().into_iter().filter_map(|owner| racing_info(ctx, owner, -1, None)).take(n).collect();
            json!({ "drivatarInfos": infos })
        }
        // telemetry: acknowledged, not kept
        (false, "GameplayLog", "Write") => json!({}),
        (false, "DailyCreditBonus", "GetPayout") => json!({ "dailyPayout": {} }),
        // The game's own offline values, read from ForzaEconomyModule in the running game by
        // the hook's economy probe (FH2_ExtraOptions.log, 2026-10-13). UseServerValues is 1 in
        // the module, so whatever is sent here replaces them: keep them identical. Change them
        // in responses/forza/User/GetEconomySettings.json ("xtsw") to tune the economy.
        (false, "User", "GetEconomySettings") => json!({
            "settings": {
                "creditDestinationBonus": 30000,
                "creditDestinationFalloff": 2000,
                "xpPerKmOffline": 800,
                "xpPerKmOnline": 640,
                "creditPerKm": 960,
                "minimumFinishLineXp": 0,
                "tombolaPrizeScale": 1.0,
                "promoBonusScale": 1.0,
                // no ForzaEconomyModule property reads it (online car-meet wager races)
                "wagerRaceCost": 0,
                "enableTokens": false,
                "enableXboxMusic": false,
            }
        }),
        // Transactional (X-TransactionId): credits granted by the service. Nothing to grant yet.
        (false, "User", "RetrieveOnlineCredits") => json!({ "credit": 0 }),
        (true, "Transaction", "Reconcile") => reconcile(ctx, params),
        // The friends the console knows (from social.xboxlive.com, i.e. ours): nothing to keep.
        (true, "User", "SetFriends") => json!({}),
        // "Meus rivais": the career compare stats of the player's friends (from their UpdateAll).
        (false, "User", "GetCompareStatsForRelatedUsers") => {
            let max = params["numDesiredUsers"].as_i64().filter(|&n| n > 0).unwrap_or(10).min(100) as usize;
            let stats: Vec<Value> = ctx
                .friend_ids()
                .into_iter()
                .take(max)
                .map(|friend| {
                    let base = ctx.store.profile_data(friend)["compareStats"][0].clone();
                    json!({ "base": if base.is_object() { base } else { json!({}) }, "xuid": friend })
                })
                .collect();
            json!({ "compareStats": stats })
        }
        // The per-bucket stats (bucketedCompareStats from UpdateAllTables) of the given players.
        (false, "User", "GetCompareStatsByBucket") => {
            let bucket = params["bucketId"].as_i64().unwrap_or(-1);
            let mut xuids: Vec<u64> = params["xuids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
            if xuids.is_empty() {
                xuids = ctx.friend_ids();
            }
            let max = params["numUsers"].as_i64().filter(|&n| n > 0).unwrap_or(10).min(100) as usize;
            let stats: Vec<Value> = xuids
                .into_iter()
                .filter(|&x| ctx.can_see(x))
                .take(max)
                .map(|x| {
                    let tables = ctx.store.tables(x);
                    let row = tables
                        .get("bucketedCompareStats")
                        .and_then(Value::as_array)
                        .and_then(|rows| rows.iter().find(|r| r["Id"].as_i64() == Some(bucket)).cloned())
                        .unwrap_or_else(|| json!({ "Id": bucket }));
                    json!({ "base": row, "xuid": x })
                })
                .collect();
            json!({ "bucketedCompareStats": stats })
        }
        // The player card (level, badge, title) shown to others.
        (false, "User", "SetPlayerCardInfo") => {
            ctx.store.set_player_card(ctx.xuid, params);
            json!({})
        }
        // The PermissionsLoader (game 0x140f09930) sends this at boot; until it gets an answer the
        // game treats the LSP as down (sessionObj+0x1131 == 0) and locks online features with
        // IDS_Popup_FeatureLocked_LSPDown_Message. Features also look their method up in this
        // table (FUN_14126c2b0): a missing id reads as access 4 ("FH2 server unavailable", e.g.
        // Meus Rivais -> "Ir para"), and rivals need 0x51/0x54/0x59/0x5b allowed to upload times
        // and ghosts (FUN_140a41c90). So every id the table holds (0x100) is enabled, access 0.
        (false, "User", "GetPermissionsTable") => json!({
            "permissions": permissions(),
            "featureAreasBanMask": 0,
            "userFlags": 0,
        }),
        // Lists that are empty until the server has the content (storefront, clubs, drivatars).
        // The two keywords a shared file must have (the share screen offers only these).
        (false, "Storefront", "GetFileKeywords") => {
            let max = params["maxKeywordResults"].as_i64().filter(|&n| n > 0).unwrap_or(128) as usize;
            let keywords: Vec<Value> = FILE_KEYWORDS
                .iter()
                .enumerate()
                .take(max)
                .map(|(i, (k, bucket))| json!({ "Keyword": k, "KeywordId": i + 1, "Bucket": bucket }))
                .collect();
            json!({ "keywords": keywords })
        }
        (false, "Storefront", "GetLiveriesForDrivatar")
        | (false, "Storefront", "GetFeaturedLiveriesForRandomCars")
        | (false, "User", "GetOwnedProducts")
        | (false, "Scoreboard", "GetTimeTrials")
        | (false, "Storefront", "GetFeaturedPhotos")
        => json!({}),
        _ => return None,
    })
}

/// An Xbox Live request as live_service sees it.
pub struct LiveRequest<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub body: &'a [u8],
}

/// Xbox Live JSON services the game needs answered with real data (not a static stub).
pub fn live_service(ctx: &Ctx, req: &LiveRequest) -> Option<Value> {
    let (host, path) = (req.host, req.path);
    // Only for an account (X-Fh2-Token); "me" and xuid(...) in the path both mean the caller:
    // nobody reads someone else's friends or posts their presence.
    let me = ctx.xuid;
    if me == 0 {
        return None;
    }
    let subject = me;

    if host == "profile.xboxlive.com" && path.ends_with("/profile/settings") {
        let request: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let ids: Vec<u64> = request["userIds"]
            .as_array()
            .map(|a| a.iter().take(100).filter_map(|v| v.as_str()?.parse().ok()).collect())
            .unwrap_or_default();
        let wanted = string_list(&request["settings"]);
        let users: Vec<Value> = ids.iter().map(|&xuid| profile_user(ctx, xuid, &wanted)).collect();
        return Some(json!({ "profileUsers": users }));
    }
    // The friends list ("People" = who the player follows), same shape as the batch above;
    // the settings come in the query string.
    if host == "profile.xboxlive.com" && path.ends_with("/profile/settings/people/People") {
        let wanted: Vec<String> = query_param(req.query, "settings")
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default();
        let users: Vec<Value> = ctx.friend_ids().into_iter().map(|friend| profile_user(ctx, friend, &wanted)).collect();
        return Some(json!({ "profileUsers": users }));
    }
    // social.xboxlive.com: who the player follows, with the favorite / follows-back flags.
    if host == "social.xboxlive.com" && path.ends_with("/people") {
        let people: Vec<Value> = ctx
            .store
            .friends(subject)
            .iter()
            .filter(|f| f.2)
            .map(|&(friend, favorite, mutual)| {
                json!({
                    "xuid": friend.to_string(),
                    "isFavorite": favorite,
                    "isFollowingCaller": mutual,
                    "socialNetworks": [],
                })
            })
            .collect();
        return Some(json!({ "totalCount": people.len(), "people": people }));
    }
    // The Xbox text filter, asked before the game posts text (club name and tag, notice board,
    // messages): everything passes. resultCode 0 = Success, and it must be a JSON number: XSAPI
    // (Microsoft.Xbox.Services.dll FUN_1801b2a00 -> FUN_1800467c0) throws 0x83750005 on any other
    // type, and the game then says the Xbox text check is off.
    if host == "client-strings.xboxlive.com" && path.ends_with("/strings/validate") {
        let body: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let n = body["stringstoVerify"].as_array().map(Vec::len).unwrap_or(0);
        let results: Vec<Value> = (0..n).map(|_| json!({ "resultCode": 0, "offendingString": "" })).collect();
        return Some(json!({ "verifyStringResult": results }));
    }
    // Rich presence (RP_SPLASH, RP_FREEROAM, ...): the game posts it every few seconds.
    if host == "userpresence.xboxlive.com" && path.ends_with("/devices/current/titles/current") {
        if req.method == "DELETE" {
            ctx.store.set_presence(subject, "offline", "");
            return Some(json!({}));
        }
        let body: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let state = if body["state"].as_str() == Some("inactive") { "inactive" } else { "active" };
        // presence ids are the game's own RP_* names
        let rich: String = body["activity"]["richPresence"]["id"]
            .as_str()
            .unwrap_or("")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(40)
            .collect();
        if ctx.store.set_presence(subject, state, &rich) {
            ctx.event_for(subject, "presence", rich_presence_text(&rich), json!({ "rich": rich, "state": state }));
        }
        return Some(json!({}));
    }
    // Presence of everyone the player follows.
    if host == "userpresence.xboxlive.com" && path.ends_with("/groups/People") {
        let records: Vec<Value> = ctx.friend_ids().into_iter().map(|friend| presence_record(ctx, friend)).collect();
        return Some(Value::Array(records));
    }
    // One player's presence (anyone on the server: a leaderboard or a grid opens their card).
    if host == "userpresence.xboxlive.com" && req.method == "GET" && path.ends_with(')') {
        if let Some(x) = path_xuid(path) {
            if ctx.can_see(x) {
                return Some(presence_record(ctx, x));
            }
            return Some(json!({ "xuid": x.to_string(), "state": "Offline" }));
        }
    }
    None
}

/// One profile.xboxlive.com user with the requested settings.
fn profile_user(ctx: &Ctx, xuid: u64, wanted: &[String]) -> Value {
    // The game looks up every grid driver here: offline AI drivatars are not players, so they
    // get a stable generated name; real players show their own.
    let known = if ctx.can_see(xuid) { ctx.store.game_tag(xuid) } else { String::new() };
    let tag = if known.is_empty() { generated_gamertag(xuid) } else { known };
    let settings: Vec<Value> = wanted
        .iter()
        .map(|name| {
            let value = match name.as_str() {
                "Gamertag" | "GameDisplayName" | "AppDisplayName" => tag.clone(),
                "AccountTier" => "Gold".into(),
                "Gamerscore" => "0".into(),
                // Must be an absolute URL: XSAPI turns it into a Uri, an empty string gives a
                // null Uri and the game crashes on it (game+0x124f916).
                "GameDisplayPicRaw" | "AppDisplayPicRaw" => crate::gamerpic::url_for(xuid),
                _ => String::new(),
            };
            json!({ "id": name, "value": value })
        })
        .collect();
    let id = xuid.to_string();
    json!({ "id": id, "hostId": id, "settings": settings, "isSponsoredUser": false })
}

/// A userpresence.xboxlive.com record: online players show Forza Horizon 2 and their activity.
fn presence_record(ctx: &Ctx, xuid: u64) -> Value {
    let Some((_, rich, updated)) = ctx.store.presence(xuid).filter(|_| ctx.store.is_online(xuid)) else {
        return json!({ "xuid": xuid.to_string(), "state": "Offline" });
    };
    json!({
        "xuid": xuid.to_string(),
        "state": "Online",
        "devices": [{
            "type": "XboxOne",
            "titles": [{
                "id": "446059611",
                "name": "Forza Horizon 2",
                "placement": "Full",
                "state": "Active",
                "lastModified": iso_utc(updated),
                "activity": { "richPresence": rich_presence_text(&rich) },
            }],
        }],
    })
}

/// The text for a rich presence id (the console read it from the title's service config).
pub fn rich_presence_text(id: &str) -> String {
    match id {
        "RP_SPLASH" => "Na tela inicial".into(),
        "RP_FREEROAM" => "Explorando o sul da Europa".into(),
        "" => "Jogando".into(),
        other => other.trim_start_matches("RP_").replace('_', " ").to_lowercase(),
    }
}

fn path_xuid(path: &str) -> Option<u64> {
    let start = path.find("xuid(")? + 5;
    let end = path[start..].find(')')? + start;
    path[start..end].parse().ok()
}

fn query_param<'q>(query: &'q str, name: &str) -> Option<&'q str> {
    query.split('&').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
}

fn string_list(value: &Value) -> Vec<String> {
    value.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// MethodPermission list for GetPermissionsTable and Logon: every id the game's table holds
/// (0x100), enabled with access 0. A method missing from the table reads as access 4.
fn permissions() -> Value {
    (0..0x100).map(|id| json!({ "methodId": id, "tier": 0, "isEnabled": true, "access": 0 })).collect()
}

/// User.Logon: the game repeats it until it gets a valid reply, and nothing else in the Forza
/// services starts before it succeeds.
fn user_logon(ctx: &Ctx, params: &Value) -> Value {
    let login = &params["loginData"];
    // The account (X-Fh2-Token) is who logs on, whatever XUID and name the game reports.
    let Some(account) = ctx.store.account(ctx.xuid) else {
        return json!({ "__xtsw_error": { "code": 1, "message": "log on with an FH2Online account" } });
    };
    let (xuid, gamertag) = (account.xuid, account.gamertag());
    ctx.store.record_logon(xuid, &gamertag, login);
    // The checkpoint of the last UpdateAllTables we stored: equal to the client's means "nothing
    // to resend"; otherwise the game uploads its tables again (we merge them).
    let checkpoint = ctx
        .store
        .profile_tran_id(xuid)
        .map(Value::from)
        .unwrap_or_else(|| login["clientProfile"]["transactionCount"].clone());
    ctx.event("login", format!("{gamertag} entrou no Horizon"), json!({ "careerLevel": login["currentCareerLevel"] }));

    json!({
        "responseData": {
            "isProfileValid": true,
            "lastOnlineProfileUpdateAllTablesTranId": checkpoint,
            // the Logon reply replaces the permissions table too
            "permissions": permissions(),
            "featureAreasBanMask": 0,
            "userFlags": 0,
            "upsellOfferId": 0,
            "upsellText": "",
            "vipOfferIds": [],
            "userData": user_data(ctx, xuid),
            "notificationNonce": 0,
            // Notifications.RetrieveForUser cadence (it also honors nextRetrieveSeconds).
            "notificationPollIntervalSeconds": 30,
            "defaultFriendsPollIntervalSeconds": 300,
            "maxFriendsPollIntervalSeconds": 900,
            // The message center's tabs: a page shows nothing (stuck on "Recuperando dados")
            // until it has categories. A notification's category is (type >> 8) & 0x7f.
            "notificationCategories": notifications::categories(),
            "gameplayLogConfig": { "Enabled": false, "FlushInterval": 0, "Throttle": "" },
            "carsToUnshareFromGame": [],
            "defaultTimeoutMs": 30000,
            "defaultContentTimeoutMs": 60000,
            "defaultInGameTimeoutMs": 15000,
            "makeDiscounts": [],
            "carDiscounts": [],
            "makePartsDiscounts": [],
            "carPartsDiscounts": [],
        }
    })
}

/// The UserData struct (leaderboards, clubs, logon) from what the player last sent.
fn user_data(ctx: &Ctx, xuid: u64) -> Value {
    let login = ctx.store.login(xuid);
    // (club id, tag, role)
    let club = ctx.store.membership(xuid).and_then(|(id, role)| ctx.store.club(id).map(|c| (id, c.tag, role)));
    json!({
        "qwXuid": xuid,
        "wzGamerTag": ctx.store.game_tag(xuid),
        "flags": 0,
        "clubId": club.as_ref().map(|c| c.0).unwrap_or(0),
        "clubTag": club.as_ref().map(|c| c.1.clone()).unwrap_or_default(),
        "clubTopTierCount": 0,
        "roleInClub": club.as_ref().map(|c| c.2).unwrap_or(0),
        "acceptsClubInvites": ctx.store.accepts_club_invites(xuid),
        "currentCareerLevel": login["currentCareerLevel"],
        "currentBadgeId": login["currentBadgeId"],
        "currentPlayerTitleId": login["currentPlayerTitleId"],
    })
}

/// The player's garage car ids, as last synced.
fn garage_cars(ctx: &Ctx) -> Vec<i64> {
    ctx.store.tables(ctx.xuid).get("garageCars").and_then(Value::as_array).map(|rows| rows.iter().filter_map(|r| r["carId"].as_i64()).collect()).unwrap_or_default()
}

/// OnlineProfile.UpdateAll sends only what changed: each section is a list holding the section,
/// or empty when it did not change. The stored profile is the last one with those sections
/// replaced.
fn merge_profile(old: &Value, update: &Value) -> Value {
    let mut merged = if old.is_object() { old.clone() } else { json!({}) };
    if let Some(sections) = update.as_object() {
        for (name, value) in sections {
            let unchanged = value.as_array().is_some_and(|a| a.is_empty());
            if !unchanged || merged.get(name).is_none() {
                merged[name] = value.clone();
            }
        }
    }
    merged
}

fn profile_update_all(ctx: &Ctx, params: &Value) -> Value {
    let merged = merge_profile(&ctx.store.profile_data(ctx.xuid), params);
    let params = &merged;
    // integrity: credits / XP / level growth and top speed within reason (gates.rs)
    let now = crate::unix_ms();
    let checked = gates::progress(params).and_then(|p| {
        gates::check_growth(ctx.store.baseline(ctx.xuid).as_ref(), &p, now)?;
        // a new top speed record must fit the cars (an old record may come from a car since sold)
        let record = ctx.store.profile_data(ctx.xuid)["playerRaceHistory"][0]["topSpeed"].as_f64().unwrap_or(0.0);
        if p.top_speed > record + 0.5 {
            gates::check_top_speed(ctx.catalog, &garage_cars(ctx), p.top_speed)?;
        }
        Ok(p)
    });
    let progress = match checked {
        Ok(p) => p,
        Err(reason) => {
            gates::refuse(ctx, "Profile sync", &reason);
            return json!({});
        }
    };
    ctx.store.set_baseline(ctx.xuid, &crate::db::integrity::Baseline {
        credits: progress.credits,
        xp: progress.xp,
        level: progress.level,
        winnings: progress.winnings,
        synced: now,
    });
    ctx.store.save_profile(ctx.xuid, params);
    let race = &params["playerRaceHistory"][0];
    let history = &params["playerHistory"][0];
    ctx.event(
        "profile",
        format!(
            "Perfil sincronizado: nível {}, {} créditos",
            // stored from 0; the game shows level + 1
            race["level"].as_u64().unwrap_or(0) + 1,
            history["credits"].as_u64().unwrap_or(0)
        ),
        json!({
            "level": race["level"], "xp": race["xp"], "credits": history["credits"],
            "races": race["numRaces"], "victories": race["numVictories"],
        }),
    );
    json!({})
}

fn profile_update_tables(ctx: &Ctx, params: &Value) -> Value {
    // integrity: only real cars, and not a garage full of them at once
    let stored = garage_cars(ctx).len();
    if let Err(reason) = gates::check_garage(ctx.catalog, params, stored, ctx.store.garage_baseline(ctx.xuid), crate::unix_ms()) {
        gates::refuse(ctx, "Garage sync", &reason);
        return json!({});
    }
    let summary = ctx.store.apply_tables(ctx.xuid, params);
    ctx.store.set_garage_baseline(ctx.xuid, garage_cars(ctx).len() as i64);
    storefront::count_downloads(ctx, params);
    let cars = params["garageCars"].as_array().map(Vec::len).unwrap_or(0);
    let removed = params["deleteGarageCars"].as_array().map(Vec::len).unwrap_or(0);
    let title = if cars + removed > 0 {
        let names: Vec<String> = params["garageCars"]
            .as_array()
            .map(|a| a.iter().take(3).map(|c| ctx.car_name(c["carId"].as_i64().unwrap_or(-1))).collect())
            .unwrap_or_default();
        format!("Garagem: {cars} atualizado(s), {removed} removido(s) — {}", names.join(", "))
    } else {
        "Tabelas do perfil sincronizadas".to_owned()
    };
    let tables: Vec<Value> =
        summary.iter().map(|(name, rows, deleted)| json!({ "table": name, "rows": rows, "deleted": deleted })).collect();
    ctx.event("garage", title, json!({ "transactionId": params["transactionId"], "tables": tables }));
    json!({})
}

fn submit_score(ctx: &Ctx, params: &Value) -> Value {
    let data = &params["data"];
    let score_type = params["type"].as_i64().unwrap_or(0);
    let score = data["score"].as_f64().unwrap_or(f64::NAN);
    if !score.is_finite() || score <= 0.0 || score > 1e9 {
        return json!({ "__xtsw_error": { "code": 1, "message": "invalid score" } });
    }
    // integrity: a race time the car could not have done
    if crate::db::lower_is_better(score_type) {
        let car = data["carId"].as_i64().unwrap_or(-1);
        if let Err(reason) = gates::check_race_time(ctx.catalog, car, data["trackId"].as_i64().unwrap_or(-1), score) {
            gates::refuse(ctx, "Score", &reason);
            return json!({ "__xtsw_error": { "code": 1, "message": "invalid score" } });
        }
    }
    let score_id = ctx.store.add_score(ctx.xuid, score_type, data);
    let car = ctx.car_name(data["carId"].as_i64().unwrap_or(-1));
    let track = data["trackId"].as_i64().filter(|t| *t >= 0).map(|t| ctx.track_name(t));
    ctx.event(
        "score",
        format!(
            "Pontuação {} com {car}{}",
            format_score(data["score"].as_f64().unwrap_or(0.0), score_type),
            track.map(|t| format!(" em {t}")).unwrap_or_default()
        ),
        json!({ "scoreId": score_id, "scoreType": score_type, "data": data }),
    );
    // friends see this time in Meus Rivais on their next poll (notifications::sync_rivals)
    // Ask for the ghost (Scoreboard.UploadFile) only for a new personal best on this event.
    json!({ "scoreId": score_id, "uploadFiles": ctx.store.is_personal_best(&score_id) })
}

/// Scoreboard.UploadFile: the ghost/replay of a score, kept only if that score is still the
/// player's best on its event.
fn upload_file(ctx: &Ctx, params: &Value) -> Value {
    let score_id = params["scoreId"].as_str().unwrap_or("");
    let hex = params["data"].as_str().unwrap_or("");
    if hex.is_empty() || hex.len() > 2 * MAX_GHOST || !hex.is_ascii() {
        return json!({ "__xtsw_error": { "code": 1, "message": "ghost missing or too big" } });
    }
    let data: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
    // only the caller's own scores take a ghost
    match ctx.store.save_ghost(ctx.xuid, score_id, &data) {
        Some((id, key)) => {
            let track = key["trackId"].as_i64().unwrap_or(-1);
            let car = key["carId"].as_i64().unwrap_or(-1);
            ctx.event(
                "ghost",
                format!(
                    "Fantasma salvo ({} KB): {} com {}{}",
                    (data.len() + 1023) / 1024,
                    format_score(key["score"].as_f64().unwrap_or(0.0), key["scoreType"].as_i64().unwrap_or(0)),
                    ctx.car_name(car),
                    if track >= 0 { format!(" em {}", ctx.track_name(track)) } else { String::new() }
                ),
                json!({ "ghostId": id, "bytes": data.len(), "event": key }),
            );
        }
        None => ctx.event(
            "ghost",
            format!("Fantasma descartado ({} KB): não é o seu melhor tempo nesse evento", (data.len() + 1023) / 1024),
            json!({ "scoreId": score_id, "bytes": data.len() }),
        ),
    }
    json!({})
}

/// Largest ghost kept (the game asks DownloadFile with maxSize 256000; real ones are ~27 KB).
const MAX_GHOST: usize = 256_000;

/// Scoreboard.DownloadFile: a stored ghost by the id the leaderboards hand out (ghostFileId):
/// anyone's on the server.
fn download_file(ctx: &Ctx, params: &Value) -> Value {
    let id = params["id"].as_i64().unwrap_or(0);
    let max = params["maxSize"].as_i64().unwrap_or(i64::MAX).max(0) as usize;
    match ctx.store.ghost(id).filter(|g| ctx.can_see(g.0)).map(|g| g.1) {
        Some(data) if data.len() <= max && data.len() <= MAX_GHOST => {
            ctx.event("ghost", format!("Fantasma #{id} enviado ao jogo ({} KB)", (data.len() + 1023) / 1024), json!({ "ghostId": id }));
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            json!({ "data": hex })
        }
        Some(data) => json!({ "__xtsw_error": { "code": 1, "message": format!("ghost too big ({} bytes, max {max})", data.len()) } }),
        None => json!({ "__xtsw_error": { "code": 1, "message": "no such ghost" } }),
    }
}

fn submit_xp(ctx: &Ctx, params: &Value) -> Value {
    // one race pays at most a few thousand XP; the club ladder adds these up
    let xp = params["xp"].as_f64().filter(|x| x.is_finite()).unwrap_or(0.0).clamp(0.0, 50_000.0);
    // integrity: XP reported in the last hour within reason
    if ctx.store.xp_since(ctx.xuid, crate::unix_ms() - 3_600_000) + xp > gates::XP_REPORTED_PER_HOUR {
        gates::refuse(ctx, "XP", &format!("{:.0} XP over an hour", gates::XP_REPORTED_PER_HOUR));
        return json!({ "weeklyTierAchieved": false, "weeklyTierInfo": {} });
    }
    ctx.store.add_xp(ctx.xuid, xp);
    ctx.event("xp", format!("+{} XP", xp.round() as i64), json!({ "xp": xp }));
    json!({ "weeklyTierAchieved": false, "weeklyTierInfo": {} })
}

fn best_score(ctx: &Ctx, params: &Value) -> Value {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    // asked before every race: remember the real events (rival challenges point at them)
    ctx.store.record_race_event(int("scoreboardType"), int("scoreType"), int("gameScoreboardId"), int("trackId"), int("carClassId"));
    let rows = ctx.store.leaderboard(
        int("scoreboardType"),
        int("scoreType"),
        Some(int("trackId")),
        Some(int("gameScoreboardId")),
    );
    match rows.iter().find(|r| r.xuid == ctx.xuid) {
        Some(r) => json!({
            "bestScore": { "score": r.score, "isClean": r.data["isClean"] },
            "scoreFound": true,
        }),
        None => json!({ "scoreFound": false }),
    }
}

/// Scoreboard.GetRowsAnthem: the board shown right after an event (view 5).
fn rows_anthem(ctx: &Ctx, params: &Value) -> Value {
    let mut out = score_rows(ctx, params);
    out["totalRowCount"] = out["available"].clone();
    out
}

/// The leaderboard filters (`view`, as the game labels them: IDS_Global, IDS_NearMe,
/// IDS_Friends, IDS_Club, IDS_FriendsAndClub; 5 is the board shown right after an event).
const VIEW_NEAR_ME: i64 = 1;
const VIEW_FRIENDS: i64 = 2;
const VIEW_CLUB: i64 = 3;
const VIEW_FRIENDS_AND_CLUB: i64 = 4;

/// A leaderboard of everyone on the server. The friends and club views keep only those players
/// (and the caller), each row still with its real position on the server; near me is the page
/// around the caller's place; global and the after-event board are everyone. isClubBoard keeps
/// the caller's club as well. Boards without a track (speed traps and zones: scoreboardType 9,
/// skills: 4) name their event in pivotId, which is the gameScoreboardId the scores were
/// submitted with; track boards are the whole route. Also Scoreboard.GetRows (VIEW SCOREBOARD).
fn score_rows(ctx: &Ctx, params: &Value) -> Value {
    let track = params["trackId"].as_i64().filter(|t| *t >= 0);
    let event = if track.is_none() { params["pivotId"].as_i64() } else { None };
    let rows = ctx.store.leaderboard(
        params["scoreboardType"].as_i64().unwrap_or(-1),
        params["scoreType"].as_i64().unwrap_or(-1),
        track,
        event,
    );
    let view = params["view"].as_i64().unwrap_or(0);
    let club_members = || -> Vec<u64> {
        let club = ctx.store.membership(ctx.xuid).map(|m| m.0);
        club.map(|c| ctx.store.club_members(c).iter().map(|m| m.0).collect()).unwrap_or_default()
    };
    let mut circle: Option<Vec<u64>> = match view {
        VIEW_FRIENDS => Some(ctx.friend_ids()),
        VIEW_CLUB => Some(club_members()),
        VIEW_FRIENDS_AND_CLUB => Some(ctx.friend_ids().into_iter().chain(club_members()).collect()),
        _ => None,
    };
    if params["isClubBoard"].as_bool() == Some(true) {
        let members = club_members();
        circle = Some(circle.map(|c| c.into_iter().filter(|x| members.contains(x)).collect()).unwrap_or(members));
    }
    // (real position on the server, row)
    let ranked: Vec<(usize, &crate::db::ScoreRow)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (i + 1, r))
        .filter(|(_, r)| r.xuid == ctx.xuid || circle.as_ref().is_none_or(|c| c.contains(&r.xuid)))
        .collect();
    let max = params["maxResults"].as_i64().unwrap_or(50).clamp(0, 500) as usize;
    let mine = ranked.iter().position(|(_, r)| r.xuid == ctx.xuid);
    let start = match (view, mine) {
        // the page with the caller in its middle
        (VIEW_NEAR_ME, Some(i)) => (i + 1).saturating_sub(max / 2).max(1),
        _ => params["startAt"].as_i64().unwrap_or(1).max(1) as usize,
    };
    let row_json = |position: usize, r: &crate::db::ScoreRow| {
        let d = &r.data;
        json!({
            "position": position, "rank": position, "user": user_data(ctx, r.xuid),
            "score": r.score, "submittedTime": iso_utc(r.time),
            "carId": d["carId"], "carClassId": d["carClassId"], "carPerformanceIndex": d["carPerformanceIndex"],
            "usedSTM": d["usedSTM"], "usedTCS": d["usedTCS"], "usedABS": d["usedABS"],
            "usedAutoBrake": d["usedAutoBrake"], "usedAutoShifting": d["usedAutoShifting"],
            "usedClutch": d["usedClutch"], "isClean": d["isClean"],
            "ghostFileId": ctx.store.ghost_id(
                r.xuid,
                d["scoreboardType"].as_i64().unwrap_or(-1),
                params["scoreType"].as_i64().unwrap_or(-1),
                d["gameScoreboardId"].as_i64().unwrap_or(-1),
                d["trackId"].as_i64().unwrap_or(-1),
            ).unwrap_or(0),
        })
    };
    let page: Vec<Value> = ranked.iter().skip(start - 1).take(max).map(|(p, r)| row_json(*p, r)).collect();
    let user_row = mine
        .map(|i| row_json(ranked[i].0, ranked[i].1))
        .unwrap_or_else(|| json!({ "user": user_data(ctx, ctx.xuid) }));
    json!({ "rows": page, "userRow": user_row, "available": ranked.len() })
}

/// Transaction.Reconcile: the client asks for the reply to a transactional request it did not
/// get. WebRequestPayload is that whole reply (12-byte header + results), which the client
/// parses as if it had just arrived (FUN_140efce10). Xls error 6 (HRESULT 0x80510006) makes
/// it close the transaction as failed and stop asking (FUN_140f04c80).
fn reconcile(ctx: &Ctx, params: &Value) -> Value {
    let id = params["transactionId"].as_str().unwrap_or("");
    match ctx.store.transaction(id) {
        Some(body) => {
            let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
            json!({ "WebRequestPayload": hex, "cbMaxAvailable": body.len() })
        }
        None => {
            ctx.event("transaction", "Transação antiga descartada (o servidor não a conhecia)", json!({ "transactionId": id }));
            json!({ "__xtsw_error": { "code": 6, "message": "unknown transaction" } })
        }
    }
}

// ---------------------------------------------------------------- drivatars

fn save_drivatar(ctx: &Ctx, params: &Value) -> Value {
    ctx.store.save_drivatar(ctx.xuid, params);
    let segments = params["segmentInformation"].as_array().map(Vec::len).unwrap_or(0);
    let curves = params["curveTypes"].as_array().map(Vec::len).unwrap_or(0);
    ctx.event(
        "drivatar",
        format!("Seu drivatar aprendeu: {segments} trecho(s), {curves} tipo(s) de curva"),
        json!({ "segments": segments, "curves": curves }),
    );
    json!({})
}

/// The car a player's drivatar brings to a race of `bucket` (<= 0: any bucket):
/// their most-raced garage car in that bucket, else the bucket's hero car.
fn drivatar_car(ctx: &Ctx, owner: u64, bucket: i64) -> Option<i64> {
    let mut garage: Vec<Value> = ctx.store.tables(owner).remove("garageCars").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    garage.sort_by_key(|c| -c["numRaces"].as_i64().unwrap_or(0));
    let in_bucket = |car: i64| bucket <= 0 || ctx.catalog.car_bucket.get(&car) == Some(&bucket);
    garage
        .iter()
        .filter_map(|c| c["carId"].as_i64())
        .find(|&car| in_bucket(car))
        .or_else(|| (bucket > 0).then(|| ctx.catalog.bucket_cars.get(&bucket).and_then(|cars| cars.first().copied())).flatten())
}

/// DrivatarRacingInfo for one player. `car` None: pick with drivatar_car; others without a
/// stored drivatar or a usable car are left out (the game fills the grid with its own AI).
fn racing_info(ctx: &Ctx, owner: u64, bucket: i64, car: Option<i64>) -> Option<Value> {
    let (data, _, _) = ctx.store.drivatar(owner)?;
    let car = car.or_else(|| drivatar_car(ctx, owner, bucket))?;
    let take = |v: &Value, max: usize| Value::Array(v.as_array().map(|a| a.iter().take(max).cloned().collect()).unwrap_or_default());
    // the car's paint, fetched by the game with ContentVersioning.RetrieveVersionedLiveryById:
    // the versioned livery its garage entry points at, else (a car never repainted, like a
    // starting car) a base livery in its manufacturer colour; 0 = the car's colour 0
    let livery = if owner == ctx.xuid {
        0
    } else {
        let tables = ctx.store.tables(owner);
        let entries: Vec<&Value> = tables
            .get("garageCars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|c| c["carId"].as_i64() == Some(car))
            .collect();
        // Only a livery without layers: building a layered one for someone else's car stops
        // Azimuth ("CreatePlacementTexture2D requires GPU-write/CPU-map/unknown placement
        // semantics", 2026-10-14, a friend's 4-layer design in a car meet). Such a car shows
        // its manufacturer colour instead.
        // (the livery may be another player's: a design downloaded from the storefront)
        let stored = entries.iter().filter_map(|c| c["versionedLiveryId"].as_i64()).find(|&id| {
            id > 0
                && ctx.store.livery(id).is_some_and(|(x, p)| {
                    (x == owner || ctx.can_see(x)) && p["numberOfLayers"].as_i64().unwrap_or(0) == 0
                })
        });
        let colour = entries.first().and_then(|c| c["defaultManufacturerColorIndex"].as_i64()).unwrap_or(0);
        stored.unwrap_or(if (1..=255).contains(&colour) { BASE_LIVERY_IDS + car * 256 + colour } else { 0 })
    };
    Some(json!({
        "GamerTag": ctx.store.game_tag(owner),
        "IsFriend": owner != ctx.xuid && ctx.friend_ids().contains(&owner),
        "Xuid": owner,
        "CarId": car,
        "LiveryId": livery,
        "TuneId": 0,
        "BucketId": bucket.max(0),
        "Segments": take(&data["segmentInformation"], 200),
        "Curves": take(&data["curveTypes"], 20),
        "UserInfo": data["info"],
        "UserSkillLevel": 1,
        "wristbandLevel": 0,
    }))
}

/// Storefront keywords: (text, bucket). Bucket 0 fills KEYWORD 1 (what the file is), 1 fills
/// KEYWORD 2 (its style). Ids are the position + 1, kept stable: shared files store them. The
/// texts are the game's own (Storefront.str); which ones went with each file type was the
/// service's, so every type gets the same list.
const FILE_KEYWORDS: &[(&str, u8)] = &[
    ("RACING", 0), ("RACE CAR", 0), ("DRIFTING", 0), ("DRAG", 0), ("REPLICA", 0), ("REAL WORLD", 0),
    ("ORIGINAL", 0), ("ANIME", 0), ("CARTOON", 0), ("VIDEO GAME", 0), ("SUPER HERO", 0), ("SCI-FI", 0),
    ("FANTASY", 0), ("HORROR", 0), ("FUNNY", 0), ("ARTISTIC", 0), ("LOGO", 0), ("LICENSE PLATE", 0),
    ("GRIP", 0), ("GOOD HANDLING", 0), ("GOOD FOR SPEED", 0), ("ACTION SHOT", 0), ("CLOSE UP", 0), ("OTHER", 0),
    ("RETRO", 1), ("VINTAGE", 1), ("CLASSIC", 1), ("MODERN", 1), ("JDM", 1), ("EUROPEAN", 1),
    ("AMERICAN MUSCLE", 1), ("EXOTIC", 1), ("SUPERCAR", 1), ("REALISTIC", 1), ("DARK", 1), ("LIGHT", 1),
    ("PRETTY", 1), ("CUTE", 1), ("CRAZY", 1), ("SCARY", 1), ("BEGINNER", 1), ("EXPERT", 1),
    ("BLACK AND WHITE", 1), ("HIGH CONTRAST", 1), ("SATURATED", 1), ("OTHER", 1),
];

/// Livery ids from here up are base liveries made by the server (car * 256 + colour), not stored.
const BASE_LIVERY_IDS: i64 = 1 << 40;
/// A car's base livery as the game uploads it (UpdateLivery "BaseLivery_<car>_<date>", made
/// when a car is bought): [u32 packed size][u32 raw size][zlib of 1034 bytes]. This one is car
/// 461 in colour 0; only the car (u16 at 16) and the manufacturer colour (u32 at 20, and one
/// byte at 255, 335, 375 and 415) differ between cars (compared with car 1304 in colour 3).
const BASE_LIVERY_461_0: &str = "4f0000000a040000789c2bcb294a66644080b3304e6551590e2b032ae803e2f4cab21c8448833d0336403761902bb1aa42966764c4a70401f0a8fb0f04c4a823d63c6aab23d67da3ea46d561a863600465130008165073";

/// The base livery of `car` painted in its manufacturer colour `colour` (hex, as a blob).
fn base_livery(car: i64, colour: i64) -> Option<String> {
    let packed: Vec<u8> = (0..BASE_LIVERY_461_0.len() / 2)
        .map(|i| u8::from_str_radix(&BASE_LIVERY_461_0[2 * i..2 * i + 2], 16).unwrap_or(0))
        .collect();
    let mut raw = miniz_oxide::inflate::decompress_to_vec_zlib(packed.get(8..)?).ok()?;
    if raw.len() != 1034 || !(0..=u16::MAX as i64).contains(&car) || !(0..=255).contains(&colour) {
        return None;
    }
    raw[16..18].copy_from_slice(&(car as u16).to_le_bytes());
    raw[20..24].copy_from_slice(&(colour as u32).to_le_bytes());
    for at in [255, 335, 375, 415] {
        raw[at] = colour as u8;
    }
    let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut out = (zlib.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&zlib);
    Some(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// The race grid: playerData is the player's own drivatar (what the server learned so far,
/// so the game keeps training it); drivatarInfos are the other players' drivatars. With a
/// single player the list is empty and the game uses its offline AI. Friends come first, then
/// the rest of the server.
fn drivatars_for_race(ctx: &Ctx, params: &Value) -> Value {
    let bucket = params["bucketId"].as_i64().unwrap_or(-1);
    let n = params["numberOfDrivatars"].as_i64().unwrap_or(0).clamp(0, 64) as usize;
    let ids = |k: &str| -> Vec<u64> { params[k].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default() };
    let (cached, new_cars) = (ids("cachedDrivatars"), ids("cachedDrivatarsNewCars"));
    let others: Vec<Value> = ctx
        .drivatar_candidates()
        .into_iter()
        .filter(|x| !cached.contains(x) || new_cars.contains(x))
        .filter_map(|owner| racing_info(ctx, owner, bucket, None))
        .take(n)
        .collect();
    let player = racing_info(ctx, ctx.xuid, bucket, Some(0)).unwrap_or_else(|| {
        json!({ "GamerTag": ctx.store.game_tag(ctx.xuid), "Xuid": ctx.xuid, "UserSkillLevel": params["skillSelection"] })
    });
    if !others.is_empty() {
        let names: Vec<&str> = others.iter().filter_map(|o| o["GamerTag"].as_str()).collect();
        ctx.event("drivatar", format!("{} drivatar(es) no grid: {}", others.len(), names.join(", ")), json!({ "bucket": bucket }));
    }
    json!({ "playerData": player, "drivatarInfos": others, "currentTime": iso_utc(crate::unix_ms()) })
}

// ---------------------------------------------------------------- rivals

const NAME_FIRST: &[&str] = &[
    "Apex", "Late", "Neon", "Curb", "Ghost", "Redline", "Slip", "Turbo", "Horizon", "Gravel",
    "Nitro", "Midnight", "Drift", "Torque", "Chrome", "Dusty", "Rapid", "Silent", "Wild", "Lucky",
    "Brave", "Crimson", "Solar", "Rally",
];
const NAME_SECOND: &[&str] = &[
    "Hunter", "Braker", "Rider", "Kisser", "Line", "Rita", "Sam", "Taxi", "Hero", "Grace",
    "Nico", "Mara", "King", "Fox", "Wolf", "Viper", "Pilot", "Comet", "Duke", "Nova", "Rex",
    "Joy", "Blaze", "Storm",
];

/// A stable, readable gamertag for an XUID that is not a player (<= 15 chars, like Xbox).
pub fn generated_gamertag(xuid: u64) -> String {
    let mut h = xuid ^ 0x9e37_79b9_7f4a_7c15;
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    let a = NAME_FIRST[(h % NAME_FIRST.len() as u64) as usize];
    let b = NAME_SECOND[((h >> 16) % NAME_SECOND.len() as u64) as usize];
    let n = (h >> 32) % 100;
    let mut tag = format!("{a}{b}{n}");
    tag.truncate(15);
    tag
}

/// Scoreboard.GetRival: the friend whose best on this route is closest to the player's own
/// (in the car class the game asks for: pivotId), else anyone on the server, raced against
/// their ghost. None (the method stays down, no rival shown) until someone else has a time there.
fn pick_rival(ctx: &Ctx, params: &Value) -> Option<Value> {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    let (board, score_type, track, class) = (int("scoreboardType"), int("scoreType"), int("trackId"), int("pivotId"));
    let rows = ctx.store.leaderboard(board, score_type, Some(track), None);
    let mine = rows.iter().find(|r| r.xuid == ctx.xuid).map(|r| r.score);
    let friends = ctx.friend_ids();
    let in_class = |r: &crate::db::ScoreRow| {
        class < 0 || ctx.catalog.car(r.data["carId"].as_i64().unwrap_or(-1)).map(|c| c.class_id) == Some(class)
    };
    let distance = |r: &crate::db::ScoreRow| mine.map(|m| (r.score - m).abs()).unwrap_or(0.0);
    let closest = |friend: bool| {
        rows.iter()
            .filter(|r| r.xuid != ctx.xuid && friends.contains(&r.xuid) == friend && in_class(r))
            .min_by(|a, b| distance(a).partial_cmp(&distance(b)).unwrap_or(std::cmp::Ordering::Equal))
    };
    let rival = closest(true).or_else(|| closest(false))?;
    let car = rival.data["carId"].as_i64().unwrap_or(461);
    let class = rival.data["carClassId"].as_i64().unwrap_or(0);
    let ghost = rival_ghost(ctx, params, rival.xuid, rival.score);
    Some(rival_reply(ctx, params, rival.xuid, rival.score, car, class, ghost))
}

/// GetRival for an event the player was challenged on (Meus Rivais -> "Ir para"), or with an
/// explicit rivalXuid: that player's time.
fn challenge_rival(ctx: &Ctx, params: &Value) -> Option<Value> {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    let (board, score_type, track) = (int("scoreboardType"), int("scoreType"), int("trackId"));
    let rival_xuid = params["rivalXuid"].as_u64().unwrap_or(0);
    let best = |x: u64| ctx.store.leaderboard(board, score_type, Some(track), None).into_iter().find(|r| r.xuid == x);
    let sender = (rival_xuid != 0).then_some(rival_xuid);
    let (xuid, score, car, class) = if let Some(p) = ctx.store.pending_challenge(ctx.xuid, track, Some(int("rivalEventId")), sender) {
        let from = p["senderXuid"].as_u64().unwrap_or(0);
        (from, p["score"].as_f64()?, p["carId"].as_i64().unwrap_or(461), p["carClassId"].as_i64().unwrap_or(0))
    } else if rival_xuid != 0 {
        let r = best(rival_xuid)?;
        (rival_xuid, r.score, r.data["carId"].as_i64().unwrap_or(461), r.data["carClassId"].as_i64().unwrap_or(0))
    } else {
        return None;
    };
    // challenges come from friends; a time asked by xuid can be anyone's on the server
    if xuid == ctx.xuid || !ctx.can_see(xuid) {
        return None;
    }
    ctx.store.record_race_event(board, score_type, int("rivalEventId"), track, int("pivotId"));
    let ghost = rival_ghost(ctx, params, xuid, score);
    Some(rival_reply(ctx, params, xuid, score, car, class, ghost))
}

/// The rival's own ghost on this route, else anyone's ghost closest to that time, else 0 (a
/// time without a car to chase).
fn rival_ghost(ctx: &Ctx, params: &Value, rival: u64, score: f64) -> i64 {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    let (board, score_type, track) = (int("scoreboardType"), int("scoreType"), int("trackId"));
    let own = ctx
        .store
        .leaderboard(board, score_type, Some(track), None)
        .into_iter()
        .find(|r| r.xuid == rival)
        .and_then(|r| ctx.store.ghost_id(rival, board, score_type, r.data["gameScoreboardId"].as_i64().unwrap_or(-1), track));
    own.or_else(|| ctx.store.nearest_ghost(None, board, score_type, track, score).map(|g| g.0))
    .unwrap_or(0)
}

fn rival_reply(ctx: &Ctx, params: &Value, xuid: u64, score: f64, car: i64, class: i64, ghost: i64) -> Value {
    let (score_type, track) = (params["scoreType"].as_i64().unwrap_or(-1), params["trackId"].as_i64().unwrap_or(-1));
    let cat = ctx.catalog.car(car);
    let bounty = 9600;
    let user = user_data(ctx, xuid);
    ctx.event(
        "rival",
        format!("Rival em {}: {} com {}", ctx.track_name(track), user["wzGamerTag"].as_str().unwrap_or(""), format_score(score, score_type)),
        json!({ "rival": xuid.to_string(), "score": score, "trackId": track, "ghostId": ghost }),
    );
    let row = json!({
        "position": 1,
        "rank": 1,
        "user": user,
        "score": score,
        "submittedTime": iso_utc(crate::unix_ms()),
        "carId": car,
        "carClassId": if class > 0 { class } else { cat.map(|c| c.class_id).unwrap_or(0) },
        "carPerformanceIndex": cat.map(|c| c.pi as f64 / 1000.0).unwrap_or(0.5),
        "usedSTM": false, "usedTCS": false, "usedABS": true, "usedAutoBrake": false,
        "usedAutoShifting": true, "usedClutch": false, "isClean": true,
        "ghostFileId": ghost,
        "bounty": bounty,
    });
    json!({ "rival": { "scoreData": row, "bounty": bounty } })
}

fn create_tune(ctx: &Ctx, params: &Value) -> Value {
    let car_id = params["carId"].as_i64().unwrap_or(-1);
    let id = ctx.store.add_tune(ctx.xuid, car_id, params);
    ctx.event("tune", format!("Novo ajuste salvo para {}", ctx.car_name(car_id)), json!({ "tuneId": id, "carId": car_id }));
    json!({ "versionedTuneId": id })
}

fn format_score(score: f64, score_type: i64) -> String {
    if crate::db::lower_is_better(score_type) {
        let ms = (score * 1000.0).round() as i64;
        format!("{}:{:02}.{:03}", ms / 60000, ms / 1000 % 60, ms % 1000)
    } else {
        format!("{}", score.round() as i64)
    }
}

/// "2026-10-13T20:04:24.123Z" from unix milliseconds (no chrono: days-from-civil inverse).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        ms.rem_euclid(1000)
    )
}

/// Player text kept and shown to others (club names, notices, messages): no control or
/// formatting characters, single spaces, at most `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let kept: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !matches!(*c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB))
        .collect();
    kept.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Objects merge key by key; anything else in `over` replaces `base`.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Players who are not friends see each other's times, ghosts and drivatars; a banned
    /// player is left out; friends come first in the grid and as rivals.
    #[test]
    fn open_to_the_whole_server() {
        let dir = std::env::temp_dir().join(format!("fh2-open-{}", crate::db::accounts::new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        let catalog = Catalog::default();
        let pc = |n: u8| format!("{n:02x}").repeat(32);
        let [a, b, c, d] = [1u8, 2, 3, 4].map(|n| store.register_account(&format!("P{n}"), "00001", &pc(n)).unwrap().0.xuid);
        let ctx = |xuid| Ctx { store: &store, catalog: &catalog, xuid, events: RefCell::new(Vec::new()) };
        let call = |xuid, method: &str, params: Value| handle(&ctx(xuid), "Scoreboard", method, &params).unwrap();
        let route = json!({ "scoreboardType": 0, "scoreType": 1, "trackId": 7, "gameScoreboardId": 70, "carId": 461 });
        for (x, time) in [(a, 100.0), (b, 90.0), (c, 80.0)] {
            let mut data = route.clone();
            data["score"] = json!(time);
            let id = store.add_score(x, 1, &data);
            assert!(store.save_ghost(x, &id, b"ghost").is_some());
        }
        store.set_banned(c, true);
        // D is nobody's friend and sees A and B on the global board, not banned C
        let board = call(d, "GetRows", json!({ "scoreboardType": 0, "scoreType": 1, "view": 0, "trackId": 7, "startAt": 1, "maxResults": 50 }));
        let rows = board["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["user"]["qwXuid"], b);
        assert_eq!(rows[1]["position"], 2);
        let ghost = rows[0]["ghostFileId"].as_i64().unwrap();
        assert!(ghost > 0);
        assert!(call(d, "DownloadFile", json!({ "id": ghost, "maxSize": 1000 }))["data"].is_string());
        let banned = store.ghost_id(c, 0, 1, 70, 7).unwrap();
        assert!(call(d, "DownloadFile", json!({ "id": banned, "maxSize": 1000 }))["__xtsw_error"].is_object());
        // drivatars: everyone not banned, a friend first
        let garage = json!({ "garageCars": [{ "id": 1, "carId": 461, "numRaces": 3 }] });
        for x in [a, b, c] {
            store.save_drivatar(x, &json!({ "info": {}, "curveTypes": [], "segmentInformation": [] }));
            store.apply_tables(x, &garage);
        }
        store.add_friend(d, a);
        // the friends view: D's friend A only, at A's real position (2nd of the server)
        let friends = call(d, "GetRows", json!({ "scoreboardType": 0, "scoreType": 1, "view": 2, "trackId": 7, "startAt": 1, "maxResults": 50 }));
        assert_eq!(friends["rows"].as_array().unwrap().len(), 1);
        assert_eq!(friends["rows"][0]["user"]["qwXuid"], a);
        assert_eq!(friends["rows"][0]["position"], 2);
        assert_eq!(friends["available"], 1);
        let cache = handle(&ctx(d), "Drivatar", "GetDrivatarsForCache", &json!({ "numberOfDrivatars": 10 })).unwrap();
        let owners: Vec<u64> = cache["drivatarInfos"].as_array().unwrap().iter().map(|i| i["Xuid"].as_u64().unwrap()).collect();
        assert_eq!(owners, vec![a, b]);
        assert_eq!(cache["drivatarInfos"][0]["IsFriend"], true);
        assert_eq!(cache["drivatarInfos"][1]["IsFriend"], false);
        // rival: the friend A, even though B is closer to D's time
        let mut mine = route.clone();
        mine["score"] = json!(91.0);
        store.add_score(d, 1, &mine);
        let rival = call(d, "GetRival", json!({ "scoreboardType": 0, "scoreType": 1, "trackId": 7, "pivotId": -1 }));
        assert_eq!(rival["rival"]["scoreData"]["score"], 100.0);
        // without friends, the closest time on the server
        let rival = call(b, "GetRival", json!({ "scoreboardType": 0, "scoreType": 1, "trackId": 7, "pivotId": -1 }));
        assert_eq!(rival["rival"]["scoreData"]["score"], 91.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The game numbers club privacy from 1: an open club (1) takes anyone, a club open to
    /// friends (2) only friends of a member (others ask).
    #[test]
    fn club_privacy() {
        use crate::db::clubs::{ClubFields, PRIVACY_FRIENDS, PRIVACY_OPEN};
        let dir = std::env::temp_dir().join(format!("fh2-privacy-{}", crate::db::accounts::new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        let catalog = Catalog::default();
        let pc = |n: u8| format!("{n:02x}").repeat(32);
        let [a, b, c, d] = [1u8, 2, 3, 4].map(|n| store.register_account(&format!("P{n}"), "00001", &pc(n)).unwrap().0.xuid);
        let fields = |tag: &str, privacy| ClubFields {
            tag: tag.into(), name: format!("Club {tag}"), description: String::new(), privacy, region: 0, attitude: 0, interest: 0,
        };
        let open = store.create_club(a, &fields("OPN", PRIVACY_OPEN)).unwrap();
        let friends = store.create_club(b, &fields("FRN", PRIVACY_FRIENDS)).unwrap();
        let ctx = |xuid| Ctx { store: &store, catalog: &catalog, xuid, events: RefCell::new(Vec::new()) };
        let join = |xuid, club: i64| clubs::handle(&ctx(xuid), true, "Club", "RequestJoiningClub", &json!({ "clubId": club })).unwrap();
        assert_eq!(join(c, open)["requestApproved"], true);
        assert_eq!(join(d, friends)["requestApproved"], false);
        store.add_friend(d, b);
        store.delete_club_request(friends, d);
        assert_eq!(join(d, friends)["requestApproved"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text() {
        assert_eq!(clean_text("  oi\u{0}\r\n  tudo\u{202E}bem  ", 200), "oi tudobem");
        assert_eq!(clean_text("abcdef", 3), "abc");
    }

    #[test]
    fn iso() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_791_932_302_627), "2026-10-13T22:58:22.627Z");
    }

    #[test]
    fn base_livery_matches_the_game() {
        // the base livery the game made for car 1304 in colour 3 (a save backup)
        let real = "540000000a040000789c2bcb294a66644000095606066610a3b2a82c87950115f401717a65590e42a4c19e011ba09b30c895585521cb3332e2946746e6e051f71f08885147ac79d45647acfb46d58daac350c7c008ca260074394fd1";
        let raw = |h: &str| {
            let b: Vec<u8> = (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect();
            miniz_oxide::inflate::decompress_to_vec_zlib(&b[8..]).unwrap()
        };
        assert_eq!(raw(&base_livery(1304, 3).unwrap()), raw(real));
        assert_eq!(raw(&base_livery(461, 0).unwrap()), raw(BASE_LIVERY_461_0));
    }

    #[test]
    fn partial_update_all() {
        let full = json!({ "compareStats": [{ "carsInGarage": 1 }], "playerHistory": [{ "credits": 50000 }], "hudOptions": [{}] });
        // a partial UpdateAll: only playerHistory changed
        let partial = json!({ "compareStats": [], "playerHistory": [{ "credits": 61000 }], "hudOptions": [], "miscStats": [] });
        let merged = merge_profile(&full, &partial);
        assert_eq!(merged["compareStats"][0]["carsInGarage"], 1);
        assert_eq!(merged["playerHistory"][0]["credits"], 61000);
        assert!(merged["hudOptions"][0].is_object());
        assert_eq!(merged["miscStats"], json!([]));
        assert_eq!(merge_profile(&Value::Null, &partial), partial);
    }
}
