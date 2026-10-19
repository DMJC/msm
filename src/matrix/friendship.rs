//! Friend-request transport.
//!
//! Friendship messages are custom to-device events, deliberately *not* room
//! invitations or messages: other Matrix clients ignore unknown to-device
//! types, so nothing odd appears in their UI and no DM room is needed. The
//! homeserver sets the sender, so the origin of an event cannot be forged by
//! the other user. Delivery is queued by the server until the recipient syncs.

use matrix_sdk::Client;
use matrix_sdk::ruma::api::client::to_device::send_event_to_device::v3::Request;
use matrix_sdk::ruma::events::AnyToDeviceEventContent;
use matrix_sdk::ruma::events::ToDeviceEventType;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::to_device::DeviceIdOrAllDevices;
use matrix_sdk::ruma::{OwnedTransactionId, UserId};
use serde_json::json;
use thiserror::Error;

use crate::contacts::contact_manager::FriendAction;

/// Namespaced, versioned event type.
pub const EVENT_TYPE: &str = "com.matrixmessenger.friend.v1";

#[derive(Debug, Error)]
pub enum FriendError {
    #[error("invalid Matrix ID: {0}")]
    InvalidUser(String),
    #[error("could not send: {0}")]
    Matrix(String),
}

/// Tell every device of `mxid` about `action`.
pub async fn send(client: &Client, mxid: &str, action: FriendAction) -> Result<(), FriendError> {
    let uid = UserId::parse(mxid).map_err(|_| FriendError::InvalidUser(mxid.to_owned()))?;
    let content = Raw::new(&json!({ "action": action.as_str() }))
        .map_err(|e| FriendError::Matrix(e.to_string()))?
        .cast_unchecked::<AnyToDeviceEventContent>();
    let mut devices = std::collections::BTreeMap::new();
    devices.insert(DeviceIdOrAllDevices::AllDevices, content);
    let mut messages = std::collections::BTreeMap::new();
    messages.insert(uid, devices);
    let req = Request::new_raw(
        ToDeviceEventType::from(EVENT_TYPE),
        OwnedTransactionId::from(format!("msm-friend-{}", uuid_like())),
        messages,
    );
    client
        .send(req)
        .await
        .map_err(|e| FriendError::Matrix(e.to_string()))?;
    Ok(())
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{n:x}")
}

/// Parse the raw to-device JSON into `(sender, action)`. Anything that is not
/// a well-formed friend event yields `None`.
pub fn parse(raw: &str) -> Option<(String, FriendAction)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    if v.get("type")?.as_str()? != EVENT_TYPE {
        return None;
    }
    let sender = v.get("sender")?.as_str()?;
    if !crate::contacts::contact::is_valid_mxid(sender) {
        return None;
    }
    let action = FriendAction::parse(v.get("content")?.get("action")?.as_str()?)?;
    Some((sender.to_owned(), action))
}

/// Forward incoming friend events (never our own) to `tx`.
pub fn watch(client: &Client, tx: async_channel::Sender<(String, FriendAction)>) {
    client.add_event_handler(
        move |raw: Raw<matrix_sdk::ruma::events::AnyToDeviceEvent>, client: Client| {
            let tx = tx.clone();
            async move {
                let Some((sender, action)) = parse(raw.json().get()) else {
                    return;
                };
                if client.user_id().is_some_and(|me| me.as_str() == sender) {
                    return;
                }
                let _ = tx.send((sender, action)).await;
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_well_formed_friend_events() {
        let ok = r#"{"type":"com.matrixmessenger.friend.v1","sender":"@a:x","content":{"action":"request"}}"#;
        assert_eq!(parse(ok), Some(("@a:x".into(), FriendAction::Request)));
        for bad in [
            r#"{"type":"m.room.message","sender":"@a:x","content":{"action":"request"}}"#,
            r#"{"type":"com.matrixmessenger.friend.v1","sender":"nope","content":{"action":"request"}}"#,
            r#"{"type":"com.matrixmessenger.friend.v1","sender":"@a:x","content":{"action":"befriend"}}"#,
            r#"{"type":"com.matrixmessenger.friend.v1","sender":"@a:x","content":{}}"#,
            "not json",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
