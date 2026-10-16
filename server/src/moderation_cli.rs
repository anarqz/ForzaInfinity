//! `fh2-server moderation ...`: the accounts players reported (db/moderation.rs) and what is
//! done about them. It works on the database directly and is never reachable over HTTP; run it
//! where the database is (the public server: over IAP SSH). Every command prints JSON, for a
//! person or for an agent that reviews the flagged accounts.
//!
//!   fh2-server moderation [--db FILE] [--root DIR] list              flagged accounts, oldest first
//!   fh2-server moderation [--db FILE] [--root DIR] show XUID         reports, records, profile, gates, log
//!   fh2-server moderation [--db FILE] [--root DIR] clear XUID [NOTE] nothing wrong: flag and reports go
//!   fh2-server moderation [--db FILE] [--root DIR] reset XUID [NOTE] cheating: XP, credits, records and
//!                                                                     cars but the first start again
//!                                                                     (db/reset.rs); career and liveries stay
//!   fh2-server moderation ... delete XUID [--yes]   the account and everything that is theirs, for good (backup first)
//!   fh2-server moderation ... season-reset [--yes] [--only XUID] [NOTE]   the same for every account (or one: --only; no drivatar
//!                                                       deleted); without --yes it only counts. It backs
//!                                                       the database up first (<db>.before-season-<ms>)
//!   fh2-server moderation ... check-saves               every account's cloud save against the save
//!                                                       checks (db/saves.rs), read only
//!   fh2-server moderation ... files                     shared files off the storefront after 3 reports
//!   fh2-server moderation ... file ID [--preview-only]  one file: details, reports (--preview-only: its
//!                                                       preview image, base64, and nothing else)
//!   fh2-server moderation ... file-keep ID [NOTE]       nothing wrong: back on the storefront
//!   fh2-server moderation ... file-delete ID [NOTE]     offensive or stolen: gone for good
//!
//! --root is the server's folder; the default database is <root>/data/fh2online.sqlite. --note-base64 B64 gives the NOTE as base64 UTF-8 (no quoting
//! through ssh: infra/moderation.sh).

use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::json;

use crate::db::Store;

const USAGE: &str = "fh2-server moderation [--db FILE] [--root DIR] list
fh2-server moderation [--db FILE] [--root DIR] show XUID
fh2-server moderation [--db FILE] [--root DIR] clear XUID [NOTE]
fh2-server moderation [--db FILE] [--root DIR] reset XUID [NOTE]
fh2-server moderation [--db FILE] [--root DIR] delete XUID [--yes]
fh2-server moderation [--db FILE] [--root DIR] season-reset [--yes] [--only XUID] [NOTE]
fh2-server moderation [--db FILE] [--root DIR] check-saves
fh2-server moderation [--db FILE] [--root DIR] files
fh2-server moderation [--db FILE] [--root DIR] file ID [--preview-only]
fh2-server moderation [--db FILE] [--root DIR] file-keep ID [NOTE]
fh2-server moderation [--db FILE] [--root DIR] file-delete ID [NOTE]";

fn fail(message: &str) -> ! {
    eprintln!("moderation: {message}\n{USAGE}");
    std::process::exit(2);
}

fn print(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap_or_default());
}

