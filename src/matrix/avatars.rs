//! Profile and room avatars: fetch scaled thumbnails, set or remove our own.
//!
//! Avatars are other people's data. Callers decode them defensively; the byte
//! count is capped here so a hostile server cannot hand us an enormous blob.

use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::profile::AvatarUrl;
use matrix_sdk::ruma::{OwnedMxcUri, UInt, UserId};
use matrix_sdk::{Client, Room};
use thiserror::Error;

/// Edge length (px) of the thumbnails we request.
pub const THUMB_PX: u32 = 64;
/// Largest avatar thumbnail we will accept.
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
/// Largest image we will upload as our own avatar.
pub const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AvatarError {
    #[error("not logged in")]
    NotLoggedIn,
    #[error("that image is too large (limit {limit} MB)")]
    TooLarge { limit: usize },
    #[error("{0}")]
    Matrix(String),
}

fn mx(e: impl std::fmt::Display) -> AvatarError {
    AvatarError::Matrix(e.to_string())
}

fn thumb() -> MediaFormat {
    MediaFormat::Thumbnail(MediaThumbnailSettings::new(
        UInt::from(THUMB_PX),
        UInt::from(THUMB_PX),
    ))
}

fn capped(bytes: Vec<u8>) -> Option<Vec<u8>> {
    (!bytes.is_empty() && bytes.len() <= MAX_AVATAR_BYTES).then_some(bytes)
}

async fn thumbnail_of(client: &Client, uri: OwnedMxcUri) -> Option<Vec<u8>> {
    client
        .media()
        .get_media_content(
            &MediaRequestParameters {
                source: MediaSource::Plain(uri),
                format: thumb(),
            },
            true,
        )
        .await
        .ok()
        .and_then(capped)
}

/// A user's avatar thumbnail, or `None` if they have none (or it can't be fetched).
pub async fn user_avatar(client: &Client, mxid: &str) -> Option<Vec<u8>> {
    let id = UserId::parse(mxid).ok()?;
    let profile = client.account().fetch_user_profile_of(&id).await.ok()?;
    let uri = profile.get_static::<AvatarUrl>().ok().flatten()?;
    thumbnail_of(client, uri).await
}

/// A room's own avatar thumbnail (not a DM peer's: see `user_avatar` for those).
pub async fn room_avatar(room: &Room) -> Option<Vec<u8>> {
    room.avatar_url()?;
    room.avatar(thumb()).await.ok().flatten().and_then(capped)
}

/// Upload `data` and make it our profile picture.
pub async fn set_own_avatar(
    client: &Client,
    mime: &mime_guess::mime::Mime,
    data: Vec<u8>,
) -> Result<(), AvatarError> {
    if data.len() > MAX_UPLOAD_BYTES {
        return Err(AvatarError::TooLarge {
            limit: MAX_UPLOAD_BYTES / (1024 * 1024),
        });
    }
    client
        .account()
        .upload_avatar(mime, data)
        .await
        .map(|_| ())
        .map_err(mx)
}

pub async fn remove_own_avatar(client: &Client) -> Result<(), AvatarError> {
    client.account().set_avatar_url(None).await.map_err(mx)
}

/// Reports the MXID of every user whose profile picture changed in a room we are in,
/// so cached avatars can be dropped instead of staying stale until a restart.
pub fn watch_changes(client: &Client, tx: async_channel::Sender<String>) {
    use matrix_sdk::ruma::events::room::member::{MembershipState, SyncRoomMemberEvent};
    client.add_event_handler(move |ev: SyncRoomMemberEvent| {
        let tx = tx.clone();
        async move {
            let Some(ev) = ev.as_original() else { return };
            if ev.content.membership != MembershipState::Join {
                return;
            }
            let before = ev
                .unsigned
                .prev_content
                .as_ref()
                .map(|p| p.avatar_url.clone());
            if before.is_some_and(|b| b != ev.content.avatar_url) {
                let _ = tx.send(ev.state_key.to_string()).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized_thumbnails() {
        assert!(capped(vec![]).is_none());
        assert!(capped(vec![1; 10]).is_some());
        assert!(capped(vec![0; MAX_AVATAR_BYTES + 1]).is_none());
    }
}
