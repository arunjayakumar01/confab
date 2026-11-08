//! SQLite behind one connection and a mutex. Every call is a single short
//! statement, so blocking the async worker for it is acceptable.

use std::sync::Mutex;

use anyhow::Context;
use confab_proto::relay::CustomStatus;
use rusqlite::{params, Connection, OptionalExtension};

// note: one connection behind a std mutex, held across no await; a pool when writes contend
pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DraftRow {
    pub space: String,
    pub thread: Option<String>,
    pub device_id: String,
    pub text: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionRow {
    pub user: String,
    pub name: String,
    pub expire_time: String,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS receipts (
  user TEXT NOT NULL, space TEXT NOT NULL, thread TEXT NOT NULL DEFAULT '',
  message TEXT NOT NULL, at TEXT NOT NULL,
  PRIMARY KEY (user, space, thread));
CREATE TABLE IF NOT EXISTS drafts (
  user TEXT NOT NULL, space TEXT NOT NULL, thread TEXT NOT NULL DEFAULT '',
  device_id TEXT NOT NULL, text TEXT NOT NULL, at TEXT NOT NULL,
  PRIMARY KEY (user, space, thread, device_id));
CREATE TABLE IF NOT EXISTS subscriptions (
  user TEXT PRIMARY KEY, name TEXT NOT NULL, expire_time TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS custom_status (
  user TEXT PRIMARY KEY, emoji TEXT, text TEXT NOT NULL, expires_at TEXT, updated_at TEXT NOT NULL);
";

fn thread_key(thread: Option<&str>) -> &str {
    thread.unwrap_or("")
}

fn thread_opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

impl Db {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open sqlite {path}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        if path != ":memory:" {
            // WAL is per-file; an in-memory db has nothing to journal.
            let _ = conn.pragma_update(None, "journal_mode", "WAL");
        }
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn upsert_receipt(
        &self,
        user: &str,
        space: &str,
        thread: Option<&str>,
        message: &str,
        at: &str,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO receipts (user, space, thread, message, at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user, space, thread) DO UPDATE SET message = excluded.message, at = excluded.at",
            params![user, space, thread_key(thread), message, at],
        )?;
        Ok(())
    }

    pub fn receipt(
        &self,
        user: &str,
        space: &str,
        thread: Option<&str>,
    ) -> anyhow::Result<Option<(String, String)>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT message, at FROM receipts WHERE user = ?1 AND space = ?2 AND thread = ?3",
                params![user, space, thread_key(thread)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Empty text clears the draft.
    pub fn upsert_draft(
        &self,
        user: &str,
        space: &str,
        thread: Option<&str>,
        device_id: &str,
        text: &str,
        at: &str,
    ) -> anyhow::Result<()> {
        let conn = self.lock();
        if text.is_empty() {
            conn.execute(
                "DELETE FROM drafts WHERE user = ?1 AND space = ?2 AND thread = ?3 AND device_id = ?4",
                params![user, space, thread_key(thread), device_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO drafts (user, space, thread, device_id, text, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(user, space, thread, device_id) DO UPDATE SET text = excluded.text, at = excluded.at",
                params![user, space, thread_key(thread), device_id, text, at],
            )?;
        }
        Ok(())
    }

    pub fn drafts_for(&self, user: &str, spaces: &[String]) -> anyhow::Result<Vec<DraftRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT space, thread, device_id, text, at FROM drafts WHERE user = ?1 AND space = ?2 ORDER BY at",
        )?;
        let mut out = Vec::new();
        for space in spaces {
            let rows = stmt.query_map(params![user, space], |r| {
                Ok(DraftRow {
                    space: r.get(0)?,
                    thread: thread_opt(r.get(1)?),
                    device_id: r.get(2)?,
                    text: r.get(3)?,
                    at: r.get(4)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn set_custom_status(
        &self,
        user: &str,
        status: Option<&CustomStatus>,
        at: &str,
    ) -> anyhow::Result<()> {
        let conn = self.lock();
        match status {
            None => {
                conn.execute("DELETE FROM custom_status WHERE user = ?1", params![user])?;
            }
            Some(s) => {
                conn.execute(
                    "INSERT INTO custom_status (user, emoji, text, expires_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(user) DO UPDATE SET emoji = excluded.emoji, text = excluded.text,
                       expires_at = excluded.expires_at, updated_at = excluded.updated_at",
                    params![user, s.emoji, s.text, s.expires_at, at],
                )?;
            }
        }
        Ok(())
    }

    pub fn custom_status(&self, user: &str) -> anyhow::Result<Option<CustomStatus>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT emoji, text, expires_at FROM custom_status WHERE user = ?1",
                params![user],
                |r| {
                    Ok(CustomStatus {
                        emoji: r.get(0)?,
                        text: r.get(1)?,
                        expires_at: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn subscription(&self, user: &str) -> anyhow::Result<Option<SubscriptionRow>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT user, name, expire_time FROM subscriptions WHERE user = ?1",
                params![user],
                |r| {
                    Ok(SubscriptionRow {
                        user: r.get(0)?,
                        name: r.get(1)?,
                        expire_time: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn upsert_subscription(
        &self,
        user: &str,
        name: &str,
        expire_time: &str,
        at: &str,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO subscriptions (user, name, expire_time, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user) DO UPDATE SET name = excluded.name, expire_time = excluded.expire_time,
               updated_at = excluded.updated_at",
            params![user, name, expire_time, at],
        )?;
        Ok(())
    }

    pub fn delete_subscription(&self, user: &str) -> anyhow::Result<()> {
        self.lock()
            .execute("DELETE FROM subscriptions WHERE user = ?1", params![user])?;
        Ok(())
    }

    pub fn all_subscriptions(&self) -> anyhow::Result<Vec<SubscriptionRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT user, name, expire_time FROM subscriptions")?;
        let rows = stmt.query_map([], |r| {
            Ok(SubscriptionRow {
                user: r.get(0)?,
                name: r.get(1)?,
                expire_time: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_and_drafts_round_trip() {
        let db = Db::open(":memory:").unwrap();
        db.upsert_receipt("users/1", "spaces/A", None, "spaces/A/messages/1", "t1")
            .unwrap();
        db.upsert_receipt("users/1", "spaces/A", None, "spaces/A/messages/2", "t2")
            .unwrap();
        db.upsert_receipt(
            "users/1",
            "spaces/A",
            Some("spaces/A/threads/T"),
            "spaces/A/messages/3",
            "t3",
        )
        .unwrap();
        assert_eq!(
            db.receipt("users/1", "spaces/A", None).unwrap().unwrap().0,
            "spaces/A/messages/2"
        );
        assert_eq!(
            db.receipt("users/1", "spaces/A", Some("spaces/A/threads/T"))
                .unwrap()
                .unwrap()
                .0,
            "spaces/A/messages/3"
        );

        db.upsert_draft("users/1", "spaces/A", None, "d1", "hello", "t1")
            .unwrap();
        db.upsert_draft(
            "users/1",
            "spaces/A",
            Some("spaces/A/threads/T"),
            "d1",
            "in thread",
            "t2",
        )
        .unwrap();
        db.upsert_draft("users/1", "spaces/B", None, "d2", "other space", "t3")
            .unwrap();
        db.upsert_draft("users/2", "spaces/A", None, "d9", "someone else", "t4")
            .unwrap();
        let rows = db.drafts_for("users/1", &["spaces/A".into()]).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.device_id == "d1"));
        db.upsert_draft("users/1", "spaces/A", None, "d1", "", "t5")
            .unwrap();
        let rows = db
            .drafts_for("users/1", &["spaces/A".into(), "spaces/B".into()])
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.text == "other space"));
    }

    #[test]
    fn custom_status_and_subscriptions() {
        let db = Db::open(":memory:").unwrap();
        assert!(db.custom_status("users/1").unwrap().is_none());
        let s = CustomStatus {
            emoji: Some("🌴".into()),
            text: "away".into(),
            expires_at: None,
        };
        db.set_custom_status("users/1", Some(&s), "t").unwrap();
        assert_eq!(db.custom_status("users/1").unwrap(), Some(s));
        db.set_custom_status("users/1", None, "t").unwrap();
        assert!(db.custom_status("users/1").unwrap().is_none());

        db.upsert_subscription("users/1", "subscriptions/x", "2026-11-06T12:00:00Z", "t")
            .unwrap();
        db.upsert_subscription("users/1", "subscriptions/y", "2026-11-06T13:00:00Z", "t")
            .unwrap();
        assert_eq!(
            db.subscription("users/1").unwrap().unwrap().name,
            "subscriptions/y"
        );
        assert_eq!(db.all_subscriptions().unwrap().len(), 1);
        db.delete_subscription("users/1").unwrap();
        assert!(db.subscription("users/1").unwrap().is_none());
    }
}
