//! `fh2-server news ...`: admin news into the in-game Message Center (db/news.rs). It writes the
//! database directly and is never reachable over HTTP; run it where the database is (for the
//! public server: infra/news.sh, over IAP SSH).
//!
//!   fh2-server news [--db FILE] send (--text TEXT | --text-base64 B64) [--to all | --to Name#12345] [--dry-run]
//!   fh2-server news [--db FILE] list
//!   fh2-server news [--db FILE] delete ID

use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::STANDARD as B64};

use crate::db::{Store, news};

const USAGE: &str = "fh2-server news [--db FILE] send (--text TEXT | --text-base64 B64) [--to all | --to Name#12345] [--dry-run]
fh2-server news [--db FILE] list
fh2-server news [--db FILE] delete ID";

fn fail(message: &str) -> ! {
    eprintln!("news: {message}\n{USAGE}");
    std::process::exit(2);
}

pub fn run(args: Vec<String>) {
    let mut db = PathBuf::from("data").join("fh2online.sqlite");
    let (mut text, mut to, mut dry_run) = (None::<String>, None::<String>, false);
    let mut words = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().unwrap_or_else(|| fail(&format!("{name} needs a value")));
        match arg.as_str() {
            "--db" => db = PathBuf::from(value("--db")),
            "--text" => text = Some(value("--text")),
            "--text-base64" => {
                let bytes = B64.decode(value("--text-base64").trim()).unwrap_or_else(|_| fail("--text-base64 is not base64"));
                text = Some(String::from_utf8(bytes).unwrap_or_else(|_| fail("the text is not UTF-8")));
            }
            "--to" => to = Some(value("--to")),
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other if other.starts_with("--") => fail(&format!("unknown option {other}")),
            other => words.push(other.to_owned()),
        }
    }
    // never create a database by mistake: the path must be the server's
    if !db.is_file() {
        fail(&format!("no database at {}", db.display()));
    }
    let store = Store::open(&db).unwrap_or_else(|e| fail(&e));
    match words.first().map(String::as_str) {
        Some("send") => {
            let text = news::clean(text.as_deref().unwrap_or(""));
            if text.is_empty() {
                fail("the text is empty");
            }
            let target = to.as_deref().filter(|t| *t != "all");
            if target.is_some_and(|t| !t.contains('#')) {
                fail("--to takes all or Name#12345");
            }
            let recipients = store.news_recipients(target);
            if recipients.is_empty() {
                fail(match target {
                    Some(_) => "no such player (or banned)",
                    None => "no players",
                });
            }
            println!("{} character(s) to {} player(s):\n---\n{text}\n---", text.chars().count(), recipients.len());
            if dry_run {
                println!("dry run: nothing sent");
                return;
            }
            match store.send_news(&text, &recipients) {
                Some(id) => println!("sent: news #{id} (players see it on their next Message Center poll, within 5 minutes)"),
                None => fail("could not write the database"),
            }
        }
        Some("list") => {
            let items = store.news_list(50);
            if items.is_empty() {
                println!("no news sent yet");
            }
            for n in items {
                let first: String = n.text.lines().next().unwrap_or("").chars().take(70).collect();
                println!("#{:<4} {}  to {:>4}, {:>4} still in Message Centers  {first}", n.id, crate::forza::iso_utc(n.created), n.recipients, n.live);
            }
        }
        Some("delete") => {
            let id: i64 = words.get(1).and_then(|w| w.parse().ok()).unwrap_or_else(|| fail("delete needs a news id"));
            match store.delete_news(id) {
                Some(n) => println!("news #{id} deleted: {n} notification(s) removed"),
                None => fail(&format!("no news #{id}")),
            }
        }
        _ => fail("say send, list or delete"),
    }
}
