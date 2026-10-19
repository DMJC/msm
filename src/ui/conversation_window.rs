//! One window per conversation. Renders a [`Conversation`] model; all name
//! display goes through the app-wide `NameResolver`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use matrix_sdk::Room;
use matrix_sdk::ruma::OwnedTransactionId;
use msm::matrix::messages::{ChatMessage, Conversation, Delivery, Parsed};
use msm::matrix::rooms;

use crate::app::{App, now_ms, spawn_async};

const PAGE: u16 = 30;

pub struct ConversationWindow {
    pub window: gtk::Window,
    pub room: Room,
    pub peer: String,
    app: Rc<App>,
    conv: RefCell<Conversation>,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    typing: gtk::Label,
    input: gtk::TextView,
    buffer: gtk::TextBuffer,
    older: RefCell<Option<String>>,
    older_button: gtk::Button,
    i_am_typing: RefCell<bool>,
}

pub fn open(app: &Rc<App>, room: Room, peer: &str) {
    let id = room.room_id().to_string();
    if let Some(existing) = app.conversations.borrow().get(&id) {
        existing.window.present();
        return;
    }
    let cw = ConversationWindow::new(app, room, peer);
    app.conversations
        .borrow_mut()
        .insert(id.clone(), cw.clone());
    cw.load_initial();
    cw.window.show_all();
}

impl ConversationWindow {
    fn new(app: &Rc<App>, room: Room, peer: &str) -> Rc<Self> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_default_size(460, 520);
        window.set_transient_for(Some(&app.window));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.set_margin(6);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        let encrypted = gtk::Label::new(None);
        encrypted.set_xalign(0.0);
        encrypted.style_context().add_class("dim-label");
        root.pack_start(&title, false, false, 0);
        root.pack_start(&encrypted, false, false, 0);

