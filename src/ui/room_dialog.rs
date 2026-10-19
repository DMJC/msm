//! Room settings and member moderation. Controls are only enabled when the
//! room's power levels allow us to use them; the server has the final say.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use matrix_sdk::Room;
use msm::matrix::rooms::{self, MemberState, RoomSettings};

use crate::app::{App, spawn_async};
use crate::ui::main_window::{error_dialog, prompt};

struct Dialog {
    app: Rc<App>,
    room: Room,
    name: gtk::Entry,
    topic: gtk::Entry,
    save: gtk::Button,
    status: gtk::Label,
    members: gtk::Box,
}

pub fn open(app: &Rc<App>, room: Room) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Room settings");
    window.set_transient_for(Some(&app.window));
    window.set_default_size(420, 480);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin(12);

    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Room name"));
    let topic = gtk::Entry::new();
    topic.set_placeholder_text(Some("Topic"));
    let save = gtk::Button::with_label("Save");
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_line_wrap(true);
    status.style_context().add_class("dim-label");
    let members = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&members);
    let heading = gtk::Label::new(None);
    heading.set_markup("<b>Members</b>");
    heading.set_xalign(0.0);

    root.pack_start(&name, false, false, 0);
    root.pack_start(&topic, false, false, 0);
    root.pack_start(&save, false, false, 0);
    root.pack_start(&status, false, false, 0);
    root.pack_start(&heading, false, false, 0);
    root.pack_start(&scroll, true, true, 0);
    window.add(&root);

    let d = Rc::new(Dialog {
        app: app.clone(),
        room,
        name,
        topic,
        save,
        status,
        members,
    });
    let dd = d.clone();
    d.save.connect_clicked(move |_| dd.save_settings());
    d.load(true);
    window.show_all();
}

impl Dialog {
    /// Fetch settings and rebuild the member list. `fill_fields` also resets the
    /// name/topic entries (not after a member action, which would discard edits).
    fn load(self: &Rc<Self>, fill_fields: bool) {
        let (this, room) = (self.clone(), self.room.clone());
        spawn_async(
            async move { rooms::room_settings(&room).await.map_err(|e| e.to_string()) },
            move |res| match res {
                Ok(s) => this.show(&s, fill_fields),
                Err(e) => this.status.set_text(&format!("Could not load: {e}")),
            },
        );
    }

    fn show(self: &Rc<Self>, s: &RoomSettings, fill_fields: bool) {
        if fill_fields {
            self.name.set_text(s.name.as_deref().unwrap_or(""));
            self.topic.set_text(s.topic.as_deref().unwrap_or(""));
        }
        self.name.set_sensitive(s.can_rename);
        self.topic.set_sensitive(s.can_set_topic);
        self.save.set_sensitive(s.can_rename || s.can_set_topic);
        if !(s.can_rename || s.can_set_topic) {
            self.status
                .set_text("You don't have permission to change this room's settings.");
        }
        for child in self.members.children() {
            self.members.remove(&child);
        }
        for m in &s.members {
            let shown = self.app.mgr.borrow().resolver().resolve(&m.mxid, None);
            let tag = match m.membership {
                MemberState::Joined if m.power >= 100 => " (admin)",
                MemberState::Joined if m.power >= 50 => " (moderator)",
                MemberState::Joined => "",
                MemberState::Invited => " (invited)",
                MemberState::Banned => " (banned)",
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_line_wrap(true);
            label.set_markup(&format!(
                "{}{}\n<small>{}</small>",
                glib::markup_escape_text(&shown),
                tag,
                glib::markup_escape_text(&m.mxid)
            ));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.pack_start(&label, true, true, 0);
            if m.can_set_role {
                row.pack_end(&self.role_button(m), false, false, 0);
            }
            if m.can_kick {
                row.pack_end(&self.action("Kick", &m.mxid, Action::Kick), false, false, 0);
            }
            if m.can_ban {
                row.pack_end(&self.action("Ban", &m.mxid, Action::Ban), false, false, 0);
            }
            if m.can_unban {
                row.pack_end(
                    &self.action("Unban", &m.mxid, Action::Unban),
                    false,
                    false,
                    0,
                );
            }
            self.members.pack_start(&row, false, false, 0);
        }
        self.members.show_all();
    }

    fn role_button(self: &Rc<Self>, m: &rooms::MemberRow) -> gtk::MenuButton {
        let button = gtk::MenuButton::new();
        button.set_label("Role");
        let menu = gtk::Menu::new();
        for (level, name) in rooms::ROLES {
            let item = gtk::CheckMenuItem::with_label(name);
            item.set_draw_as_radio(true);
            item.set_active(m.power == level);
            let (this, mxid) = (self.clone(), m.mxid.clone());
            item.connect_activate(move |i| {
                // Activation also fires when the check state is set programmatically.
                if i.is_active() {
                    this.set_role(&mxid, level);
                }
            });
            menu.append(&item);
        }
        menu.show_all();
        button.set_popup(Some(&menu));
        button
    }

    fn set_role(self: &Rc<Self>, mxid: &str, level: i64) {
        let (this, room, id) = (self.clone(), self.room.clone(), mxid.to_owned());
        spawn_async(
            async move {
                rooms::set_role(&room, &id, level)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(()) => {
                    this.status.set_text("Role changed.");
                    this.load(false);
                    let later = this.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(2000),
                        move || later.load(false),
                    );
                }
                Err(e) => error_dialog(&this.app, &format!("Could not change the role: {e}")),
            },
        );
    }

