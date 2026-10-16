//! The Storefront among friends: sharing designs, vinyl groups and tuning setups
//! (Forza.WebServices.StorefrontLivery / StorefrontLayerGroup / StorefrontTuning.Create),
//! browsing and downloading them (Forza.WebServices.Storefront), the libraries the game syncs
//! (OnlineProfile) and the versioned content behind a shared file (ContentVersioning).
//!
//! There are no public files: a player sees their own, their friends' and their club's. A file
//! points at versioned content; the game downloads it with ContentVersioning.Retrieve*ById
//! (VersionedContentId) and then calls AddReferenceTo*, which counts as a download.
//!
//! fileType 2 = livery (seen in StorefrontLivery.Create); the other types are kept as the game
//! sends them in the file header.

use serde_json::{Value, json};

use super::{Ctx, clean_text, iso_utc, user_data};
use crate::db::storefront::{MAX_PREVIEW_HEX, NewFile, Report, STRIKES_TO_REMOVE, SharedFile};

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(-1)
}

fn preview(v: &Value) -> &str {
    v.as_str().filter(|h| h.len() <= MAX_PREVIEW_HEX && h.bytes().all(|b| b.is_ascii_hexdigit())).unwrap_or("")
}

fn error(message: &str) -> Value {
    json!({ "__xtsw_error": { "code": 1, "message": message } })
}

/// ForzaStorefrontFileLightweightData (search results).
fn lightweight(ctx: &Ctx, f: &SharedFile) -> Value {
    json!({
        "Id": f.id,
        "PopularityRating": f.likes.min(5),
        "Creator": user_data(ctx, f.xuid),
        "IsCreatorFollowed": ctx.store.follows(ctx.xuid, f.xuid),
        "SubmissionTime": iso_utc(f.created),
        "Ordinal": 0,
        "MethodObtained": 0,
    })
}

/// ForzaStorefrontFile (details, preview and the content to download).
fn heavyweight(ctx: &Ctx, f: &SharedFile) -> Value {
    json!({
        "Id": f.id,
        "Type": f.file_type,
        "Title": f.title,
        "Description": f.description,
        "SubmissionTime": iso_utc(f.created),
        "Owner": user_data(ctx, f.xuid),
        "PayloadSize": 0,
        "PreviewPayload": ctx.store.shared_file_preview(f.id),
        "Ordinal": 0,
        "IsUnderReview": false,
        "VersionedContentId": f.versioned_id,
        "PopularityRating": f.likes.min(5),
        "IsCreatorFollowed": ctx.store.follows(ctx.xuid, f.xuid),
    })
}

/// SearchLiveryFiles / SearchLayerGroupFiles / SearchTuningFiles: the visible files of `kind`
/// that match the search (fields <= 0 or empty mean "any").
fn search(ctx: &Ctx, kind: &str, params: &Value) -> Value {
    let s = &params["searchParameters"];
    let max = params["maxFiles"].as_i64().filter(|&n| n > 0).unwrap_or(50).min(200) as usize;
    let (make, model, class) = (int(&s["MakeId"]), int(&s["ModelId"]), int(&s["ClassId"]));
    let (k1, k2, track, layers) = (int(&s["KeywordOneId"]), int(&s["KeywordTwoId"]), int(&s["TrackId"]), int(&s["NumberOfLayers"]));
    let text = s["ManualSearchKeywords"].as_str().unwrap_or("").trim().to_lowercase();
    let gamertag = s["GamerTag"].as_str().unwrap_or("").trim().to_lowercase();
    let results: Vec<Value> = ctx
        .store
        .shared_files(Some(kind), None)
        .into_iter()
        .filter(|f| ctx.can_see(f.xuid))
        .filter(|f| {
            let car = ctx.catalog.car(f.car_id);
            (model <= 0 || f.car_id == model)
                && (make <= 0 || car.is_some_and(|c| c.make_id == make))
                && (class <= 0 || car.is_some_and(|c| c.class_id == class) || f.meta["ClassId"].as_i64() == Some(class))
                && (k1 <= 0 || f.keyword_one == k1)
                && (k2 <= 0 || f.keyword_two == k2)
                && (track <= 0 || f.meta["TrackId"].as_i64() == Some(track))
                && (layers <= 0 || f.layers <= layers)
                && (text.is_empty() || f.title.to_lowercase().contains(&text) || f.description.to_lowercase().contains(&text))
                && (gamertag.is_empty() || ctx.store.gamertag(f.xuid).to_lowercase().contains(&gamertag))
        })
        .take(max)
        .map(|f| lightweight(ctx, &f))
        .collect();
    json!({ "searchResults": results })
}

