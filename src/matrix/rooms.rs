//! Room-level operations: DM discovery/creation, history, sending, redaction,
//! and the live event feed.

use matrix_sdk::RoomMemberships;
use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::api::client::room::create_room::v3::{
    Request as CreateRoomRequest, RoomPreset,
};
use matrix_sdk::ruma::events::AnySyncTimelineEvent;
use matrix_sdk::ruma::events::InitialStateEvent;
use matrix_sdk::ruma::events::presence::PresenceEvent;
use matrix_sdk::ruma::events::reaction::ReactionEventContent;
use matrix_sdk::ruma::events::receipt::ReceiptThread;
use matrix_sdk::ruma::events::relation::{Annotation, Reply};
use matrix_sdk::ruma::events::room::encryption::RoomEncryptionEventContent;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::events::room::message::{
    Relation, ReplacementMetadata, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
};
use matrix_sdk::ruma::events::typing::SyncTypingEvent;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{OwnedTransactionId, UserId};
use matrix_sdk::{Client, Room};
use thiserror::Error;

use super::messages::{Parsed, Trust, parse_event};
use super::presence::{ContactPresence, from_wire};

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
/// With several candidate DMs, prefer one where the peer is joined, then invited.
pub async fn open_or_create_dm(client: &Client, mxid: &str) -> Result<Room, RoomError> {
    let uid = <&UserId>::try_from(mxid).map_err(|_| RoomError::InvalidUser(mxid.to_owned()))?;
    let mut best: Option<(u8, Room)> = None;
    for room in client.get_dm_rooms(uid) {
        let rank = match room.get_member(uid).await {
            Ok(Some(m)) => match m.membership() {
                MembershipState::Join => 2,
                MembershipState::Invite => 1,
                _ => 0,
            },
            _ => 0,
        };
        if best.as_ref().is_none_or(|(r, _)| rank > *r) {
            best = Some((rank, room));
        }
    }
    match best {
        Some((_, room)) => Ok(room),
        None => client.create_dm(uid).await.map_err(mx),
    }
}

/// Unread messages in `room` as computed by the SDK (works for encrypted rooms).
pub fn unread_count(room: &Room) -> u64 {
    room.num_unread_messages()
        .max(room.unread_notification_counts().notification_count)
}

/// Send a read receipt for `event_id` (best effort).
pub async fn mark_read(room: &Room, event_id: &str) {
    let Ok(id) = <&matrix_sdk::ruma::EventId>::try_from(event_id) else {
        return;
    };
    if let Err(e) = room
        .send_single_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, id.to_owned())
        .await
    {
        tracing::debug!("read receipt failed: {e}");
    }
}

/// A room invitation waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub room_id: String,
    pub name: String,
    pub inviter: Option<String>,
    pub is_direct: bool,
}

pub async fn pending_invites(client: &Client) -> Vec<Invite> {
    let mut out = Vec::new();
    for room in client.invited_rooms() {
        let name = room_title(&room).await;
        let inviter = room
            .invite_details()
            .await
            .ok()
            .map(|d| d.inviter_id.to_string());
        let is_direct = room.is_direct().await.unwrap_or(false);
        out.push(Invite {
            room_id: room.room_id().to_string(),
            name,
            inviter,
            is_direct,
        });
    }
    out
}

fn find_room(client: &Client, room_id: &str) -> Result<Room, RoomError> {
    let id = <&matrix_sdk::ruma::RoomId>::try_from(room_id).map_err(mx)?;
    client
        .get_room(id)
        .ok_or_else(|| RoomError::Matrix(format!("unknown room {room_id}")))
}

pub async fn accept_invite(client: &Client, room_id: &str) -> Result<(), RoomError> {
    find_room(client, room_id)?.join().await.map_err(mx)
}

/// Decline an invitation (or leave a joined room).
pub async fn leave_room(client: &Client, room_id: &str) -> Result<(), RoomError> {
    find_room(client, room_id)?.leave().await.map_err(mx)
}

/// Joined rooms that are not DMs or spaces.
pub fn group_rooms(client: &Client) -> Vec<Room> {
    client
        .joined_rooms()
        .into_iter()
        .filter(|r| r.direct_targets().is_empty() && !r.is_space())
        .collect()
}

/// Best available room name; never blocks on the network.
pub fn cached_title(room: &Room) -> String {
    room.cached_display_name()
        .map(|n| n.to_string())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| room.room_id().to_string())
}

pub async fn room_title(room: &Room) -> String {
    match room.display_name().await {
        Ok(n) if !n.to_string().trim().is_empty() => n.to_string(),
        _ => cached_title(room),
    }
}

