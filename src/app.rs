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

//! Application state shared by the GTK widgets. Lives on the GTK main thread
//! (`Rc`/`RefCell`); async work is dispatched to Tokio via [`spawn_async`] and
//! results come back on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gtk::glib;
use gtk::prelude::*;
use ksni::TrayMethods;
use matrix_sdk::Client;
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::api::client::profile::DisplayName;
use msm::contacts::contact::Friendship;
use msm::contacts::contact_manager::{ContactManager, FriendAction};
use msm::matrix::messages::Parsed;
use msm::matrix::presence::{self, ContactPresence, UiStatus};
use msm::matrix::rooms::{self, LiveEvent};
use msm::matrix::synchronization::{self, SyncEvent};
use msm::matrix::{authentication, avatars, contacts_sync, media};
use msm::storage::database::Database;
use tokio::task::JoinHandle;

use crate::tray::{MsmTray, TrayCommand};
use crate::ui;

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to start async runtime")
    })
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run `fut` on the Tokio runtime and deliver its output to `on_done` on the GTK main thread.
pub fn spawn_async<T, F>(fut: F, on_done: impl FnOnce(T) + 'static)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    runtime().spawn(async move {
        let _ = tx.send(fut.await).await;
    });
    glib::MainContext::default().spawn_local(async move {
        if let Ok(v) = rx.recv().await {
            // Run the callback from a plain main-loop source, not inside this future:
            // GLib panics (`EnterError`) if a callback starts a nested main loop,
            // which `Dialog::run()` does.
            glib::idle_add_local_once(move || on_done(v));
        }
    });
}

pub struct App {
    pub window: gtk::ApplicationWindow,
    pub stack: gtk::Stack,
    pub db: Arc<Database>,
    pub mgr: RefCell<ContactManager>,
    pub client: RefCell<Option<Client>>,
    pub sync_task: RefCell<Option<JoinHandle<()>>>,
    pub login_ui: RefCell<Option<ui::login_window::LoginUi>>,
    pub main_ui: RefCell<Option<ui::main_window::MainUi>>,
    pub conversations: RefCell<HashMap<String, Rc<ui::conversation_window::ConversationWindow>>>,
    /// Our own chosen status (see `msm::matrix::presence` for the wire mapping).
    pub presence: RefCell<presence::AutoAway>,
    pub idle: RefCell<Option<crate::idle::IdleHandle>>,
    /// Minutes of inactivity before automatic Away (0 = off).
    pub away_minutes: Cell<u32>,
    pub presence_msg: RefCell<String>,
    /// Wire presence for the sync loop (see `matrix::synchronization::run`).
    pub presence_tx: tokio::sync::watch::Sender<presence::Desired>,
    pub tray: RefCell<Option<ksni::Handle<MsmTray>>>,
    /// Room invitations waiting for a decision.
    pub invites: RefCell<Vec<rooms::Invite>>,
    /// Latest known presence of other users; absent means Unknown.
    pub contact_presence: RefCell<HashMap<String, (ContactPresence, Option<String>)>>,
    /// Decoded avatars by key (`u:<mxid>` / `r:<room id>`); `None` = fetched, has none.
    avatars: RefCell<HashMap<String, Option<gtk::gdk_pixbuf::Pixbuf>>>,
    avatar_pending: RefCell<HashSet<String>>,
    /// Bumped whenever an avatar arrives, so views know to redraw.
    pub avatar_gen: Cell<u32>,
    /// Contacts whose cross-signing identity we have verified.
    pub verified_users: RefCell<HashSet<String>>,
    /// People on the account's ignore list (blocked).
    pub blocked_users: RefCell<HashSet<String>>,
}