    fn action(self: &Rc<Self>, label: &str, mxid: &str, action: Action) -> gtk::Button {
        let b = gtk::Button::with_label(label);
        let (this, mxid) = (self.clone(), mxid.to_owned());
        b.connect_clicked(move |_| this.moderate(&mxid, action));
        b
    }

    fn moderate(self: &Rc<Self>, mxid: &str, action: Action) {
        let reason = match action {
            Action::Unban => String::new(),
            _ => {
                let title = match action {
                    Action::Kick => format!("Remove {mxid} from this room?"),
                    _ => format!("Ban {mxid} from this room?"),
                };
                match prompt(&self.app, &title, "Reason (optional)", "") {
                    Some(r) => r,
                    None => return,
                }
            }
        };
        let (this, room, id) = (self.clone(), self.room.clone(), mxid.to_owned());
        spawn_async(
            async move {
                match action {
                    Action::Kick => rooms::kick(&room, &id, &reason).await,
                    Action::Ban => rooms::ban(&room, &id, &reason).await,
                    Action::Unban => rooms::unban(&room, &id).await,
                }
                .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(()) => {
                    this.status.set_text("Done.");
                    // Membership shows up in the local state only after the next sync.
                    this.load(false);
                    let later = this.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(2000),
                        move || later.load(false),
                    );
                }
                Err(e) => error_dialog(&this.app, &format!("Could not do that: {e}")),
            },
        );
    }

    fn save_settings(self: &Rc<Self>) {
        let (name, topic) = (self.name.text().to_string(), self.topic.text().to_string());
        let (change_name, change_topic) = (self.name.is_sensitive(), self.topic.is_sensitive());
        let (this, room) = (self.clone(), self.room.clone());
        self.status.set_text("Saving…");
        spawn_async(
            async move {
                if change_name
                    && !name.trim().is_empty()
                    && Some(name.trim()) != room.name().as_deref()
                {
                    rooms::set_room_name(&room, &name)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                if change_topic && Some(topic.trim()) != room.topic().as_deref().or(Some("")) {
                    rooms::set_room_topic(&room, &topic)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(())
            },
            move |res| match res {
                Ok(()) => {
                    this.status.set_text("Saved.");
                    crate::ui::main_window::refresh(&this.app);
                }
                Err(e) => this.status.set_text(&format!("Could not save: {e}")),
            },
        );
    }
}

#[derive(Clone, Copy)]
enum Action {
    Kick,
    Ban,
    Unban,
}