/// The *.Create of a shared file: the header and the versioned content it shares.
fn share(ctx: &Ctx, kind: &str, params: &Value, versioned_id: i64, car_id: i64, layers: i64, meta: &Value) -> Value {
    let header = &params["fileHeader"];
    let title = clean_text(header["Title"].as_str().unwrap_or(""), 16);
    let description = clean_text(header["Description"].as_str().unwrap_or(""), 256);
    let id = ctx.store.share_file(
        ctx.xuid,
        &NewFile {
            kind,
            file_type: int(&header["Type"]),
            versioned_id,
            car_id,
            title: &title,
            description: &description,
            keyword_one: int(&header["KeywordOne"]),
            keyword_two: int(&header["KeywordTwo"]),
            file_name: &clean_text(params["fileName"].as_str().unwrap_or(""), 64),
            layers,
            meta,
            preview: preview(&params["previewData"]),
        },
    );
    if id == 0 {
        return error("too many shared files");
    }
    if id < 0 {
        return error("this file was taken off the storefront after reports");
    }
    let what = match kind {
        "livery" => "Design",
        "group" => "Grupo de vinil",
        _ => "Ajuste",
    };
    let car = if car_id > 0 { format!(" para {}", ctx.car_name(car_id)) } else { String::new() };
    ctx.event("storefront", format!("{what} compartilhado{car}: {title}"), json!({ "fileId": id, "kind": kind }));
    json!({ "contentFileId": id })
}

/// Counts a download when someone adds a reference to versioned content another player shared.
fn referenced(ctx: &Ctx, kind: &str, versioned_id: i64) {
    if let Some(f) = ctx.store.shared_file_of(kind, versioned_id).filter(|f| f.xuid != ctx.xuid && ctx.can_see(f.xuid)) {
        ctx.store.count_file(f.id, ctx.xuid, "downloads");
    }
}

/// A downloaded design or tune is not copied: the game puts the shared versioned id on the
/// player's own car (seen in UpdateAllTables right after RetrieveVersionedLiveryById). Each
/// garage car using another player's shared content counts one download of that file.
pub fn count_downloads(ctx: &Ctx, tables: &Value) {
    for car in tables["garageCars"].as_array().into_iter().flatten() {
        for (kind, field) in [("livery", "versionedLiveryId"), ("tune", "versionedTuneId")] {
            let id = car[field].as_i64().unwrap_or(-1);
            if id > 0 {
                referenced(ctx, kind, id);
            }
        }
    }
}

