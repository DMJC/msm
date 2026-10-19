//! Room-level operations: DM discovery/creation, history, sending, redaction,
//! and the live event feed.

use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::events::AnySyncTimelineEvent;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::events::typing::SyncTypingEvent;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{OwnedTransactionId, UserId};
use matrix_sdk::{Client, Room};
use thiserror::Error;

use super::messages::{Parsed, parse_event};

#[derive(Debug, Error)]
pub enum RoomError {
    #[error("invalid Matrix ID: {0}")]
    InvalidUser(String),
    #[error("Matrix error: {0}")]
    Matrix(String),
}

fn mx(e: impl std::fmt::Display) -> RoomError {
    RoomError::Matrix(e.to_string())
}

/// Existing DMs (per `m.direct`) with `mxid`, possibly several.
pub fn existing_dms(client: &Client, mxid: &str) -> Result<Vec<Room>, RoomError> {
    let uid = <&UserId>::try_from(mxid).map_err(|_| RoomError::InvalidUser(mxid.to_owned()))?;
    Ok(client.get_dm_rooms(uid).collect())
}

/// Open the existing DM with `mxid`, or create one (encrypted by default) if none exists.
pub async fn open_or_create_dm(client: &Client, mxid: &str) -> Result<Room, RoomError> {
    let uid = <&UserId>::try_from(mxid).map_err(|_| RoomError::InvalidUser(mxid.to_owned()))?;
    if let Some(room) = client.get_dm_room(uid) {
        return Ok(room);
    }
    client.create_dm(uid).await.map_err(mx)
}

pub struct History {
    /// Oldest-first.
    pub events: Vec<Parsed>,
    /// Pass back to fetch older messages; `None` at the start of the room.
    pub older_token: Option<String>,
}

/// Fetch up to `limit` events going backwards from `from` (or the live end).
pub async fn load_history(
    room: &Room,
    from: Option<&str>,
    limit: u16,
) -> Result<History, RoomError> {
    let mut opts = MessagesOptions::backward().from(from);
    opts.limit = limit.into();
    let msgs = room.messages(opts).await.map_err(mx)?;
    let mut events: Vec<Parsed> = msgs.chunk.iter().filter_map(parse_timeline_event).collect();
    events.reverse();
    Ok(History {
        events,
        older_token: msgs.end,
    })
}

fn parse_timeline_event(ev: &TimelineEvent) -> Option<Parsed> {
    parse_event(ev.raw())
}

/// Send plain text. Re-sending with the same `txn_id` is idempotent on the
/// server, so retrying a failed send cannot create a duplicate.
pub async fn send_text(
    room: &Room,
    body: &str,
    txn_id: OwnedTransactionId,
) -> Result<String, RoomError> {
    let r = room
        .send(RoomMessageEventContent::text_plain(body))
        .with_transaction_id(txn_id)
        .await
        .map_err(mx)?;
    Ok(r.response.event_id.to_string())
}

pub async fn redact(room: &Room, event_id: &str) -> Result<(), RoomError> {
    let id = <&matrix_sdk::ruma::EventId>::try_from(event_id).map_err(mx)?;
    room.redact(id, None, None).await.map_err(mx)?;
    Ok(())
}

pub async fn set_typing(room: &Room, typing: bool) {
    // Typing notices are best effort.
    if let Err(e) = room.typing_notice(typing).await {
        tracing::debug!("typing notice failed: {e}");
    }
}

#[derive(Debug, Clone)]
pub enum LiveEvent {
    Timeline { room_id: String, event: Parsed },
    Typing { room_id: String, users: Vec<String> },
}

/// Register handlers that forward live room events to `tx`.
pub fn forward_live_events(client: &Client, tx: async_channel::Sender<LiveEvent>) {
    let t = tx.clone();
    client.add_event_handler(move |raw: Raw<AnySyncTimelineEvent>, room: Room| {
        let t = t.clone();
        async move {
            if let Some(event) = parse_event(&raw) {
                let _ = t
                    .send(LiveEvent::Timeline {
                        room_id: room.room_id().to_string(),
                        event,
                    })
                    .await;
            }
        }
    });
    client.add_event_handler(move |ev: SyncTypingEvent, room: Room| {
        let tx = tx.clone();
        async move {
            let users = ev.content.user_ids.iter().map(|u| u.to_string()).collect();
            let _ = tx
                .send(LiveEvent::Typing {
                    room_id: room.room_id().to_string(),
                    users,
                })
                .await;
        }
    });
}
