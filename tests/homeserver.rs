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

#![recursion_limit = "256"]
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
use msm::matrix::ignore;

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
    // Never touch the real app's saved session.
    msm::matrix::secrets::set_namespace("com.matrixmessenger.Msm.test");
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
        !rooms::existing_dms(&alice, "@darkmage:localhost")
            .unwrap()
            .is_empty()
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
    let e1 = rooms::send_text(&room, &body, txn.clone(), None)
        .await
        .unwrap();
    let e2 = rooms::send_text(&room, &body, txn, None).await.unwrap(); // simulated retry
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
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new(), None)
        .await
        .unwrap();
    let mut seen = false;
    for _ in 0..10 {
        sync_a_bit(&dark).await;
        while let Ok(ev) = rx.try_recv() {
            if let rooms::LiveEvent::Timeline { event, .. } = ev
                && let Parsed::Message(m) = *event
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

/// The sync loop must carry our chosen presence, otherwise /sync flips us back to online.
#[tokio::test]
async fn sync_loop_keeps_chosen_presence() {
    use matrix_sdk::ruma::api::client::presence::get_presence;
    use matrix_sdk::ruma::presence::PresenceState;
    let Some(hs) = homeserver() else { return };
    let c = login_as(&hs, "frank", "pres-frank").await;
    let me = c.user_id().unwrap().to_owned();
    let (tx, _rx) = async_channel::unbounded();
    use msm::matrix::presence::{Desired, UiStatus};
    let (ptx, prx) = tokio::sync::watch::channel(Desired::new(UiStatus::Online, ""));
    let task = tokio::spawn(msm::matrix::synchronization::run(c.clone(), tx, prx));

    async fn seen(c: &Client, me: &matrix_sdk::ruma::OwnedUserId, want: PresenceState) -> bool {
        // Synapse can take well over 10s to apply a presence change.
        for _ in 0..90 {
            if let Ok(r) = c.send(get_presence::v3::Request::new(me.clone())).await
                && r.presence == want
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        false
    }

    assert!(seen(&c, &me, PresenceState::Online).await, "online");
    let set = |st: UiStatus| {
        let ptx = ptx.clone();
        async move {
            ptx.send_replace(Desired::new(st, ""));
        }
    };
    set(UiStatus::Offline).await;
    assert!(
        seen(&c, &me, PresenceState::Offline).await,
        "offline must stick across syncs"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        seen(&c, &me, PresenceState::Offline).await,
        "offline must not flip back"
    );
    set(UiStatus::Busy).await;
    assert!(seen(&c, &me, PresenceState::Unavailable).await, "away/busy");
    set(UiStatus::Online).await;
    assert!(seen(&c, &me, PresenceState::Online).await, "back online");
    task.abort();
}

#[tokio::test]
async fn group_room_invite_accept_reject_message_and_members() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "grp-alice").await;
    let bob = login_as(&hs, "bob", "grp-bob").await;
    let name = format!("Test room {}", std::process::id());

    assert!(rooms::create_group(&alice, "  ", &[]).await.is_err());
    assert!(
        rooms::create_group(&alice, "x", &["nope".into()])
            .await
            .is_err()
    );

    let room = rooms::create_group(&alice, &name, &["@bob:localhost".into()])
        .await
        .unwrap();

    // Bob sees the invitation, with inviter and name, and it is not a DM.
    let mut invite = None;
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        invite = rooms::pending_invites(&bob)
            .await
            .into_iter()
            .find(|i| i.room_id == room.room_id().as_str());
        if invite.is_some() {
            break;
        }
    }
    let invite = invite.expect("bob should see the invitation");
    assert_eq!(invite.inviter.as_deref(), Some("@alice:localhost"));
    assert_eq!(invite.name, name);
    assert!(!invite.is_direct);

    rooms::accept_invite(&bob, &invite.room_id).await.unwrap();
    sync_a_bit(&bob).await;
    sync_a_bit(&alice).await;
    assert!(
        room.encryption_state().is_encrypted(),
        "group rooms are created encrypted"
    );
    assert!(
        rooms::pending_invites(&bob)
            .await
            .iter()
            .all(|i| i.room_id != invite.room_id)
    );
    let bobs_groups = rooms::group_rooms(&bob);
    let joined = bobs_groups
        .iter()
        .find(|r| r.room_id() == room.room_id())
        .expect("group room listed");
    assert_eq!(rooms::room_title(joined).await, name);

    // Messages flow and are readable from history.
    let body = format!("group hello {}", std::process::id());
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new(), None)
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    let hist = rooms::load_history(joined, None, 20).await.unwrap();
    let mut conv = Conversation::default();
    for ev in hist.events {
        conv.apply(ev);
    }
    let m = conv
        .messages()
        .iter()
        .find(|m| m.body == body)
        .expect("bob reads alice's message");
    assert_eq!(m.sender, "@alice:localhost");

    // Members carry their own (public/room) names; local nicknames are not involved.
    let names = rooms::member_names(&room).await.unwrap();
    assert!(names.iter().any(|(id, _)| id == "@alice:localhost"));
    assert!(names.iter().any(|(id, _)| id == "@bob:localhost"));

    // Inviting more people; bad IDs are rejected before any request.
    assert!(rooms::invite_user(&room, "bad").await.is_err());
    rooms::invite_user(&room, "@darkmage:localhost")
        .await
        .unwrap();

    // Declining an invitation removes it.
    let other = rooms::create_group(
        &alice,
        &format!("{name} (decline)"),
        &["@bob:localhost".into()],
    )
    .await
    .unwrap();
    let mut seen = false;
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        if rooms::pending_invites(&bob)
            .await
            .iter()
            .any(|i| i.room_id == other.room_id().as_str())
        {
            seen = true;
            break;
        }
    }
    assert!(seen);
    rooms::leave_room(&bob, other.room_id().as_str())
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    assert!(
        rooms::pending_invites(&bob)
            .await
            .iter()
            .all(|i| i.room_id != other.room_id().as_str())
    );

    // Leaving a joined room.
    rooms::leave_room(&bob, room.room_id().as_str())
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    assert!(
        rooms::group_rooms(&bob)
            .iter()
            .all(|r| r.room_id() != room.room_id())
    );
}

/// Register a brand-new user (a user can only have one key backup, so reruns need fresh accounts).
async fn register_fresh(hs: &str) -> (Client, String, String) {
    use matrix_sdk::ruma::api::client::account::register;
    use matrix_sdk::ruma::api::client::uiaa::{AuthData, Dummy};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("sec{}x{}", std::process::id(), nanos % 1_000_000_000);
    let pw = format!("pw-{name}-123");
    let client = Client::builder().homeserver_url(hs).build().await.unwrap();
    let mut req = register::v3::Request::new();
    req.username = Some(name.clone());
    req.password = Some(pw.clone());
    req.auth = Some(AuthData::Dummy(Dummy::new()));
    client.matrix_auth().register(req).await.expect("register");
    (client, name, pw)
}

