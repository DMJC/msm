//! Conversation model: ordering, de-duplication, edits, redactions and local
//! echo for sent messages. Pure data; no GTK, no network. Edits/redactions that
//! arrive before their target are held and applied when the target shows up.

use std::collections::HashMap;

use matrix_sdk::ruma::events::room::message::Relation;
use matrix_sdk::ruma::events::room::redaction::SyncRoomRedactionEvent;
use matrix_sdk::ruma::events::{
    AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
};
use matrix_sdk::ruma::serde::Raw;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Pending,
    Sent,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Event ID once known, otherwise the local transaction ID.
    pub id: String,
    /// Local transaction ID for messages we sent (used to match the remote echo).
    pub txn_id: Option<String>,
    pub sender: String,
    pub body: String,
    pub ts: u64,
    pub edited: bool,
    pub redacted: bool,
    pub undecryptable: bool,
    pub delivery: Delivery,
}

/// A decoded timeline event relevant to the conversation view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Message(ChatMessage),
    Edit { target: String, body: String },
    Redaction { target: String },
}

#[derive(Debug, Default)]
pub struct Conversation {
    messages: Vec<ChatMessage>,
    index: HashMap<String, usize>,
    early_edits: HashMap<String, String>,
    early_redactions: Vec<String>,
}

impl Conversation {
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn apply(&mut self, p: Parsed) {
        match p {
            Parsed::Message(m) => self.insert(m),
            Parsed::Edit { target, body } => self.edit(&target, body),
            Parsed::Redaction { target } => self.redact(&target),
        }
    }

    /// Insert a message from the server (live or history). Duplicates are ignored;
    /// a remote echo of our own pending message replaces the local one.
    pub fn insert(&mut self, mut m: ChatMessage) {
        if self.index.contains_key(&m.id) {
            return;
        }
        if let Some(txn) = m.txn_id.clone()
            && let Some(&i) = self.index.get(&txn)
        {
            let old_id = self.messages[i].id.clone();
            self.index.remove(&old_id);
            self.messages[i] = m.clone();
            self.index.insert(m.id.clone(), i);
            self.index.insert(txn, i); // keep txn resolvable for late status updates
            return;
        }
        if let Some(body) = self.early_edits.remove(&m.id) {
            m.body = body;
            m.edited = true;
        }
        if let Some(pos) = self.early_redactions.iter().position(|r| *r == m.id) {
            self.early_redactions.swap_remove(pos);
            m.redacted = true;
            m.body.clear();
        }
        // Stable insertion by timestamp (later arrivals with equal ts go after).
        let at = self.messages.partition_point(|x| x.ts <= m.ts);
        self.messages.insert(at, m);
        self.reindex();
    }

    pub fn add_pending(&mut self, txn_id: &str, sender: &str, body: &str, ts: u64) {
        self.insert(ChatMessage {
            id: txn_id.to_owned(),
            txn_id: Some(txn_id.to_owned()),
            sender: sender.to_owned(),
            body: body.to_owned(),
            ts,
            edited: false,
            redacted: false,
            undecryptable: false,
            delivery: Delivery::Pending,
        });
    }

    pub fn mark_failed(&mut self, txn_id: &str, reason: &str) {
        if let Some(m) = self.get_mut(txn_id) {
            m.delivery = Delivery::Failed(reason.to_owned());
        }
    }

    pub fn mark_pending(&mut self, txn_id: &str) {
        if let Some(m) = self.get_mut(txn_id) {
            m.delivery = Delivery::Pending;
        }
    }

    /// The server accepted our message and assigned `event_id`. Idempotent
    /// with respect to the remote echo arriving first.
    pub fn mark_sent(&mut self, txn_id: &str, event_id: &str) {
        if let Some(&i) = self.index.get(txn_id) {
            if self.messages[i].id == txn_id {
                let m = &mut self.messages[i];
                m.id = event_id.to_owned();
                m.delivery = Delivery::Sent;
                self.index.remove(txn_id);
                self.index.insert(event_id.to_owned(), i);
                self.index.insert(txn_id.to_owned(), i);
            } else {
                self.messages[i].delivery = Delivery::Sent;
            }
        }
    }

    pub fn edit(&mut self, target: &str, body: String) {
        match self.get_mut(target) {
            Some(m) if !m.redacted => {
                m.body = body;
                m.edited = true;
            }
            Some(_) => {}
            None => {
                self.early_edits.insert(target.to_owned(), body);
            }
        }
    }

    pub fn redact(&mut self, target: &str) {
        match self.get_mut(target) {
            Some(m) => {
                m.redacted = true;
                m.body.clear();
            }
            None => self.early_redactions.push(target.to_owned()),
        }
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut ChatMessage> {
        let i = *self.index.get(id)?;
        self.messages.get_mut(i)
    }

    fn reindex(&mut self) {
        let txns: Vec<(String, usize)> = self
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.txn_id.clone().map(|t| (t, i)))
            .collect();
        self.index.clear();
        for (i, m) in self.messages.iter().enumerate() {
            self.index.insert(m.id.clone(), i);
        }
        for (t, i) in txns {
            self.index.insert(t, i);
        }
    }
}

