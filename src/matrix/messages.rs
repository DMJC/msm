//! Conversation model: ordering, de-duplication, edits, redactions and local
//! echo for sent messages. Pure data; no GTK, no network. Edits/redactions that
//! arrive before their target are held and applied when the target shows up.

use std::collections::HashMap;

use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::room::message::{MessageType, Relation};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    File,
    Image,
    Audio,
    Video,
}

/// Media attached to a message. `filename` is untrusted: sanitise before using as a path.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub filename: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub source: MediaSource,
}

impl Attachment {
    /// The media URL (`mxc://...`), which identifies the content.
    pub fn url(&self) -> String {
        match &self.source {
            MediaSource::Plain(u) => u.to_string(),
            MediaSource::Encrypted(f) => f.url.to_string(),
        }
    }
}

// `MediaSource` has no equality of its own; the URL identifies the content.
impl PartialEq for Attachment {
    fn eq(&self, o: &Self) -> bool {
        (self.kind, &self.filename, self.size, &self.mime, self.url())
            == (o.kind, &o.filename, o.size, &o.mime, o.url())
    }
}
impl Eq for Attachment {}

/// Describe the media in a message, if it has any.
pub fn attachment_of(t: &MessageType) -> Option<Attachment> {
    let (kind, filename, source, size, mime) = match t {
        MessageType::File(c) => (
            AttachmentKind::File,
            c.filename().to_owned(),
            c.source.clone(),
            c.info.as_ref().and_then(|i| i.size),
            c.info.as_ref().and_then(|i| i.mimetype.clone()),
        ),
        MessageType::Image(c) => (
            AttachmentKind::Image,
            c.filename().to_owned(),
            c.source.clone(),
            c.info.as_ref().and_then(|i| i.size),
            c.info.as_ref().and_then(|i| i.mimetype.clone()),
        ),
        MessageType::Audio(c) => (
            AttachmentKind::Audio,
            c.filename().to_owned(),
            c.source.clone(),
            c.info.as_ref().and_then(|i| i.size),
            c.info.as_ref().and_then(|i| i.mimetype.clone()),
        ),
        MessageType::Video(c) => (
            AttachmentKind::Video,
            c.filename().to_owned(),
            c.source.clone(),
            c.info.as_ref().and_then(|i| i.size),
            c.info.as_ref().and_then(|i| i.mimetype.clone()),
        ),
        _ => return None,
    };
    Some(Attachment {
        kind,
        filename,
        size: size.map(u64::from),
        mime,
        source,
    })
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
    pub attachment: Option<Attachment>,
}

/// A decoded timeline event relevant to the conversation view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Message(ChatMessage),
    /// `sender` and `ts` matter: only the original sender may edit, and the newest edit wins.
    Edit {
        target: String,
        sender: String,
        ts: u64,
        body: String,
    },
    Reaction(Reaction),
    Redaction {
        target: String,
    },
}

/// One user's `m.reaction` annotation on a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    /// Event ID of the reaction itself (redact this to remove it).
    pub event_id: String,
    pub target: String,
    pub key: String,
    pub sender: String,
}

/// Reactions on a message, grouped by emoji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionSummary {
    pub key: String,
    pub senders: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Conversation {
    messages: Vec<ChatMessage>,
    index: HashMap<String, usize>,
    /// Edits that arrived before their target: target -> (sender, ts, body).
    early_edits: HashMap<String, (String, u64, String)>,
    early_redactions: Vec<String>,
    /// Timestamp of the edit currently applied, so an older edit cannot overwrite a newer one.
    edit_ts: HashMap<String, u64>,
    /// Reactions by target message, in arrival order.
    reactions: HashMap<String, Vec<Reaction>>,
    /// Reaction event ID -> target, for dedupe and removal.
    reaction_index: HashMap<String, String>,
}