        let older_button = gtk::Button::with_label("Load older messages");
        older_button.set_no_show_all(true);
        older_button.set_visible(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.pack_start(&older_button, false, false, 0);
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.add(&list);
        root.pack_start(&scroll, true, true, 0);

        let typing = gtk::Label::new(None);
        typing.set_xalign(0.0);
        typing.style_context().add_class("dim-label");
        root.pack_start(&typing, false, false, 0);

        let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let input = gtk::TextView::with_buffer(&buffer);
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        let in_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        in_scroll.set_min_content_height(56);
        in_scroll.set_shadow_type(gtk::ShadowType::In);
        in_scroll.add(&input);
        let send = gtk::Button::with_label("Send");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.pack_start(&in_scroll, true, true, 0);
        row.pack_start(&send, false, false, 0);
        root.pack_start(&row, false, false, 0);
        window.add(&root);

        let is_encrypted = room.encryption_state().is_encrypted();
        encrypted.set_text(if is_encrypted {
            "🔒 End-to-end encrypted"
        } else {
            "Not encrypted"
        });

        let cw = Rc::new(Self {
            window,
            room,
            peer: peer.to_owned(),
            app: app.clone(),
            conv: RefCell::new(Conversation::default()),
            list,
            scroll,
            title,
            typing,
            input,
            buffer,
            older: RefCell::new(None),
            older_button,
            i_am_typing: RefCell::new(false),
        });

        let c = cw.clone();
        cw.older_button.connect_clicked(move |_| c.load_older());
        let c = cw.clone();
        send.connect_clicked(move |_| c.send_input());
        let c = cw.clone();
        cw.input.connect_key_press_event(move |_, e| {
            let enter = matches!(
                e.keyval(),
                gdk::keys::constants::Return | gdk::keys::constants::KP_Enter
            );
            if enter && !e.state().contains(gdk::ModifierType::SHIFT_MASK) {
                c.send_input();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let c = cw.clone();
        cw.buffer.connect_changed(move |b| {
            let non_empty = b.char_count() > 0;
            c.set_typing(non_empty);
        });
        let c = cw.clone();
        cw.window.connect_focus_in_event(move |_, _| {
            c.mark_read_latest();
            glib::Propagation::Proceed
        });
        let (app, id) = (app.clone(), cw.room.room_id().to_string());
        let c = cw.clone();
        cw.window.connect_delete_event(move |_, _| {
            c.set_typing(false);
            app.conversations.borrow_mut().remove(&id);
            glib::Propagation::Proceed
        });
        cw.update_header();
        cw
    }

    fn me(&self) -> String {
        self.app
            .client
            .borrow()
            .as_ref()
            .and_then(|c| c.user_id().map(|u| u.to_string()))
            .unwrap_or_default()
    }

    fn name_of(&self, mxid: &str) -> String {
        self.app
            .mgr
            .borrow()
            .resolver()
            .resolve(mxid, Some(self.room.room_id().as_str()))
    }

    fn update_header(&self) {
        let name = self.name_of(&self.peer);
        self.window.set_title(&name);
        self.title.set_markup(&format!(
            "<b>{}</b>  <small>{}</small>",
            glib::markup_escape_text(&name),
            glib::markup_escape_text(&self.peer)
        ));
    }

    fn load_initial(self: &Rc<Self>) {
        self.fetch(None);
    }

    fn load_older(self: &Rc<Self>) {
        let token = self.older.borrow().clone();
        if token.is_some() {
            self.fetch(token);
        }
    }

    fn fetch(self: &Rc<Self>, from: Option<String>) {
        let room = self.room.clone();
        let this = self.clone();
        let initial = from.is_none();
        self.older_button.set_sensitive(false);
        spawn_async(
            async move {
                rooms::load_history(&room, from.as_deref(), PAGE)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(h) => {
                    for ev in h.events {
                        this.conv.borrow_mut().apply(ev);
                    }
                    this.older_button.set_visible(h.older_token.is_some());
                    this.older_button.set_sensitive(true);
                    *this.older.borrow_mut() = h.older_token;
                    this.redraw(initial);
                    if initial {
                        this.mark_read_latest();
                    }
                }
                Err(e) => {
                    this.older_button.set_sensitive(true);
                    this.typing
                        .set_text(&format!("Could not load history: {e}"));
                }
            },
        );
    }

    /// Live event from the sync feed.
    pub fn on_event(self: &Rc<Self>, ev: Parsed) {
        self.conv.borrow_mut().apply(ev);
        self.redraw(true);
        if self.window.is_active() {
            self.mark_read_latest();
        }
    }

    /// Tell the server we have read up to the newest confirmed message.
    pub fn mark_read_latest(self: &Rc<Self>) {
        let latest = self
            .conv
            .borrow()
            .messages()
            .iter()
            .rev()
            .find(|m| m.delivery == Delivery::Sent && m.id.starts_with('$'))
            .map(|m| m.id.clone());
        let Some(id) = latest else { return };
        let room = self.room.clone();
        let app = self.app.clone();
        spawn_async(
            async move { rooms::mark_read(&room, &id).await },
            move |_| crate::ui::main_window::refresh(&app),
        );
    }

    pub fn on_typing(&self, users: &[String]) {
        let me = self.me();
        let names: Vec<String> = users
            .iter()
            .filter(|u| **u != me)
            .map(|u| self.name_of(u))
            .collect();
        self.typing.set_text(&match names.len() {
            0 => String::new(),
            1 => format!("{} is typing…", names[0]),
            _ => format!("{} are typing…", names.join(", ")),
        });
    }

    pub fn rerender(self: &Rc<Self>) {
        self.update_header();
        self.redraw(false);
    }

    fn set_typing(self: &Rc<Self>, typing: bool) {
        if *self.i_am_typing.borrow() == typing {
            return;
        }
        *self.i_am_typing.borrow_mut() = typing;
        let room = self.room.clone();
        spawn_async(
            async move { rooms::set_typing(&room, typing).await },
            |_| {},
        );
    }

    fn send_input(self: &Rc<Self>) {
        let buf = &self.buffer;
        let (s, e) = buf.bounds();
        let body = buf
            .text(&s, &e, false)
            .map(|t| t.to_string())
            .unwrap_or_default();
        if body.trim().is_empty() {
            return;
        }
        buf.set_text("");
        let txn = matrix_sdk::ruma::TransactionId::new();
        self.conv
            .borrow_mut()
            .add_pending(txn.as_str(), &self.me(), &body, now_ms());
        self.redraw(true);
        self.deliver(txn.to_string(), body);
    }

    fn deliver(self: &Rc<Self>, txn: String, body: String) {
        let room = self.room.clone();
        let this = self.clone();
        let txn_id = OwnedTransactionId::from(txn.as_str());
        spawn_async(
            async move {
                rooms::send_text(&room, &body, txn_id)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                match res {
                    Ok(event_id) => this.conv.borrow_mut().mark_sent(&txn, &event_id),
                    Err(e) => this.conv.borrow_mut().mark_failed(&txn, &e),
                }
                this.redraw(false);
            },
        );
    }

    fn retry(self: &Rc<Self>, txn: &str) {
        let body = self
            .conv
            .borrow()
            .messages()
            .iter()
            .find(|m| m.txn_id.as_deref() == Some(txn))
            .map(|m| m.body.clone());
        if let Some(body) = body {
            self.conv.borrow_mut().mark_pending(txn);
            self.redraw(false);
            self.deliver(txn.to_owned(), body);
        }
    }

    fn delete(self: &Rc<Self>, event_id: &str) {
        let room = self.room.clone();
        let id = event_id.to_owned();
        let this = self.clone();
        spawn_async(
            async move { rooms::redact(&room, &id).await.map_err(|e| e.to_string()) },
            move |res| {
                if let Err(e) = res {
                    this.typing
                        .set_text(&format!("Could not delete message: {e}"));
                }
            },
        );
    }

    fn redraw(self: &Rc<Self>, scroll_to_end: bool) {
        for child in self.list.children() {
            if child != *self.older_button.upcast_ref::<gtk::Widget>() {
                self.list.remove(&child);
            }
        }
        let me = self.me();
        let msgs: Vec<ChatMessage> = self.conv.borrow().messages().to_vec();
        for m in &msgs {
            self.list
                .pack_start(&self.message_row(m, &me), false, false, 0);
        }
        self.list.show_all();
        if scroll_to_end {
            let adj = self.scroll.vadjustment();
            glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
        }
    }

    fn message_row(self: &Rc<Self>, m: &ChatMessage, me: &str) -> gtk::Widget {
        let name = self.name_of(&m.sender);
        let time = glib::DateTime::from_unix_local((m.ts / 1000) as i64)
            .ok()
            .and_then(|d| d.format("%H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let text = if m.redacted {
            "<i>message deleted</i>".to_owned()
        } else if m.undecryptable {
            "<i>🔒 Unable to decrypt this message (keys not available yet)</i>".to_owned()
        } else {
            glib::markup_escape_text(&m.body).to_string()
        };
        let mark = match &m.delivery {
            Delivery::Pending => " <small>⏳ sending…</small>".to_owned(),
            Delivery::Failed(_) => {
                " <small><span foreground='red'>⚠ failed</span></small>".to_owned()
            }
            Delivery::Sent => String::new(),
        };
        let edited = if m.edited {
            " <small>(edited)</small>"
        } else {
            ""
        };
        let label = gtk::Label::new(None);
        label.set_markup(&format!(
            "<small>{time}</small> <b>{}</b>: {text}{edited}{mark}",
            glib::markup_escape_text(&name)
        ));
        label.set_xalign(0.0);
        label.set_line_wrap(true);
        label.set_selectable(true);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.pack_start(&label, true, true, 0);
        if let Delivery::Failed(reason) = &m.delivery {
            let btn = gtk::Button::with_label("Retry");
            btn.set_tooltip_text(Some(reason));
            let (this, txn) = (
                self.clone(),
                m.txn_id.clone().unwrap_or_else(|| m.id.clone()),
            );
            btn.connect_clicked(move |_| this.retry(&txn));
            row.pack_start(&btn, false, false, 0);
        }
        let ev = gtk::EventBox::new();
        ev.add(&row);
        if m.sender == me && !m.redacted && m.delivery == Delivery::Sent {
            let (this, id) = (self.clone(), m.id.clone());
            ev.connect_button_press_event(move |_, e| {
                if e.button() == 3 {
                    let menu = gtk::Menu::new();
                    let item = gtk::MenuItem::with_label("Delete message");
                    let (t2, id2) = (this.clone(), id.clone());
                    item.connect_activate(move |_| t2.delete(&id2));
                    menu.append(&item);
                    menu.show_all();
                    menu.popup_at_pointer(Some(e));
                }
                glib::Propagation::Proceed
            });
        }
        ev.upcast()
    }
}