/// Decode a timeline event. Returns `None` for events the conversation view ignores.
pub fn parse_event(raw: &Raw<AnySyncTimelineEvent>) -> Option<Parsed> {
    let Ok(ev) = raw.deserialize() else {
        return parse_redacted_fallback(raw);
    };
    let AnySyncTimelineEvent::MessageLike(ev) = ev else {
        return None;
    };
    let ts = |t: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch| u64::from(t.get());
    match ev {
        AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(e)) => {
            if let Some(Relation::Replacement(r)) = &e.content.relates_to {
                return Some(Parsed::Edit {
                    target: r.event_id.to_string(),
                    body: r.new_content.msgtype.body().to_owned(),
                });
            }
            let txn = e.unsigned.transaction_id.as_ref().map(|t| t.to_string());
            Some(Parsed::Message(ChatMessage {
                id: e.event_id.to_string(),
                txn_id: txn,
                sender: e.sender.to_string(),
                body: e.content.msgtype.body().to_owned(),
                ts: ts(e.origin_server_ts),
                edited: false,
                redacted: false,
                undecryptable: false,
                delivery: Delivery::Sent,
            }))
        }
        AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Redacted(e)) => {
            Some(Parsed::Message(ChatMessage {
                id: e.event_id.to_string(),
                txn_id: None,
                sender: e.sender.to_string(),
                body: String::new(),
                ts: ts(e.origin_server_ts),
                edited: false,
                redacted: true,
                undecryptable: false,
                delivery: Delivery::Sent,
            }))
        }
        AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Redacted(e)) => {
            Some(Parsed::Message(ChatMessage {
                id: e.event_id.to_string(),
                txn_id: None,
                sender: e.sender.to_string(),
                body: String::new(),
                ts: ts(e.origin_server_ts),
                edited: false,
                redacted: true,
                undecryptable: false,
                delivery: Delivery::Sent,
            }))
        }
        AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Original(e)) => {
            Some(Parsed::Message(ChatMessage {
                id: e.event_id.to_string(),
                txn_id: None,
                sender: e.sender.to_string(),
                body: String::new(),
                ts: ts(e.origin_server_ts),
                edited: false,
                redacted: false,
                undecryptable: true,
                delivery: Delivery::Sent,
            }))
        }
        AnySyncMessageLikeEvent::RoomRedaction(SyncRoomRedactionEvent::Original(e)) => {
            let target = e.redacts.clone().or(e.content.redacts.clone())?;
            Some(Parsed::Redaction {
                target: target.to_string(),
            })
        }
        _ => None,
    }
}

