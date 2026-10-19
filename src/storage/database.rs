//! Application-local SQLite state (contact cache, settings). Message storage
//! stays with the Matrix SDK. All access goes through one `Database` handle
//! guarded by a mutex (single writer).

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, params};
use thiserror::Error;

use super::migrations::MIGRATIONS;
use crate::contacts::contact::Contact;
use crate::contacts::synchronization::ContactsDocument;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database is from a newer version (schema {found}, supported {supported})")]
    TooNew { found: usize, supported: usize },
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
                "INSERT INTO contacts (mxid, nickname, grp, favorite, updated_at, deleted) VALUES (?1,?2,?3,?4,?5,?6)",
                params![ct.mxid, ct.nickname, ct.group, ct.favorite, ct.updated_at as i64, ct.deleted],
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
            .prepare("SELECT mxid, nickname, grp, favorite, updated_at, deleted FROM contacts")?
            .query_map([], |r| {
                Ok(Contact {
                    mxid: r.get(0)?,
                    nickname: r.get(1)?,
                    group: r.get(2)?,
                    favorite: r.get(3)?,
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
}
