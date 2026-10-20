// Copyright (C) 2026 James Carthew
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Application-local SQLite state (contact cache, settings). Message storage
//! stays with the Matrix SDK. All access goes through one `Database` handle
//! guarded by a mutex (single writer).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::calendar::Response;
use thiserror::Error;

use super::migrations::MIGRATIONS;
use crate::contacts::contact::{Contact, Friendship};
use crate::contacts::synchronization::ContactsDocument;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database is from a newer version (schema {found}, supported {supported})")]
    TooNew { found: usize, supported: usize },
}

/// An invitation we have answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEntry {
    pub event_id: String,
    pub room_id: String,
    pub sender: String,
    pub title: String,
    pub start_ms: u64,
    pub description: String,
    pub response: Response,
}

/// Event IDs of entries the user made themselves start with this; Matrix event IDs start with `$`.
pub const PERSONAL_PREFIX: &str = "local:";

/// An organiser's change to an invitation, as recorded from their messages.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CalendarChange {
    pub cancelled: bool,
    /// New title/time/details, if the organiser rescheduled.
    pub update: Option<(String, u64, String)>,
}

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY);",
        )?;
        let current: usize = conn.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        if current > MIGRATIONS.len() {
            return Err(DbError::TooNew {
                found: current,
                supported: MIGRATIONS.len(),
            });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                [i + 1],
            )?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn schema_version(&self) -> Result<usize, DbError> {
        let c = self.lock();
        Ok(c.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-query; the
        // connection itself is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Atomically replace the cached contacts document.
    pub fn save_contacts(&self, doc: &ContactsDocument) -> Result<(), DbError> {
        let mut c = self.lock();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM contacts", [])?;
        tx.execute("DELETE FROM contact_groups", [])?;
        for (i, g) in doc.groups.iter().enumerate() {
            tx.execute(
                "INSERT INTO contact_groups (position, name) VALUES (?1, ?2)",
                params![i as i64, g],
            )?;
        }
        for ct in doc.contacts.values() {
            tx.execute(
                "INSERT INTO contacts (mxid, nickname, grp, favorite, updated_at, deleted, friendship) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![ct.mxid, ct.nickname, ct.group, ct.favorite, ct.updated_at as i64, ct.deleted, ct.friendship.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_contacts(&self) -> Result<ContactsDocument, DbError> {
        let c = self.lock();
        let mut doc = ContactsDocument::default();
        let groups: Vec<String> = c
            .prepare("SELECT name FROM contact_groups ORDER BY position")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if !groups.is_empty() {
            doc.groups = groups;
        }
        let rows = c
            .prepare("SELECT mxid, nickname, grp, favorite, updated_at, deleted, friendship FROM contacts")?
            .query_map([], |r| {
                Ok(Contact {
                    mxid: r.get(0)?,
                    nickname: r.get(1)?,
                    group: r.get(2)?,
                    favorite: r.get(3)?,
                    friendship: Friendship::parse(&r.get::<_, String>(6)?),
                    updated_at: r.get::<_, i64>(4)? as u64,
                    deleted: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for ct in rows {
            doc.contacts.insert(ct.mxid.clone(), ct);
        }
        Ok(doc)
    }

    /// Record (or change) our answer to an invitation.
    pub fn save_calendar_entry(&self, e: &CalendarEntry) -> Result<(), DbError> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO calendar_entries (event_id, room_id, sender, title, start_ms, description, response)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(event_id) DO UPDATE SET response = excluded.response",
            params![
                e.event_id,
                e.room_id,
                e.sender,
                e.title,
                e.start_ms as i64,
                e.description,
                e.response.key()
            ],
        )?;
        Ok(())
    }

    /// Create or change an entry the user made for themselves (not an invitation, so
    /// every field may change).
    pub fn save_personal_entry(&self, e: &CalendarEntry) -> Result<(), DbError> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO calendar_entries (event_id, room_id, sender, title, start_ms, description, response)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(event_id) DO UPDATE SET
                title = excluded.title, start_ms = excluded.start_ms,
                description = excluded.description",
            params![
                e.event_id,
                e.room_id,
                e.sender,
                e.title,
                e.start_ms as i64,
                e.description,
                e.response.key()
            ],
        )?;
        Ok(())
    }

    pub fn calendar_response(&self, event_id: &str) -> Result<Option<Response>, DbError> {
        let key: Option<String> = self
            .conn
            .lock()
            .expect("db lock")
            .query_row(
                "SELECT response FROM calendar_entries WHERE event_id = ?1",
                [event_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(key.and_then(|k| Response::from_key(&k)))
    }

    /// Record an organiser's `Update`/`Cancel` message from `sender`. Whether it counts is
    /// decided when reading: only a change from the invitation's own sender applies.
    pub fn note_calendar_change(
        &self,
        sender: &str,
        entry: &crate::calendar::Entry,
    ) -> Result<(), DbError> {
        use crate::calendar::Entry;
        let conn = self.conn.lock().expect("db lock");
        match entry {
            Entry::Cancel { event_id } => {
                conn.execute(
                    "INSERT INTO calendar_changes (event_id, sender, cancelled) VALUES (?1, ?2, 1)
                     ON CONFLICT(event_id, sender) DO UPDATE SET cancelled = 1",
                    params![event_id, sender],
                )?;
            }
            Entry::Update { event_id, invite } => {
                conn.execute(
                    "INSERT INTO calendar_changes (event_id, sender, title, start_ms, description)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(event_id, sender) DO UPDATE SET
                        title = excluded.title, start_ms = excluded.start_ms,
                        description = excluded.description",
                    params![
                        event_id,
                        sender,
                        invite.title,
                        invite.start_ms as i64,
                        invite.description
                    ],
                )?;
            }
            Entry::Invite(_) | Entry::Rsvp { .. } => {}
        }
        Ok(())
    }

    /// The organiser's changes to invitation `event_id` sent by `invite_sender`.
    pub fn calendar_change(
        &self,
        event_id: &str,
        invite_sender: &str,
    ) -> Result<CalendarChange, DbError> {
        let row = self
            .conn
            .lock()
            .expect("db lock")
            .query_row(
                "SELECT cancelled, title, start_ms, description FROM calendar_changes
                 WHERE event_id = ?1 AND sender = ?2",
                params![event_id, invite_sender],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? != 0,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            None => CalendarChange::default(),
            Some((cancelled, title, start, desc)) => CalendarChange {
                cancelled,
                update: title
                    .zip(start)
                    .map(|(t, s)| (t, s as u64, desc.unwrap_or_default())),
            },
        })
    }

    /// Entries on the calendar (not declined or cancelled), soonest first, with the
    /// organiser's latest title/time/details.
    pub fn calendar_entries(&self) -> Result<Vec<CalendarEntry>, DbError> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT e.event_id, e.room_id, e.sender,
                    COALESCE(c.title, e.title), COALESCE(c.start_ms, e.start_ms),
                    COALESCE(c.description, e.description), e.response
             FROM calendar_entries e
             LEFT JOIN calendar_changes c ON c.event_id = e.event_id AND c.sender = e.sender
             WHERE e.response != 'declined' AND COALESCE(c.cancelled, 0) = 0
             ORDER BY COALESCE(c.start_ms, e.start_ms), e.event_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CalendarEntry {
                event_id: r.get(0)?,
                room_id: r.get(1)?,
                sender: r.get(2)?,
                title: r.get(3)?,
                start_ms: r.get::<_, i64>(4)? as u64,
                description: r.get(5)?,
                response: Response::from_key(&r.get::<_, String>(6)?)
                    .unwrap_or(Response::Tentative),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn remove_calendar_entry(&self, event_id: &str) -> Result<(), DbError> {
        self.conn.lock().expect("db lock").execute(
            "DELETE FROM calendar_entries WHERE event_id = ?1",
            [event_id],
        )?;
        Ok(())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.lock().execute(
            "INSERT INTO application_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        let c = self.lock();
        let mut st = c.prepare("SELECT value FROM application_settings WHERE key = ?1")?;
        let mut rows = st.query([key])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_db() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn reopen_is_idempotent_and_persists() {
        let dir = std::env::temp_dir().join(format!("msm-test-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.db");
        {
            let db = Database::open(&path).unwrap();
            let mut doc = ContactsDocument::default();
            let mut c = Contact::new("@u:x", 7);
            c.nickname = Some("日本語".into());
            c.favorite = true;
            c.friendship = Friendship::Outgoing;
            doc.contacts.insert("@u:x".into(), c);
            doc.groups.push("Gamers".into());
            db.save_contacts(&doc).unwrap();
            db.set_setting("away_minutes", "10").unwrap();
            db.set_setting("away_minutes", "15").unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
        let doc = db.load_contacts().unwrap();
        assert_eq!(doc.contacts["@u:x"].nickname.as_deref(), Some("日本語"));
        assert!(doc.contacts["@u:x"].favorite);
        assert_eq!(doc.contacts["@u:x"].friendship, Friendship::Outgoing);
        assert_eq!(doc.groups.last().unwrap(), "Gamers");
        assert_eq!(
            db.get_setting("away_minutes").unwrap().as_deref(),
            Some("15")
        );
        assert_eq!(db.get_setting("missing").unwrap(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejects_newer_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY); INSERT INTO schema_migrations VALUES (99);").unwrap();
        assert!(matches!(Database::init(conn), Err(DbError::TooNew { .. })));
    }

    #[test]
    fn personal_entries_can_be_created_changed_and_removed() {
        let db = Database::open_in_memory().unwrap();
        let mut e = CalendarEntry {
            event_id: format!("{PERSONAL_PREFIX}abc"),
            room_id: String::new(),
            sender: "@me:x".into(),
            title: "Dentist".into(),
            start_ms: 2000,
            description: "".into(),
            response: Response::Accepted,
        };
        db.save_personal_entry(&e).unwrap();
        e.title = "Dentist (moved)".into();
        e.start_ms = 1000;
        db.save_personal_entry(&e).unwrap();
        let got = db.calendar_entries().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].title.as_str(), got[0].start_ms),
            ("Dentist (moved)", 1000)
        );
        db.remove_calendar_entry(&e.event_id).unwrap();
        assert!(db.calendar_entries().unwrap().is_empty());
    }

    #[test]
    fn organiser_changes_apply_only_from_the_invitations_own_sender() {
        use crate::calendar::{Entry, Invite};
        let db = Database::open_in_memory().unwrap();
        db.save_calendar_entry(&CalendarEntry {
            event_id: "$e".into(),
            room_id: "!r".into(),
            sender: "@org:x".into(),
            title: "Lunch".into(),
            start_ms: 1000,
            description: "".into(),
            response: Response::Accepted,
        })
        .unwrap();
        let moved = Entry::Update {
            event_id: "$e".into(),
            invite: Invite {
                title: "Brunch".into(),
                start_ms: 5000,
                description: "later".into(),
            },
        };
        // Someone else cannot move or cancel it.
        db.note_calendar_change("@evil:x", &moved).unwrap();
        db.note_calendar_change(
            "@evil:x",
            &Entry::Cancel {
                event_id: "$e".into(),
            },
        )
        .unwrap();
        assert_eq!(db.calendar_entries().unwrap()[0].title, "Lunch");
        assert_eq!(
            db.calendar_change("$e", "@org:x").unwrap(),
            CalendarChange::default()
        );
        // The organiser can reschedule, then cancel.
        db.note_calendar_change("@org:x", &moved).unwrap();
        let e = &db.calendar_entries().unwrap()[0];
        assert_eq!(
            (e.title.as_str(), e.start_ms, e.description.as_str()),
            ("Brunch", 5000, "later")
        );
        db.note_calendar_change(
            "@org:x",
            &Entry::Cancel {
                event_id: "$e".into(),
            },
        )
        .unwrap();
        assert!(db.calendar_entries().unwrap().is_empty());
        assert!(db.calendar_change("$e", "@org:x").unwrap().cancelled);
    }

    #[test]
    fn calendar_entries_keep_the_latest_answer_and_hide_declined() {
        use crate::calendar::Response;
        let db = Database::open_in_memory().unwrap();
        let mk = |id: &str, start: u64, r| CalendarEntry {
            event_id: id.into(),
            room_id: "!r:x".into(),
            sender: "@a:x".into(),
            title: format!("event {id}"),
            start_ms: start,
            description: String::new(),
            response: r,
        };
        db.save_calendar_entry(&mk("$late", 2000, Response::Accepted))
            .unwrap();
        db.save_calendar_entry(&mk("$soon", 1000, Response::Tentative))
            .unwrap();
        let ids: Vec<_> = db
            .calendar_entries()
            .unwrap()
            .into_iter()
            .map(|e| e.event_id)
            .collect();
        assert_eq!(ids, ["$soon", "$late"]);
        // Changing our mind updates the answer; declining takes it off the calendar.
        db.save_calendar_entry(&mk("$soon", 1000, Response::Accepted))
            .unwrap();
        assert_eq!(
            db.calendar_response("$soon").unwrap(),
            Some(Response::Accepted)
        );
        db.save_calendar_entry(&mk("$late", 2000, Response::Declined))
            .unwrap();
        assert_eq!(db.calendar_entries().unwrap().len(), 1);
        assert_eq!(
            db.calendar_response("$late").unwrap(),
            Some(Response::Declined)
        );
        assert_eq!(db.calendar_response("$none").unwrap(), None);
        db.remove_calendar_entry("$soon").unwrap();
        assert!(db.calendar_entries().unwrap().is_empty());
    }
}