impl App {
    pub fn new(window: gtk::ApplicationWindow, stack: gtk::Stack, db: Database) -> Rc<Self> {
        let mgr = match db.load_contacts() {
            Ok(doc) => ContactManager::new(doc),
            Err(e) => {
                tracing::error!("could not load cached contacts: {e}");
                ContactManager::default()
            }
        };
        let presence = db
            .get_setting("presence")
            .ok()
            .flatten()
            .and_then(|k| UiStatus::from_key(&k))
            .unwrap_or_default();
        let presence_msg = db
            .get_setting("presence_msg")
            .ok()
            .flatten()
            .unwrap_or_default();
        let away_minutes = db
            .get_setting("away_minutes")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        let (presence_tx, _) =
            tokio::sync::watch::channel(presence::Desired::new(presence, &presence_msg));
        Rc::new(Self {
            window,
            stack,
            db: Arc::new(db),
            mgr: RefCell::new(mgr),
            client: RefCell::new(None),
            sync_task: RefCell::new(None),
            login_ui: RefCell::new(None),
            main_ui: RefCell::new(None),
            conversations: RefCell::new(HashMap::new()),
            presence: RefCell::new(presence::AutoAway::new(presence)),
            idle: RefCell::new(None),
            away_minutes: Cell::new(away_minutes),
            presence_msg: RefCell::new(presence_msg),
            presence_tx,
            tray: RefCell::new(None),
            invites: RefCell::new(Vec::new()),
            contact_presence: RefCell::new(HashMap::new()),
            avatars: RefCell::new(HashMap::new()),
            avatar_pending: RefCell::new(HashSet::new()),
            avatar_gen: Cell::new(0),
            verified_users: RefCell::new(HashSet::new()),
            blocked_users: RefCell::new(HashSet::new()),
        })
    }

