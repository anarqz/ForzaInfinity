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
mod notifications;
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
        (false, "ContentVersioning", "CreateVersionedTuneFromStructs") => create_tune(ctx, params),
        (false, "ContentVersioning", "CreateVersionedLivery") => {
            let car = params["carId"].as_i64().unwrap_or(-1);
            let id = ctx.store.add_livery(ctx.xuid, params["name"].as_str().unwrap_or(""), car, params);
            json!({ "versionedLiveryId": id })
        }
        (false, "OnlineProfile", "UpdateLivery") => {
            let car = params["carId"].as_i64().unwrap_or(-1);
            ctx.store.add_livery(ctx.xuid, params["name"].as_str().unwrap_or(""), car, params);
            ctx.event("livery", format!("Pintura salva para {}", ctx.car_name(car)), json!({ "name": params["name"], "carId": car }));
            json!({})
        }
        (false, "Scoreboard", "GetRival") => random_rival(ctx, params),
        (false, "Scoreboard", "UploadFile") => upload_file(ctx, params),
        (false, "Scoreboard", "DownloadFile") => download_file(ctx, params),
        (false, "Drivatar", "SaveDrivatarInformation") => save_drivatar(ctx, params),
        (false, "Drivatar", "GetDrivatarInformationForRace") => drivatars_for_race(ctx, params),
        (false, "Drivatar", "GetDrivatarsForCache") => {
            let n = params["numberOfDrivatars"].as_i64().unwrap_or(0).clamp(0, 200) as usize;
            let infos: Vec<Value> = ctx
                .store
                .drivatar_owners(ctx.xuid, 500)
                .into_iter()
                .filter_map(|owner| racing_info(ctx, owner, -1, None))
                .take(n)
                .collect();
            json!({ "drivatarInfos": infos })
        }
        (false, "GameplayLog", "Write") => {
            ctx.event("gameplay", "Log de gameplay recebido", params.clone());
            json!({})
        }
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
            let max = params["numDesiredUsers"].as_i64().filter(|&n| n > 0).unwrap_or(10) as usize;
            let stats: Vec<Value> = ctx
                .store
                .friends(ctx.xuid)
                .into_iter()
                .take(max)
                .map(|(friend, _, _)| {
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
                xuids = ctx.store.friends(ctx.xuid).into_iter().map(|f| f.0).collect();
            }
            let max = params["numUsers"].as_i64().filter(|&n| n > 0).unwrap_or(10) as usize;
            let stats: Vec<Value> = xuids
                .into_iter()
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
        (false, "Storefront", "GetFileKeywords")
        | (false, "Storefront", "GetLiveriesForDrivatar")
        | (false, "Storefront", "GetFeaturedLiveriesForRandomCars")
        | (false, "Storefront", "GetCuratedContent")
        | (false, "Storefront", "GetMyFileList")
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
    // "me" and xuid(...) in the path. Xbox Live calls carry no X-UserId, only the token: until
    // the server authenticates players, "me" is the player who logged on last.
    let me = match ctx.xuid {
        0 => ctx.store.latest_player(),
        x => x,
    };
    let subject = path_xuid(path).unwrap_or(me);

    if host == "profile.xboxlive.com" && path.ends_with("/profile/settings") {
        let request: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let ids: Vec<u64> = request["userIds"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()?.parse().ok()).collect())
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
        let users: Vec<Value> =
            ctx.store.friends(subject).iter().map(|&(friend, _, _)| profile_user(ctx, friend, &wanted)).collect();
        return Some(json!({ "profileUsers": users }));
    }
    // social.xboxlive.com: who the player follows, with the favorite / follows-back flags.
    if host == "social.xboxlive.com" && path.ends_with("/people") {
        let people: Vec<Value> = ctx
            .store
            .friends(subject)
            .iter()
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
    // The Xbox text filter, asked before the game posts text (club notice board, messages):
    // everything passes. resultCode 0 = Success.
    if host == "client-strings.xboxlive.com" && path.ends_with("/strings/validate") {
        let body: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let n = body["stringstoVerify"].as_array().map(Vec::len).unwrap_or(0);
        let results: Vec<Value> = (0..n).map(|_| json!({ "resultCode": "0", "offendingString": "" })).collect();
        return Some(json!({ "verifyStringResult": results }));
    }
    // Rich presence (RP_SPLASH, RP_FREEROAM, ...): the game posts it every few seconds.
    if host == "userpresence.xboxlive.com" && path.ends_with("/devices/current/titles/current") {
        if req.method == "DELETE" {
            ctx.store.set_presence(subject, "offline", "");
            return Some(json!({}));
        }
        let body: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
        let state = body["state"].as_str().unwrap_or("active");
        let rich = body["activity"]["richPresence"]["id"].as_str().unwrap_or("");
        if ctx.store.set_presence(subject, state, rich) {
            ctx.event_for(subject, "presence", rich_presence_text(rich), json!({ "rich": rich, "state": state }));
        }
        return Some(json!({}));
    }
    // Presence of everyone the player follows.
    if host == "userpresence.xboxlive.com" && path.ends_with("/groups/People") {
        let records: Vec<Value> =
            ctx.store.friends(subject).iter().map(|&(friend, _, _)| presence_record(ctx, friend)).collect();
        return Some(Value::Array(records));
    }
    // One player's presence.
    if host == "userpresence.xboxlive.com" && req.method == "GET" && path_xuid(path).is_some() && path.ends_with(')') {
        return Some(presence_record(ctx, subject));
    }
    None
}