#[tokio::test]
async fn recovery_key_lets_a_new_device_read_old_encrypted_history() {
    use msm::matrix::encryption::{self, Recovery};
    let Some(hs) = homeserver() else { return };
    let (a, name, pw) = register_fresh(&hs).await;
    sync_a_bit(&a).await;

    // Wrong password must fail cleanly before anything is set up.
    let before = encryption::status(&a).await;
    assert!(!before.cross_signing_ready);
    assert_eq!(before.recovery, Recovery::Disabled);

    let key = encryption::setup_recovery(&a, &pw)
        .await
        .expect("setup recovery");
    assert!(
        key.split_whitespace().count() >= 4,
        "recovery key should look like a grouped key"
    );
    let after = encryption::status(&a).await;
    assert!(after.cross_signing_ready, "{after:?}");
    assert_eq!(after.recovery, Recovery::Enabled);
    assert!(after.backup_enabled);

    // An encrypted message sent *before* the second device exists.
    let room = rooms::create_group(&a, "Secret history", &[])
        .await
        .unwrap();
    sync_a_bit(&a).await;
    let body = format!("old secret {name}");
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new(), None)
        .await
        .unwrap();
    sync_a_bit(&a).await;
    a.encryption()
        .backups()
        .wait_for_steady_state()
        .await
        .unwrap();

    // Device B: same user, brand new device.
    let b = login_as_with(&hs, &name, &pw, "sec-b").await;
    for _ in 0..3 {
        sync_a_bit(&b).await;
    }
    let st = encryption::status(&b).await;
    assert!(!st.this_device_verified, "a new device starts unverified");
    assert_eq!(encryption::own_devices(&b).await.unwrap().len(), 2);
    assert!(
        encryption::recover(&b, "definitely not the key")
            .await
            .is_err()
    );

    let read = |b: Client, room_id: matrix_sdk::ruma::OwnedRoomId, body: String| async move {
        let room = b.get_room(&room_id)?;
        let h = rooms::load_history(&room, None, 20).await.ok()?;
        h.events.into_iter().find_map(|e| match e {
            Parsed::Message(m) if !m.undecryptable && m.body == body => Some(m),
            _ => None,
        })
    };
    assert!(
        read(b.clone(), room.room_id().to_owned(), body.clone())
            .await
            .is_none(),
        "without the recovery key the old message must stay unreadable"
    );

    encryption::recover(&b, &key)
        .await
        .expect("recover with the real key");
    assert_eq!(encryption::status(&b).await.recovery, Recovery::Enabled);
    b.encryption()
        .backups()
        .download_room_keys_for_room(room.room_id())
        .await
        .unwrap();
    let mut got = None;
    for _ in 0..20 {
        sync_a_bit(&b).await;
        got = read(b.clone(), room.room_id().to_owned(), body.clone()).await;
        if got.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        got.is_some(),
        "after recovery the new device reads the old message"
    );

    // Replacing the key: the old one stops working on a fresh device, the new one works.
    let new_key = encryption::reset_recovery_key(&b)
        .await
        .expect("reset recovery key");
    assert_ne!(new_key, key);
    let c = login_as_with(&hs, &name, &pw, "sec-c").await;
    for _ in 0..3 {
        sync_a_bit(&c).await;
    }
    assert!(
        encryption::recover(&c, &key).await.is_err(),
        "the replaced key must not unlock secret storage"
    );
    encryption::recover(&c, &new_key)
        .await
        .expect("the new key works");

    // Turning recovery off removes it from the server.
    encryption::disable_recovery(&b).await.expect("disable");
    assert_eq!(encryption::status(&b).await.recovery, Recovery::Disabled);
    let d = login_as_with(&hs, &name, &pw, "sec-d").await;
    for _ in 0..3 {
        sync_a_bit(&d).await;
    }
    assert!(encryption::recover(&d, &new_key).await.is_err());
}

async fn login_as_with(hs: &str, user: &str, pw: &str, tag: &str) -> Client {
    login_unsaved(hs, user, pw, &tmp(tag)).await.unwrap().0
}

async fn history_of(room: &matrix_sdk::Room) -> Conversation {
    let h = rooms::load_history(room, None, 50).await.unwrap();
    let mut c = Conversation::default();
    for e in h.events {
        c.apply(e);
    }
    c
}

#[tokio::test]
async fn edits_and_reactions_between_two_users() {
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "er-alice").await;
    let bob = login_as(&hs, "bob", "er-bob").await;
    let room = rooms::create_group(
        &alice,
        &format!("Edits {}", std::process::id()),
        &["@bob:localhost".into()],
    )
    .await
    .unwrap();
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        if !bob.invited_rooms().is_empty() {
            break;
        }
    }
    for r in bob.invited_rooms() {
        r.join().await.unwrap();
    }
    sync_a_bit(&bob).await;
    // Alice must learn Bob joined (and his devices) before sending, or she won't share the room key.
    sync_a_bit(&alice).await;
    let broom = bob.get_room(room.room_id()).expect("bob joined");

    let id = rooms::send_text(
        &room,
        "teh original",
        matrix_sdk::ruma::TransactionId::new(),
        None,
    )
    .await
    .unwrap();
    sync_a_bit(&alice).await;
    // Bob receives the room key via sync; reading history before that finds only undecryptable events.
    sync_a_bit(&bob).await;

    // Author edits: everyone sees the new text, marked edited.
    rooms::send_edit(&room, &id, "the original").await.unwrap();
    // Someone else "edits" it: the server accepts the event, clients must ignore it.
    rooms::send_edit(&broom, &id, "bob was here").await.unwrap();
    let conv = history_of(&broom).await;
    let m = conv
        .messages()
        .iter()
        .find(|m| m.id == id)
        .expect("message");
    assert_eq!(m.body, "the original");
    assert!(m.edited);
    assert_ne!(
        m.trust,
        msm::matrix::messages::Trust::Unencrypted,
        "decrypted messages carry their trust level"
    );

    // Reactions: grouped, attributed, removable.
    let r1 = rooms::send_reaction(&broom, &id, "👍").await.unwrap();
    rooms::send_reaction(&room, &id, "👍").await.unwrap();
    let conv = history_of(&room).await;
    let rs = conv.reactions_for(&id);
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].key, "👍");
    assert_eq!(rs[0].senders.len(), 2);
    assert_eq!(
        conv.own_reaction_event(&id, "👍", "@bob:localhost")
            .as_deref(),
        Some(r1.as_str())
    );

    rooms::redact(&broom, &r1).await.unwrap();
    let conv = history_of(&room).await;
    let rs = conv.reactions_for(&id);
    assert_eq!(rs[0].senders, ["@alice:localhost"]);

    // Replies: the relation survives the encrypted round trip, and search finds by text.
    let reply_id = rooms::send_text(
        &broom,
        "agreed, thanks",
        matrix_sdk::ruma::TransactionId::new(),
        Some(&id),
    )
    .await
    .unwrap();
    // Alice needs Bob's room key (to-device) before she can read his message.
    sync_a_bit(&alice).await;
    let conv = history_of(&room).await;
    let reply = conv.find(&reply_id).expect("reply visible to alice");
    assert_eq!(reply.reply_to.as_deref(), Some(id.as_str()));
    assert!(reply.matches("THANKS"));
    assert!(conv.find(&id).unwrap().reply_to.is_none());
}

