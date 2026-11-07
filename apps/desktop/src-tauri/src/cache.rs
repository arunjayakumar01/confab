//! SQLite cache (rusqlite, bundled): spaces, members, messages with an FTS5
//! index kept in sync by triggers, and the outbox for optimistic sends.
//! Every list result and every relay event is upserted here; `search` reads
//! only from here.

use prism_proto::chat::{Attachment, Membership, Message, Space};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS spaces (
  name TEXT PRIMARY KEY,
  display_name TEXT,
  last_active_time TEXT,
  json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS members (
  name TEXT PRIMARY KEY,
  space TEXT NOT NULL,
  json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_space ON members(space);
CREATE TABLE IF NOT EXISTS messages (
  name TEXT PRIMARY KEY,
  space TEXT NOT NULL,
  thread TEXT,
  sender TEXT,
  create_time TEXT,
  last_update_time TEXT,
  text TEXT,
  json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_space_time ON messages(space, create_time DESC);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  text, content='messages', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
  INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TABLE IF NOT EXISTS outbox (
  client_id TEXT PRIMARY KEY,
  space TEXT NOT NULL,
  thread TEXT,
  text TEXT NOT NULL,
  attachments TEXT NOT NULL DEFAULT '[]',
  attempts INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL DEFAULT 'pending',
  error TEXT,
  created_at INTEGER NOT NULL
);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboxState {
    Pending,
    Sending,
    Sent,
    Failed,
}

impl OutboxState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "sending" => Self::Sending,
            "sent" => Self::Sent,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    /// pending → sending → sent | failed; sending → sending (re-driven after a
    /// restart) and failed → sending (manual retry) are allowed; sent is final.
    pub fn can_transition(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Pending, Self::Sending)
                | (Self::Sending, Self::Sending)
                | (Self::Sending, Self::Sent)
                | (Self::Sending, Self::Failed)
                | (Self::Failed, Self::Sending)
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutboxRow {
    pub client_id: String,
    pub space: String,
    pub thread: Option<String>,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub attempts: i64,
    pub state: OutboxState,
    pub error: Option<String>,
}

pub struct Cache {
    conn: Mutex<Connection>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("cache: {e}")
}

/// Turn free text into an FTS5 query: each word quoted, prefix-matched, ANDed.
pub fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let conn = Connection::open(path).map_err(err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(err)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(err)?;
        Self::init(conn)
    }

    /// The cache belongs to one Google account. A different user signing in on the
    /// same profile (the twin in the morning, the real org in the afternoon) starts
    /// from an empty cache instead of seeing and subscribing to the other's spaces.
    /// Returns true when the cache was wiped.
    pub fn ensure_owner(&self, user: &str) -> Result<bool, String> {
        let conn = self.lock();
        let owner: Option<String> = conn
            .query_row("SELECT v FROM meta WHERE k = 'owner'", [], |r| r.get(0))
            .optional()
            .map_err(err)?;
        if owner.as_deref() == Some(user) {
            return Ok(false);
        }
        conn.execute_batch(
            "DELETE FROM messages; DELETE FROM members; DELETE FROM spaces; DELETE FROM outbox;",
        )
        .map_err(err)?;
        conn.execute(
            "INSERT INTO meta (k, v) VALUES ('owner', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![user],
        )
        .map_err(err)?;
        Ok(true)
    }

    pub fn in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ----- spaces -----

    pub fn upsert_spaces(&self, spaces: &[Space]) -> Result<(), String> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(err)?;
        for s in spaces {
            let json = serde_json::to_string(s).map_err(err)?;
            tx.execute(
                "INSERT INTO spaces(name, display_name, last_active_time, json) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET display_name=excluded.display_name,
                 last_active_time=excluded.last_active_time, json=excluded.json",
                params![s.name, s.display_name, s.last_active_time, json],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    pub fn list_spaces(&self) -> Result<Vec<Space>, String> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT json FROM spaces ORDER BY last_active_time DESC, display_name")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .filter_map(|j| serde_json::from_str(&j).ok())
            .collect();
        Ok(rows)
    }

    pub fn space_names(&self) -> Result<Vec<String>, String> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT name FROM spaces ORDER BY name")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ----- members -----

    pub fn upsert_members(&self, space: &str, members: &[Membership]) -> Result<(), String> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(err)?;
        for m in members {
            let json = serde_json::to_string(m).map_err(err)?;
            tx.execute(
                "INSERT INTO members(name, space, json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(name) DO UPDATE SET space=excluded.space, json=excluded.json",
                params![m.name, space, json],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    pub fn delete_member(&self, name: &str) -> Result<(), String> {
        self.lock()
            .execute("DELETE FROM members WHERE name = ?1", params![name])
            .map(|_| ())
            .map_err(err)
    }

    pub fn list_members(&self, space: &str) -> Result<Vec<Membership>, String> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT json FROM members WHERE space = ?1 ORDER BY name")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![space], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .filter_map(|j| serde_json::from_str(&j).ok())
            .collect();
        Ok(rows)
    }

    // ----- messages -----

    fn upsert_message_in(tx: &Connection, m: &Message) -> Result<(), String> {
        if m.delete_time.is_some() {
            tx.execute("DELETE FROM messages WHERE name = ?1", params![m.name])
                .map_err(err)?;
            return Ok(());
        }
        let json = serde_json::to_string(m).map_err(err)?;
        let thread = m.thread.as_ref().and_then(|t| t.name.clone());
        let sender = m.sender.as_ref().map(|u| u.name.clone());
        tx.execute(
            "INSERT INTO messages(name, space, thread, sender, create_time, last_update_time, text, json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(name) DO UPDATE SET space=excluded.space, thread=excluded.thread,
             sender=excluded.sender, create_time=excluded.create_time,
             last_update_time=excluded.last_update_time, text=excluded.text, json=excluded.json",
            params![m.name, m.space_name(), thread, sender, m.create_time, m.last_update_time, m.text, json],
        )
        .map_err(err)?;
        Ok(())
    }

    /// Upsert; a message carrying `deleteTime` is removed instead.
    pub fn upsert_messages(&self, messages: &[Message]) -> Result<(), String> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(err)?;
        for m in messages {
            Self::upsert_message_in(&tx, m)?;
        }
        tx.commit().map_err(err)
    }

    pub fn upsert_message(&self, m: &Message) -> Result<(), String> {
        Self::upsert_message_in(&self.lock(), m)
    }

    pub fn delete_message(&self, name: &str) -> Result<(), String> {
        self.lock()
            .execute("DELETE FROM messages WHERE name = ?1", params![name])
            .map(|_| ())
            .map_err(err)
    }

    pub fn get_message(&self, name: &str) -> Result<Option<Message>, String> {
        let conn = self.lock();
        let json: Option<String> = conn
            .query_row(
                "SELECT json FROM messages WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Up to 50 matches, newest first. Empty query → empty result.
    pub fn search(&self, query: &str) -> Result<Vec<Message>, String> {
        let q = fts_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT m.json FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
                 WHERE messages_fts MATCH ?1 ORDER BY m.create_time DESC LIMIT 50",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![q], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .filter_map(|j| serde_json::from_str(&j).ok())
            .collect();
        Ok(rows)
    }

    // ----- outbox -----

    pub fn outbox_insert(&self, row: &OutboxRow) -> Result<(), String> {
        let attachments = serde_json::to_string(&row.attachments).map_err(err)?;
        self.lock()
            .execute(
                "INSERT INTO outbox(client_id, space, thread, text, attachments, attempts, state, error, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.client_id,
                    row.space,
                    row.thread,
                    row.text,
                    attachments,
                    row.attempts,
                    row.state.as_str(),
                    row.error,
                    chrono::Utc::now().timestamp()
                ],
            )
            .map(|_| ())
            .map_err(err)
    }

    fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxRow> {
        let attachments: String = r.get(4)?;
        let state: String = r.get(6)?;
        Ok(OutboxRow {
            client_id: r.get(0)?,
            space: r.get(1)?,
            thread: r.get(2)?,
            text: r.get(3)?,
            attachments: serde_json::from_str(&attachments).unwrap_or_default(),
            attempts: r.get(5)?,
            state: OutboxState::parse(&state),
            error: r.get(7)?,
        })
    }

    const OUTBOX_COLS: &'static str =
        "client_id, space, thread, text, attachments, attempts, state, error";

    pub fn outbox_get(&self, client_id: &str) -> Result<Option<OutboxRow>, String> {
        let conn = self.lock();
        conn.query_row(
            &format!(
                "SELECT {} FROM outbox WHERE client_id = ?1",
                Self::OUTBOX_COLS
            ),
            params![client_id],
            Self::row_from,
        )
        .optional()
        .map_err(err)
    }

    /// Rows still owed to Google (pending or stuck in sending), oldest first.
    pub fn outbox_unsent(&self) -> Result<Vec<OutboxRow>, String> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM outbox WHERE state IN ('pending', 'sending') ORDER BY created_at",
                Self::OUTBOX_COLS
            ))
            .map_err(err)?;
        let rows = stmt
            .query_map([], Self::row_from)
            .map_err(err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Move a row to `to`, enforcing the state machine; entering `sending`
    /// counts an attempt. Returns the updated row.
    pub fn outbox_transition(
        &self,
        client_id: &str,
        to: OutboxState,
        error: Option<&str>,
    ) -> Result<OutboxRow, String> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(err)?;
        let current = tx
            .query_row(
                &format!(
                    "SELECT {} FROM outbox WHERE client_id = ?1",
                    Self::OUTBOX_COLS
                ),
                params![client_id],
                Self::row_from,
            )
            .optional()
            .map_err(err)?
            .ok_or_else(|| format!("cache: no outbox row {client_id}"))?;
        if !current.state.can_transition(to) {
            return Err(format!(
                "cache: outbox {client_id} cannot go {} → {}",
                current.state.as_str(),
                to.as_str()
            ));
        }
        let attempts = if to == OutboxState::Sending {
            current.attempts + 1
        } else {
            current.attempts
        };
        tx.execute(
            "UPDATE outbox SET state = ?2, attempts = ?3, error = ?4 WHERE client_id = ?1",
            params![client_id, to.as_str(), attempts, error],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(OutboxRow {
            state: to,
            attempts,
            error: error.map(str::to_string),
            ..current
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_different_owner_wipes_the_cache() {
        let c = Cache::in_memory().unwrap();
        assert!(
            c.ensure_owner("users/1").unwrap(),
            "an unowned cache is reset once"
        );
        c.upsert_spaces(&[prism_proto::chat::Space {
            name: "spaces/A".into(),
            ..Default::default()
        }])
        .unwrap();
        assert!(!c.ensure_owner("users/1").unwrap());
        assert_eq!(c.space_names().unwrap(), vec!["spaces/A".to_string()]);
        assert!(c.ensure_owner("users/2").unwrap(), "new owner wipes");
        assert!(c.space_names().unwrap().is_empty());
    }
    use prism_proto::chat::{Thread, User};

    fn msg(name: &str, text: &str, t: &str) -> Message {
        Message {
            name: name.into(),
            sender: Some(User {
                name: "users/1".into(),
                ..Default::default()
            }),
            create_time: Some(t.into()),
            text: Some(text.into()),
            thread: Some(Thread {
                name: Some("spaces/A/threads/T".into()),
                thread_key: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_and_fts_search() {
        let c = Cache::in_memory().unwrap();
        c.upsert_messages(&[
            msg(
                "spaces/A/messages/1",
                "the quarterly budget review",
                "2026-11-06T10:00:00.000Z",
            ),
            msg(
                "spaces/A/messages/2",
                "lunch at noon?",
                "2026-11-06T11:00:00.000Z",
            ),
            msg(
                "spaces/B/messages/3",
                "Budget is approved, café later",
                "2026-11-06T12:00:00.000Z",
            ),
        ])
        .unwrap();
        let hits = c.search("budget").unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "spaces/B/messages/3", "newest first");
        assert_eq!(c.search("cafe").unwrap().len(), 1, "diacritics folded");
        assert_eq!(c.search("quart").unwrap().len(), 1, "prefix match");
        assert!(c.search("").unwrap().is_empty());
        assert!(
            c.search("\"unbalanced").unwrap().is_empty(),
            "quotes are escaped, not a syntax error"
        );

        // Update keeps the index in sync.
        c.upsert_message(&msg(
            "spaces/A/messages/2",
            "dinner at eight?",
            "2026-11-06T11:00:00.000Z",
        ))
        .unwrap();
        assert!(c.search("lunch").unwrap().is_empty());
        assert_eq!(c.search("dinner").unwrap().len(), 1);
        assert_eq!(
            c.get_message("spaces/A/messages/2")
                .unwrap()
                .unwrap()
                .text
                .as_deref(),
            Some("dinner at eight?")
        );

        // Delete, and a deleteTime-bearing upsert, both remove.
        c.delete_message("spaces/A/messages/1").unwrap();
        assert_eq!(c.search("budget").unwrap().len(), 1);
        let mut gone = msg("spaces/B/messages/3", "x", "2026-11-06T12:00:00.000Z");
        gone.delete_time = Some("2026-11-06T13:00:00.000Z".into());
        c.upsert_message(&gone).unwrap();
        assert!(c.search("budget").unwrap().is_empty());
        assert!(c.get_message("spaces/B/messages/3").unwrap().is_none());
    }

    #[test]
    fn huddle_annotations_survive_the_cache() {
        use prism_proto::chat::Annotation;
        let c = Cache::in_memory().unwrap();
        let mut huddle = msg("spaces/A/messages/H", "", "2026-11-07T10:00:00.000Z");
        huddle.text = None;
        huddle.annotations = vec![Annotation::huddle(
            "https://meet.google.com/kyf-vogr-zmm",
            "kyf-vogr-zmm",
            "STARTED",
        )];
        c.upsert_message(&huddle).unwrap();
        let back = c.get_message("spaces/A/messages/H").unwrap().unwrap();
        assert_eq!(back, huddle);
        assert_eq!(
            back.annotations[0].huddle_link(),
            Some(("kyf-vogr-zmm", "STARTED"))
        );
        // The ended update replaces it in place.
        huddle.annotations = vec![Annotation::huddle(
            "https://meet.google.com/kyf-vogr-zmm",
            "kyf-vogr-zmm",
            "ENDED",
        )];
        huddle.last_update_time = Some("2026-11-07T10:20:00.000Z".into());
        c.upsert_message(&huddle).unwrap();
        let back = c.get_message("spaces/A/messages/H").unwrap().unwrap();
        assert_eq!(
            back.annotations[0].huddle_link(),
            Some(("kyf-vogr-zmm", "ENDED"))
        );
        assert_eq!(
            back.last_update_time.as_deref(),
            Some("2026-11-07T10:20:00.000Z")
        );
    }

    #[test]
    fn spaces_and_members_round_trip() {
        let c = Cache::in_memory().unwrap();
        c.upsert_spaces(&[
            Space {
                name: "spaces/A".into(),
                display_name: Some("Alpha".into()),
                ..Default::default()
            },
            Space {
                name: "spaces/B".into(),
                display_name: Some("Beta".into()),
                ..Default::default()
            },
        ])
        .unwrap();
        c.upsert_spaces(&[Space {
            name: "spaces/A".into(),
            display_name: Some("Alpha 2".into()),
            ..Default::default()
        }])
        .unwrap();
        assert_eq!(c.space_names().unwrap(), vec!["spaces/A", "spaces/B"]);
        let spaces = c.list_spaces().unwrap();
        assert_eq!(
            spaces
                .iter()
                .find(|s| s.name == "spaces/A")
                .unwrap()
                .display_name
                .as_deref(),
            Some("Alpha 2")
        );
        c.upsert_members(
            "spaces/A",
            &[Membership {
                name: "spaces/A/members/1".into(),
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(c.list_members("spaces/A").unwrap().len(), 1);
        c.delete_member("spaces/A/members/1").unwrap();
        assert!(c.list_members("spaces/A").unwrap().is_empty());
    }

    #[test]
    fn outbox_state_transitions() {
        let c = Cache::in_memory().unwrap();
        let row = OutboxRow {
            client_id: "c1".into(),
            space: "spaces/A".into(),
            thread: None,
            text: "hi".into(),
            attachments: vec![],
            attempts: 0,
            state: OutboxState::Pending,
            error: None,
        };
        c.outbox_insert(&row).unwrap();
        assert_eq!(c.outbox_unsent().unwrap().len(), 1);
        assert!(
            c.outbox_transition("c1", OutboxState::Sent, None).is_err(),
            "pending cannot skip to sent"
        );
        let r = c
            .outbox_transition("c1", OutboxState::Sending, None)
            .unwrap();
        assert_eq!((r.state, r.attempts), (OutboxState::Sending, 1));
        let r = c
            .outbox_transition("c1", OutboxState::Failed, Some("boom"))
            .unwrap();
        assert_eq!(r.error.as_deref(), Some("boom"));
        assert!(
            c.outbox_unsent().unwrap().is_empty(),
            "failed rows are not auto-retried"
        );
        let r = c
            .outbox_transition("c1", OutboxState::Sending, None)
            .unwrap();
        assert_eq!(r.attempts, 2, "manual retry counts an attempt");
        c.outbox_transition("c1", OutboxState::Sent, None).unwrap();
        assert!(
            c.outbox_transition("c1", OutboxState::Sending, None)
                .is_err(),
            "sent is final"
        );
        assert_eq!(
            c.outbox_get("c1").unwrap().unwrap().state,
            OutboxState::Sent
        );
        assert!(c.outbox_get("nope").unwrap().is_none());
        assert!(c.outbox_insert(&row).is_err(), "client ids are unique");
    }
}
