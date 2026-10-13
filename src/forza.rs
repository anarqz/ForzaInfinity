//! Built-in handlers for Forza / Xls web service methods.
//!
//! A handler gets the decoded `_Parameters` (JSON, see xtsw.rs) and returns the `_Results` as
//! JSON. Fields it leaves out are sent as zero / empty. A spec file
//! `responses/forza/<Class>/<Method>.json` with an `xtsw` object is merged over the result, so
//! any field can be tweaked without rebuilding.

use std::{
    fs,
    path::Path,
};

use serde_json::{Value, json};

use crate::unix_ms;

/// `class` is the X-ClassName without the assembly, e.g. "Forza.WebServices.User".
pub fn handle(root: &Path, class: &str, method: &str, params: &Value) -> Option<Value> {
    match (class, method) {
        ("Forza.WebServices.User", "Logon") => Some(user_logon(root, params)),
        _ => None,
    }
}

/// User.Logon: the game repeats it until it gets a valid reply, and nothing else in the Forza
/// services starts before it succeeds.
fn user_logon(root: &Path, params: &Value) -> Value {
    let login = &params["loginData"];
    let xuid = login["xuid"].as_u64().unwrap_or(0);
    let gamertag = login["gamerTag"].as_str().unwrap_or("").to_owned();

    let mut profile = load_user(root, xuid);
    profile["xuid"] = xuid.into();
    profile["gamerTag"] = gamertag.clone().into();
    profile["lastLogonMs"] = (unix_ms() as u64).into();
    profile["logons"] = (profile["logons"].as_u64().unwrap_or(0) + 1).into();
    profile["lastLoginData"] = login.clone();
    save_user(root, xuid, &profile);

    json!({
        "responseData": {
            "isProfileValid": true,
            // echo the client's checkpoint so it does not think the server copy is stale
            "lastOnlineProfileUpdateAllTablesTranId": login["clientProfile"]["transactionCount"],
            "permissions": [],
            "featureAreasBanMask": 0,
            "userFlags": 0,
            "upsellOfferId": 0,
            "upsellText": "",
            "vipOfferIds": [],
            "userData": {
                "qwXuid": xuid,
                "wzGamerTag": gamertag,
                "flags": 0,
                "clubId": 0,
                "clubTag": "",
                "clubTopTierCount": 0,
                "roleInClub": 0,
                "acceptsClubInvites": true,
                "currentCareerLevel": login["currentCareerLevel"],
                "currentBadgeId": login["currentBadgeId"],
                "currentPlayerTitleId": login["currentPlayerTitleId"],
            },
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

fn user_path(root: &Path, xuid: u64) -> std::path::PathBuf {
    root.join("data").join("users").join(format!("{xuid:016x}.json"))
}

fn load_user(root: &Path, xuid: u64) -> Value {
    fs::read_to_string(user_path(root, xuid))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn save_user(root: &Path, xuid: u64, profile: &Value) {
    let path = user_path(root, xuid);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(profile) {
        if let Err(e) = fs::write(&path, text) {
            eprintln!("{}: {e}", path.display());
        }
    }
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
