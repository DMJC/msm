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
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new())
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
    rooms::send_text(&room, &body, matrix_sdk::ruma::TransactionId::new())
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
}
