//! Admin news: a message from ForzaInfinity in every player's in-game Message Center, the same
//! notification (Message, 0x0200) the server sends when players become friends.
//!
//! There is no HTTP route for it on purpose: news are written straight into the database by
//! `fh2-server news ...` (see main.rs), run on the server's machine. For the public server,
//! infra/news.sh runs it on the VM over IAP SSH, so only someone with access to the GCP project
//! can send one; nothing secret lives in the repository.
//!
//! Each send is kept in `news` (id, text, time, recipients); its notifications carry the same
//! `newsId` in their parameters (ignored when the game's struct is encoded), so a send can be
//! listed and taken back.

use rusqlite::params;
use serde_json::{Value, json};

use super::Store;
use super::notifications::MESSAGE;
use crate::unix_ms;

pub const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS news (
        id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL, created INTEGER NOT NULL,
        recipients INTEGER NOT NULL);";

/// The game's Message field holds 512 characters.
pub const MAX_CHARS: usize = 512;

/// News text as the game shows it: no control or formatting characters except line breaks
/// ("\n" typed as two characters also counts), at most MAX_CHARS characters.
pub fn clean(text: &str) -> String {
    let text = text.replace("\\n", "\n").replace("\r\n", "\n");
    let lines: Vec<String> = text.split('\n').map(|l| crate::forza::clean_text(l, MAX_CHARS)).collect();
    let joined = lines.join("\n");
    joined.trim_matches('\n').chars().take(MAX_CHARS).collect()
}

pub struct NewsItem {
    pub id: i64,
    pub text: String,
    pub created: i64,
    pub recipients: i64,
    /// notifications of it still in the database (not cleared by the player)
    pub live: i64,
}

impl Store {
    /// The players a send goes to: every account not banned, or the one named `gamertag`
    /// ("Name#12345").
    pub fn news_recipients(&self, gamertag: Option<&str>) -> Vec<u64> {
        let (sql, arg) = match gamertag.and_then(|t| t.split_once('#')) {
            Some((name, discriminator)) => (
                "SELECT xuid FROM accounts WHERE banned = 0 AND name = ?1 COLLATE NOCASE AND discriminator = ?2",
                Some((name.to_owned(), discriminator.to_owned())),
            ),
            None => ("SELECT xuid FROM accounts WHERE banned = 0 ORDER BY xuid", None),
        };
        self.with(|db| {
            let mut stmt = db.prepare(sql)?;
            let map = |r: &rusqlite::Row| r.get::<_, i64>(0);
            let found: Vec<i64> = match &arg {
                Some((n, d)) => stmt.query_map(params![n, d], map)?.flatten().collect(),
                None => stmt.query_map([], map)?.flatten().collect(),
            };
            Ok(found.into_iter().map(|x| x as u64).collect::<Vec<_>>())
        })
        .unwrap_or_default()
    }

    /// Sends `text` (already cleaned) to `recipients` in one transaction. Returns the news id.
    pub fn send_news(&self, text: &str, recipients: &[u64]) -> Option<i64> {
        let now = unix_ms();
        let db = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let run = || -> rusqlite::Result<i64> {
            db.execute_batch("BEGIN IMMEDIATE")?;
            db.execute("INSERT INTO news (text, created, recipients) VALUES (?1, ?2, ?3)", params![text, now, recipients.len() as i64])?;
            let id = db.last_insert_rowid();
            let message: Value = json!({ "Message": text, "SRAUri": "", "newsId": id });
            let message = message.to_string();
            let mut insert =
                db.prepare("INSERT INTO notifications (xuid, type, struct, params, sent) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for &xuid in recipients {
                insert.execute(params![xuid as i64, MESSAGE, "MessageNotificationParameters", message, now])?;
            }
            drop(insert);
            db.execute_batch("COMMIT")?;
            Ok(id)
        };
        match run() {
            Ok(id) => Some(id),
            Err(e) => {
                let _ = db.execute_batch("ROLLBACK");
                eprintln!("news: {e}");
                None
            }
        }
    }

    /// Past sends, newest first.
    pub fn news_list(&self, limit: i64) -> Vec<NewsItem> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT n.id, n.text, n.created, n.recipients,
                        (SELECT COUNT(*) FROM notifications WHERE type = ?2 AND json_extract(params, '$.newsId') = n.id)
                 FROM news n ORDER BY n.id DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit, MESSAGE], |r| {
                    Ok(NewsItem { id: r.get(0)?, text: r.get(1)?, created: r.get(2)?, recipients: r.get(3)?, live: r.get(4)? })
                })?
                .flatten()
                .collect::<Vec<_>>();
            Ok(rows)
        })
        .unwrap_or_default()
    }

    /// Takes a send back: its notifications leave every Message Center (on the game's next poll).
    /// Returns how many notifications were removed, None for an unknown id.
    pub fn delete_news(&self, id: i64) -> Option<usize> {
        self.with(|db| {
            if db.execute("DELETE FROM news WHERE id = ?1", [id])? == 0 {
                return Ok(None);
            }
            Ok(Some(db.execute(
                "DELETE FROM notifications WHERE type = ?1 AND json_extract(params, '$.newsId') = ?2",
                params![MESSAGE, id],
            )?))
        })
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_list_delete() {
        let dir = std::env::temp_dir().join(format!("fh2-news-{}", crate::db::accounts::new_token()));
        let store = Store::open(&dir.join("t.sqlite")).unwrap();
        let pc = |n: u8| format!("{n:02x}").repeat(32);
        let [a, b, c] = [1u8, 2, 3].map(|n| store.register_account(&format!("P{n}"), "00001", &pc(n), crate::db::accounts::MODE_HARDCORE).unwrap().0.xuid);
        store.set_banned(c, true);
        assert_eq!(store.news_recipients(None), vec![a, b]);
        assert_eq!(store.news_recipients(Some("p2#00001")), vec![b]);
        assert!(store.news_recipients(Some("P3#00001")).is_empty());
        let text = clean("  Welcome\\nto  the\u{202E} server  ");
        assert_eq!(text, "Welcome\nto the server");
        let id = store.send_news(&text, &[a, b]).unwrap();
        let mine = store.notifications(a, 50);
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].kind, MESSAGE);
        assert_eq!(mine[0].params["Message"], text);
        let list = store.news_list(10);
        assert_eq!((list[0].id, list[0].recipients, list[0].live), (id, 2, 2));
        assert_eq!(store.delete_news(id), Some(2));
        assert!(store.notifications(a, 50).is_empty());
        assert_eq!(store.delete_news(id), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