pub fn run(args: Vec<String>) {
    let mut root = PathBuf::from(".");
    let mut db = None::<PathBuf>;
    let mut note_given = None::<String>;
    let mut preview_only = false;
    let mut yes = false;
    let mut only = None::<u64>;
    let mut words = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().unwrap_or_else(|| fail(&format!("{name} needs a value")));
        match arg.as_str() {
            "--db" => db = Some(PathBuf::from(value("--db"))),
            "--root" => root = PathBuf::from(value("--root")),
            "--note-base64" => {
                let bytes = B64.decode(value("--note-base64").trim()).unwrap_or_else(|_| fail("--note-base64 is not base64"));
                note_given = Some(String::from_utf8(bytes).unwrap_or_else(|_| fail("the note is not UTF-8")));
            }
            "--preview-only" => preview_only = true,
            "--yes" => yes = true,
            "--only" => only = Some(value("--only").parse().unwrap_or_else(|_| fail("--only XUID"))),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other if other.starts_with("--") => fail(&format!("unknown option {other}")),
            other => words.push(other.to_owned()),
        }
    }
    let db = db.unwrap_or_else(|| root.join("data").join("fh2online.sqlite"));
    // never create a database by mistake: the path must be the server's
    if !db.is_file() {
        fail(&format!("no database at {}", db.display()));
    }
    let store = Store::open(&db).unwrap_or_else(|e| fail(&e));
    let xuid = |w: Option<&String>| -> u64 {
        w.and_then(|x| x.parse::<u64>().ok()).filter(|&x| x != 0).unwrap_or_else(|| fail("XUID: the account's number"))
    };
    let note = |from: usize| note_given.clone().unwrap_or_else(|| words.get(from..).map(|w| w.join(" ")).unwrap_or_default());
    match words.first().map(String::as_str) {
        Some("list") => print(json!({ "flagged": store.flagged_accounts() })),
        Some("show") => match store.moderation_detail(xuid(words.get(1))) {
            Some(d) => print(d),
            None => fail("there is no such account"),
        },
        Some("clear") => {
            let x = xuid(words.get(1));
            if !store.moderation_clear(x, &note(2)) {
                fail("there is no such account");
            }
            print(json!({ "xuid": x.to_string(), "cleared": true }));
        }
        Some("reset") => {
            let x = xuid(words.get(1));
            match store.reset_account(x, crate::db::reset::ResetKind::Moderation, &note(2)) {
                Ok(done) => print(done),
                Err(e) => fail(&e),
            }
        }
        Some("season-reset") => {
            let accounts: Vec<u64> = store.all_accounts().into_iter().filter(|x| only.is_none_or(|o| o == *x)).collect();
            let with_save = accounts.iter().filter(|x| store.save_info(**x).is_some()).count();
            if !yes {
                print(json!({ "dryRun": true, "accounts": accounts.len(), "withCloudSave": with_save,
                              "next": "run again with --yes (the database is backed up first)" }));
                return;
            }
            let backup = db.with_extension(format!("before-season-{}", crate::unix_ms()));
            if let Err(e) = store.backup_to(&backup) {
                fail(&format!("backup failed ({e}); nothing was reset"));
            }
            let note = note(1);
            let (mut done, mut failed) = (0, Vec::new());
            for x in &accounts {
                match store.reset_account(*x, crate::db::reset::ResetKind::Season, &note) {
                    Ok(_) => done += 1,
                    Err(e) => failed.push(json!({ "xuid": x.to_string(), "error": e })),
                }
            }
            print(json!({ "reset": done, "failed": failed, "backup": backup.display().to_string() }));
        }
        Some("delete") => {
            let x = xuid(words.get(1));
            let Some(d) = store.moderation_detail(x) else { fail("there is no such account") };
            if !yes {
                print(json!({ "dryRun": true, "account": d, "next": "run again with --yes (the database is backed up first)" }));
                return;
            }
            let backup = db.with_extension(format!("before-delete-{}", crate::unix_ms()));
            if let Err(e) = store.backup_to(&backup) {
                fail(&format!("backup failed ({e}); nothing was deleted"));
            }
            print(json!({ "xuid": x.to_string(), "deleted": store.delete_user(x), "backup": backup.display().to_string() }));
        }
        Some("check-saves") => {
            let (mut good, mut bad) = (0, Vec::new());
            for x in store.all_accounts() {
                let Some(save) = store.save_data(x) else { continue };
                match crate::db::saves::check_bundle(&save).and_then(|_| crate::db::saves::check_contents(&save)) {
                    Ok(()) => good += 1,
                    Err(e) => bad.push(json!({ "xuid": x.to_string(), "error": e, "bytes": save.len() })),
                }
            }
            print(json!({ "pass": good, "fail": bad }));
        }
        Some("files") => print(json!({ "underReview": store.files_under_review() })),
        Some("file") => {
            let id = words.get(1).and_then(|w| w.parse::<i64>().ok()).unwrap_or_else(|| fail("ID: the file's number"));
            let Some((mut detail, preview)) = store.file_detail(id) else { fail("there is no such file") };
            if preview_only {
                println!("{}", B64.encode(&preview));
                return;
            }
            detail["previewFormat"] = json!(match preview.get(..4) {
                Some([0x89, b'P', b'N', b'G']) => "png",
                Some([0xff, 0xd8, ..]) => "jpeg",
                Some([b'D', b'D', b'S', b' ']) => "dds",
                Some(_) => "unknown",
                None => "none",
            });
            print(detail);
        }
        Some(action @ ("file-keep" | "file-delete")) => {
            let id = words.get(1).and_then(|w| w.parse::<i64>().ok()).unwrap_or_else(|| fail("ID: the file's number"));
            let keep = action == "file-keep";
            if !store.moderate_file(id, keep, &note(2)) {
                fail("there is no such file");
            }
            print(json!({ "id": id, "state": if keep { "shared" } else { "removed by moderation" } }));
        }
        _ => fail("list, show, clear, reset, files, file, file-keep or file-delete"),
    }
}
