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

//! One window per conversation. Renders a [`Conversation`] model; all name
//! display goes through the app-wide `NameResolver`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use matrix_sdk::Room;
use matrix_sdk::ruma::OwnedTransactionId;
use msm::matrix::media;
use msm::matrix::messages::{
    Attachment, AttachmentKind, ChatMessage, Conversation, Delivery, Parsed, Reaction, Trust,
};
use msm::matrix::rooms;

use crate::app::{App, now_ms, spawn_async};

const PAGE: u16 = 30;
/// Most history pages one search scan loads (older messages are fetched on demand).
const SEARCH_PAGES: u32 = 30;
const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "🎉", "😮", "😢"];
const EMOJI_PICKER: [&str; 32] = [
    "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🙂", "😉", "😊", "😍", "😘", "😎", "🤔", "😐", "😢",
    "😭", "😡", "👍", "👎", "👏", "🙏", "💪", "🎉", "❤️", "🔥", "✨", "🌱", "☕", "🍕", "🎂", "👀",
];

pub struct ConversationWindow {
    pub window: gtk::Window,
    pub room: Room,
    /// Set for DMs; `None` for group rooms.
    pub peer: Option<String>,
    app: Rc<App>,
    conv: RefCell<Conversation>,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    avatar: gtk::Image,
    typing: gtk::Label,
    cancel_upload: gtk::Button,
    /// Set while an attachment is uploading; a message cancels it.
    upload_cancel: RefCell<Option<async_channel::Sender<()>>>,
    input: gtk::TextView,
    buffer: gtk::TextBuffer,
    /// Joined member count once the member list has been loaded.
    member_count: RefCell<Option<usize>>,
    /// Decoded image previews by event ID, and which ones were already requested.
    thumbs: RefCell<HashMap<String, gtk::gdk_pixbuf::Pixbuf>>,
    thumb_requested: RefCell<HashSet<String>>,
    older: RefCell<Option<String>>,
    older_button: gtk::Button,
    i_am_typing: RefCell<bool>,
    /// Message being replied to: (event id, preview).
    reply_to: RefCell<Option<(String, String)>>,
    reply_bar: gtk::Box,
    reply_label: gtk::Label,
    search_row: gtk::Box,
    search: gtk::SearchEntry,
    search_count: gtk::Label,
    /// Bumped on every query change/activation so a stale history scan stops.
    search_gen: std::cell::Cell<u32>,
}