impl Conversation {
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn apply(&mut self, p: Parsed) {
        match p {
            Parsed::Message(m) => self.insert(m),
            Parsed::Edit {
                target,
                sender,
                ts,
                body,
            } => self.edit(&target, &sender, ts, body),
            Parsed::Reaction(r) => self.add_reaction(r),
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
        if let Some((sender, ts, body)) = self.early_edits.remove(&m.id)
            && sender == m.sender
        {
            m.body = body;
            m.edited = true;
            self.edit_ts.insert(m.id.clone(), ts);
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
            attachment: None,
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

    /// Apply an edit. Edits from anyone but the original sender are ignored
    /// (Matrix requires the same sender), and a stale edit never replaces a newer one.
    pub fn edit(&mut self, target: &str, sender: &str, ts: u64, body: String) {
        let newer_exists = self.edit_ts.get(target).is_some_and(|&t| t > ts);
        match self.get_mut(target) {
            Some(m) if m.sender != sender => {}
            Some(m) if !m.redacted && !newer_exists => {
                m.body = body;
                m.edited = true;
                self.edit_ts.insert(target.to_owned(), ts);
            }
            Some(_) => {}
            None => {
                let replace = self
                    .early_edits
                    .get(target)
                    .is_none_or(|(_, t, _)| *t <= ts);
                if replace {
                    self.early_edits
                        .insert(target.to_owned(), (sender.to_owned(), ts, body));
                }
            }
        }
    }

    pub fn add_reaction(&mut self, r: Reaction) {
        if self.reaction_index.contains_key(&r.event_id) {
            return;
        }
        if let Some(pos) = self.early_redactions.iter().position(|x| *x == r.event_id) {
            self.early_redactions.swap_remove(pos);
            return; // redacted before we saw it
        }
        self.reaction_index
            .insert(r.event_id.clone(), r.target.clone());
        self.reactions.entry(r.target.clone()).or_default().push(r);
    }

    /// Reactions on `target`, grouped by emoji in order of first use.
    pub fn reactions_for(&self, target: &str) -> Vec<ReactionSummary> {
        let mut out: Vec<ReactionSummary> = Vec::new();
        for r in self.reactions.get(target).into_iter().flatten() {
            match out.iter_mut().find(|s| s.key == r.key) {
                Some(s) => {
                    if !s.senders.contains(&r.sender) {
                        s.senders.push(r.sender.clone());
                    }
                }
                None => out.push(ReactionSummary {
                    key: r.key.clone(),
                    senders: vec![r.sender.clone()],
                }),
            }
        }
        out
    }

    /// The event ID of `sender`'s reaction `key` on `target`, if any (to remove it).
    pub fn own_reaction_event(&self, target: &str, key: &str, sender: &str) -> Option<String> {
        self.reactions
            .get(target)?
            .iter()
            .find(|r| r.key == key && r.sender == sender)
            .map(|r| r.event_id.clone())
    }

    pub fn redact(&mut self, target: &str) {
        if let Some(msg) = self.reaction_index.remove(target) {
            if let Some(list) = self.reactions.get_mut(&msg) {
                list.retain(|r| r.event_id != target);
            }
            return;
        }
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
                    sender: e.sender.to_string(),
                    ts: ts(e.origin_server_ts),
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
                attachment: attachment_of(&e.content.msgtype),
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
                attachment: None,
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
                attachment: None,
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
                attachment: None,
            }))
        }
        AnySyncMessageLikeEvent::Reaction(SyncMessageLikeEvent::Original(e)) => {
            Some(Parsed::Reaction(Reaction {
                event_id: e.event_id.to_string(),
                target: e.content.relates_to.event_id.to_string(),
                key: e.content.relates_to.key.clone(),
                sender: e.sender.to_string(),
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
        attachment: None,
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
            attachment: None,
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
        c.edit("$1", "@a:x", 5, "fixed".into());
        c.redact("$2");
        c.insert(msg("$1", 1, "typo"));
        c.insert(msg("$2", 2, "secret"));
        assert_eq!(c.messages()[0].body, "fixed");
        assert!(c.messages()[0].edited);
        assert!(c.messages()[1].redacted);
        assert_eq!(c.messages()[1].body, "");
        c.redact("$1");
        c.edit("$1", "@a:x", 6, "ignored".into());
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
                sender: "@a:x".into(),
                ts: 43,
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

    #[test]
    fn edits_from_other_users_and_stale_edits_are_ignored() {
        let mut c = Conversation::default();
        c.insert(msg("$1", 1, "original"));
        c.edit("$1", "@mallory:x", 10, "hijacked".into());
        assert_eq!(c.messages()[0].body, "original");
        assert!(!c.messages()[0].edited);
        c.edit("$1", "@a:x", 20, "second".into());
        c.edit("$1", "@a:x", 15, "older edit arriving late".into());
        assert_eq!(c.messages()[0].body, "second");
        // Early edits: wrong sender is dropped when the target arrives; newest early edit wins.
        let mut d = Conversation::default();
        d.edit("$2", "@mallory:x", 5, "hijacked".into());
        d.edit("$3", "@a:x", 9, "newest".into());
        d.edit("$3", "@a:x", 7, "stale".into());
        d.insert(msg("$2", 1, "mine"));
        d.insert(msg("$3", 2, "orig"));
        assert_eq!(d.messages()[0].body, "mine");
        assert_eq!(d.messages()[1].body, "newest");
    }

    fn reaction(id: &str, target: &str, key: &str, sender: &str) -> Reaction {
        Reaction {
            event_id: id.into(),
            target: target.into(),
            key: key.into(),
            sender: sender.into(),
        }
    }

    #[test]
    fn reactions_group_dedupe_and_remove() {
        let mut c = Conversation::default();
        c.insert(msg("$1", 1, "hi"));
        c.add_reaction(reaction("$r1", "$1", "👍", "@a:x"));
        c.add_reaction(reaction("$r2", "$1", "👍", "@b:x"));
        c.add_reaction(reaction("$r3", "$1", "❤️", "@a:x"));
        c.add_reaction(reaction("$r1", "$1", "👍", "@a:x")); // duplicate delivery
        let r = c.reactions_for("$1");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key, "👍");
        assert_eq!(r[0].senders, ["@a:x", "@b:x"]);
        assert_eq!(
            c.own_reaction_event("$1", "👍", "@b:x").as_deref(),
            Some("$r2")
        );
        // Redacting the reaction event removes just that user's reaction.
        c.redact("$r2");
        assert_eq!(c.reactions_for("$1")[0].senders, ["@a:x"]);
        c.redact("$r1");
        assert_eq!(c.reactions_for("$1").len(), 1);
        assert_eq!(
            c.messages().len(),
            1,
            "reaction redaction must not touch messages"
        );
        assert!(!c.messages()[0].redacted);
    }

    #[test]
    fn reaction_redacted_before_it_arrives_never_shows() {
        let mut c = Conversation::default();
        c.redact("$r1");
        c.add_reaction(reaction("$r1", "$1", "👍", "@a:x"));
        assert!(c.reactions_for("$1").is_empty());
    }

    #[test]
    fn parses_reaction_event() {
        let r = parse_event(&raw(json!({
            "type":"m.reaction","event_id":"$r","sender":"@b:x","origin_server_ts":9,
            "content":{"m.relates_to":{"rel_type":"m.annotation","event_id":"$1","key":"🎉"}}
        })));
        assert_eq!(
            r,
            Some(Parsed::Reaction(reaction("$r", "$1", "🎉", "@b:x")))
        );
    }

    #[test]
    fn parses_attachments() {
        let f = parse_event(&raw(json!({
            "type":"m.room.message","event_id":"$f","sender":"@a:x","origin_server_ts":1,
            "content":{"msgtype":"m.file","body":"report.pdf","filename":"report.pdf",
                       "url":"mxc://example.org/abc","info":{"size":2048,"mimetype":"application/pdf"}}
        })));
        match f {
            Some(Parsed::Message(m)) => {
                let a = m.attachment.expect("file attachment");
                assert_eq!(a.kind, AttachmentKind::File);
                assert_eq!(a.filename, "report.pdf");
                assert_eq!(a.size, Some(2048));
                assert_eq!(a.mime.as_deref(), Some("application/pdf"));
                assert_eq!(a.url(), "mxc://example.org/abc");
            }
            other => panic!("{other:?}"),
        }
        let i = parse_event(&raw(json!({
            "type":"m.room.message","event_id":"$i","sender":"@a:x","origin_server_ts":2,
            "content":{"msgtype":"m.image","body":"cat.png","url":"mxc://example.org/img"}
        })));
        match i {
            Some(Parsed::Message(m)) => {
                let a = m.attachment.expect("image attachment");
                assert_eq!(a.kind, AttachmentKind::Image);
                assert_eq!(a.filename, "cat.png");
                assert_eq!(a.size, None);
            }
            other => panic!("{other:?}"),
        }
        // Plain text has none.
        match parse_event(&raw(json!({
            "type":"m.room.message","event_id":"$t","sender":"@a:x","origin_server_ts":3,
            "content":{"msgtype":"m.text","body":"hello"}
        }))) {
            Some(Parsed::Message(m)) => assert!(m.attachment.is_none()),
            other => panic!("{other:?}"),
        }
    }
}
