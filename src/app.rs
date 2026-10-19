//! Application state shared by the GTK widgets. Lives on the GTK main thread
//! (`Rc`/`RefCell`); async work is dispatched to Tokio via [`spawn_async`] and
//! results come back on the main thread.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gtk::glib;
use gtk::prelude::*;
use matrix_sdk::Client;
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::api::client::profile::DisplayName;
use msm::contacts::contact_manager::ContactManager;
use msm::matrix::synchronization::{self, SyncEvent};
use msm::matrix::{authentication, contacts_sync};
use msm::storage::database::Database;
use tokio::task::JoinHandle;

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
            on_done(v);
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
        Rc::new(Self {
            window,
            stack,
            db: Arc::new(db),
            mgr: RefCell::new(mgr),
            client: RefCell::new(None),
            sync_task: RefCell::new(None),
            login_ui: RefCell::new(None),
            main_ui: RefCell::new(None),
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
        if let Some(old) = self
            .sync_task
            .borrow_mut()
            .replace(runtime().spawn(synchronization::run(client.clone(), tx)))
        {
            old.abort();
        }
        let app = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(ev) = rx.recv().await {
                match ev {
                    SyncEvent::Connected => app.set_status("Connected"),
                    SyncEvent::Disconnected(_) => app.set_status("Reconnecting…"),
                    SyncEvent::SessionExpired => {
                        app.end_session(false);
                        app.show_login(Some("Your session expired. Please log in again."));
                        break;
                    }
                }
            }
        });

        ui::main_window::show(self, &me);
        // Own display name.
        let c = client.clone();
        let app = self.clone();
        spawn_async(
            async move { c.account().get_display_name().await.ok().flatten() },
            move |name| {
                if let Some(m) = app.main_ui.borrow().as_ref() {
                    m.set_own_name(name.as_deref());
                }
            },
        );
        self.push_contacts();
        self.fetch_profile_names();
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

    pub fn refresh(self: &Rc<Self>) {
        ui::main_window::refresh(self);
    }
}
