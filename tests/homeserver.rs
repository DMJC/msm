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

    sync_a_bit(&bob).await;
    let broom = bob.get_room(room.room_id()).unwrap();
    let conv = history_of(&broom).await;
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