pub fn handle(ctx: &Ctx, xls: bool, class: &str, method: &str, params: &Value) -> Option<Value> {
    if xls {
        return None;
    }
    let me = ctx.xuid;
    Some(match (class, method) {
        // ---------------------------------------------------------------- sharing
        ("StorefrontLivery", "Create") => {
            let d = &params["liveryDescription"];
            let versioned = int(&d["base"]["VersionedLiveryId"]);
            let car = int(&d["carId"]);
            // the player's own versioned livery (a base livery made by the server is anyone's)
            let layers = match ctx.store.livery(versioned) {
                Some((owner, p)) if owner == me => p["numberOfLayers"].as_i64().unwrap_or(0),
                _ if versioned >= super::BASE_LIVERY_IDS => 0,
                _ => return Some(error("no such livery")),
            };
            share(ctx, "livery", params, versioned, car, layers, &json!({}))
        }
        ("StorefrontLayerGroup", "Create") => {
            let d = &params["layerGroupDescription"]["base"];
            let versioned = int(&d["VersionedLayerGroupId"]);
            if !ctx.store.versioned_group(versioned).is_some_and(|g| g.0 == me) {
                return Some(error("no such vinyl group"));
            }
            share(ctx, "group", params, versioned, -1, int(&d["NumberOfLayers"]).max(0), &json!({}))
        }
        ("StorefrontTuning", "Create") => {
            let d = &params["tuningDescription"]["base"];
            let versioned = int(&d["VersionedTuneId"]);
            let Some((owner, tune)) = ctx.store.tune(versioned) else { return Some(error("no such tune")) };
            if owner != me {
                return Some(error("no such tune"));
            }
            let meta = json!({
                "MakeId": d["MakeId"], "ModelId": d["ModelId"], "ClassId": d["ClassId"], "EnvironmentId": d["EnvironmentId"],
                "TrackId": d["TrackId"], "PerformanceIndex": d["PerformanceIndex"],
            });
            share(ctx, "tune", params, versioned, tune["carId"].as_i64().unwrap_or(int(&d["ModelId"])), 0, &meta)
        }

        // ---------------------------------------------------------------- browsing
        ("Storefront", "SearchLiveryFiles") => search(ctx, "livery", params),
        ("Storefront", "SearchLayerGroupFiles") => search(ctx, "group", params),
        ("Storefront", "SearchTuningFiles") => search(ctx, "tune", params),
        // RECOMMENDED DESIGNS (ContentType = the file type, 2 = designs): every visible file of
        // that type for the car, the most used first (friends' before the player's own).
        ("Storefront", "GetCuratedContent") => {
            let c = &params["curationParameters"];
            let (file_type, car, track) = (int(&c["ContentType"]), int(&c["CarId"]), int(&c["TrackId"]));
            let max = params["maxContentResults"].as_i64().filter(|&n| n > 0).unwrap_or(20).min(200) as usize;
            let mut files: Vec<SharedFile> = ctx
                .store
                .shared_files(None, None)
                .into_iter()
                .filter(|f| f.file_type == file_type && ctx.can_see(f.xuid))
                .filter(|f| car <= 0 || f.car_id == car)
                .filter(|f| track <= 0 || f.meta["TrackId"].as_i64().is_none_or(|t| t <= 0 || t == track))
                .collect();
            // newest first already; a stable sort keeps that as the tie-break
            files.sort_by_key(|f| (f.xuid == me, -(3 * f.uses + 2 * f.downloads + f.likes)));
            json!({ "curatedContent": files.iter().take(max).map(|f| lightweight(ctx, f)).collect::<Vec<_>>() })
        }
        ("Storefront", "GetFileHeavyWeightData") => {
            let ids: Vec<i64> = params["fileIds"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
            let files: Vec<Value> = ids
                .into_iter()
                .take(100)
                .filter_map(|id| ctx.store.shared_file(id))
                .filter(|f| ctx.can_see(f.xuid))
                .map(|f| heavyweight(ctx, &f))
                .collect();
            json!({ "fileData": files })
        }
        ("Storefront", "GetMyFileList") => {
            let file_type = int(&params["fileType"]);
            let max = params["maxFiles"].as_i64().filter(|&n| n > 0).unwrap_or(100).min(500) as usize;
            let files: Vec<Value> = ctx
                .store
                .shared_files(None, Some(me))
                .into_iter()
                .filter(|f| f.file_type == file_type)
                .take(max)
                .map(|f| {
                    json!({
                        "Id": f.id, "Title": f.title, "Description": f.description, "SubmissionTime": iso_utc(f.created),
                        "IsUnderReview": false, "VersionedContentId": f.versioned_id, "PopularityRating": f.likes.min(5),
                        "DownloadCount": f.downloads, "LikeCount": f.likes, "UseCount": f.uses,
                    })
                })
                .collect();
            json!({ "files": files })
        }
        // Is this content of mine (or a friend's) shared? The share screen asks it.
        ("Storefront", "GetFileDataIfShared") => {
            let (versioned, file_type) = (int(&params["versionedItemId"]), int(&params["fileType"]));
            match ctx
                .store
                .shared_files(None, None)
                .into_iter()
                .find(|f| f.versioned_id == versioned && f.file_type == file_type && ctx.can_see(f.xuid))
            {
                Some(f) => json!({ "found": true, "storefrontData": lightweight(ctx, &f) }),
                None => json!({ "found": false }),
            }
        }

        // ---------------------------------------------------------------- the owner's edits
        ("Storefront", "RemoveFile") => {
            if ctx.store.edit_shared_file(me, int(&params["fileId"]), "remove", "") {
                ctx.event("storefront", "Arquivo removido da Storefront", json!({ "fileId": params["fileId"] }));
            }
            json!({})
        }
        ("Storefront", "ChangeFileTitle") => {
            ctx.store.edit_shared_file(me, int(&params["fileId"]), "title", &clean_text(params["newTitle"].as_str().unwrap_or(""), 16));
            json!({})
        }
        ("Storefront", "ChangeFileDescription") => {
            let text = clean_text(params["newDescription"].as_str().unwrap_or(""), 256);
            ctx.store.edit_shared_file(me, int(&params["fileId"]), "description", &text);
            json!({})
        }
        // Shared files are kept until their owner removes them: nothing to reconcile.
        ("Storefront", "SynchronizeSharedFiles") => json!({}),

        // ---------------------------------------------------------------- counters, follows
        ("Storefront", "IncrementLikes") | ("Storefront", "IncrementUsage") | ("Storefront", "IncrementViews") => {
            let counter = match method {
                "IncrementLikes" => "likes",
                "IncrementUsage" => "uses",
                _ => "views",
            };
            if let Some(f) = ctx.store.shared_file(int(&params["fileId"])).filter(|f| ctx.can_see(f.xuid) && f.xuid != me) {
                ctx.store.count_file(f.id, me, counter);
            }
            json!({})
        }
        ("Storefront", "BatchIncrementViews") => {
            for id in params["fileIds"].as_array().into_iter().flatten().filter_map(Value::as_i64).take(200) {
                if ctx.store.shared_file(id).is_some_and(|f| f.xuid != me && ctx.can_see(f.xuid)) {
                    ctx.store.count_file(id, me, "views");
                }
            }
            json!({})
        }
        // A report is a strike; STRIKES_TO_REMOVE from different real players take the file off
        // the storefront until moderation decides (fh2-server moderation files): it comes back or
        // goes for good. Only players who can see the file (not its owner) count, once each.
        ("Storefront", "ReportContent") => {
            let Some(f) = ctx.store.shared_file(int(&params["fileId"])).filter(|f| f.xuid != me && ctx.can_see(f.xuid)) else {
                return Some(json!({}));
            };
            match ctx.store.report_file(f.id, me) {
                Report::Counted(strikes) => {
                    ctx.event("storefront", format!("Denúncia de \"{}\" ({strikes}/{STRIKES_TO_REMOVE})", f.title), json!({ "fileId": f.id }));
                }
                Report::Removed => {
                    ctx.event("storefront", format!("\"{}\" em revisão após {STRIKES_TO_REMOVE} denúncias", f.title), json!({ "fileId": f.id }));
                    ctx.event_for(
                        f.xuid,
                        "storefront",
                        format!("Seu arquivo \"{}\" saiu da Storefront para revisão da moderação após {STRIKES_TO_REMOVE} denúncias", f.title),
                        json!({ "fileId": f.id }),
                    );
                }
                Report::Ignored => {}
            }
            json!({})
        }
        ("Storefront", "AddFavoriteUser") => {
            ctx.store.set_followed(me, params["favoriteXuid"].as_u64().unwrap_or(0), true);
            json!({})
        }
        ("Storefront", "RemoveFavoriteUser") => {
            ctx.store.set_followed(me, params["notSoFavoriteXuid"].as_u64().unwrap_or(0), false);
            json!({})
        }
        ("Storefront", "ToggleFollowedCreator") => {
            ctx.store.set_followed(me, params["creatorXuid"].as_u64().unwrap_or(0), params["followCreator"].as_bool().unwrap_or(false));
            json!({})
        }

        // ---------------------------------------------------------------- libraries
        ("OnlineProfile", "UpdateLiveryGroup") => {
            ctx.store.save_livery_group(me, &clean_text(params["name"].as_str().unwrap_or(""), 512), params);
            json!({})
        }
        ("OnlineProfile", "DeleteLiveryGroup") => {
            ctx.store.edit_livery_group(me, params["name"].as_str().unwrap_or(""), None);
            json!({})
        }
        ("OnlineProfile", "RenameLiveryGroup") => {
            let to = clean_text(params["newName"].as_str().unwrap_or(""), 512);
            ctx.store.edit_livery_group(me, params["oldName"].as_str().unwrap_or(""), Some(&to));
            json!({})
        }
        ("OnlineProfile", "UpdateCarSetup") => {
            ctx.store.save_car_setup(me, int(&params["carId"]), &clean_text(params["name"].as_str().unwrap_or(""), 512), params);
            json!({})
        }
        ("OnlineProfile", "DeleteCarSetup") => {
            ctx.store.edit_car_setup(me, int(&params["carId"]), params["name"].as_str().unwrap_or(""), None);
            json!({})
        }
        ("OnlineProfile", "RenameCarSetup") => {
            let to = clean_text(params["newName"].as_str().unwrap_or(""), 512);
            ctx.store.edit_car_setup(me, int(&params["carId"]), params["oldName"].as_str().unwrap_or(""), Some(&to));
            json!({})
        }
        // The livery library lives in the save; versioned liveries keep their own copy.
        ("OnlineProfile", "DeleteLivery") | ("OnlineProfile", "RenameLivery") => json!({}),

        // ---------------------------------------------------------------- versioned content
        ("ContentVersioning", "CreateVersionedLayerGroup") => {
            let name = params["name"].as_str().unwrap_or("");
            let Some(group) = ctx.store.livery_group(me, name) else { return Some(error("no such vinyl group")) };
            let id = ctx.store.add_versioned_group(me, name, &group, preview(&params["thumbnail"]));
            json!({ "versionedLayerGroupId": id })
        }
        ("ContentVersioning", "RetrieveVersionedLayerGroupById") => {
            match ctx.store.versioned_group(int(&params["versionedLayerGroupId"])).filter(|g| ctx.can_see(g.0)) {
                Some((owner, name, group, thumbnail)) => json!({ "versionedLayerGroupData": {
                    "name": name,
                    "numberOfLayers": group["numberOfLayers"],
                    "creatorXuid": if group["creatorXuid"].as_u64().unwrap_or(0) != 0 { group["creatorXuid"].clone() } else { json!(owner) },
                    "data": group["data"],
                    "thumbnailData": thumbnail,
                } }),
                None => error("no such vinyl group"),
            }
        }
        // A named setup made versioned (for sharing): a tune with that setup's parts and tuning.
        ("ContentVersioning", "CreateVersionedTune") => {
            let car = int(&params["carId"]);
            let Some(setup) = ctx.store.car_setup(me, car, params["name"].as_str().unwrap_or("")) else {
                return Some(error("no such setup"));
            };
            let tune = json!({
                "carId": car, "installedParts": setup["installedParts"], "tuning": setup["tuning"],
                "binaryVersion": setup["binaryVersion"], "creatorName": setup["creatorName"], "name": setup["name"],
            });
            json!({ "versionedTuneId": ctx.store.add_tune(me, car, &tune) })
        }
        ("ContentVersioning", "RetrieveVersionedTuneById") => {
            match ctx.store.tune(int(&params["versionedTuneId"])).filter(|t| ctx.can_see(t.0)) {
                Some((owner, t)) => json!({ "versionedTuneData": {
                    "CarId": t["carId"],
                    "BinaryVersion": t["binaryVersion"].as_i64().unwrap_or(0),
                    "CreatorName": t["creatorName"].as_str().map(str::to_owned).unwrap_or_else(|| ctx.store.game_tag(owner)),
                    "CreatorXuid": owner,
                    "InstalledParts": t["installedParts"],
                    "Tuning": t["tuning"],
                } }),
                None => error("no such tune"),
            }
        }
        ("ContentVersioning", "AddReferenceToVersionedLivery") => {
            referenced(ctx, "livery", int(&params["versionedLiveryId"]));
            json!({})
        }
        ("ContentVersioning", "AddReferenceToVersionedLayerGroup") => {
            referenced(ctx, "group", int(&params["versionedLayerGroupId"]));
            json!({})
        }
        ("ContentVersioning", "AddReferenceToVersionedTune") => {
            referenced(ctx, "tune", int(&params["versionedTuneId"]));
            json!({})
        }
        // References only matter to the old service's storage: content is kept while shared.
        ("ContentVersioning", "RemoveReferenceToVersionedLivery")
        | ("ContentVersioning", "RemoveReferenceToVersionedLayerGroup")
        | ("ContentVersioning", "RemoveReferenceToVersionedTune") => json!({}),
        _ => return None,
    })
}