/// Servers return redacted messages with empty content and `redacted_because`
/// (sometimes at the top level rather than in `unsigned`, and in encrypted
/// rooms with type `m.room.encrypted`), which the typed parser rejects.
/// Show those as redacted messages instead of dropping them.
fn parse_redacted_fallback(raw: &Raw<AnySyncTimelineEvent>) -> Option<Parsed> {
    let v: serde_json::Value = serde_json::from_str(raw.json().get()).ok()?;
    let ty = v.get("type")?.as_str()?;
    if ty != "m.room.message" && ty != "m.room.encrypted" {
        return None;
    }
    let redacted =
        v.get("redacted_because").is_some() || v.pointer("/unsigned/redacted_because").is_some();
    if !redacted {
        return None;
    }
    Some(Parsed::Message(ChatMessage {
        id: v.get("event_id")?.as_str()?.to_owned(),
        txn_id: None,
        sender: v.get("sender")?.as_str()?.to_owned(),
        body: String::new(),
        ts: v.get("origin_server_ts")?.as_u64()?,
        edited: false,
        redacted: true,
        undecryptable: false,
        delivery: Delivery::Sent,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(v: serde_json::Value) -> Raw<AnySyncTimelineEvent> {
        Raw::from_json_string(v.to_string()).unwrap()
    }

    fn msg(id: &str, ts: u64, body: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            txn_id: None,
            sender: "@a:x".into(),
            body: body.into(),
            ts,
            edited: false,
            redacted: false,
            undecryptable: false,
            delivery: Delivery::Sent,
        }
    }

    #[test]
    fn orders_by_timestamp_and_dedupes() {
        let mut c = Conversation::default();
        c.insert(msg("$2", 20, "second"));
        c.insert(msg("$1", 10, "first"));
        c.insert(msg("$2", 20, "second again"));
        let bodies: Vec<_> = c.messages().iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["first", "second"]);
    }

    #[test]
    fn edit_and_redaction_apply_even_when_early() {
        let mut c = Conversation::default();
        c.edit("$1", "fixed".into());
        c.redact("$2");
        c.insert(msg("$1", 1, "typo"));
        c.insert(msg("$2", 2, "secret"));
        assert_eq!(c.messages()[0].body, "fixed");
        assert!(c.messages()[0].edited);
        assert!(c.messages()[1].redacted);
        assert_eq!(c.messages()[1].body, "");
        c.redact("$1");
        c.edit("$1", "ignored".into());
        assert_eq!(c.messages()[0].body, "");
    }

    #[test]
    fn local_echo_lifecycle_no_duplicates() {
        let mut c = Conversation::default();
        c.add_pending("t1", "@me:x", "hi", 5);
        assert_eq!(c.messages()[0].delivery, Delivery::Pending);
        // Remote echo arrives before the send call returns.
        let mut remote = msg("$e", 6, "hi");
        remote.txn_id = Some("t1".into());
        c.insert(remote);
        assert_eq!(c.messages().len(), 1);
        assert_eq!(c.messages()[0].id, "$e");
        c.mark_sent("t1", "$e");
        assert_eq!(c.messages().len(), 1);
        assert_eq!(c.messages()[0].delivery, Delivery::Sent);
    }

    #[test]
    fn send_return_before_echo() {
        let mut c = Conversation::default();
        c.add_pending("t1", "@me:x", "hi", 5);
        c.mark_sent("t1", "$e");
        assert_eq!(c.messages()[0].id, "$e");
        let mut remote = msg("$e", 6, "hi");
        remote.txn_id = Some("t1".into());
        c.insert(remote);
        assert_eq!(c.messages().len(), 1);
    }

    #[test]
    fn failed_send_can_be_retried_without_duplicate() {
        let mut c = Conversation::default();
        c.add_pending("t1", "@me:x", "hi", 5);
        c.mark_failed("t1", "offline");
        assert_eq!(c.messages()[0].delivery, Delivery::Failed("offline".into()));
        c.mark_pending("t1");
        c.mark_sent("t1", "$e");
        assert_eq!(c.messages().len(), 1);
        assert_eq!(c.messages()[0].delivery, Delivery::Sent);
    }

    #[test]
    fn parses_text_edit_redaction_and_encrypted() {
        let m = parse_event(&raw(json!({
            "type":"m.room.message","event_id":"$1","sender":"@a:x","origin_server_ts":42,
            "content":{"msgtype":"m.text","body":"héllo\nworld"}
        })));
        match m {
            Some(Parsed::Message(m)) => {
                assert_eq!(m.body, "héllo\nworld");
                assert_eq!(m.ts, 42);
            }
            other => panic!("{other:?}"),
        }
        let e = parse_event(&raw(json!({
            "type":"m.room.message","event_id":"$2","sender":"@a:x","origin_server_ts":43,
            "content":{"msgtype":"m.text","body":"* new","m.new_content":{"msgtype":"m.text","body":"new"},
                       "m.relates_to":{"rel_type":"m.replace","event_id":"$1"}}
        })));
        assert_eq!(
            e,
            Some(Parsed::Edit {
                target: "$1".into(),
                body: "new".into()
            })
        );
        let r = parse_event(&raw(json!({
            "type":"m.room.redaction","event_id":"$3","sender":"@a:x","origin_server_ts":44,
            "redacts":"$1","content":{}
        })));
        assert_eq!(
            r,
            Some(Parsed::Redaction {
                target: "$1".into()
            })
        );
        let d = parse_event(&raw(json!({
            "type":"m.room.encrypted","event_id":"$4","sender":"@a:x","origin_server_ts":45,
            "content":{"algorithm":"m.megolm.v1.aes-sha2","ciphertext":"x","sender_key":"k","session_id":"s","device_id":"d"}
        })));
        assert!(matches!(d, Some(Parsed::Message(m)) if m.undecryptable));
        assert_eq!(
            parse_event(&raw(
                json!({"type":"m.room.topic","event_id":"$5","sender":"@a:x","origin_server_ts":1,"state_key":"","content":{"topic":"x"}})
            )),
            None
        );
    }

    #[test]
    fn redacted_encrypted_event_with_top_level_redacted_because() {
        let m = parse_event(&raw(json!({
            "type":"m.room.encrypted","event_id":"$9","sender":"@a:x","origin_server_ts":7,
            "content":{},"redacted_because":{"type":"m.room.redaction","event_id":"$r","sender":"@a:x","origin_server_ts":8,"content":{"redacts":"$9"}}
        })));
        match m {
            Some(Parsed::Message(m)) => assert!(m.redacted && !m.undecryptable && m.id == "$9"),
            other => panic!("{other:?}"),
        }
        // Empty content without a redaction marker is not treated as redacted.
        assert_eq!(
            parse_event(&raw(
                json!({"type":"m.room.encrypted","event_id":"$9","sender":"@a:x","origin_server_ts":7,"content":{}})
            )),
            None
        );
    }
}
