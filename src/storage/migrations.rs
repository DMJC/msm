//! Ordered, append-only schema migrations. Never edit a shipped entry.

pub const MIGRATIONS: &[&str] = &[
    // 1: contacts cache + settings
    "CREATE TABLE contact_groups (
        position INTEGER NOT NULL,
        name TEXT PRIMARY KEY
     );
     CREATE TABLE contacts (
        mxid TEXT PRIMARY KEY,
        nickname TEXT,
        grp TEXT NOT NULL DEFAULT 'Other',
        favorite INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL DEFAULT 0,
        deleted INTEGER NOT NULL DEFAULT 0
     );
     CREATE TABLE application_settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
     );",
];