/// Create an encrypted private group room and invite `invitees` (Matrix IDs).
pub async fn create_group(
    client: &Client,
    name: &str,
    invitees: &[String],
) -> Result<Room, RoomError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(RoomError::Matrix("room name must not be empty".into()));
    }
    let invite = invitees
        .iter()
        .map(|i| {
            matrix_sdk::ruma::UserId::parse(i.trim()).map_err(|_| RoomError::InvalidUser(i.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut req = CreateRoomRequest::new();
    req.name = Some(name.to_owned());
    req.invite = invite;
    req.preset = Some(RoomPreset::PrivateChat);
    req.initial_state = vec![
        InitialStateEvent::with_empty_state_key(
            RoomEncryptionEventContent::with_recommended_defaults(),
        )
        .to_raw_any(),
    ];
    client.create_room(req).await.map_err(mx)
}

pub async fn invite_user(room: &Room, mxid: &str) -> Result<(), RoomError> {
    let uid =
        <&UserId>::try_from(mxid.trim()).map_err(|_| RoomError::InvalidUser(mxid.to_owned()))?;
    room.invite_user_by_id(uid).await.map_err(mx)
}

/// Joined members with their room-specific display names (from member state events).
pub async fn member_names(room: &Room) -> Result<Vec<(String, Option<String>)>, RoomError> {
    let members = room.members(RoomMemberships::JOIN).await.map_err(mx)?;
    Ok(members
        .iter()
        .map(|m| (m.user_id().to_string(), m.display_name().map(str::to_owned)))
        .collect())
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
        .map(|p| p.with_trust(Trust::from_info(ev.encryption_info().map(|a| &**a))))
}

/// Send plain text. Re-sending with the same `txn_id` is idempotent on the
/// server, so retrying a failed send cannot create a duplicate.
pub async fn send_text(
    room: &Room,
    body: &str,
    txn_id: OwnedTransactionId,
    reply_to: Option<&str>,
) -> Result<String, RoomError> {
    let mut content = RoomMessageEventContent::text_plain(body);
    if let Some(id) = reply_to {
        let id = matrix_sdk::ruma::OwnedEventId::try_from(id).map_err(mx)?;
        content.relates_to = Some(Relation::Reply(Reply::with_event_id(id)));
    }
    let r = room
        .send(content)
        .with_transaction_id(txn_id)
        .await
        .map_err(mx)?;
    Ok(r.response.event_id.to_string())
}

/// Replace the text of one of our own messages (an `m.replace` edit).
pub async fn send_edit(
    room: &Room,
    target_event_id: &str,
    new_body: &str,
) -> Result<String, RoomError> {
    let id = matrix_sdk::ruma::OwnedEventId::try_from(target_event_id).map_err(mx)?;
    let content = RoomMessageEventContentWithoutRelation::text_plain(new_body)
        .make_replacement(ReplacementMetadata::new(id, None));
    let r = room.send(content).await.map_err(mx)?;
    Ok(r.response.event_id.to_string())
}

/// React to a message; returns the reaction's event ID (redact it to remove the reaction).
pub async fn send_reaction(
    room: &Room,
    target_event_id: &str,
    key: &str,
) -> Result<String, RoomError> {
    let id = matrix_sdk::ruma::OwnedEventId::try_from(target_event_id).map_err(mx)?;
    let r = room
        .send(ReactionEventContent::new(Annotation::new(
            id,
            key.to_owned(),
        )))
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
    Timeline {
        room_id: String,
        event: Box<Parsed>,
    },
    Typing {
        room_id: String,
        users: Vec<String>,
    },
    Presence {
        user: String,
        state: ContactPresence,
        message: Option<String>,
    },
}

/// Register handlers that forward live room events to `tx`.
pub fn forward_live_events(client: &Client, tx: async_channel::Sender<LiveEvent>) {
    let t = tx.clone();
    client.add_event_handler(
        move |raw: Raw<AnySyncTimelineEvent>,
              room: Room,
              info: Option<matrix_sdk::deserialized_responses::EncryptionInfo>| {
            let t = t.clone();
            async move {
                if let Some(event) =
                    parse_event(&raw).map(|p| p.with_trust(Trust::from_info(info.as_ref())))
                {
                    let _ = t
                        .send(LiveEvent::Timeline {
                            room_id: room.room_id().to_string(),
                            event: Box::new(event),
                        })
                        .await;
                }
            }
        },
    );
    let ttx = tx.clone();
    client.add_event_handler(move |ev: SyncTypingEvent, room: Room| {
        let tx = ttx.clone();
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
    client.add_event_handler(move |ev: PresenceEvent| {
        let tx = tx.clone();
        async move {
            let (state, message) =
                from_wire(&ev.content.presence, ev.content.status_msg.as_deref());
            let _ = tx
                .send(LiveEvent::Presence {
                    user: ev.sender.to_string(),
                    state,
                    message,
                })
                .await;
        }
    });
}
