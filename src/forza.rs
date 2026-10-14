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
        let e = self.store.add_event(self.xuid, kind, title.as_ref(), detail);
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
        (false, "GameplayLog", "Write") => {
            ctx.event("gameplay", "Log de gameplay recebido", params.clone());
            json!({})
        }
        (true, "Notifications", "RetrieveForUser") => json!({
            "results": [], "parameterBlock": "", "nextRetrieveSeconds": 300,
            "reconcileActiveNotifications": false, "activeNotificationIds": [],
        }),
        (false, "DailyCreditBonus", "GetPayout") => json!({ "dailyPayout": {} }),
        // Lists that are empty until the server has the content (storefront, clubs, drivatars).
        (false, "Storefront", "GetFileKeywords")
        | (false, "Storefront", "GetLiveriesForDrivatar")
        | (false, "Storefront", "GetFeaturedLiveriesForRandomCars")
        | (false, "CarClub", "GetClubKeywords")
        | (false, "CarClub", "GetRecommendedClubs")
        | (false, "User", "GetOwnedProducts")
        | (false, "User", "GetCompareStatsForRelatedUsers")
        | (false, "Drivatar", "GetDrivatarsForCache") => json!({}),
        _ => return None,
    })
}

/// Xbox Live JSON services the game needs answered with real data (not a static stub).
pub fn live_service(ctx: &Ctx, host: &str, path: &str, body: &[u8]) -> Option<Value> {
    if host == "profile.xboxlive.com" && path.ends_with("/profile/settings") {
        let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let ids: Vec<String> = request["userIds"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let wanted: Vec<String> = request["settings"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let users: Vec<Value> = ids
            .iter()
            .map(|id| {
                let xuid = id.parse::<u64>().unwrap_or(0);
                let tag = ctx.store.gamertag(xuid);
                let settings: Vec<Value> = wanted
                    .iter()
                    .map(|name| {
                        let value = match name.as_str() {
                            "Gamertag" | "GameDisplayName" | "AppDisplayName" => tag.clone(),
                            "AccountTier" => "Gold".into(),
                            "Gamerscore" => "0".into(),
                            // Must be an absolute URL: XSAPI turns it into a Uri, an empty string
                            // gives a null Uri and the game crashes on it (game+0x124f916).
                            "GameDisplayPicRaw" | "AppDisplayPicRaw" => {
                                "http://images-eds.xboxlive.com/image?url=fh2online-gamerpic".into()
                            }
                            _ => String::new(),
                        };
                        json!({ "id": name, "value": value })
                    })
                    .collect();
                json!({ "id": id, "hostId": id, "settings": settings, "isSponsoredUser": false })
            })
            .collect();
        return Some(json!({ "profileUsers": users }));
    }
    None
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
            "permissions": [],
            "featureAreasBanMask": 0,
            "userFlags": 0,
            "upsellOfferId": 0,
            "upsellText": "",
            "vipOfferIds": [],
            "userData": user_data(ctx, xuid),
            "notificationNonce": 0,
            "notificationPollIntervalSeconds": 300,
            "defaultFriendsPollIntervalSeconds": 300,
            "maxFriendsPollIntervalSeconds": 900,
            "notificationCategories": [],
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
    json!({
        "qwXuid": xuid,
        "wzGamerTag": ctx.store.gamertag(xuid),
        "flags": 0,
        "clubId": 0,
        "clubTag": "",
        "clubTopTierCount": 0,
        "roleInClub": 0,
        "acceptsClubInvites": true,
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
    json!({ "scoreId": score_id, "uploadFiles": false })
}

fn submit_xp(ctx: &Ctx, params: &Value) -> Value {
    let xp = params["xp"].as_f64().unwrap_or(0.0);
    ctx.store.add_xp(ctx.xuid, xp);
    ctx.event("xp", format!("+{} XP", xp.round() as i64), json!({ "xp": xp }));
    json!({ "weeklyTierAchieved": false, "weeklyTierInfo": {} })
}

fn best_score(ctx: &Ctx, params: &Value) -> Value {
    let int = |k: &str| params[k].as_i64().unwrap_or(-1);
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
