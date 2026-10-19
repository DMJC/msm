//! Versioned account-data schema and merge strategy for contact sync.
//!
//! Matrix account data replaces whole event content, so concurrent edits from
//! two devices would otherwise clobber each other. Strategy: each contact
//! record carries `updated_at` (ms) and a `deleted` tombstone; documents are
//! merged per-contact, last-writer-wins, ties broken deterministically so all
//! devices converge. Callers should re-fetch, merge, and write back.
//!
//! Privacy: account data is NOT end-to-end encrypted; nicknames are visible
//! to the homeserver operator.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::contact::{Contact, DEFAULT_GROUPS, is_valid_mxid};

pub const ACCOUNT_DATA_TYPE: &str = "com.matrixmessenger.contacts.v1";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub contacts: BTreeMap<String, Contact>,
}

impl Default for ContactsDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            groups: DEFAULT_GROUPS.iter().map(|s| s.to_string()).collect(),
            contacts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    Ok,
    /// Content was malformed; an empty document was substituted.
    Recovered,
    /// Content came from a newer schema; parsed best-effort, should not be overwritten blindly.
    NewerSchema(u32),
}

impl ContactsDocument {
    /// Parse account-data content defensively. Never fails: bad entries are
    /// dropped, and wholly malformed content yields an empty document.
    pub fn parse(value: &serde_json::Value) -> (Self, ParseOutcome) {
        let Some(obj) = value.as_object() else {
            return (Self::default(), ParseOutcome::Recovered);
        };
        let version = obj
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let mut doc = Self::default();
        if let Some(groups) = obj.get("groups").and_then(|g| g.as_array()) {
            let g: Vec<String> = groups
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if !g.is_empty() {
                doc.groups = dedup(g);
            }
        }
        let mut dropped = false;
        if let Some(contacts) = obj.get("contacts").and_then(|c| c.as_object()) {
            for (key, v) in contacts {
                // Accept records missing `mxid` (as in the example schema) by filling from the key.
                let mut v = v.clone();
                if let Some(o) = v.as_object_mut() {
                    o.entry("mxid").or_insert_with(|| key.clone().into());
                }
                match serde_json::from_value::<Contact>(v) {
                    Ok(mut c) if is_valid_mxid(key) => {
                        c.mxid = key.clone();
                        doc.contacts.insert(key.clone(), c);
                    }
                    _ => dropped = true,
                }
            }
        } else if !obj.contains_key("contacts") && version == 0 {
            return (Self::default(), ParseOutcome::Recovered);
        }
        doc.schema_version = SCHEMA_VERSION;
        let outcome = if version > SCHEMA_VERSION {
            ParseOutcome::NewerSchema(version)
        } else if dropped || version == 0 {
            ParseOutcome::Recovered
        } else {
            ParseOutcome::Ok
        };
        (doc, outcome)
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }

    /// Live (non-tombstoned) contacts.
    pub fn live(&self) -> impl Iterator<Item = &Contact> {
        self.contacts.values().filter(|c| !c.deleted)
    }

    /// Merge `other` into `self`, per contact, last-writer-wins.
    pub fn merge(&mut self, other: &ContactsDocument) {
        for (k, theirs) in &other.contacts {
            match self.contacts.get(k) {
                Some(mine) if !wins(theirs, mine) => {}
                _ => {
                    self.contacts.insert(k.clone(), theirs.clone());
                }
            }
        }
        let mut groups = std::mem::take(&mut self.groups);
        for g in &other.groups {
            if !groups.contains(g) {
                groups.push(g.clone());
            }
        }
        self.groups = groups;
    }
}

/// Does `a` beat `b`? Newer timestamp wins; on ties a tombstone wins, then a
/// deterministic field comparison so all devices converge.
fn wins(a: &Contact, b: &Contact) -> bool {
    if a.updated_at != b.updated_at {
        return a.updated_at > b.updated_at;
    }
    if a.deleted != b.deleted {
        return a.deleted;
    }
    let ka = (&a.nickname, &a.group, a.favorite);
    let kb = (&b.nickname, &b.group, b.favorite);
    ka > kb
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(m: &str, nick: &str, t: u64) -> Contact {
        let mut c = Contact::new(m, t);
        c.nickname = Some(nick.into());
        c
    }

    #[test]
    fn parses_spec_example() {
        let v = json!({"schema_version":1,"contacts":{"@dmjc:example.com":
            {"nickname":"James Carthew","group":"Friends","favorite":true}}});
        let (d, o) = ContactsDocument::parse(&v);
        assert_eq!(o, ParseOutcome::Ok);
        let c = &d.contacts["@dmjc:example.com"];
        assert_eq!(c.nickname.as_deref(), Some("James Carthew"));
        assert!(c.favorite);
        assert_eq!(c.mxid, "@dmjc:example.com");
    }

    #[test]
    fn malformed_content_recovers() {
        for v in [json!(null), json!("x"), json!([1]), json!({})] {
            let (d, o) = ContactsDocument::parse(&v);
            assert_eq!(o, ParseOutcome::Recovered);
            assert!(d.contacts.is_empty());
        }
    }

    #[test]
    fn bad_entries_dropped_good_kept() {
        let v = json!({"schema_version":1,"contacts":{
            "not-an-mxid":{"nickname":"x"},
            "@a:x":{"nickname":5},
            "@b:x":{"nickname":"ok"}}});
        let (d, o) = ContactsDocument::parse(&v);
        assert_eq!(o, ParseOutcome::Recovered);
        assert_eq!(d.contacts.len(), 1);
        assert!(d.contacts.contains_key("@b:x"));
    }

    #[test]
    fn newer_schema_flagged() {
        let v = json!({"schema_version":9,"contacts":{}});
        assert_eq!(ContactsDocument::parse(&v).1, ParseOutcome::NewerSchema(9));
    }

    #[test]
    fn merge_last_writer_wins_and_converges() {
        let mut a = ContactsDocument::default();
        let mut b = ContactsDocument::default();
        a.contacts.insert("@u:x".into(), c("@u:x", "old", 1));
        b.contacts.insert("@u:x".into(), c("@u:x", "new", 2));
        b.contacts.insert("@v:x".into(), c("@v:x", "v", 1));
        let mut ab = a.clone();
        ab.merge(&b);
        let mut ba = b.clone();
        ba.merge(&a);
        assert_eq!(ab.contacts, ba.contacts);
        assert_eq!(ab.contacts["@u:x"].nickname.as_deref(), Some("new"));
        assert!(ab.contacts.contains_key("@v:x"));
    }

    #[test]
    fn tombstone_beats_stale_edit_and_tie() {
        let mut a = ContactsDocument::default();
        let mut b = ContactsDocument::default();
        let mut dead = c("@u:x", "n", 5);
        dead.deleted = true;
        a.contacts.insert("@u:x".into(), dead);
        b.contacts.insert("@u:x".into(), c("@u:x", "edit", 5));
        let mut ab = a.clone();
        ab.merge(&b);
        let mut ba = b.clone();
        ba.merge(&a);
        assert!(ab.contacts["@u:x"].deleted);
        assert_eq!(ab.contacts, ba.contacts);
        assert_eq!(ab.live().count(), 0);
    }

    #[test]
    fn round_trip() {
        let mut d = ContactsDocument::default();
        d.contacts.insert("@u:x".into(), c("@u:x", "日本", 3));
        let (back, o) = ContactsDocument::parse(&d.to_value());
        assert_eq!(o, ParseOutcome::Ok);
        assert_eq!(back, d);
    }
}
