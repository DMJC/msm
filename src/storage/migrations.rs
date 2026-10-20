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
    // 2: application-level friendship state
    "ALTER TABLE contacts ADD COLUMN friendship TEXT NOT NULL DEFAULT 'none';",
];