pub fn open(app: &Rc<App>, room: Room, peer: Option<&str>) {
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
    fn new(app: &Rc<App>, room: Room, peer: Option<&str>) -> Rc<Self> {
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
        let avatar = gtk::Image::from_pixbuf(Some(&crate::ui::avatar::placeholder("")));
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.pack_start(&avatar, false, false, 0);
        head.pack_start(&title, true, true, 0);
        let search_btn = gtk::Button::with_label("🔍");
        search_btn.set_tooltip_text(Some("Search this conversation (Ctrl+F)"));
        head.pack_start(&search_btn, false, false, 0);
        root.pack_start(&head, false, false, 0);
        root.pack_start(&encrypted, false, false, 0);
        let is_room = peer.is_none();
        let (invite_btn, settings_btn, leave_btn) = (
            gtk::Button::with_label("Invite…"),
            gtk::Button::with_label("Members & settings…"),
            gtk::Button::with_label("Leave room"),
        );
        if is_room {
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            controls.pack_start(&invite_btn, false, false, 0);
            controls.pack_start(&settings_btn, false, false, 0);
            controls.pack_start(&leave_btn, false, false, 0);
            root.pack_start(&controls, false, false, 0);
        }

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search messages (Enter: also search older history)"));
        let search_count = gtk::Label::new(None);
        search_count.style_context().add_class("dim-label");
        search_count.set_ellipsize(gtk::pango::EllipsizeMode::End);
        search_count.set_max_width_chars(28);
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_row.pack_start(&search, true, true, 0);
        search_row.pack_start(&search_count, false, false, 0);
        search_row.set_no_show_all(true);
        search_row.set_visible(false);
        root.pack_start(&search_row, false, false, 0);

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
        let cancel_upload = gtk::Button::with_label("Cancel");
        cancel_upload.set_no_show_all(true);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status_row.pack_start(&typing, true, true, 0);
        status_row.pack_end(&cancel_upload, false, false, 0);
        root.pack_start(&status_row, false, false, 0);

        let reply_label = gtk::Label::new(None);
        reply_label.set_xalign(0.0);
        reply_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let reply_cancel = gtk::Button::with_label("✕");
        reply_cancel.set_relief(gtk::ReliefStyle::None);
        reply_cancel.set_tooltip_text(Some("Cancel reply"));
        let reply_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        reply_bar.pack_start(&reply_label, true, true, 0);
        reply_bar.pack_start(&reply_cancel, false, false, 0);
        reply_bar.set_no_show_all(true);
        reply_bar.set_visible(false);
        root.pack_start(&reply_bar, false, false, 0);

        let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let input = gtk::TextView::with_buffer(&buffer);
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        let in_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        in_scroll.set_min_content_height(56);
        in_scroll.set_shadow_type(gtk::ShadowType::In);
        in_scroll.add(&input);
        let send = gtk::Button::with_label("Send");
        let attach_btn = gtk::Button::with_label("📎");
        attach_btn.set_tooltip_text(Some("Send a file"));
        let emoji_btn = gtk::MenuButton::new();
        emoji_btn.set_label("😀");
        emoji_btn.set_tooltip_text(Some("Insert emoji"));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.pack_start(&in_scroll, true, true, 0);
        let side = gtk::Box::new(gtk::Orientation::Vertical, 4);
        side.pack_start(&emoji_btn, false, false, 0);
        side.pack_start(&attach_btn, false, false, 0);
        let calendar_btn = gtk::Button::with_label("📅");
        calendar_btn.set_tooltip_text(Some("Send a calendar invitation"));
        side.pack_start(&calendar_btn, false, false, 0);
        side.pack_start(&send, false, false, 0);
        row.pack_start(&side, false, false, 0);
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
            peer: peer.map(str::to_owned),
            app: app.clone(),
            conv: RefCell::new(Conversation::default()),
            list,
            scroll,
            title,
            avatar,
            typing,
            cancel_upload,
            upload_cancel: RefCell::new(None),
            input,
            buffer,
            member_count: RefCell::new(None),
            thumbs: RefCell::new(HashMap::new()),
            thumb_requested: RefCell::new(HashSet::new()),
            older: RefCell::new(None),
            older_button,
            i_am_typing: RefCell::new(false),
            reply_to: RefCell::new(None),
            reply_bar,
            reply_label,
            search_row,
            search,
            search_count,
            search_gen: std::cell::Cell::new(0),
        });

        let c = cw.clone();
        reply_cancel.connect_clicked(move |_| c.set_reply(None));
        let c = cw.clone();
        search_btn.connect_clicked(move |_| c.toggle_search());
        let c = cw.clone();
        cw.search.connect_search_changed(move |_| {
            c.search_gen.set(c.search_gen.get() + 1);
            c.redraw(false);
        });
        let c = cw.clone();
        cw.search.connect_activate(move |_| {
            c.search_gen.set(c.search_gen.get() + 1);
            c.search_older(c.search_gen.get(), SEARCH_PAGES);
        });
        let c = cw.clone();
        cw.window.connect_key_press_event(move |_, e| {
            if e.state().contains(gdk::ModifierType::CONTROL_MASK)
                && e.keyval() == gdk::keys::constants::f
            {
                c.toggle_search();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });

        let c = cw.clone();
        invite_btn.connect_clicked(move |_| {
            if let Some(id) = crate::ui::main_window::prompt(
                &c.app,
                "Invite to room",
                "Matrix ID (@user:server)",
                "",
            ) {
                let room = c.room.clone();
                let this = c.clone();
                spawn_async(
                    async move {
                        rooms::invite_user(&room, &id)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |res| {
                        let msg = match res {
                            Ok(()) => "Invitation sent".to_owned(),
                            Err(e) => format!("Could not invite: {e}"),
                        };
                        this.typing.set_text(&msg);
                    },
                );
            }
        });
        let c = cw.clone();
        settings_btn.connect_clicked(move |_| {
            crate::ui::room_dialog::open(&c.app, c.room.clone());
        });
        let c = cw.clone();
        leave_btn.connect_clicked(move |_| {
            let (app, rid) = (c.app.clone(), c.room.room_id().to_string());
            let this = c.clone();
            spawn_async(
                {
                    let (client, rid) = (app.client.borrow().clone(), rid.clone());
                    async move {
                        match client {
                            Some(client) => rooms::leave_room(&client, &rid)
                                .await
                                .map_err(|e| e.to_string()),
                            None => Err("not logged in".to_owned()),
                        }
                    }
                },
                move |res| match res {
                    Ok(()) => {
                        app.conversations.borrow_mut().remove(&rid);
                        this.window.close();
                        app.refresh();
                    }
                    Err(e) => this.typing.set_text(&format!("Could not leave: {e}")),
                },
            );
        });
        let c = cw.clone();
        cw.older_button.connect_clicked(move |_| c.load_older());
        let c = cw.clone();
        cw.cancel_upload.connect_clicked(move |_| {
            if let Some(tx) = c.upload_cancel.borrow().as_ref() {
                let _ = tx.try_send(());
            }
        });
        let c = cw.clone();
        calendar_btn.connect_clicked(move |_| c.calendar_invite_dialog(None));
        let c = cw.clone();
        attach_btn.connect_clicked(move |_| c.choose_and_send_file());
        let c = cw.clone();
        cw.input.connect_paste_clipboard(move |input| {
            if c.paste_image() {
                input.stop_signal_emission_by_name("paste-clipboard");
            }
        });
        // Dropping files from a file manager sends them (local files only).
        let c = cw.clone();
        cw.window.drag_dest_set(
            gtk::DestDefaults::ALL,
            &[gtk::TargetEntry::new(
                "text/uri-list",
                gtk::TargetFlags::OTHER_APP,
                0,
            )],
            gtk::gdk::DragAction::COPY,
        );
        cw.window
            .connect_drag_data_received(move |_, _, _, _, data, _, _| {
                for uri in data.uris() {
                    if let Some(path) = gtk::gio::File::for_uri(&uri).path() {
                        c.send_file(path);
                    }
                }
            });
        let pop = gtk::Popover::new(Some(&emoji_btn));
        let grid = gtk::Grid::new();
        grid.set_margin(6);
        for (i, e) in EMOJI_PICKER.iter().enumerate() {
            let b = gtk::Button::with_label(e);
            b.set_relief(gtk::ReliefStyle::None);
            let (c, pop2, e) = (cw.clone(), pop.clone(), *e);
            b.connect_clicked(move |_| {
                c.buffer.insert_at_cursor(e);
                pop2.popdown();
                c.input.grab_focus();
            });
            grid.attach(&b, (i % 8) as i32, (i / 8) as i32, 1, 1);
        }
        grid.show_all();
        pop.add(&grid);
        emoji_btn.set_popover(Some(&pop));
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
        let (name, sub) = match &self.peer {
            Some(peer) => (self.name_of(peer), peer.clone()),
            None => (
                rooms::cached_title(&self.room),
                format!(
                    "{} members",
                    self.member_count
                        .borrow()
                        .unwrap_or_else(|| self.room.joined_members_count() as usize)
                ),
            ),
        };
        let key = match &self.peer {
            Some(p) => format!("u:{p}"),
            None => App::room_avatar_key(&self.room),
        };
        self.avatar.set_from_pixbuf(Some(&self.app.avatar(&key)));
        self.window.set_title(&name);
        let badge = match &self.peer {
            Some(p) if self.app.verified_users.borrow().contains(p) => {
                " <span foreground='#2a9d3f'>✔ verified</span>"
            }
            _ => "",
        };
        self.title.set_markup(&format!(
            "<b>{}</b>{badge}  <small>{}</small>",
            glib::markup_escape_text(&name),
            glib::markup_escape_text(&sub)
        ));
    }

    /// Room-specific display names feed the shared resolver (below local nicknames).
    fn load_member_names(self: &Rc<Self>) {
        let room = self.room.clone();
        let app = self.app.clone();
        let rid = self.room.room_id().to_string();
        let this = self.clone();
        spawn_async(
            async move {
                let names = rooms::member_names(&room).await.unwrap_or_default();
                (names, rooms::room_title(&room).await)
            },
            move |(names, _title)| {
                *this.member_count.borrow_mut() = Some(names.len());
                {
                    let mut m = app.mgr.borrow_mut();
                    for (mxid, name) in names {
                        m.resolver_mut().set_room_name(&rid, &mxid, name.as_deref());
                    }
                }
                app.refresh();
            },
        );
    }

    fn load_initial(self: &Rc<Self>) {
        self.fetch(None);
        self.load_member_names();
    }

    fn load_older(self: &Rc<Self>) {
        let token = self.older.borrow().clone();
        if token.is_some() {
            self.fetch(token);
        }
    }

    /// Load older pages while a query is active so it covers more than what is
    /// on screen; stops at the start of the room, the page cap, or a new query.
    fn search_older(self: &Rc<Self>, generation: u32, pages_left: u32) {
        if self.search_gen.get() != generation || self.search.text().trim().is_empty() {
            return;
        }
        let Some(token) = self.older.borrow().clone() else {
            self.note_search_end("reached the start of the conversation");
            return;
        };
        if pages_left == 0 {
            self.note_search_end("stopped; press Enter to search further back");
            return;
        }
        self.search_count.set_text(&format!(
            "{} found · searching older…",
            self.search_matches()
        ));
        let (room, this) = (self.room.clone(), self.clone());
        spawn_async(
            async move {
                rooms::load_history(&room, Some(&token), PAGE)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                if this.search_gen.get() != generation {
                    return;
                }
                match res {
                    Ok(h) => {
                        for ev in h.events {
                            this.conv.borrow_mut().apply(ev);
                        }
                        this.older_button.set_visible(h.older_token.is_some());
                        *this.older.borrow_mut() = h.older_token;
                        this.redraw(false);
                        this.search_older(generation, pages_left - 1);
                    }
                    Err(e) => this.note_search_end(&format!("history failed: {e}")),
                }
            },
        );
    }

    fn search_matches(&self) -> usize {
        let query = self.search.text().to_string();
        self.conv
            .borrow()
            .messages()
            .iter()
            .filter(|m| m.matches(&query))
            .count()
    }

    fn note_search_end(&self, why: &str) {
        self.search_count
            .set_text(&format!("{} found · {why}", self.search_matches()));
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
        let reply = self.reply_to.borrow().as_ref().map(|(id, _)| id.clone());
        self.set_reply(None);
        let txn = matrix_sdk::ruma::TransactionId::new();
        self.conv.borrow_mut().add_pending(
            txn.as_str(),
            &self.me(),
            &body,
            now_ms(),
            reply.as_deref(),
        );
        self.redraw(true);
        self.deliver(txn.to_string(), body, reply);
    }

    fn set_reply(&self, target: Option<(String, String)>) {
        match &target {
            Some((_, preview)) => {
                self.reply_label.set_markup(&format!(
                    "↪ Replying to <i>{}</i>",
                    glib::markup_escape_text(preview)
                ));
                self.reply_bar.set_no_show_all(false);
                self.reply_bar.show_all();
                self.input.grab_focus();
            }
            None => self.reply_bar.hide(),
        }
        *self.reply_to.borrow_mut() = target;
    }

    fn toggle_search(self: &Rc<Self>) {
        if self.search_row.is_visible() {
            self.search.set_text("");
            self.search_row.hide();
            self.redraw(false);
        } else {
            self.search_row.set_no_show_all(false);
            self.search_row.show_all();
            self.search.grab_focus();
        }
    }

    fn deliver(self: &Rc<Self>, txn: String, body: String, reply: Option<String>) {
        let room = self.room.clone();
        let this = self.clone();
        let txn_id = OwnedTransactionId::from(txn.as_str());
        spawn_async(
            async move {
                rooms::send_text(&room, &body, txn_id, reply.as_deref())
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
        let found = self
            .conv
            .borrow()
            .messages()
            .iter()
            .find(|m| m.txn_id.as_deref() == Some(txn))
            .map(|m| (m.body.clone(), m.reply_to.clone()));
        if let Some((body, reply)) = found {
            self.conv.borrow_mut().mark_pending(txn);
            self.redraw(false);
            self.deliver(txn.to_owned(), body, reply);
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
        // Organiser changes may sit later in the history than the invitation they change.
        for m in self.conv.borrow().messages() {
            if let (Some(e), false) = (m.calendar.as_deref(), m.redacted) {
                let _ = self.app.db.note_calendar_change(&m.sender, e);
            }
        }
        let query = self.search.text().to_string();
        let searching = !query.trim().is_empty();
        let msgs: Vec<ChatMessage> = self
            .conv
            .borrow()
            .messages()
            .iter()
            .filter(|m| !searching || m.matches(&query))
            .cloned()
            .collect();
        self.search_count.set_text(&if searching {
            format!("{} found", msgs.len())
        } else {
            String::new()
        });
        for m in &msgs {
            self.list
                .pack_start(&self.message_row(m, &me), false, false, 0);
        }
        self.list.show_all();
        if scroll_to_end {
            let adj = self.scroll.vadjustment();
            // Wait for the new rows (images, buttons) to be allocated before measuring the end.
            let scroll = move || adj.set_value(adj.upper() - adj.page_size());
            glib::idle_add_local_once(scroll.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(120), scroll);
        }
    }

    /// Scroll to the message with `event_id`, clearing a search that hides it.
    /// Does nothing if it is older than what has been loaded.
    fn jump_to(self: &Rc<Self>, event_id: &str) {
        if self.conv.borrow().find(event_id).is_none() {
            self.typing
                .set_text("That message is older than what is loaded; use “Load older messages”.");
            return;
        }
        if !self.search.text().trim().is_empty() {
            self.search.set_text("");
            self.redraw(false);
        }
        let (this, id) = (self.clone(), event_id.to_owned());
        glib::idle_add_local_once(move || {
            let target = this
                .list
                .children()
                .into_iter()
                .find(|c| c.widget_name() == id);
            if let Some(y) = target.and_then(|w| w.translate_coordinates(&this.list, 0, 0)) {
                let adj = this.scroll.vadjustment();
                let y = f64::from(y.1).clamp(0.0, (adj.upper() - adj.page_size()).max(0.0));
                adj.set_value(y);
            }
        });
    }

    fn message_row(self: &Rc<Self>, m: &ChatMessage, me: &str) -> gtk::Widget {
        let name = self.name_of(&m.sender);
        let time = glib::DateTime::from_unix_local((m.ts / 1000) as i64)
            .ok()
            .and_then(|d| d.format("%H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let text = if let (Some(a), false) = (&m.attachment, m.redacted) {
            let icon = match a.kind {
                AttachmentKind::Image => "🖼",
                AttachmentKind::Audio => "🎵",
                AttachmentKind::Video => "🎬",
                AttachmentKind::File => "📎",
            };
            let size = a
                .size
                .map(|s| format!(" ({})", media::human_size(s)))
                .unwrap_or_default();
            let name = glib::markup_escape_text(&media::sanitize_filename(&a.filename));
            // A caption differs from the filename; show it too.
            let caption = if !m.body.is_empty() && m.body != a.filename {
                format!("\n{}", glib::markup_escape_text(&m.body))
            } else {
                String::new()
            };
            format!("{icon} {name}{size}{caption}")
        } else if m.redacted {
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
        let edited = if m.edited && !m.redacted {
            " <small>(edited)</small>"
        } else {
            ""
        };
        let label = gtk::Label::new(None);
        let (shield, why) = match m.trust {
            Trust::Verified => (
                " <span foreground='#2a9d3f'>✔</span>",
                "End-to-end encrypted, sent from a device you have verified",
            ),
            Trust::Unverified => (
                " <span foreground='#c77700'>⚠</span>",
                "End-to-end encrypted, but the sending device is not verified",
            ),
            Trust::Unencrypted => ("", ""),
        };
        label.set_markup(&format!(
            "<small>{time}</small> <b>{}</b>{shield}: {text}{edited}{mark}",
            glib::markup_escape_text(&name)
        ));
        if !why.is_empty() {
            label.set_tooltip_text(Some(why));
        }
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

        if let (Some(att), false) = (&m.attachment, m.redacted) {
            let save = gtk::Button::with_label("Save…");
            let open = gtk::Button::with_label("Open");
            let (this, a, id) = (self.clone(), att.clone(), m.id.clone());
            save.connect_clicked(move |_| this.save_attachment(&a));
            let (this, a) = (self.clone(), att.clone());
            open.connect_clicked(move |_| this.open_attachment(&id, &a));
            row.pack_start(&save, false, false, 0);
            row.pack_start(&open, false, false, 0);
        }

        // A selectable label handles right-clicks itself, so extend its own menu.
        if m.delivery == Delivery::Sent && !m.redacted && m.id.starts_with('$') {
            let (this, msg, mine) = (self.clone(), m.clone(), m.sender == me);
            label.connect_populate_popup(move |_, menu| {
                this.extend_menu(menu.upcast_ref(), &msg, mine);
            });
        }

        let outer = gtk::Box::new(gtk::Orientation::Vertical, 2);
        outer.set_widget_name(&m.id);
        if let Some(target) = &m.reply_to {
            let quote = gtk::Label::new(None);
            quote.set_xalign(0.0);
            quote.set_margin_start(16);
            quote.set_ellipsize(gtk::pango::EllipsizeMode::End);
            quote.style_context().add_class("dim-label");
            let text = match self.conv.borrow().find(target) {
                Some(orig) => format!(
                    "↪ <b>{}</b>: {}",
                    glib::markup_escape_text(&self.name_of(&orig.sender)),
                    glib::markup_escape_text(&orig.snippet(80))
                ),
                None => "↪ an earlier message".to_owned(),
            };
            quote.set_markup(&format!("<small>{text}</small>"));
            let click = gtk::EventBox::new();
            click.add(&quote);
            click.set_tooltip_text(Some("Jump to the original message"));
            let (this, target) = (self.clone(), target.clone());
            click.connect_button_press_event(move |_, ev| {
                if ev.button() == 1 {
                    this.jump_to(&target);
                }
                glib::Propagation::Stop
            });
            outer.pack_start(&click, false, false, 0);
        }
        outer.pack_start(&row, false, false, 0);
        if let (Some(entry), false) = (&m.calendar, m.redacted)
            && let msm::calendar::Entry::Invite(inv) = &**entry
        {
            outer.pack_start(&self.invite_card(m, inv, m.sender == me), false, false, 0);
        }
        if let (Some(att), false) = (&m.attachment, m.redacted)
            && att.kind == AttachmentKind::Image
        {
            self.image_preview(&outer, &m.id, att);
        }
        let chips = self.reaction_chips(m, me);
        if let Some(chips) = chips {
            outer.pack_start(&chips, false, false, 0);
        }
        outer.upcast()
    }

    /// The invitation's time and, for the invitee, Tentative / Accept / Decline; for the
    /// organiser, Edit / Cancel event. Shows the organiser's latest changes.
    fn invite_card(
        self: &Rc<Self>,
        m: &ChatMessage,
        inv: &msm::calendar::Invite,
        mine: bool,
    ) -> gtk::Widget {
        use msm::calendar::{Response, format_start};
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        card.set_margin_start(16);
        let change = self
            .app
            .db
            .calendar_change(&m.id, &m.sender)
            .unwrap_or_default();
        let mut shown = inv.clone();
        if let Some((title, start_ms, description)) = &change.update {
            shown = msm::calendar::Invite {
                title: title.clone(),
                start_ms: *start_ms,
                description: description.clone(),
            };
        }
        let current = self.app.db.calendar_response(&m.id).ok().flatten();
        let status = if change.cancelled {
            "Cancelled by the organiser".to_owned()
        } else if mine {
            if change.update.is_some() {
                "Your invitation (rescheduled)".to_owned()
            } else {
                "Your invitation".to_owned()
            }
        } else {
            let moved = if change.update.is_some() {
                " · changed by the organiser"
            } else {
                ""
            };
            match current {
                Some(r) => format!(
                    "You answered: {}{moved}",
                    r.label()
                        .replace("Accept", "Accepted")
                        .replace("Decline", "Declined")
                ),
                None => format!("Not on your calendar until you answer{moved}"),
            }
        };
        let title = glib::markup_escape_text(&shown.title);
        let title = if change.cancelled {
            format!("<s>{title}</s>")
        } else {
            title.to_string()
        };
        let info = gtk::Label::new(None);
        info.set_xalign(0.0);
        info.set_line_wrap(true);
        info.set_markup(&format!(
            "<b>{title}</b> · {}\n<small>{}</small>",
            glib::markup_escape_text(&format_start(shown.start_ms)),
            glib::markup_escape_text(&status)
        ));
        card.pack_start(&info, true, true, 0);
        if !change.cancelled && m.id.starts_with('$') {
            if mine {
                let edit = gtk::Button::with_label("Edit…");
                let (this, id, cur) = (self.clone(), m.id.clone(), shown.clone());
                edit.connect_clicked(move |_| {
                    this.calendar_invite_dialog(Some((id.clone(), cur.clone())))
                });
                let cancel = gtk::Button::with_label("Cancel event");
                let (this, id, cur) = (self.clone(), m.id.clone(), shown.clone());
                cancel.connect_clicked(move |_| this.cancel_invite(&id, &cur));
                card.pack_start(&edit, false, false, 0);
                card.pack_start(&cancel, false, false, 0);
            } else {
                for r in Response::ALL {
                    let b = gtk::Button::with_label(r.label());
                    b.set_sensitive(current != Some(r));
                    let (this, id, inv, sender) =
                        (self.clone(), m.id.clone(), shown.clone(), m.sender.clone());
                    b.connect_clicked(move |_| this.answer_invite(&id, &sender, &inv, r));
                    card.pack_start(&b, false, false, 0);
                }
            }
        }
        card.upcast()
    }

    /// Cancel one of our own invitations after confirming.
    fn cancel_invite(self: &Rc<Self>, event_id: &str, inv: &msm::calendar::Invite) {
        let d = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Question,
            gtk::ButtonsType::None,
            &format!("Cancel “{}” for everyone in this conversation?", inv.title),
        );
        d.add_button("Keep it", gtk::ResponseType::Cancel);
        d.add_button("Cancel event", gtk::ResponseType::Accept);
        d.set_default_response(gtk::ResponseType::Cancel);
        let ok = d.run() == gtk::ResponseType::Accept;
        d.close();
        if !ok {
            return;
        }
        let entry = msm::calendar::Entry::Cancel {
            event_id: event_id.to_owned(),
        };
        let _ = self.app.db.note_calendar_change(&self.me(), &entry);
        self.redraw(false);
        let (room, this) = (self.room.clone(), self.clone());
        let (title, id) = (inv.title.clone(), event_id.to_owned());
        spawn_async(
            async move {
                rooms::send_cancel(&room, &title, &id)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                if let Err(e) = res {
                    this.typing.set_text(&format!(
                        "Cancelled here, but the cancellation was not sent: {e}"
                    ));
                }
            },
        );
    }

    fn answer_invite(
        self: &Rc<Self>,
        event_id: &str,
        sender: &str,
        inv: &msm::calendar::Invite,
        response: msm::calendar::Response,
    ) {
        let entry = msm::storage::database::CalendarEntry {
            event_id: event_id.to_owned(),
            room_id: self.room.room_id().to_string(),
            sender: sender.to_owned(),
            title: inv.title.clone(),
            start_ms: inv.start_ms,
            description: inv.description.clone(),
            response,
            repeat: msm::calendar::Repeat::Never,
        };
        if let Err(e) = self.app.db.save_calendar_entry(&entry) {
            self.typing
                .set_text(&format!("Could not save to your calendar: {e}"));
            return;
        }
        self.redraw(false);
        let (room, this) = (self.room.clone(), self.clone());
        let (title, id) = (inv.title.clone(), event_id.to_owned());
        spawn_async(
            async move {
                rooms::send_rsvp(&room, &title, &id, response)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(()) => this.typing.set_text(""),
                Err(e) => this
                    .typing
                    .set_text(&format!("Saved locally, but the answer was not sent: {e}")),
            },
        );
    }

    /// New invitation, or (with `existing`) change one of ours that is already sent.
    fn calendar_invite_dialog(self: &Rc<Self>, existing: Option<(String, msm::calendar::Invite)>) {
        let invite = crate::ui::calendar_window::event_form(
            self.window.upcast_ref(),
            if existing.is_some() {
                "Edit invitation"
            } else {
                "Calendar invitation"
            },
            "Send",
            existing.as_ref().map(|(_, inv)| inv),
        );
        let Some(invite) = invite else { return };
        if let Some((event_id, _)) = existing {
            let entry = msm::calendar::Entry::Update {
                event_id: event_id.clone(),
                invite: invite.clone(),
            };
            let _ = self.app.db.note_calendar_change(&self.me(), &entry);
            self.redraw(false);
            let (room, this) = (self.room.clone(), self.clone());
            spawn_async(
                async move {
                    rooms::send_update(&room, &event_id, &invite)
                        .await
                        .map_err(|e| e.to_string())
                },
                move |res| {
                    if let Err(e) = res {
                        this.typing
                            .set_text(&format!("Changed here, but the update was not sent: {e}"));
                    }
                },
            );
            return;
        }
        let (room, this) = (self.room.clone(), self.clone());
        let inv = invite.clone();
        self.typing.set_text("Sending invitation…");
        spawn_async(
            async move {
                rooms::send_invite(&room, &inv)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(event_id) => {
                    this.typing.set_text("");
                    // The organiser's own event goes straight onto their calendar.
                    let me = this.me();
                    let _ =
                        this.app
                            .db
                            .save_calendar_entry(&msm::storage::database::CalendarEntry {
                                event_id,
                                room_id: this.room.room_id().to_string(),
                                sender: me,
                                title: invite.title,
                                start_ms: invite.start_ms,
                                description: invite.description,
                                response: msm::calendar::Response::Accepted,
                                repeat: msm::calendar::Repeat::Never,
                            });
                }
                Err(e) => this
                    .typing
                    .set_text(&format!("Could not send the invitation: {e}")),
            },
        );
    }

    fn extend_menu(self: &Rc<Self>, menu: &gtk::Menu, m: &ChatMessage, mine: bool) {
        menu.append(&gtk::SeparatorMenuItem::new());
        let react = gtk::MenuItem::with_label("React");
        let sub = gtk::Menu::new();
        for emoji in QUICK_REACTIONS {
            let (this, id) = (self.clone(), m.id.clone());
            let item = gtk::MenuItem::with_label(emoji);
            item.connect_activate(move |_| this.toggle_reaction(&id, emoji));
            sub.append(&item);
        }
        react.set_submenu(Some(&sub));
        menu.append(&react);
        let (this, id, preview) = (self.clone(), m.id.clone(), m.snippet(60));
        let reply = gtk::MenuItem::with_label("Reply");
        reply.connect_activate(move |_| this.set_reply(Some((id.clone(), preview.clone()))));
        menu.append(&reply);
        if mine {
            let (this, id, body) = (self.clone(), m.id.clone(), m.body.clone());
            let edit = gtk::MenuItem::with_label("Edit message…");
            edit.connect_activate(move |_| this.edit_message(&id, &body));
            menu.append(&edit);
            let (this, id) = (self.clone(), m.id.clone());
            let del = gtk::MenuItem::with_label("Delete message");
            del.connect_activate(move |_| this.delete(&id));
            menu.append(&del);
        }
        menu.show_all();
    }

    fn client(&self) -> Option<matrix_sdk::Client> {
        self.app.client.borrow().clone()
    }

    /// Show a cached preview, or fetch one (small images only) and redraw when it arrives.
    fn image_preview(self: &Rc<Self>, outer: &gtk::Box, id: &str, att: &Attachment) {
        const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
        if let Some(pb) = self.thumbs.borrow().get(id) {
            let img = gtk::Image::from_pixbuf(Some(pb));
            img.set_halign(gtk::Align::Start);
            img.set_margin_start(16);
            outer.pack_start(&img, false, false, 0);
            return;
        }
        if att.size.is_some_and(|s| s > MAX_PREVIEW_BYTES)
            || !self.thumb_requested.borrow_mut().insert(id.to_owned())
        {
            return;
        }
        let Some(client) = self.client() else { return };
        let (source, size, this, id) = (att.source.clone(), att.size, self.clone(), id.to_owned());
        spawn_async(
            async move { media::download(&client, &source, size).await.ok() },
            move |bytes| {
                let Some(bytes) = bytes else { return };
                if let Some(pb) = decode_preview(&bytes) {
                    this.thumbs.borrow_mut().insert(id, pb);
                    this.redraw(false);
                }
            },
        );
    }

    fn choose_and_send_file(self: &Rc<Self>) {
        let d = gtk::FileChooserDialog::with_buttons(
            Some("Send a file"),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Send", gtk::ResponseType::Accept),
            ],
        );
        let chosen = (d.run() == gtk::ResponseType::Accept)
            .then(|| d.filename())
            .flatten();
        d.close();
        if let Some(path) = chosen {
            self.send_file(path);
        }
    }

    fn send_file(self: &Rc<Self>, path: std::path::PathBuf) {
        self.upload(path, false);
    }

    /// Paste an image from the clipboard as a PNG attachment. Returns false if
    /// the clipboard holds no image (the normal text paste then proceeds).
    fn paste_image(self: &Rc<Self>) -> bool {
        let clipboard = gtk::Clipboard::default(&gtk::gdk::Display::default().expect("display"))
            .expect("clipboard");
        if !clipboard.wait_is_image_available() {
            return false;
        }
        let Some(pixbuf) = clipboard.wait_for_image() else {
            return false;
        };
        let stamp = glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y%m%d-%H%M%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("msm-paste-{}", std::process::id()));
        let path = dir.join(format!("pasted-image-{stamp}.png"));
        let saved =
            std::fs::create_dir_all(&dir).is_ok() && pixbuf.savev(&path, "png", &[]).is_ok();
        if !saved {
            self.typing.set_text("Could not read the pasted image.");
            return true;
        }
        self.upload(path, true);
        true
    }

    /// Upload `path`; `temporary` files are deleted afterwards.
    fn upload(self: &Rc<Self>, path: std::path::PathBuf, temporary: bool) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.upload_cancel.borrow().is_some() {
            if temporary {
                let _ = std::fs::remove_file(&path);
            }
            self.typing
                .set_text("Wait for the current upload to finish (or cancel it).");
            return;
        }
        self.typing.set_text(&format!("Uploading {name}…"));
        let (cancel_tx, cancel_rx) = async_channel::bounded(1);
        let (progress_tx, progress_rx) = async_channel::unbounded();
        *self.upload_cancel.borrow_mut() = Some(cancel_tx);
        self.cancel_upload.show();
        let this = self.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((sent, total)) = progress_rx.recv().await {
                if total > 0 && this.upload_cancel.borrow().is_some() {
                    this.typing
                        .set_text(&format!("Uploading {name}… {}%", sent * 100 / total));
                }
            }
        });
        let (room, this) = (self.room.clone(), self.clone());
        spawn_async(
            async move {
                let res = media::send_file_with(
                    &room,
                    &path,
                    media::Transfer {
                        progress: Some(progress_tx),
                        cancel: Some(cancel_rx),
                    },
                )
                .await;
                if temporary {
                    let _ = std::fs::remove_file(&path);
                }
                res
            },
            move |res| {
                this.upload_cancel.borrow_mut().take();
                this.cancel_upload.hide();
                match res {
                    // The message itself arrives through the live event feed.
                    Ok(_) => this.typing.set_text(""),
                    Err(media::MediaError::Cancelled) => this.typing.set_text("Upload cancelled."),
                    Err(e) => this.typing.set_text(&format!("Could not send file: {e}")),
                }
            },
        );
    }

    fn save_attachment(self: &Rc<Self>, att: &Attachment) {
        let d = gtk::FileChooserDialog::with_buttons(
            Some("Save attachment"),
            Some(&self.window),
            gtk::FileChooserAction::Save,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Save", gtk::ResponseType::Accept),
            ],
        );
        d.set_do_overwrite_confirmation(true);
        d.set_current_name(&media::sanitize_filename(&att.filename));
        let chosen = (d.run() == gtk::ResponseType::Accept)
            .then(|| d.filename())
            .flatten();
        d.close();
        let (Some(path), Some(client)) = (chosen, self.client()) else {
            return;
        };
        let (source, size, this) = (att.source.clone(), att.size, self.clone());
        self.typing.set_text("Downloading…");
        spawn_async(
            async move {
                let bytes = media::download(&client, &source, size)
                    .await
                    .map_err(|e| e.to_string())?;
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            },
            move |res| match res {
                Ok(p) => this.typing.set_text(&format!("Saved to {}", p.display())),
                Err(e) => this.typing.set_text(&format!("Could not save: {e}")),
            },
        );
    }

    /// Download to a private cache folder (one subfolder per message) and open with the default app.
    fn open_attachment(self: &Rc<Self>, event_id: &str, att: &Attachment) {
        let Some(client) = self.client() else { return };
        let folder: String = event_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(24)
            .collect();
        let dir = msm::matrix::authentication::data_dir()
            .join("opened")
            .join(folder);
        let path = dir.join(media::sanitize_filename(&att.filename));
        let (source, size, this) = (att.source.clone(), att.size, self.clone());
        self.typing.set_text("Downloading…");
        spawn_async(
            async move {
                let bytes = media::download(&client, &source, size)
                    .await
                    .map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            },
            move |res| match res {
                Ok(p) => {
                    this.typing.set_text("");
                    let uri = gtk::gio::File::for_path(&p).uri();
                    if let Err(e) = gtk::gio::AppInfo::launch_default_for_uri(
                        &uri,
                        None::<&gtk::gio::AppLaunchContext>,
                    ) {
                        this.typing
                            .set_text(&format!("No application could open it: {e}"));
                    }
                }
                Err(e) => this.typing.set_text(&format!("Could not open: {e}")),
            },
        );
    }

    /// Clickable reaction summary under a message; clicking toggles our own reaction.
    fn reaction_chips(self: &Rc<Self>, m: &ChatMessage, me: &str) -> Option<gtk::Box> {
        let summary = self.conv.borrow().reactions_for(&m.id);
        if summary.is_empty() || m.redacted {
            return None;
        }
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.set_margin_start(16);
        for r in summary {
            let mine = r.senders.iter().any(|s| s == me);
            let btn = gtk::Button::new();
            btn.set_relief(gtk::ReliefStyle::None);
            let label = gtk::Label::new(None);
            let text = format!("{} {}", glib::markup_escape_text(&r.key), r.senders.len());
            label.set_markup(&if mine { format!("<b>{text}</b>") } else { text });
            btn.add(&label);
            let names: Vec<String> = r.senders.iter().map(|s| self.name_of(s)).collect();
            btn.set_tooltip_text(Some(&names.join(", ")));
            let (this, id, key) = (self.clone(), m.id.clone(), r.key.clone());
            btn.connect_clicked(move |_| this.toggle_reaction(&id, &key));
            chips.pack_start(&btn, false, false, 0);
        }
        Some(chips)
    }

    /// Add our reaction, or remove it if we already reacted with the same emoji.
    fn toggle_reaction(self: &Rc<Self>, target: &str, key: &str) {
        let me = self.me();
        let existing = self.conv.borrow().own_reaction_event(target, key, &me);
        let (room, this) = (self.room.clone(), self.clone());
        let (target, key) = (target.to_owned(), key.to_owned());
        match existing {
            Some(rid) => spawn_async(
                {
                    let rid = rid.clone();
                    async move { rooms::redact(&room, &rid).await.map_err(|e| e.to_string()) }
                },
                move |res| match res {
                    Ok(()) => {
                        this.conv.borrow_mut().redact(&rid);
                        this.redraw(false);
                    }
                    Err(e) => this
                        .typing
                        .set_text(&format!("Could not remove reaction: {e}")),
                },
            ),
            None => spawn_async(
                {
                    let (target, key) = (target.clone(), key.clone());
                    async move {
                        rooms::send_reaction(&room, &target, &key)
                            .await
                            .map_err(|e| e.to_string())
                    }
                },
                move |res| match res {
                    Ok(event_id) => {
                        let me = this.me();
                        this.conv.borrow_mut().add_reaction(Reaction {
                            event_id,
                            target,
                            key,
                            sender: me,
                        });
                        this.redraw(false);
                    }
                    Err(e) => this.typing.set_text(&format!("Could not react: {e}")),
                },
            ),
        }
    }

    fn edit_message(self: &Rc<Self>, event_id: &str, current: &str) {
        let Some(new_body) =
            crate::ui::main_window::prompt(&self.app, "Edit message", "Message", current)
        else {
            return;
        };
        if new_body.trim().is_empty() || new_body == current {
            return;
        }
        let (room, id, this) = (self.room.clone(), event_id.to_owned(), self.clone());
        spawn_async(
            async move {
                rooms::send_edit(&room, &id, &new_body)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                // The edit shows up through the live event feed.
                if let Err(e) = res {
                    this.typing
                        .set_text(&format!("Could not edit message: {e}"));
                }
            },
        );
    }
}

/// Decode an image and scale it down for inline display (untrusted data: failures are ignored).
fn decode_preview(bytes: &[u8]) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    const MAX_W: i32 = 260;
    const MAX_H: i32 = 200;
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pb = loader.pixbuf()?;
    let (w, h) = (pb.width(), pb.height());
    if w <= 0 || h <= 0 {
        return None;
    }
    let scale = (f64::from(MAX_W) / f64::from(w))
        .min(f64::from(MAX_H) / f64::from(h))
        .min(1.0);
    let (nw, nh) = (
        ((f64::from(w) * scale) as i32).max(1),
        ((f64::from(h) * scale) as i32).max(1),
    );
    pb.scale_simple(nw, nh, gtk::gdk_pixbuf::InterpType::Bilinear)
}