    /// Called after a successful login or session restore.
    pub fn start_session(self: &Rc<Self>, client: Client) {
        let me = client.user_id().map(|u| u.to_string()).unwrap_or_default();
        // Cached contacts belong to one account; switching accounts starts clean.
        if self.db.get_setting("account").ok().flatten().as_deref() != Some(me.as_str()) {
            *self.mgr.borrow_mut() = ContactManager::default();
            let _ = self.db.save_contacts(self.mgr.borrow().document());
            let _ = self.db.set_setting("account", &me);
        }
        *self.client.borrow_mut() = Some(client.clone());

        let (tx, rx) = async_channel::unbounded();
        if let Some(old) =
            self.sync_task
                .borrow_mut()
                .replace(runtime().spawn(synchronization::run(
                    client.clone(),
                    tx,
                    self.presence_tx.subscribe(),
                )))
        {
            old.abort();
        }
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    SyncEvent::Connected => app.set_status("Connected"),
                    SyncEvent::Synced => {
                        app.refresh_trust();
                        app.refresh_blocked();
                        app.refresh_invites();
                        ui::main_window::refresh(&app);
                    }
                    SyncEvent::PresenceFailed(e) => {
                        // Many servers disable or restrict presence; degrade quietly.
                        tracing::warn!("presence update failed: {e}");
                        app.set_status("Presence is not available on this server");
                    }
                    SyncEvent::Disconnected(_) => app.set_status("Reconnecting…"),
                    SyncEvent::SessionExpired => {
                        app.end_session(false);
                        app.show_login(Some("Your session expired. Please log in again."));
                        break;
                    }
                }
            }
        });

        // Live room events (messages, edits, redactions, typing).
        let (ltx, lrx) = async_channel::unbounded();
        rooms::forward_live_events(&client, ltx);
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(ev) = lrx.recv().await {
                app.on_live_event(ev);
            }
        });

        crate::reminders::start(self);

        // Incoming verification requests open the verification window.
        let (vtx, vrx) = async_channel::unbounded();
        msm::matrix::verification::watch_incoming(&client, vtx);
        let c = client.clone();
        spawn_async(
            async move { msm::matrix::verification::refresh_own_devices(&c).await },
            |_| {},
        );
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(req) = vrx.recv().await {
                let app = app.clone();
                spawn_async(
                    async move { msm::matrix::verification::run(req) },
                    move |flow| ui::verification_dialog::open(&app, flow),
                );
            }
        });

        // Friend requests and answers from other installations of this app.
        let (ftx, frx) = async_channel::unbounded();
        msm::matrix::friendship::watch(&client, ftx);
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((from, action)) = frx.recv().await {
                app.on_friend_event(&from, action);
            }
        });

        // Other people's profile pictures changing while we are running.
        let (atx, arx) = async_channel::unbounded();
        avatars::watch_changes(&client, atx);
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(mxid) = arx.recv().await {
                app.forget_avatar(&format!("u:{mxid}"));
                app.avatar_gen.set(app.avatar_gen.get() + 1);
                ui::main_window::refresh(&app);
            }
        });

        ui::main_window::show(self, &me);
        // Own display name.
        let c = client.clone();
        let app = self.clone();
        let own_id = me.clone();
        spawn_async(
            async move { c.account().get_display_name().await.ok().flatten() },
            move |name| {
                if let Some(m) = app.main_ui.borrow().as_ref() {
                    m.set_own_name(name.as_deref());
                }
                // So our own messages show our name, not the raw Matrix ID.
                app.mgr
                    .borrow_mut()
                    .resolver_mut()
                    .set_profile_name(&own_id, name.as_deref());
                app.refresh();
            },
        );
        self.push_contacts();
        self.fetch_profile_names();
    }

    /// The status shown and published (manual choice, or Away while automatically away).
    pub fn effective_presence(&self) -> UiStatus {
        self.presence.borrow().effective()
    }

    /// Change our own status. `state` is a manual choice (cancels automatic Away);
    /// `None` keeps the current one, e.g. when only the message changed.
    pub fn set_my_presence(self: &Rc<Self>, state: Option<UiStatus>, message: Option<String>) {
        if let Some(s) = state {
            tracing::info!("status chosen manually: {}", s.key());
            self.presence.borrow_mut().set_manual(s);
        }
        if let Some(m) = message {
            *self.presence_msg.borrow_mut() = m;
        }
        let _ = self
            .db
            .set_setting("presence", self.presence.borrow().manual().key());
        let _ = self
            .db
            .set_setting("presence_msg", &self.presence_msg.borrow());
        self.presence_changed();
    }

    /// Push the effective status to the sync loop, tray, icon and selector.
    fn presence_changed(self: &Rc<Self>) {
        // The sync loop picks this up and publishes it to the homeserver.
        self.presence_tx.send_replace(presence::Desired::new(
            self.effective_presence(),
            &self.presence_msg.borrow(),
        ));
        ui::main_window::sync_presence_controls(self);
        self.update_window_icon();
        self.update_tray();
    }

    /// Start idle detection for automatic Away.
    pub fn start_idle_monitor(self: &Rc<Self>) {
        let (tx, rx) = async_channel::unbounded();
        let handle = crate::idle::start(tx);
        *self.idle.borrow_mut() = Some(handle);
        self.apply_away_timeout();
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            use crate::idle::IdleEvent;
            while let Ok(ev) = rx.recv().await {
                tracing::info!("idle event: {ev:?}");
                let changed = match ev {
                    IdleEvent::Idle => app.presence.borrow_mut().idle(),
                    IdleEvent::Resumed => app.presence.borrow_mut().resume(),
                    IdleEvent::Unavailable(_) => {
                        ui::main_window::set_auto_away_available(&app, false);
                        false
                    }
                };
                if changed {
                    app.presence_changed();
                }
            }
        });
    }

    /// Set the inactivity limit in minutes (0 disables automatic Away).
    pub fn set_away_minutes(self: &Rc<Self>, minutes: u32) {
        self.away_minutes.set(minutes);
        let _ = self.db.set_setting("away_minutes", &minutes.to_string());
        self.apply_away_timeout();
        if minutes == 0 && self.presence.borrow_mut().resume() {
            self.presence_changed();
        }
    }

    fn apply_away_timeout(&self) {
        // MSM_AWAY_SECONDS is a testing override.
        let secs = std::env::var("MSM_AWAY_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                (self.away_minutes.get() > 0).then(|| u64::from(self.away_minutes.get()) * 60)
            });
        if let Some(h) = self.idle.borrow().as_ref() {
            h.set_timeout(secs.map(std::time::Duration::from_secs));
        }
    }

    fn update_window_icon(&self) {
        use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
        const SIZE: usize = 64;
        let bytes =
            glib::Bytes::from_owned(msm::icon::status_icon_rgba(self.effective_presence(), SIZE));
        let pb = Pixbuf::from_bytes(
            &bytes,
            Colorspace::Rgb,
            true,
            8,
            SIZE as i32,
            SIZE as i32,
            (SIZE * 4) as i32,
        );
        gtk::Window::set_default_icon(&pb);
        self.window.set_icon(Some(&pb));
    }

    pub fn update_tray(&self) {
        let Some(h) = self.tray.borrow().clone() else {
            return;
        };
        let (status, visible) = (self.effective_presence(), self.window.is_visible());
        runtime().spawn(async move {
            h.update(|t| {
                t.status = status;
                t.window_visible = visible;
            })
            .await;
        });
    }

    /// Start the tray icon. Without a StatusNotifier host the tray is simply absent
    /// and closing the window quits the app instead of hiding it.
    pub fn start_tray(self: &Rc<Self>) {
        self.update_window_icon();
        let (tx, rx) = async_channel::unbounded();
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(cmd) = rx.recv().await {
                app.on_tray_command(cmd);
            }
        });
        let tray = MsmTray {
            status: self.effective_presence(),
            window_visible: self.window.is_visible(),
            tx,
        };
        let app = self.clone();
        spawn_async(
            async move { tray.spawn().await.map_err(|e| e.to_string()) },
            move |res| match res {
                Ok(h) => {
                    *app.tray.borrow_mut() = Some(h);
                    // The window may already be showing; sync the menu label.
                    app.update_tray();
                }
                Err(e) => tracing::warn!("system tray unavailable: {e}"),
            },
        );
    }

    fn on_tray_command(self: &Rc<Self>, cmd: TrayCommand) {
        match cmd {
            TrayCommand::SetStatus(s) => self.set_my_presence(Some(s), None),
            TrayCommand::ToggleWindow => {
                self.toggle_window();
            }
            TrayCommand::Quit => {
                if let Some(app) = self.window.application() {
                    app.quit();
                }
            }
        }
    }

    pub fn toggle_window(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.window.hide();
        } else {
            self.window.present();
        }
        self.update_tray();
    }

    /// Close button: hide to the tray if there is one, otherwise let the app quit.
    pub fn hide_to_tray(self: &Rc<Self>) -> bool {
        if self.tray.borrow().is_none() {
            return false;
        }
        self.window.hide();
        self.update_tray();
        true
    }

    pub fn set_status(&self, text: &str) {
        if let Some(m) = self.main_ui.borrow().as_ref() {
            m.set_status(text);
        }
    }

    pub fn show_login(self: &Rc<Self>, message: Option<&str>) {
        if let Some(l) = self.login_ui.borrow().as_ref() {
            l.reset(message);
        }
        self.stack.set_visible_child_name("login");
    }

    /// On quit: stop syncing and tell the server we are offline, so contacts
    /// don't see us as online until the server's presence timeout. Best effort,
    /// bounded to two seconds.
    pub fn shutdown(&self) {
        if let Some(t) = self.sync_task.borrow_mut().take() {
            t.abort();
        }
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let offline = presence::Desired::new(presence::UiStatus::Offline, "");
        let res = runtime().block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                presence::publish(&client, &offline),
            )
            .await
        });
        match res {
            Ok(Ok(())) => tracing::info!("set presence offline on quit"),
            Ok(Err(e)) => tracing::warn!("could not set presence offline on quit: {e}"),
            Err(_) => tracing::warn!("timed out setting presence offline on quit"),
        }
    }

    pub fn end_session(&self, server_logout: bool) {
        if let Some(t) = self.sync_task.borrow_mut().take() {
            t.abort();
        }
        if let Some(c) = self.client.borrow_mut().take()
            && server_logout
        {
            spawn_async(
                async move { authentication::logout(&c, &authentication::data_dir()).await },
                |_| {},
            );
        }
    }

    /// Persist locally, then merge with and push to the server copy.
    pub fn persist_and_sync(self: &Rc<Self>) {
        if let Err(e) = self.db.save_contacts(self.mgr.borrow().document()) {
            tracing::error!("could not save contacts: {e}");
        }
        self.push_contacts();
    }

    fn push_contacts(self: &Rc<Self>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let doc = self.mgr.borrow().document().clone();
        let app = self.clone();
        spawn_async(
            async move { contacts_sync::sync(&client.account(), &doc).await },
            move |res| match res {
                Ok(merged) => {
                    app.mgr.borrow_mut().apply_remote(&merged);
                    let _ = app.db.save_contacts(app.mgr.borrow().document());
                    app.refresh();
                    app.fetch_profile_names();
                }
                Err(e) => {
                    tracing::warn!("contact sync failed: {e}");
                    app.set_status("Contact sync failed; will retry on next change");
                }
            },
        );
    }

    /// Look up public display names so the resolver can fall back to them.
    fn fetch_profile_names(self: &Rc<Self>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let ids: Vec<String> = self
            .mgr
            .borrow()
            .list()
            .iter()
            .map(|c| c.mxid.clone())
            .collect();
        for id in ids {
            let client = client.clone();
            let app = self.clone();
            let id2 = id.clone();
            spawn_async(
                async move {
                    let uid = <&UserId>::try_from(id.as_str()).ok()?;
                    let p = client.account().fetch_user_profile_of(uid).await.ok()?;
                    p.get_static::<DisplayName>().ok().flatten()
                },
                move |name| {
                    if name.is_some() {
                        app.mgr
                            .borrow_mut()
                            .resolver_mut()
                            .set_profile_name(&id2, name.as_deref());
                        app.refresh();
                    }
                },
            );
        }
    }

    /// The avatar for `key` (`u:<mxid>` or `r:<room id>`), or a stable placeholder
    /// while it loads or if there is none. Fetches at most once per key.
    pub fn avatar(self: &Rc<Self>, key: &str) -> gtk::gdk_pixbuf::Pixbuf {
        if let Some(Some(pb)) = self.avatars.borrow().get(key) {
            return pb.clone();
        }
        // `r:<room>#` carries the room's current avatar URL; empty means none to fetch.
        if key.starts_with("r:") && key.ends_with('#') {
            return ui::avatar::placeholder(key);
        }
        let known = self.avatars.borrow().contains_key(key);
        if !known && self.avatar_pending.borrow_mut().insert(key.to_owned()) {
            self.fetch_avatar(key);
        }
        ui::avatar::placeholder(key)
    }

    fn fetch_avatar(self: &Rc<Self>, key: &str) {
        let Some(client) = self.client.borrow().clone() else {
            self.avatar_pending.borrow_mut().remove(key);
            return;
        };
        let (app, k) = (self.clone(), key.to_owned());
        let fetch_key = key.to_owned();
        spawn_async(
            async move {
                match fetch_key.split_once(':') {
                    Some(("u", mxid)) => avatars::user_avatar(&client, mxid).await,
                    Some(("r", id)) => match <&matrix_sdk::ruma::RoomId>::try_from(
                        id.split('#').next().unwrap_or(id),
                    )
                    .ok()
                    .and_then(|rid| client.get_room(rid))
                    {
                        Some(room) => avatars::room_avatar(&room).await,
                        None => None,
                    },
                    _ => None,
                }
            },
            move |bytes| {
                app.avatar_pending.borrow_mut().remove(&k);
                // A session change mid-fetch must not poison the cache.
                if app.client.borrow().is_none() {
                    return;
                }
                let pb = bytes.and_then(|b| ui::avatar::decode(&b));
                app.avatars.borrow_mut().insert(k, pb);
                app.avatar_gen.set(app.avatar_gen.get() + 1);
                app.refresh();
            },
        );
    }

    /// Reload the ignore list from synced account data; redraws if it changed.
    pub fn refresh_blocked(self: &Rc<Self>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let app = self.clone();
        spawn_async(
            async move { msm::matrix::ignore::blocked(&client).await },
            move |ids| {
                let set: HashSet<String> = ids.into_iter().collect();
                if *app.blocked_users.borrow() != set {
                    *app.blocked_users.borrow_mut() = set;
                    app.refresh();
                }
            },
        );
    }

    /// Block or unblock a person on the account-wide ignore list.
    pub fn set_blocked(self: &Rc<Self>, mxid: &str, block: bool) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let (app, id) = (self.clone(), mxid.to_owned());
        spawn_async(
            async move {
                if block {
                    msm::matrix::ignore::block(&client, &id).await
                } else {
                    msm::matrix::ignore::unblock(&client, &id).await
                }
                .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(()) => app.refresh_blocked(),
                Err(e) => ui::main_window::error_dialog(&app, &e),
            },
        );
    }

    /// Recompute which contacts (and we) are verified; redraws if that changed.
    pub fn refresh_trust(self: &Rc<Self>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let ids: Vec<String> = self
            .mgr
            .borrow()
            .list()
            .iter()
            .map(|c| c.mxid.clone())
            .collect();
        let app = self.clone();
        spawn_async(
            async move {
                let mut out = HashSet::new();
                for id in ids {
                    let Ok(uid) = UserId::parse(&id) else {
                        continue;
                    };
                    if let Ok(Some(identity)) = client.encryption().get_user_identity(&uid).await
                        && identity.is_verified()
                    {
                        out.insert(id);
                    }
                }
                out
            },
            move |set| {
                if *app.verified_users.borrow() != set {
                    *app.verified_users.borrow_mut() = set;
                    app.refresh();
                }
            },
        );
    }

    /// Cache key for a room's avatar; changes when the room's avatar does.
    pub fn room_avatar_key(room: &matrix_sdk::Room) -> String {
        format!(
            "r:{}#{}",
            room.room_id(),
            room.avatar_url().map(|u| u.to_string()).unwrap_or_default()
        )
    }

    /// Drop a cached avatar so it is fetched again on next use.
    pub fn forget_avatar(&self, key: &str) {
        self.avatars.borrow_mut().remove(key);
    }

    /// Our own MXID, if signed in.
    pub fn own_id(&self) -> Option<String> {
        self.client
            .borrow()
            .as_ref()
            .and_then(|c| c.user_id().map(|u| u.to_string()))
    }

    /// Upload `path` as our profile picture (`None` removes it).
    pub fn set_own_avatar(self: &Rc<Self>, path: Option<std::path::PathBuf>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let app = self.clone();
        spawn_async(
            async move {
                match path {
                    Some(p) => {
                        let data = std::fs::read(&p)
                            .map_err(|e| format!("could not read the image: {e}"))?;
                        avatars::set_own_avatar(&client, &media::guess_mime(&p), data)
                            .await
                            .map_err(|e| e.to_string())
                    }
                    None => avatars::remove_own_avatar(&client)
                        .await
                        .map_err(|e| e.to_string()),
                }
            },
            move |r| match r {
                Ok(()) => {
                    if let Some(me) = app.own_id() {
                        app.forget_avatar(&format!("u:{me}"));
                    }
                    app.refresh();
                }
                Err(e) => ui::main_window::error_dialog(&app, &e),
            },
        );
    }

    pub fn refresh(self: &Rc<Self>) {
        ui::main_window::refresh(self);
        for c in self.conversations.borrow().values() {
            c.rerender();
        }
    }

    /// Unread count across all DM rooms with `mxid`.
    pub fn unread_for(&self, mxid: &str) -> u64 {
        let Some(client) = self.client.borrow().clone() else {
            return 0;
        };
        let Ok(dms) = rooms::existing_dms(&client, mxid) else {
            return 0;
        };
        dms.iter().map(rooms::unread_count).sum()
    }

    fn on_live_event(self: &Rc<Self>, ev: LiveEvent) {
        match ev {
            LiveEvent::Presence {
                user,
                state,
                message,
            } => {
                self.contact_presence
                    .borrow_mut()
                    .insert(user, (state, message));
                ui::main_window::refresh(self);
            }
            LiveEvent::Typing { room_id, users } => {
                if let Some(c) = self.conversations.borrow().get(&room_id) {
                    c.on_typing(&users);
                }
            }
            LiveEvent::Timeline { room_id, event } => {
                let window = self.conversations.borrow().get(&room_id).cloned();
                let me = self
                    .client
                    .borrow()
                    .as_ref()
                    .and_then(|c| c.user_id().map(|u| u.to_string()));
                // Count/notify only for new, visible messages from others.
                if let Parsed::Message(m) = &*event
                    && Some(&m.sender) != me.as_ref()
                    && !m.redacted
                {
                    let focused = window.as_ref().is_some_and(|w| w.window.is_active());
                    if !focused {
                        let name = self
                            .mgr
                            .borrow()
                            .resolver()
                            .resolve(&m.sender, Some(&room_id));
                        self.notify(
                            &room_id,
                            &name,
                            if m.undecryptable {
                                "Encrypted message"
                            } else {
                                &m.body
                            },
                        );
                        ui::main_window::refresh(self);
                    }
                }
                // An organiser's reschedule/cancel applies to the calendar even when this
                // conversation is closed.
                if let Parsed::Message(m) = &*event
                    && !m.redacted
                    && let Some(e) = m.calendar.as_deref()
                {
                    let _ = self.db.note_calendar_change(&m.sender, e);
                }
                if let Some(w) = window {
                    w.on_event(*event);
                }
            }
        }
    }

    pub fn notify(&self, room_id: &str, sender: &str, body: &str) {
        let Some(app) = self.window.application() else {
            return;
        };
        let n = gtk::gio::Notification::new(sender);
        // Message text is only shown for the notification; it is never logged.
        n.set_body(Some(&body.chars().take(140).collect::<String>()));
        app.send_notification(Some(&format!("msg-{room_id}")), &n);
    }

    /// Open (or create) the DM with `mxid` and show its window.
    /// Open a joined group room by ID.
    pub fn open_room(self: &Rc<Self>, room_id: &str) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        if let Ok(id) = <&matrix_sdk::ruma::RoomId>::try_from(room_id)
            && let Some(room) = client.get_room(id)
        {
            ui::conversation_window::open(self, room, None);
        }
    }

    /// Re-read pending invitations; notify when a new one arrives.
    pub fn refresh_invites(self: &Rc<Self>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let app = self.clone();
        spawn_async(
            async move { rooms::pending_invites(&client).await },
            move |list| {
                let known: Vec<String> = app
                    .invites
                    .borrow()
                    .iter()
                    .map(|i| i.room_id.clone())
                    .collect();
                for inv in list.iter().filter(|i| !known.contains(&i.room_id)) {
                    let who = inv
                        .inviter
                        .as_deref()
                        .map(|m| app.mgr.borrow().resolver().resolve(m, None))
                        .unwrap_or_else(|| "Someone".to_owned());
                    app.notify(&inv.room_id, &who, &format!("invited you to {}", inv.name));
                }
                let changed = *app.invites.borrow() != list;
                *app.invites.borrow_mut() = list;
                if changed {
                    ui::main_window::refresh(&app);
                }
            },
        );
    }

    /// Send a friendship message in the background; the local state change has
    /// already been made, so a failure only needs reporting.
    fn send_friend_action(self: &Rc<Self>, mxid: &str, action: FriendAction) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let (app, id) = (self.clone(), mxid.to_owned());
        let target = id.clone();
        spawn_async(
            async move {
                msm::matrix::friendship::send(&client, &target, action)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                if let Err(e) = res {
                    app.set_status(&format!("Could not reach {id}: {e}"));
                }
            },
        );
    }

    /// Ask `mxid` to be friends (adds them as a contact if needed).
    pub fn send_friend_request(self: &Rc<Self>, mxid: &str) {
        let res = self.mgr.borrow_mut().request_friend(mxid, now_ms());
        match res {
            Ok(action) => {
                self.persist_and_sync();
                self.refresh();
                self.send_friend_action(mxid, action);
            }
            Err(e) => self.set_status(&format!("Could not send friend request: {e}")),
        }
    }

    /// Answer an incoming request.
    pub fn answer_friend_request(self: &Rc<Self>, mxid: &str, accept: bool) {
        let res = if accept {
            self.mgr.borrow_mut().accept_friend(mxid, now_ms())
        } else {
            self.mgr.borrow_mut().reject_friend(mxid, now_ms())
        };
        if res.is_ok() {
            self.persist_and_sync();
            self.refresh();
            let action = if accept {
                FriendAction::Accept
            } else {
                FriendAction::Reject
            };
            self.send_friend_action(mxid, action);
        }
    }

    /// End a friendship or withdraw a pending request; the contact stays.
    pub fn end_friendship(self: &Rc<Self>, mxid: &str) {
        if self.mgr.borrow_mut().end_friendship(mxid, now_ms()).is_ok() {
            self.persist_and_sync();
            self.refresh();
            self.send_friend_action(mxid, FriendAction::Remove);
        }
    }

    fn on_friend_event(self: &Rc<Self>, from: &str, action: FriendAction) {
        let before = self.mgr.borrow().friendship(from);
        let reply = self
            .mgr
            .borrow_mut()
            .apply_friend_event(from, action, now_ms());
        let after = self.mgr.borrow().friendship(from);
        if before == after && reply.is_none() {
            return;
        }
        self.persist_and_sync();
        self.refresh();
        let who = self.mgr.borrow().resolver().resolve(from, None);
        match (before, after) {
            (_, Friendship::Incoming) => {
                self.set_status(&format!("{who} sent you a friend request"));
                self.notify("friend-request", &who, "wants to add you as a friend");
            }
            (Friendship::Outgoing, Friendship::Friends) => {
                self.set_status(&format!("{who} accepted your friend request"));
            }
            (Friendship::Outgoing, Friendship::None) => {
                self.set_status(&format!("{who} declined your friend request"));
            }
            _ => {}
        }
        if let Some(r) = reply {
            self.send_friend_action(from, r);
        }
    }

    pub fn answer_invite(self: &Rc<Self>, room_id: &str, accept: bool) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let (app, rid) = (self.clone(), room_id.to_owned());
        let rid2 = rid.clone();
        spawn_async(
            async move {
                if accept {
                    rooms::accept_invite(&client, &rid).await
                } else {
                    rooms::leave_room(&client, &rid).await
                }
                .map_err(|e| e.to_string())
            },
            move |res| {
                match res {
                    Ok(()) => app.invites.borrow_mut().retain(|i| i.room_id != rid2),
                    Err(e) => app.set_status(&format!("Could not answer invitation: {e}")),
                }
                app.refresh_invites();
                ui::main_window::refresh(&app);
            },
        );
    }

    pub fn create_room(self: &Rc<Self>, name: &str, invitees: Vec<String>) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let (app, name) = (self.clone(), name.to_owned());
        spawn_async(
            async move {
                rooms::create_group(&client, &name, &invitees)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(room) => ui::conversation_window::open(&app, room, None),
                Err(e) => app.set_status(&format!("Could not create room: {e}")),
            },
        );
    }

    pub fn open_conversation(self: &Rc<Self>, mxid: &str) {
        let Some(client) = self.client.borrow().clone() else {
            return;
        };
        let app = self.clone();
        let id = mxid.to_owned();
        let id2 = id.clone();
        spawn_async(
            async move {
                rooms::open_or_create_dm(&client, &id)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(room) => ui::conversation_window::open(&app, room, Some(&id2)),
                Err(e) => app.set_status(&format!("Could not open conversation: {e}")),
            },
        );
    }
}
