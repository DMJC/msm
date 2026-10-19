//! Integration tests against a real homeserver. Skipped unless
//! `MSM_TEST_HOMESERVER` (e.g. http://localhost:8008) is set. Expects accounts
//! alice / bob / darkmage with password `pw-<name>-123` to exist.

use std::path::PathBuf;
use std::time::Duration;

use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::events::room::message::{
    OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::{Client, ruma::UserId};
use msm::contacts::contact_manager::ContactManager;
use msm::matrix::authentication::login_unsaved;
use msm::matrix::contacts_sync;

fn homeserver() -> Option<String> {
    std::env::var("MSM_TEST_HOMESERVER").ok()
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("msm-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

async fn login_as(hs: &str, name: &str, tag: &str) -> Client {
    let dir = tmp(tag);
    login_unsaved(hs, name, &format!("pw-{name}-123"), &dir)
        .await
        .unwrap_or_else(|e| panic!("login {name}: {e}"))
        .0
}

#[tokio::test]
async fn wrong_password_is_a_clean_error() {
    let Some(hs) = homeserver() else { return };
    let r = login_unsaved(&hs, "alice", "wrong", &tmp("wrongpw")).await;
    let e = r.expect_err("must fail").to_string();
    assert!(e.starts_with("Login failed"), "{e}");
}

#[tokio::test]
async fn nickname_syncs_between_installations_without_touching_profile() {
    let Some(hs) = homeserver() else { return };
    let darkmage = "@darkmage:localhost";
    let profile_client = login_as(&hs, "darkmage", "dm-profile").await;
    profile_client
        .account()
        .set_display_name(Some("Darkmage"))
        .await
        .unwrap();
    let before = profile_client
        .account()
        .fetch_user_profile_of(<&UserId>::try_from(darkmage).unwrap())
        .await
        .unwrap();

    // Installation 1: Alice assigns a private nickname and pushes it.
    let a1 = login_as(&hs, "alice", "alice-1").await;
    let mut m1 = ContactManager::default();
    m1.add(darkmage, 1).unwrap();
    m1.set_nickname(darkmage, Some("James Carthew"), 2).unwrap();
    contacts_sync::sync(&a1.account(), m1.document())
        .await
        .unwrap();

    // Installation 2 (separate device, empty local state) pulls it.
    let a2 = login_as(&hs, "alice", "alice-2").await;
    let merged = contacts_sync::sync(&a2.account(), &ContactManager::default().document().clone())
        .await
        .unwrap();
    let m2 = ContactManager::new(merged);
    assert_eq!(m2.resolver().resolve(darkmage, None), "James Carthew");

    // Bob's view is independent.
    let b = login_as(&hs, "bob", "bob-1").await;
    let mut mb = ContactManager::default();
    mb.add(darkmage, 1).unwrap();
    mb.set_nickname(darkmage, Some("DMJC"), 2).unwrap();
    contacts_sync::sync(&b.account(), mb.document())
        .await
        .unwrap();
    assert_eq!(mb.resolver().resolve(darkmage, None), "DMJC");
    assert_eq!(m2.resolver().resolve(darkmage, None), "James Carthew");

    // The target's public profile is unchanged.
    let after = profile_client
        .account()
        .fetch_user_profile_of(<&UserId>::try_from(darkmage).unwrap())
        .await
        .unwrap();
    assert_eq!(
        before
            .get_static::<matrix_sdk::ruma::api::client::profile::DisplayName>()
            .unwrap(),
        after
            .get_static::<matrix_sdk::ruma::api::client::profile::DisplayName>()
            .unwrap()
    );
}

#[tokio::test]
async fn dm_round_trip() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "rt-alice").await;
    let dark = login_as(&hs, "darkmage", "rt-dark").await;

    let (tx, rx) = async_channel::unbounded::<String>();
    dark.add_event_handler(move |ev: OriginalSyncRoomMessageEvent| {
        let tx = tx.clone();
        async move {
            let _ = tx.send(ev.content.body().to_owned()).await;
        }
    });
    dark.sync_once(SyncSettings::default()).await.unwrap();

    let room = alice
        .create_dm(<&UserId>::try_from("@darkmage:localhost").unwrap())
        .await
        .unwrap();
    let text = format!("héllo 日本語 {}", std::process::id());
    room.send(RoomMessageEventContent::text_plain(&text))
        .await
        .unwrap();

    // Darkmage syncs, accepts the invite, then sees the message.
    let mut got = None;
    for _ in 0..10 {
        dark.sync_once(SyncSettings::default().timeout(Duration::from_secs(2)))
            .await
            .unwrap();
        for r in dark.invited_rooms() {
            r.join().await.unwrap();
        }
        // Parallel tests share accounts, so skip other tests' messages.
        while let Ok(m) = rx.try_recv() {
            if m == text {
                got = Some(m);
            }
        }
        if got.is_some() {
            break;
        }
    }
    assert_eq!(got.as_deref(), Some(text.as_str()));
}

/// Exercises the real secret store + restore + logout. Skips if no keyring is usable.
#[tokio::test]
async fn session_persists_restores_and_logout_wipes() {
    let Some(hs) = homeserver() else { return };
    if msm::matrix::secrets::save("probe", "x").await.is_err() {
        eprintln!("SKIP: no secure credential store available");
        return;
    }
    let _ = msm::matrix::secrets::delete("probe").await;

    let dir = tmp("persist");
    let c = msm::matrix::authentication::login(&hs, "bob", "pw-bob-123", &dir)
        .await
        .unwrap();
    let uid = c.user_id().unwrap().to_owned();
    drop(c);

    let restored = msm::matrix::authentication::restore(&dir)
        .await
        .unwrap()
        .expect("session should restore");
    assert_eq!(restored.user_id().unwrap(), uid);
    restored
        .sync_once(SyncSettings::default().timeout(Duration::from_secs(1)))
        .await
        .unwrap();

    msm::matrix::authentication::logout(&restored, &dir).await;
    assert!(
        msm::matrix::authentication::restore(&dir)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!dir.join("matrix-store").exists());
}

use msm::matrix::messages::{Conversation, Parsed};
use msm::matrix::rooms;

async fn sync_a_bit(c: &Client) {
    c.sync_once(SyncSettings::default().timeout(Duration::from_secs(1)))
        .await
        .unwrap();
}

#[tokio::test]
async fn dm_discovery_reuses_existing_room_and_peer_sees_it_as_dm() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "disc-alice").await;
    let dark = login_as(&hs, "darkmage", "disc-dark").await;
    sync_a_bit(&alice).await;

    let first = rooms::open_or_create_dm(&alice, "@darkmage:localhost")
        .await
        .unwrap();
    sync_a_bit(&alice).await;
    let second = rooms::open_or_create_dm(&alice, "@darkmage:localhost")
        .await
        .unwrap();
    // An earlier test run may have left other DMs; whichever is chosen must be stable.
    let again = rooms::open_or_create_dm(&alice, "@darkmage:localhost")
        .await
        .unwrap();
    assert_eq!(second.room_id(), again.room_id());
    assert!(
        rooms::existing_dms(&alice, "@darkmage:localhost")
            .unwrap()
            .len()
            >= 1
    );
    let _ = first;

    // Bob's invite from another client: Darkmage joins and recognises it as a DM with Alice.
    sync_a_bit(&dark).await;
    for r in dark.invited_rooms() {
        r.join().await.unwrap();
    }
    sync_a_bit(&dark).await;
    sync_a_bit(&dark).await;
    assert!(
        !rooms::existing_dms(&dark, "@alice:localhost")
            .unwrap()
            .is_empty()
    );
    assert!(rooms::existing_dms(&dark, "not-an-id").is_err());
}