#[tokio::test]
async fn attachments_round_trip_encrypted() {
    use msm::matrix::media;
    use msm::matrix::messages::AttachmentKind;
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "att-alice").await;
    let bob = login_as(&hs, "bob", "att-bob").await;
    let room = rooms::create_group(
        &alice,
        &format!("Files {}", std::process::id()),
        &["@bob:localhost".into()],
    )
    .await
    .unwrap();
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        if !bob.invited_rooms().is_empty() {
            break;
        }
    }
    for r in bob.invited_rooms() {
        r.join().await.unwrap();
    }
    sync_a_bit(&bob).await;
    sync_a_bit(&alice).await;
    assert!(room.encryption_state().is_encrypted());

    // A binary file with a hostile-looking name on disk is sent under a safe name.
    let dir = tmp("attach");
    std::fs::create_dir_all(&dir).unwrap();
    let payload: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    let path = dir.join("data.bin");
    std::fs::write(&path, &payload).unwrap();
    let png = dir.join("pixel.png");
    // 1x1 PNG
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0, 5, 0, 1, 0xff, 0x89, 0x99,
        0x3d, 0x1d, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    std::fs::write(&png, &png_bytes).unwrap();

    let id_file = media::send_file(&room, &path).await.unwrap();
    let id_png = media::send_file(&room, &png).await.unwrap();
    assert!(
        media::send_file(&room, &dir.join("missing.bin"))
            .await
            .is_err()
    );

    // Progress is reported while uploading, and a cancel abandons the upload.
    let (ptx, prx) = async_channel::unbounded();
    let id_progress = media::send_file_with(
        &room,
        &path,
        media::Transfer {
            progress: Some(ptx),
            cancel: None,
        },
    )
    .await
    .unwrap();
    let mut last = None;
    while let Ok(p) = prx.try_recv() {
        last = Some(p);
    }
    let (sent, total) = last.expect("progress was reported");
    assert!(total > 0 && sent <= total);
    let (ctx, crx) = async_channel::bounded(1);
    ctx.send(()).await.unwrap();
    let cancelled = media::send_file_with(
        &room,
        &dir.join("pixel.png"),
        media::Transfer {
            progress: None,
            cancel: Some(crx),
        },
    )
    .await;
    assert!(matches!(cancelled, Err(media::MediaError::Cancelled)));

    sync_a_bit(&bob).await;
    let broom = bob.get_room(room.room_id()).unwrap();
    let conv = history_of(&broom).await;
    assert!(conv.messages().iter().any(|m| m.id == id_progress));
    assert_eq!(
        conv.messages()
            .iter()
            .filter(|m| m
                .attachment
                .as_ref()
                .is_some_and(|a| a.filename == "pixel.png"))
            .count(),
        1,
        "the cancelled upload must not have produced a message"
    );
    let get = |id: &str| {
        conv.messages()
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .expect("message")
    };

    let mf = get(&id_file);
    let att = mf.attachment.clone().expect("file attachment");
    assert_eq!(att.kind, AttachmentKind::File);
    assert_eq!(att.filename, "data.bin");
    assert_eq!(att.size, Some(payload.len() as u64));
    let bytes = media::download(&bob, &att.source, att.size).await.unwrap();
    assert_eq!(bytes, payload, "decrypted download must match the upload");

    let mp = get(&id_png);
    let att = mp.attachment.clone().expect("image attachment");
    assert_eq!(att.kind, AttachmentKind::Image);
    assert_eq!(
        media::download(&bob, &att.source, att.size).await.unwrap(),
        png_bytes
    );

    // Oversized declared downloads are refused before any request.
    assert!(
        media::download(&bob, &att.source, Some(media::MAX_DOWNLOAD_BYTES + 1))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn avatar_set_fetch_and_remove() {
    let Some(hs) = homeserver() else { return };
    use msm::matrix::avatars;
    let alice = login_as(&hs, "alice", "av-alice").await;
    let bob = login_as(&hs, "bob", "av-bob").await;
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    avatars::remove_own_avatar(&alice).await.unwrap();
    assert!(
        avatars::user_avatar(&bob, "@alice:localhost")
            .await
            .is_none()
    );

    avatars::set_own_avatar(&alice, &"image/png".parse().unwrap(), png)
        .await
        .unwrap();
    let got = avatars::user_avatar(&bob, "@alice:localhost")
        .await
        .expect("bob sees alice's avatar");
    assert!(got.len() > 8, "thumbnail bytes");

    // Oversized uploads are refused locally, before any request.
    let huge = vec![0u8; avatars::MAX_UPLOAD_BYTES + 1];
    assert!(
        avatars::set_own_avatar(&alice, &"image/png".parse().unwrap(), huge)
            .await
            .is_err()
    );

    avatars::remove_own_avatar(&alice).await.unwrap();
    assert!(
        avatars::user_avatar(&bob, "@alice:localhost")
            .await
            .is_none()
    );
}

async fn next_ui(
    f: &mut msm::matrix::verification::Flow,
    want: impl Fn(&msm::matrix::verification::VerifyUi) -> bool,
) -> msm::matrix::verification::VerifyUi {
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(u) = f.ui.recv().await {
            if want(&u) {
                return u;
            }
        }
        panic!("flow ended early");
    })
    .await
    .expect("timed out waiting for verification state")
}