/// One profile.xboxlive.com user with the requested settings.
fn profile_user(ctx: &Ctx, xuid: u64, wanted: &[String]) -> Value {
    // The game looks up every grid driver and rival here: offline AI drivatars and our
    // made-up rivals are not players, so they get a stable generated name.
    let known = ctx.store.gamertag(xuid);
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
    let xuid = login["xuid"].as_u64().unwrap_or(ctx.xuid);
    let gamertag = login["gamerTag"].as_str().unwrap_or("").to_owned();
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
        "wzGamerTag": ctx.store.gamertag(xuid),
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

fn profile_update_all(ctx: &Ctx, params: &Value) -> Value {
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
    let summary = ctx.store.apply_tables(ctx.xuid, params);
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
    notifications::rivals_beaten(ctx, score_type, data);
    // Ask for the ghost (Scoreboard.UploadFile) only for a new personal best on this event.
    json!({ "scoreId": score_id, "uploadFiles": ctx.store.is_personal_best(&score_id) })
}

/// Scoreboard.UploadFile: the ghost/replay of a score, kept only if that score is still the
/// player's best on its event.
fn upload_file(ctx: &Ctx, params: &Value) -> Value {
    let score_id = params["scoreId"].as_str().unwrap_or("");
    let data: Vec<u8> = params["data"]
        .as_str()
        .map(|h| (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect())
        .unwrap_or_default();
    match ctx.store.save_ghost(score_id, &data) {
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

/// Scoreboard.DownloadFile: a stored ghost by the id the leaderboards hand out (ghostFileId).
fn download_file(ctx: &Ctx, params: &Value) -> Value {
    let id = params["id"].as_i64().unwrap_or(0);
    let max = params["maxSize"].as_i64().unwrap_or(i64::MAX).max(0) as usize;
    match ctx.store.ghost(id) {
        Some(data) if data.len() <= max && data.len() <= 0x80000 => {
            ctx.event("ghost", format!("Fantasma #{id} enviado ao jogo ({} KB)", (data.len() + 1023) / 1024), json!({ "ghostId": id }));
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            json!({ "data": hex })
        }
        Some(data) => json!({ "__xtsw_error": { "code": 1, "message": format!("ghost too big ({} bytes, max {max})", data.len()) } }),
        None => json!({ "__xtsw_error": { "code": 1, "message": "no such ghost" } }),
    }
}

fn submit_xp(ctx: &Ctx, params: &Value) -> Value {
    let xp = params["xp"].as_f64().unwrap_or(0.0);
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

fn rows_anthem(ctx: &Ctx, params: &Value) -> Value {
    let track = params["trackId"].as_i64().filter(|t| *t >= 0);
    let rows = ctx.store.leaderboard(
        params["scoreboardType"].as_i64().unwrap_or(-1),
        params["scoreType"].as_i64().unwrap_or(-1),
        track,
        None,
    );
    let start = params["startAt"].as_i64().unwrap_or(1).max(1) as usize;
    let max = params["maxResults"].as_i64().unwrap_or(50).clamp(0, 500) as usize;
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
    let page: Vec<Value> =
        rows.iter().enumerate().skip(start - 1).take(max).map(|(i, r)| row_json(i + 1, r)).collect();
    let user_row = rows
        .iter()
        .position(|r| r.xuid == ctx.xuid)
        .map(|i| row_json(i + 1, &rows[i]))
        .unwrap_or_else(|| json!({ "user": user_data(ctx, ctx.xuid) }));
    json!({ "rows": page, "userRow": user_row, "available": rows.len(), "totalRowCount": rows.len() })
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
    Some(json!({
        "GamerTag": ctx.store.gamertag(owner),
        "IsFriend": false,
        "Xuid": owner,
        "CarId": car,
        "LiveryId": 0,
        "TuneId": 0,
        "BucketId": bucket.max(0),
        "Segments": take(&data["segmentInformation"], 200),
        "Curves": take(&data["curveTypes"], 20),
        "UserInfo": data["info"],
        "UserSkillLevel": 1,
        "wristbandLevel": 0,
    }))
}

/// The race grid: playerData is the player's own drivatar (what the server learned so far,
/// so the game keeps training it); drivatarInfos are the other players' drivatars. With a
/// single player the list is empty and the game uses its offline AI.
fn drivatars_for_race(ctx: &Ctx, params: &Value) -> Value {
    let bucket = params["bucketId"].as_i64().unwrap_or(-1);
    let n = params["numberOfDrivatars"].as_i64().unwrap_or(0).clamp(0, 64) as usize;
    let ids = |k: &str| -> Vec<u64> { params[k].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default() };
    let (cached, new_cars) = (ids("cachedDrivatars"), ids("cachedDrivatarsNewCars"));
    let others: Vec<Value> = ctx
        .store
        .drivatar_owners(ctx.xuid, 500)
        .into_iter()
        .filter(|x| !cached.contains(x) || new_cars.contains(x))
        .filter_map(|owner| racing_info(ctx, owner, bucket, None))
        .take(n)
        .collect();
    let player = racing_info(ctx, ctx.xuid, bucket, Some(0)).unwrap_or_else(|| {
        json!({ "GamerTag": ctx.store.gamertag(ctx.xuid), "Xuid": ctx.xuid, "UserSkillLevel": params["skillSelection"] })
    });
    if !others.is_empty() {
        let names: Vec<&str> = others.iter().filter_map(|o| o["GamerTag"].as_str()).collect();
        ctx.event("drivatar", format!("{} drivatar(es) no grid: {}", others.len(), names.join(", ")), json!({ "bucket": bucket }));
    }
    json!({ "playerData": player, "drivatarInfos": others, "currentTime": iso_utc(crate::unix_ms()) })
}

// ---------------------------------------------------------------- rivals

/// splitmix64 seeded from the clock and a call counter: random-looking rivals without a
/// dependency, different even for two calls in the same millisecond.
fn rng() -> impl FnMut() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let mut s = (crate::unix_ms() as u64) ^ CALLS.fetch_add(1, Ordering::Relaxed).wrapping_mul(0xd1b5_4a32_d192_ed03);
    move || {
        s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

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

/// Scoreboard.GetRival: a made-up rival around the player's own time on that track (or a
/// time from the track length), so the rival shows up in game. Real rivals come with friends.
fn random_rival(ctx: &Ctx, params: &Value) -> Value {
    let mut rand = rng();
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    let (board, score_type, track) = (int("scoreboardType"), int("scoreType"), int("trackId"));
    if let Some(reply) = challenge_rival(ctx, params) {
        return reply;
    }
    let mine = ctx.store.leaderboard(board, score_type, Some(track), None).into_iter().find(|r| r.xuid == ctx.xuid);
    let lower = crate::db::lower_is_better(score_type);
    let base = match &mine {
        Some(r) => r.score,
        None if lower => {
            // seconds at ~110 km/h over the track length, or 90 s
            ctx.catalog.tracks.get(&track).map(|t| t.length_m as f64 / 30.5).filter(|s| *s > 5.0).unwrap_or(90.0)
        }
        None => 25_000.0,
    };
    // a bit faster or slower than you: 6 % quicker to 3 % slower (times), similar for points
    let factor = 0.94 + rand() * 0.09;
    let score = if lower { base * factor } else { base * (2.0 - factor) };
    let score = (score * 1000.0).round() / 1000.0;
    // a ghost recorded on this route makes the rival raceable: take its time and car
    // (same car class as asked for: pivotId)
    let class = int("pivotId");
    let ghost = ctx
        .store
        .nearest_ghost(board, score_type, track, score)
        .filter(|g| class < 0 || ctx.catalog.car(g.2).map(|c| c.class_id) == Some(class));
    let score = ghost.map(|g| g.1).unwrap_or(score);

    let car = ghost
        .map(|g| g.2)
        .or_else(|| mine.as_ref().and_then(|r| r.data["carId"].as_i64()))
        .or_else(|| drivatar_car(ctx, ctx.xuid, -1))
        // never an invalid id: any real car (the Supra if present)
        .or_else(|| ctx.catalog.car(461).map(|c| c.id).or_else(|| ctx.catalog.cars.keys().next().copied()))
        .unwrap_or(461);
    let cat = ctx.catalog.car(car);
    // the game asks profile.xboxlive.com for this XUID's name: generate it the same way
    let xuid = 0x0009_0000_0100_0000u64 + (rand() * 1_000_000.0) as u64;
    let name = generated_gamertag(xuid);
    let name = name.as_str();
    let bounty = ((1000.0 + rand() * 9000.0) / 100.0).round() as i64 * 100;
    let days_ago = (rand() * 20.0) as i64;
    ctx.event(
        "rival",
        format!(
            "Rival em {}: {name} com {}{}",
            ctx.track_name(track),
            format_score(score, score_type),
            cat.map(|c| format!(" ({})", c.name)).unwrap_or_default()
        ),
        json!({ "rival": name, "score": score, "trackId": track, "bounty": bounty }),
    );
    let row = json!({
        "position": 1,
        "rank": 1,
        "user": {
            "qwXuid": xuid, "wzGamerTag": name, "flags": 0, "clubId": 0, "clubTag": "",
            "clubTopTierCount": 0, "roleInClub": 0, "acceptsClubInvites": false,
            "currentCareerLevel": 20 + (rand() * 200.0) as u32, "currentBadgeId": 0, "currentPlayerTitleId": 0,
        },
        "score": score,
        "submittedTime": iso_utc(crate::unix_ms() - days_ago * 86_400_000),
        "carId": car,
        "carClassId": cat.map(|c| c.class_id).unwrap_or(0),
        "carPerformanceIndex": cat.map(|c| c.pi as f64 / 1000.0).unwrap_or(0.5),
        "usedSTM": false, "usedTCS": false, "usedABS": true, "usedAutoBrake": false,
        "usedAutoShifting": rand() < 0.5, "usedClutch": false, "isClean": true,
        "ghostFileId": ghost.map(|g| g.0).unwrap_or(0),
        "bounty": bounty,
    });
    json!({ "rival": { "scoreData": row, "bounty": bounty } })
}

/// GetRival for an event the player was challenged on (Meus Rivais -> "Ir para"), or with an
/// explicit rivalXuid: the friend's time instead of a random rival.
fn challenge_rival(ctx: &Ctx, params: &Value) -> Option<Value> {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
    let (board, score_type, track) = (int("scoreboardType"), int("scoreType"), int("trackId"));
    let rival_xuid = params["rivalXuid"].as_u64().unwrap_or(0);
    // the friend's own best on this route, if they raced it
    let best = |x: u64| ctx.store.leaderboard(board, score_type, Some(track), None).into_iter().find(|r| r.xuid == x);
    let (xuid, score, car, class, ghost) = if let Some(p) = ctx.store.pending_challenge(ctx.xuid, track, Some(int("rivalEventId"))) {
        let from = p["senderXuid"].as_u64().unwrap_or(0);
        let ghost = best(from).and_then(|r| ctx.store.ghost_id(from, board, score_type, r.data["gameScoreboardId"].as_i64().unwrap_or(-1), track));
        (from, p["score"].as_f64()?, p["carId"].as_i64().unwrap_or(461), p["carClassId"].as_i64().unwrap_or(0), ghost.unwrap_or(0))
    } else if rival_xuid != 0 {
        let r = best(rival_xuid)?;
        let ghost = ctx.store.ghost_id(rival_xuid, board, score_type, r.data["gameScoreboardId"].as_i64().unwrap_or(-1), track);
        (rival_xuid, r.score, r.data["carId"].as_i64().unwrap_or(461), r.data["carClassId"].as_i64().unwrap_or(0), ghost.unwrap_or(0))
    } else {
        return None;
    };
    // a rival without an uploaded ghost races with the closest one recorded on the route
    let ghost = if ghost != 0 { ghost } else { ctx.store.nearest_ghost(board, score_type, track, score).map(|g| g.0).unwrap_or(0) };
    ctx.store.record_race_event(board, score_type, int("rivalEventId"), track, int("pivotId"));
    let cat = ctx.catalog.car(car);
    let bounty = 9600;
    let mut user = user_data(ctx, xuid);
    if user["wzGamerTag"].as_str().unwrap_or("").is_empty() {
        user["wzGamerTag"] = json!(generated_gamertag(xuid));
    }
    ctx.event(
        "rival",
        format!("Desafio em {}: {} com {}", ctx.track_name(track), user["wzGamerTag"].as_str().unwrap_or(""), format_score(score, score_type)),
        json!({ "rival": xuid.to_string(), "score": score, "trackId": track }),
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
    Some(json!({ "rival": { "scoreData": row, "bounty": bounty } }))
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

    #[test]
    fn iso() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_791_932_302_627), "2026-10-13T22:58:22.627Z");
    }
}