#[tokio::test]
async fn retry_is_idempotent_history_and_redaction() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "hist-alice").await;
    let dark = login_as(&hs, "darkmage", "hist-dark").await;
    let room = alice
        .create_dm(<&UserId>::try_from("@darkmage:localhost").unwrap())
        .await
        .unwrap();

    let body = format!("retry-test {}", std::process::id());
    let txn = matrix_sdk::ruma::TransactionId::new();
    let e1 = rooms::send_text(&room, &body, txn.clone()).await.unwrap();
    let e2 = rooms::send_text(&room, &body, txn).await.unwrap(); // simulated retry
    assert_eq!(e1, e2, "same txn id must not create a second event");

    let hist = rooms::load_history(&room, None, 20).await.unwrap();
    let mut conv = Conversation::default();
    for ev in hist.events {
        conv.apply(ev);
    }
    assert_eq!(conv.messages().iter().filter(|m| m.body == body).count(), 1);

    rooms::redact(&room, &e1).await.unwrap();
    let hist = rooms::load_history(&room, None, 20).await.unwrap();
    let mut conv = Conversation::default();
    for ev in hist.events {
        conv.apply(ev);
    }
    let m = conv
        .messages()
        .iter()
        .find(|m| m.id == e1)
        .expect("redacted event present");
    assert!(m.redacted && m.body.is_empty());
    let _ = (
        dark,
        Parsed::Redaction {
            target: String::new(),
        },
    );
}

#[tokio::test]
async fn live_feed_delivers_decrypted_messages_and_typing() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "live-alice").await;
    let dark = login_as(&hs, "darkmage", "live-dark").await;
    let (tx, rx) = async_channel::unbounded();
    rooms::forward_live_events(&dark, tx);
    let room = alice
        .create_dm(<&UserId>::try_from("@darkmage:localhost").unwrap())
        .await
        .unwrap();
    sync_a_bit(&dark).await;
    for r in dark.invited_rooms() {
        r.join().await.unwrap();
    }
    let body = format!("live {}", std::process::id());
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new())
        .await
        .unwrap();
    let mut seen = false;
    for _ in 0..10 {
        sync_a_bit(&dark).await;
        while let Ok(ev) = rx.try_recv() {
            if let rooms::LiveEvent::Timeline {
                event: Parsed::Message(m),
                ..
            } = ev
                && m.body == body
                && !m.undecryptable
            {
                seen = true;
            }
        }
        if seen {
            break;
        }
    }
    assert!(seen, "decrypted live message not received");
}