#[tokio::test]
async fn emoji_verification_between_two_users() {
    use msm::matrix::encryption;
    use msm::matrix::verification::{self, VerifyCmd, VerifyUi};
    let Some(hs) = homeserver() else { return };
    let (alice, _, apw) = register_fresh(&hs).await;
    let (bob, _, bpw) = register_fresh(&hs).await;
    encryption::setup_recovery(&alice, &apw).await.unwrap();
    encryption::setup_recovery(&bob, &bpw).await.unwrap();
    let bob_id = bob.user_id().unwrap().to_string();

    let (tx, rx) = async_channel::unbounded();
    verification::watch_incoming(&bob, tx);
    for c in [&alice, &bob] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    // The request travels in a DM, which Bob has to accept first (as a user would).
    let b2 = bob.clone();
    tokio::spawn(async move {
        loop {
            for r in b2.invited_rooms() {
                let _ = r.join().await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // Each side must already know the other's keys, as it would after any shared room.
    bob.encryption()
        .request_user_identity(alice.user_id().unwrap())
        .await
        .unwrap();
    let req = verification::request(&alice, &bob_id).await.unwrap();
    let mut af = verification::run(req);
    let incoming = tokio::time::timeout(Duration::from_secs(60), rx.recv())
        .await
        .expect("bob sees the request")
        .unwrap();
    let mut bf = verification::run(incoming);
    assert!(!bf.self_verification);
    next_ui(&mut bf, |u| *u == VerifyUi::Requested).await;
    bf.cmd.send(VerifyCmd::Accept).unwrap();
    next_ui(&mut af, |u| matches!(u, VerifyUi::Ready { .. })).await;
    af.cmd.send(VerifyCmd::StartEmoji).unwrap();

    let VerifyUi::Emojis(ea) = next_ui(&mut af, |u| matches!(u, VerifyUi::Emojis(_))).await else {
        unreachable!()
    };
    let VerifyUi::Emojis(eb) = next_ui(&mut bf, |u| matches!(u, VerifyUi::Emojis(_))).await else {
        unreachable!()
    };
    assert_eq!(ea.len(), 7);
    assert_eq!(ea, eb, "both sides must show the same emoji");

    // Nothing is verified until a person confirms.
    let id = alice
        .encryption()
        .get_user_identity(bob.user_id().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!id.is_verified());
    af.cmd.send(VerifyCmd::Match).unwrap();
    bf.cmd.send(VerifyCmd::Match).unwrap();
    next_ui(&mut af, |u| *u == VerifyUi::Done).await;
    next_ui(&mut bf, |u| *u == VerifyUi::Done).await;
    // Trust is recorded just after the flow reports Done.
    let mut trusted = false;
    for _ in 0..40 {
        let id = alice
            .encryption()
            .get_user_identity(bob.user_id().unwrap())
            .await
            .unwrap()
            .unwrap();
        if id.is_verified() {
            trusted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(trusted, "alice now trusts bob");
}

/// Paint a QR grid into a 640x480 grayscale frame, as a camera pointed at a screen would see it.
fn frame_from_grid(grid: &[Vec<bool>]) -> Vec<u8> {
    let (w, h, scale) = (640usize, 480usize, 8usize);
    let n = grid.len();
    let (ox, oy) = ((w - n * scale) / 2, (h - n * scale) / 2);
    let mut f = vec![255u8; w * h];
    for (y, row) in grid.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        f[(oy + y * scale + dy) * w + ox + x * scale + dx] = 0;
                    }
                }
            }
        }
    }
    f
}

#[tokio::test]
async fn qr_verification_between_two_users() {
    use msm::matrix::encryption;
    use msm::matrix::verification::{self, VerifyCmd, VerifyUi};
    let Some(hs) = homeserver() else { return };
    let (alice, _, apw) = register_fresh(&hs).await;
    let (bob, _, bpw) = register_fresh(&hs).await;
    encryption::setup_recovery(&alice, &apw).await.unwrap();
    encryption::setup_recovery(&bob, &bpw).await.unwrap();
    let bob_id = bob.user_id().unwrap().to_string();
    let (tx, rx) = async_channel::unbounded();
    verification::watch_incoming(&bob, tx);
    for c in [&alice, &bob] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    let b2 = bob.clone();
    tokio::spawn(async move {
        loop {
            for r in b2.invited_rooms() {
                let _ = r.join().await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    bob.encryption()
        .request_user_identity(alice.user_id().unwrap())
        .await
        .unwrap();
    let mut af = verification::run(verification::request(&alice, &bob_id).await.unwrap());
    let incoming = tokio::time::timeout(Duration::from_secs(60), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let mut bf = verification::run(incoming);
    next_ui(&mut bf, |u| *u == VerifyUi::Requested).await;
    bf.cmd.send(VerifyCmd::Accept).unwrap();

    let VerifyUi::Ready {
        qr: Some(alice_grid),
    } = next_ui(&mut af, |u| matches!(u, VerifyUi::Ready { .. })).await
    else {
        panic!("alice should be offered a QR code")
    };
    next_ui(&mut bf, |u| matches!(u, VerifyUi::Ready { .. })).await;

    // Bob "points the camera" at Alice's code: grid -> frame -> decoder -> scan.
    let frame = frame_from_grid(&alice_grid);
    let payload = msm::camera::decode_frame(640, 480, &frame).expect("code is readable");
    // Nothing is verified by merely scanning.
    bf.cmd.send(VerifyCmd::ScanQr(payload)).unwrap();
    next_ui(&mut af, |u| *u == VerifyUi::QrScanned).await;
    let id = bob
        .encryption()
        .get_user_identity(alice.user_id().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!id.is_verified(), "scanning alone verifies nothing");

    // Alice confirms that Bob scanned it; both finish.
    af.cmd.send(VerifyCmd::Match).unwrap();
    next_ui(&mut af, |u| *u == VerifyUi::Done).await;
    next_ui(&mut bf, |u| *u == VerifyUi::Done).await;
    let mut trusted = false;
    for _ in 0..40 {
        let id = bob
            .encryption()
            .get_user_identity(alice.user_id().unwrap())
            .await
            .unwrap()
            .unwrap();
        if id.is_verified() {
            trusted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(trusted, "bob now trusts alice");
}

/// The device that accepted a request may be the one that picks emoji.
#[tokio::test]
async fn self_verification_emoji_started_by_the_accepting_device() {
    use msm::matrix::encryption;
    use msm::matrix::verification::{self, VerifyCmd, VerifyUi};
    let Some(hs) = homeserver() else { return };
    let (old, name, pw) = register_fresh(&hs).await;
    encryption::setup_recovery(&old, &pw).await.unwrap();
    let new = login_as_with(&hs, &name, &pw, "selfv-new").await;
    let (tx, rx) = async_channel::unbounded();
    verification::watch_incoming(&old, tx);
    for c in [&old, &new] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    // The old device can only act on a request from a device whose keys it already knows.
    let new_id = new.device_id().unwrap().to_owned();
    let me = old.user_id().unwrap().to_owned();
    let mut known = false;
    for _ in 0..60 {
        let _ = old.encryption().request_user_identity(&me).await;
        if old
            .encryption()
            .get_device(&me, &new_id)
            .await
            .unwrap()
            .is_some()
        {
            known = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(known, "old device learns about the new one");
    let mut nf = verification::run(verification::request_self(&new).await.unwrap());
    let incoming = tokio::time::timeout(Duration::from_secs(60), rx.recv())
        .await
        .expect("the old device sees the request")
        .unwrap();
    let mut of = verification::run(incoming);
    assert!(of.self_verification);
    next_ui(&mut of, |u| *u == VerifyUi::Requested).await;
    of.cmd.send(VerifyCmd::Accept).unwrap();
    next_ui(&mut nf, |u| matches!(u, VerifyUi::Ready { .. })).await;
    next_ui(&mut of, |u| matches!(u, VerifyUi::Ready { .. })).await;
    of.cmd.send(VerifyCmd::StartEmoji).unwrap();
    let VerifyUi::Emojis(a) = next_ui(&mut of, |u| matches!(u, VerifyUi::Emojis(_))).await else {
        unreachable!()
    };
    let VerifyUi::Emojis(b) = next_ui(&mut nf, |u| matches!(u, VerifyUi::Emojis(_))).await else {
        unreachable!()
    };
    assert_eq!(a, b);
}

/// The trusted device asks the brand-new one to verify (the new device only accepts).
#[tokio::test]
async fn self_verification_requested_by_the_trusted_device() {
    use msm::matrix::encryption;
    use msm::matrix::verification::{self, VerifyCmd, VerifyUi};
    let Some(hs) = homeserver() else { return };
    let (old, name, pw) = register_fresh(&hs).await;
    encryption::setup_recovery(&old, &pw).await.unwrap();
    let new = login_as_with(&hs, &name, &pw, "selfv-new2").await;
    let (tx, rx) = async_channel::unbounded();
    verification::watch_incoming(&new, tx);
    for c in [&old, &new] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    let new_id = new.device_id().unwrap().to_owned();
    let me = old.user_id().unwrap().to_owned();
    let mut known = false;
    for _ in 0..60 {
        let _ = old.encryption().request_user_identity(&me).await;
        if old
            .encryption()
            .get_device(&me, &new_id)
            .await
            .unwrap()
            .is_some()
        {
            known = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(known);
    let mut of = verification::run(verification::request_self(&old).await.unwrap());
    let incoming = tokio::time::timeout(Duration::from_secs(30), rx.recv())
        .await
        .expect("the new device sees the request")
        .unwrap();
    let mut nf = verification::run(incoming);
    next_ui(&mut nf, |u| *u == VerifyUi::Requested).await;
    nf.cmd.send(VerifyCmd::Accept).unwrap();
    next_ui(&mut of, |u| matches!(u, VerifyUi::Ready { .. })).await;
    next_ui(&mut nf, |u| matches!(u, VerifyUi::Ready { .. })).await;
    nf.cmd.send(VerifyCmd::StartEmoji).unwrap();
    next_ui(&mut nf, |u| matches!(u, VerifyUi::Emojis(_))).await;
    next_ui(&mut of, |u| matches!(u, VerifyUi::Emojis(_))).await;
}

#[tokio::test]
async fn emoji_mismatch_cancels_without_verifying() {
    use msm::matrix::encryption;
    use msm::matrix::verification::{self, VerifyCmd, VerifyUi};
    let Some(hs) = homeserver() else { return };
    let (alice, _, apw) = register_fresh(&hs).await;
    let (bob, _, bpw) = register_fresh(&hs).await;
    encryption::setup_recovery(&alice, &apw).await.unwrap();
    encryption::setup_recovery(&bob, &bpw).await.unwrap();
    let bob_id = bob.user_id().unwrap().to_string();
    let (tx, rx) = async_channel::unbounded();
    verification::watch_incoming(&bob, tx);
    for c in [&alice, &bob] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    // The request travels in a DM, which Bob has to accept first (as a user would).
    let b2 = bob.clone();
    tokio::spawn(async move {
        loop {
            for r in b2.invited_rooms() {
                let _ = r.join().await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    bob.encryption()
        .request_user_identity(alice.user_id().unwrap())
        .await
        .unwrap();
    let mut af = verification::run(verification::request(&alice, &bob_id).await.unwrap());
    let incoming = tokio::time::timeout(Duration::from_secs(60), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let mut bf = verification::run(incoming);
    next_ui(&mut bf, |u| *u == VerifyUi::Requested).await;
    bf.cmd.send(VerifyCmd::Accept).unwrap();
    next_ui(&mut af, |u| matches!(u, VerifyUi::Ready { .. })).await;
    af.cmd.send(VerifyCmd::StartEmoji).unwrap();
    next_ui(&mut af, |u| matches!(u, VerifyUi::Emojis(_))).await;
    next_ui(&mut bf, |u| matches!(u, VerifyUi::Emojis(_))).await;
    af.cmd.send(VerifyCmd::Mismatch).unwrap();
    next_ui(&mut af, |u| matches!(u, VerifyUi::Cancelled(_))).await;
    next_ui(&mut bf, |u| matches!(u, VerifyUi::Cancelled(_))).await;
    let id = alice
        .encryption()
        .get_user_identity(bob.user_id().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(!id.is_verified());
}

#[tokio::test]
async fn friend_request_accept_and_remove_between_two_users() {
    use msm::contacts::contact::Friendship;
    use msm::contacts::contact_manager::{ContactManager, FriendAction};
    use msm::matrix::friendship;
    let Some(hs) = homeserver() else { return };
    let (alice, _, _) = register_fresh(&hs).await;
    let (bob, _, _) = register_fresh(&hs).await;
    let (aid, bid) = (
        alice.user_id().unwrap().to_string(),
        bob.user_id().unwrap().to_string(),
    );
    let (atx, arx) = async_channel::unbounded();
    let (btx, brx) = async_channel::unbounded();
    friendship::watch(&alice, atx);
    friendship::watch(&bob, btx);
    for c in [&alice, &bob] {
        let c = c.clone();
        tokio::spawn(async move {
            let _ = c.sync(SyncSettings::default()).await;
        });
    }
    let recv = |rx: &async_channel::Receiver<(String, FriendAction)>| {
        let rx = rx.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(30), rx.recv())
                .await
                .expect("friend event not delivered")
                .unwrap()
        }
    };

    let (mut am, mut bm) = (ContactManager::default(), ContactManager::default());
    let act = am.request_friend(&bid, 1).unwrap();
    friendship::send(&alice, &bid, act).await.unwrap();
    assert_eq!(am.friendship(&bid), Friendship::Outgoing);

    let (from, action) = recv(&brx).await;
    assert_eq!(
        (from.as_str(), action),
        (aid.as_str(), FriendAction::Request)
    );
    assert_eq!(bm.apply_friend_event(&from, action, 2), None);
    assert_eq!(bm.friendship(&aid), Friendship::Incoming);
    // A friend request must not look like a room invitation to anyone.
    assert!(bob.invited_rooms().is_empty());
    assert!(bob.rooms().is_empty());

    bm.accept_friend(&aid, 3).unwrap();
    friendship::send(&bob, &aid, FriendAction::Accept)
        .await
        .unwrap();
    let (from, action) = recv(&arx).await;
    assert_eq!(
        (from.as_str(), action),
        (bid.as_str(), FriendAction::Accept)
    );
    am.apply_friend_event(&from, action, 4);
    assert_eq!(am.friendship(&bid), Friendship::Friends);
    assert_eq!(bm.friendship(&aid), Friendship::Friends);

    // Removal reaches the other side.
    am.end_friendship(&bid, 5).unwrap();
    friendship::send(&alice, &bid, FriendAction::Remove)
        .await
        .unwrap();
    let (from, action) = recv(&brx).await;
    bm.apply_friend_event(&from, action, 6);
    assert_eq!(bm.friendship(&aid), Friendship::None);
    // The contact itself survives: friendship is separate from having a contact.
    assert!(bm.get(&aid).is_some());
}

#[tokio::test]
async fn three_accounts_in_a_group_keep_independent_nicknames() {
    use matrix_sdk::ruma::api::client::profile::DisplayName;
    let Some(hs) = homeserver() else { return };
    let (alice, _, _) = register_fresh(&hs).await;
    let (bob, _, _) = register_fresh(&hs).await;
    let (carol, _, _) = register_fresh(&hs).await;
    let id = |c: &Client| c.user_id().unwrap().to_string();
    let (a, b, c) = (id(&alice), id(&bob), id(&carol));
    for (cl, n) in [(&alice, "Alice A"), (&bob, "Bob B"), (&carol, "Carol C")] {
        cl.account().set_display_name(Some(n)).await.unwrap();
    }

    let room = rooms::create_group(&alice, "Trio", &[b.clone(), c.clone()])
        .await
        .unwrap();
    for cl in [&bob, &carol] {
        for _ in 0..10 {
            sync_a_bit(cl).await;
            if let Some(i) = rooms::pending_invites(cl).await.into_iter().next() {
                rooms::accept_invite(cl, &i.room_id).await.unwrap();
                break;
            }
        }
    }
    for (cl, who) in [(&alice, "alice"), (&bob, "bob"), (&carol, "carol")] {
        sync_a_bit(cl).await;
        let r = cl.get_room(room.room_id()).expect("joined");
        rooms::send_text(
            &r,
            &format!("hi from {who}"),
            matrix_sdk::ruma::TransactionId::new(),
            None,
        )
        .await
        .unwrap();
    }
    for cl in [&alice, &bob, &carol] {
        sync_a_bit(cl).await;
    }

    // Each viewer names the other two differently, and also renames one of them
    // so the three views disagree about every person.
    let nick = |viewer: &Client, others: [(&str, &str); 2]| {
        let mut m = ContactManager::default();
        for (mx, n) in others {
            m.add(mx, 1).unwrap();
            m.set_nickname(mx, Some(n), 2).unwrap();
        }
        let v = viewer.clone();
        let doc = m.document().clone();
        async move {
            let merged = contacts_sync::sync(&v.account(), &doc).await.unwrap();
            ContactManager::new(merged)
        }
    };
    let ma = nick(&alice, [(&b, "Alice's Bob"), (&c, "Alice's Carol")]).await;
    let mb = nick(&bob, [(&a, "Bob's Alice"), (&c, "Bob's Carol")]).await;
    let mc = nick(&carol, [(&a, "Carol's Alice"), (&b, "Carol's Bob")]).await;

    // Every viewer reads the same Matrix history but resolves names privately.
    for (viewer, mgr, expect) in [
        (&alice, &ma, [(&b, "Alice's Bob"), (&c, "Alice's Carol")]),
        (&bob, &mb, [(&a, "Bob's Alice"), (&c, "Bob's Carol")]),
        (&carol, &mc, [(&a, "Carol's Alice"), (&b, "Carol's Bob")]),
    ] {
        let r = viewer.get_room(room.room_id()).unwrap();
        let conv = history_of(&r).await;
        assert_eq!(conv.messages().len(), 3, "everyone sees all three messages");
        for (mx, want) in expect {
            assert!(conv.messages().iter().any(|m| &m.sender == mx));
            assert_eq!(mgr.resolver().resolve(mx, None), want);
        }
    }

    // A nickname is stored only in its owner's account data, never in anyone else's...
    for (viewer, foreign) in [
        (&alice, ["Bob's", "Carol's"]),
        (&bob, ["Alice's", "Carol's"]),
        (&carol, ["Alice's", "Bob's"]),
    ] {
        let (doc, _) = contacts_sync::fetch(&viewer.account()).await.unwrap();
        let blob = serde_json::to_string(&doc.to_value()).unwrap();
        for f in foreign {
            assert!(!blob.contains(&format!("{f} ")), "{blob}");
        }
    }
    // ...and the public profiles and room member names are untouched.
    for (mx, want) in [(&a, "Alice A"), (&b, "Bob B"), (&c, "Carol C")] {
        let p = carol
            .account()
            .fetch_user_profile_of(<&UserId>::try_from(mx.as_str()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            p.get_static::<DisplayName>().unwrap().as_deref(),
            Some(want)
        );
    }
    let names = rooms::member_names(&room).await.unwrap();
    for (mx, want) in [(&a, "Alice A"), (&b, "Bob B"), (&c, "Carol C")] {
        assert!(
            names
                .iter()
                .any(|(i, n)| i == mx && n.as_deref() == Some(want)),
            "{names:?}"
        );
    }
}

#[tokio::test]
async fn kick_ban_unban_and_room_settings_respect_power_levels() {
    use msm::matrix::rooms::MemberState;
    let Some(hs) = homeserver() else { return };
    let (alice, _, _) = register_fresh(&hs).await;
    let (bob, _, _) = register_fresh(&hs).await;
    let (carol, _, _) = register_fresh(&hs).await;
    let id = |c: &Client| c.user_id().unwrap().to_string();
    let (b, c) = (id(&bob), id(&carol));
    let room = rooms::create_group(&alice, "Moderated", &[b.clone(), c.clone()])
        .await
        .unwrap();
    for cl in [&bob, &carol] {
        for _ in 0..10 {
            sync_a_bit(cl).await;
            if let Some(i) = rooms::pending_invites(cl).await.into_iter().next() {
                rooms::accept_invite(cl, &i.room_id).await.unwrap();
                break;
            }
        }
    }
    sync_a_bit(&alice).await;
    sync_a_bit(&bob).await;

    // The creator may moderate and edit; an ordinary member may not.
    let s = rooms::room_settings(&room).await.unwrap();
    assert!(s.can_rename && s.can_set_topic);
    let row = |s: &rooms::RoomSettings, m: &str| s.members.iter().find(|r| r.mxid == m).cloned();
    let carol_row = row(&s, &c).unwrap();
    assert!(carol_row.can_kick && carol_row.can_ban);
    assert!(
        !row(&s, &id(&alice)).unwrap().can_kick,
        "never offered on ourselves"
    );
    let broom = bob.get_room(room.room_id()).unwrap();
    let bs = rooms::room_settings(&broom).await.unwrap();
    assert!(!bs.can_rename);
    assert!(row(&bs, &c).is_some_and(|r| !r.can_kick && !r.can_ban));
    assert!(rooms::kick(&broom, &c, "").await.is_err());
    assert!(rooms::set_room_name(&broom, "hijack").await.is_err());

    // Roles: the creator may promote, an ordinary member may not.
    assert!(carol_row.can_set_role);
    assert!(!row(&s, &id(&alice)).unwrap().can_set_role);
    assert!(!row(&bs, &c).unwrap().can_set_role);
    assert!(rooms::set_role(&broom, &c, 50).await.is_err());
    rooms::set_role(&room, &b, 50).await.unwrap();
    sync_a_bit(&alice).await;
    sync_a_bit(&bob).await;
    let s = rooms::room_settings(&room).await.unwrap();
    assert_eq!(row(&s, &b).unwrap().power, 50);
    // A moderator can now kick but not promote above themselves, nor touch the admin.
    let bs = rooms::room_settings(&broom).await.unwrap();
    assert!(row(&bs, &c).unwrap().can_kick);
    assert!(!row(&bs, &id(&alice)).unwrap().can_set_role);
    assert!(rooms::set_role(&broom, &c, 100).await.is_err());
    rooms::set_role(&room, &b, 0).await.unwrap();
    sync_a_bit(&alice).await;
    let s = rooms::room_settings(&room).await.unwrap();
    assert_eq!(row(&s, &b).unwrap().power, 0, "back to member");

    // Arbitrary levels work too.
    rooms::set_role(&room, &b, 25).await.unwrap();
    sync_a_bit(&alice).await;
    let s = rooms::room_settings(&room).await.unwrap();
    assert_eq!(row(&s, &b).unwrap().power, 25);
    rooms::set_role(&room, &b, 0).await.unwrap();
    sync_a_bit(&alice).await;

    // Room picture: only those allowed to send m.room.avatar, and it reaches members.
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    let mime = "image/png".parse().unwrap();
    assert!(rooms::room_settings(&room).await.unwrap().can_set_avatar);
    sync_a_bit(&bob).await;
    assert!(!rooms::room_settings(&broom).await.unwrap().can_set_avatar);
    assert!(
        msm::matrix::avatars::set_room_avatar(&broom, &mime, png.clone())
            .await
            .is_err()
    );
    msm::matrix::avatars::set_room_avatar(&room, &mime, png)
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    assert!(broom.avatar_url().is_some(), "members see the room picture");
    assert!(
        msm::matrix::avatars::room_avatar(&broom).await.is_some(),
        "and can fetch it"
    );
    msm::matrix::avatars::remove_room_avatar(&room)
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    assert!(broom.avatar_url().is_none());

    // Join rules: a moderator-less member can't change them; the admin can, and it syncs.
    use msm::matrix::rooms::JoinAccess;
    assert_eq!(
        rooms::room_settings(&room).await.unwrap().join_access,
        JoinAccess::InviteOnly
    );
    assert!(
        !rooms::room_settings(&broom)
            .await
            .unwrap()
            .can_set_join_access
    );
    assert!(
        rooms::set_join_access(&broom, JoinAccess::Anyone)
            .await
            .is_err()
    );
    rooms::set_join_access(&room, JoinAccess::Anyone)
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    assert_eq!(
        rooms::room_settings(&broom).await.unwrap().join_access,
        JoinAccess::Anyone
    );
    rooms::set_join_access(&room, JoinAccess::InviteOnly)
        .await
        .unwrap();
    assert!(
        rooms::set_join_access(&room, JoinAccess::Other)
            .await
            .is_err()
    );

    // Directory listing: starts unlisted; the admin can publish and withdraw it.
    assert_eq!(
        rooms::room_settings(&room).await.unwrap().listed,
        Some(false)
    );
    rooms::set_listed(&room, true).await.unwrap();
    assert_eq!(
        rooms::room_settings(&room).await.unwrap().listed,
        Some(true)
    );
    rooms::set_listed(&room, false).await.unwrap();
    assert_eq!(
        rooms::room_settings(&room).await.unwrap().listed,
        Some(false)
    );

    // Name and topic changes reach the other members.
    assert!(rooms::set_room_name(&room, "  ").await.is_err());
    rooms::set_room_name(&room, "Renamed").await.unwrap();
    rooms::set_room_topic(&room, "About things").await.unwrap();
    sync_a_bit(&bob).await;
    let bs = rooms::room_settings(&broom).await.unwrap();
    assert_eq!(bs.name.as_deref(), Some("Renamed"));
    assert_eq!(bs.topic.as_deref(), Some("About things"));

    // Kick: carol is out but may be invited back.
    rooms::kick(&room, &c, "tidying").await.unwrap();
    sync_a_bit(&alice).await;
    let s = rooms::room_settings(&room).await.unwrap();
    assert!(row(&s, &c).is_none(), "kicked members are not listed");

    // Ban: bob is listed as banned and offered an unban, not a kick.
    rooms::ban(&room, &b, "spam").await.unwrap();
    sync_a_bit(&alice).await;
    let s = rooms::room_settings(&room).await.unwrap();
    let br = row(&s, &b).unwrap();
    assert_eq!(br.membership, MemberState::Banned);
    assert!(br.can_unban && !br.can_kick && !br.can_ban);
    // A banned user cannot be invited back until unbanned.
    assert!(rooms::invite_user(&room, &b).await.is_err());

    rooms::unban(&room, &b).await.unwrap();
    sync_a_bit(&alice).await;
    let s = rooms::room_settings(&room).await.unwrap();
    assert!(row(&s, &b).is_none());
    rooms::invite_user(&room, &b).await.unwrap();
}

#[tokio::test]
async fn history_pagination_ends_at_the_start_of_the_room() {
    let Some(hs) = homeserver() else { return };
    let (alice, _, _) = register_fresh(&hs).await;
    let room = rooms::create_group(&alice, "Paging", &[]).await.unwrap();
    for i in 0..12 {
        rooms::send_text(
            &room,
            &format!("msg {i}"),
            matrix_sdk::ruma::TransactionId::new(),
            None,
        )
        .await
        .unwrap();
    }
    sync_a_bit(&alice).await;
    let mut seen = std::collections::HashSet::new();
    let (mut token, mut pages) = (None::<String>, 0);
    loop {
        let h = rooms::load_history(&room, token.as_deref(), 5)
            .await
            .unwrap();
        pages += 1;
        seen.extend(h.events.iter().map(|e| format!("{e:?}")));
        token = h.older_token;
        if token.is_none() {
            break;
        }
        assert!(pages < 20, "pagination must terminate");
    }
    assert!(pages >= 3, "12 messages need several pages of 5");
    let all = rooms::load_history(&room, None, 500).await.unwrap();
    assert!(
        all.older_token.is_none(),
        "no 'load older' when the whole room fits in one page"
    );
    assert!(seen.len() >= 12);
}

#[tokio::test]
async fn avatar_change_in_a_shared_room_is_reported() {
    let Some(hs) = homeserver() else { return };
    use msm::matrix::{avatars, rooms};
    let alice = login_as(&hs, "alice", "avw-alice").await;
    let bob = login_as(&hs, "bob", "avw-bob").await;
    let room = rooms::create_group(
        &alice,
        &format!("Avatar watch {}", std::process::id()),
        &["@bob:localhost".into()],
    )
    .await
    .unwrap();
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        if let Some(r) = bob
            .get_room(room.room_id())
            .filter(|r| r.state() == matrix_sdk::RoomState::Invited)
        {
            r.join().await.unwrap();
            break;
        }
    }
    sync_a_bit(&bob).await;

    let (tx, rx) = async_channel::unbounded();
    avatars::watch_changes(&bob, tx);
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    avatars::set_own_avatar(&alice, &"image/png".parse().unwrap(), png)
        .await
        .unwrap();

    let mut who = None;
    for _ in 0..15 {
        sync_a_bit(&bob).await;
        if let Ok(m) = rx.try_recv() {
            who = Some(m);
            break;
        }
    }
    assert_eq!(who.as_deref(), Some("@alice:localhost"));
    let _ = avatars::remove_own_avatar(&alice).await;
}

#[tokio::test]
async fn blocking_uses_the_account_ignore_list_and_syncs_to_other_sessions() {
    let Some(hs) = homeserver() else { return };
    let first = login_as(&hs, "darkmage", "ign-a").await;
    let second = login_as(&hs, "darkmage", "ign-b").await;
    sync_a_bit(&first).await;
    sync_a_bit(&second).await;
    // Start clean in case an earlier run was interrupted.
    ignore::unblock(&first, "@blocked-nobody:localhost")
        .await
        .unwrap();

    assert!(ignore::block(&first, "nonsense").await.is_err());
    assert!(
        ignore::block(&first, "@darkmage:localhost").await.is_err(),
        "cannot block yourself"
    );
    ignore::block(&first, "@blocked-nobody:localhost")
        .await
        .unwrap();
    assert_eq!(ignore::blocked(&first).await, ["@blocked-nobody:localhost"]);

    // Another session of the same account sees the block (it is account data).
    let mut seen = false;
    for _ in 0..10 {
        sync_a_bit(&second).await;
        if ignore::blocked(&second).await == ["@blocked-nobody:localhost"] {
            seen = true;
            break;
        }
    }
    assert!(seen, "block did not reach the other session");

    ignore::unblock(&first, "@blocked-nobody:localhost")
        .await
        .unwrap();
    assert!(ignore::blocked(&first).await.is_empty());
}

#[tokio::test]
async fn resetting_the_identity_replaces_cross_signing_keys_and_the_recovery_key() {
    use msm::matrix::encryption::{self, Recovery};
    let Some(hs) = homeserver() else { return };
    let (a, _name, pw) = register_fresh(&hs).await;
    sync_a_bit(&a).await;
    let old_key = encryption::setup_recovery(&a, &pw).await.unwrap();
    let me = a.user_id().unwrap().to_owned();
    let master = |c: Client, me: matrix_sdk::ruma::OwnedUserId| async move {
        c.encryption()
            .request_user_identity(&me)
            .await
            .unwrap()
            .expect("own identity")
            .master_key()
            .get_first_key()
            .map(|k| k.to_base64())
    };
    let before = master(a.clone(), me.clone()).await;
    assert!(before.is_some());

    assert!(encryption::reset_identity(&a, "wrong").await.is_err());
    assert_eq!(
        master(a.clone(), me.clone()).await,
        before,
        "failed reset changes nothing"
    );

    let new_key = encryption::reset_identity(&a, &pw)
        .await
        .unwrap()
        .expect("recovery was on, so a new key is returned");
    assert_ne!(new_key, old_key);
    sync_a_bit(&a).await;
    let after = master(a.clone(), me).await;
    assert!(after.is_some());
    assert_ne!(after, before, "a new master key");
    let st = encryption::status(&a).await;
    assert!(st.cross_signing_ready, "{st:?}");
    assert_eq!(st.recovery, Recovery::Enabled);
}

#[tokio::test]
async fn calendar_invitation_and_rsvp_cross_an_encrypted_room() {
    use msm::calendar::{Entry, Invite, Response};
    let Some(hs) = homeserver() else { return };
    let alice = login_as(&hs, "alice", "cal-alice").await;
    let bob = login_as(&hs, "bob", "cal-bob").await;
    let room = rooms::create_group(
        &alice,
        &format!("Calendar {}", std::process::id()),
        &["@bob:localhost".into()],
    )
    .await
    .unwrap();
    for _ in 0..10 {
        sync_a_bit(&bob).await;
        if !bob.invited_rooms().is_empty() {
            break;
        }
    }
    for r in bob.invited_rooms() {
        r.join().await.unwrap();
    }
    sync_a_bit(&bob).await;
    sync_a_bit(&alice).await;
    let broom = bob.get_room(room.room_id()).expect("bob joined");
    assert!(room.encryption_state().is_encrypted());

    let invite = Invite {
        title: "Lunch".into(),
        start_ms: msm::calendar::parse_local("2030-05-06", "12:30").unwrap(),
        description: "At the usual place".into(),
    };
    let invite_id = rooms::send_invite(&room, &invite).await.unwrap();
    sync_a_bit(&bob).await;

    // Bob reads the structured invitation; it also reads as plain text for other clients.
    let conv = history_of(&broom).await;
    let m = conv
        .messages()
        .iter()
        .find(|m| m.id == invite_id)
        .cloned()
        .expect("invitation arrived");
    assert_eq!(m.calendar.as_deref(), Some(&Entry::Invite(invite.clone())));
    assert!(
        m.body.contains("Lunch") && m.body.contains("12:30"),
        "{}",
        m.body
    );

    // Bob answers tentatively; Alice sees the answer, tied to the invitation.
    rooms::send_rsvp(&broom, "Lunch", &invite_id, Response::Tentative)
        .await
        .unwrap();
    sync_a_bit(&alice).await;
    let conv = history_of(&room).await;
    let answer = conv
        .messages()
        .iter()
        .find_map(|m| match m.calendar.as_deref() {
            Some(Entry::Rsvp { event_id, response }) if event_id == &invite_id => {
                Some((m.sender.clone(), *response))
            }
            _ => None,
        })
        .expect("answer arrived");
    assert_eq!(answer, ("@bob:localhost".to_owned(), Response::Tentative));

    // The organiser reschedules, then cancels; Bob's calendar follows, a forged change from
    // Bob does not touch Alice's own event.
    let moved = Invite {
        start_ms: invite.start_ms + 2 * 3_600_000,
        ..invite.clone()
    };
    rooms::send_update(&room, &invite_id, &moved).await.unwrap();
    rooms::send_cancel(&broom, "Lunch", &invite_id)
        .await
        .unwrap(); // not the organiser
    sync_a_bit(&bob).await;
    sync_a_bit(&alice).await;
    let db = msm::storage::database::Database::open_in_memory().unwrap();
    let save = |db: &msm::storage::database::Database, sender: &str| {
        db.save_calendar_entry(&msm::storage::database::CalendarEntry {
            event_id: invite_id.clone(),
            room_id: room.room_id().to_string(),
            sender: sender.to_owned(),
            title: invite.title.clone(),
            start_ms: invite.start_ms,
            description: invite.description.clone(),
            response: Response::Accepted,
            repeat: msm::calendar::Repeat::Never,
        })
        .unwrap();
    };
    save(&db, "@alice:localhost");
    for m in history_of(&broom).await.messages() {
        if let Some(e) = m.calendar.as_deref() {
            db.note_calendar_change(&m.sender, e).unwrap();
        }
    }
    let on_calendar = db.calendar_entries().unwrap();
    assert_eq!(on_calendar.len(), 1, "Bob's forged cancel is ignored");
    assert_eq!(on_calendar[0].start_ms, moved.start_ms);

    rooms::send_cancel(&room, "Lunch", &invite_id)
        .await
        .unwrap();
    sync_a_bit(&bob).await;
    for m in history_of(&broom).await.messages() {
        if let Some(e) = m.calendar.as_deref() {
            db.note_calendar_change(&m.sender, e).unwrap();
        }
    }
    assert!(
        db.calendar_entries().unwrap().is_empty(),
        "the organiser's cancel applies"
    );
}
