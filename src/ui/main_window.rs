use std::rc::Rc;

use gtk::prelude::*;
use msm::contacts::contact::Contact;
use msm::icon::status_rgb;
use msm::matrix::presence::{ContactPresence, UiStatus};

use crate::app::{App, now_ms};

pub struct MainUi {
    name: gtk::Label,
    mxid: gtk::Label,
    status: gtk::Label,
    search: gtk::SearchEntry,
    list: gtk::Box,
    combo: gtk::ComboBoxText,
    msg_entry: gtk::Entry,
    updating: std::cell::Cell<bool>,
    /// What the list currently shows; unchanged views are not rebuilt, because
    /// destroying rows mid-click would swallow double-clicks.
    last_view: std::cell::RefCell<String>,
}

impl MainUi {
    pub fn set_own_name(&self, name: Option<&str>) {
        if let Some(n) = name {
            self.name
                .set_markup(&format!("<b>{}</b>", gtk::glib::markup_escape_text(n)));
        }
    }
    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.set_margin(8);

    let name = gtk::Label::new(None);
    name.set_xalign(0.0);
    let mxid = gtk::Label::new(None);
    mxid.set_xalign(0.0);
    mxid.set_selectable(true);
    mxid.set_can_focus(false);
    mxid.style_context().add_class("dim-label");
    let status = gtk::Label::new(Some("Connecting…"));
    status.set_xalign(0.0);
    status.style_context().add_class("dim-label");
    for w in [&name, &mxid, &status] {
        root.pack_start(w, false, false, 0);
    }

    let combo = gtk::ComboBoxText::new();
    for st in UiStatus::ALL {
        combo.append(Some(st.key()), st.label());
    }
    combo.set_active_id(Some(app.presence.get().key()));
    let msg_entry = gtk::Entry::new();
    msg_entry.set_placeholder_text(Some("Set a status message"));
    msg_entry.set_text(&app.presence_msg.borrow());
    root.pack_start(&combo, false, false, 0);
    root.pack_start(&msg_entry, false, false, 0);

    let a = app.clone();
    combo.connect_changed(move |c| {
        let updating = a
            .main_ui
            .borrow()
            .as_ref()
            .is_some_and(|m| m.updating.get());
        if updating {
            return;
        }
        if let Some(st) = c.active_id().and_then(|id| UiStatus::from_key(&id)) {
            a.set_my_presence(st, None);
        }
    });
    let a = app.clone();
    msg_entry.connect_activate(move |e| {
        a.set_my_presence(a.presence.get(), Some(e.text().to_string()));
    });

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search contacts"));
    root.pack_start(&search, false, false, 0);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&list);
    root.pack_start(&scroll, true, true, 0);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let add = gtk::Button::with_label("Add contact");
    let group = gtk::Button::with_label("New group");
    let room = gtk::Button::with_label("New room");
    let logout = gtk::Button::with_label("Sign out");
    for b in [&add, &group] {
        bar.pack_start(b, true, true, 0);
    }
    let bar2 = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for b in [&room, &logout] {
        bar2.pack_start(b, true, true, 0);
    }
    root.pack_start(&bar, false, false, 0);
    root.pack_start(&bar2, false, false, 0);

    let a = app.clone();
    room.connect_clicked(move |_| {
        let Some(name) = prompt(&a, "New room", "Room name", "") else {
            return;
        };
        let Some(who) = prompt(
            &a,
            "Invite people (optional)",
            "Matrix IDs, comma separated",
            "",
        ) else {
            return;
        };
        let invitees = who
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        a.create_room(&name, invitees);
    });

    let a = app.clone();
    search.connect_search_changed(move |_| a.refresh());
    let a = app.clone();
    add.connect_clicked(move |_| {
        if let Some(id) = prompt(&a, "Add contact", "Matrix ID (@user:server)", "") {
            if let Err(e) = a.mgr.borrow_mut().add(&id, now_ms()) {
                error_dialog(&a, &e.to_string());
                return;
            }
            a.persist_and_sync();
            a.refresh();
        }
    });
    let a = app.clone();
    group.connect_clicked(move |_| {
        if let Some(n) = prompt(&a, "New group", "Group name", "") {
            let r = a.mgr.borrow_mut().create_group(&n);
            match r {
                Ok(()) => {
                    a.persist_and_sync();
                    a.refresh();
                }
                Err(e) => error_dialog(&a, &e.to_string()),
            }
        }
    });
    let a = app.clone();
    logout.connect_clicked(move |_| {
        a.end_session(true);
        a.show_login(None);
    });

    *app.main_ui.borrow_mut() = Some(MainUi {
        name,
        mxid,
        status,
        search,
        list,
        combo,
        msg_entry,
        updating: std::cell::Cell::new(false),
        last_view: std::cell::RefCell::new(String::new()),
    });
    root.upcast()
}

/// Reflect `app.presence` in the selector without re-triggering its handlers.
pub fn sync_presence_controls(app: &Rc<App>) {
    if let Some(m) = app.main_ui.borrow().as_ref() {
        m.updating.set(true);
        m.combo.set_active_id(Some(app.presence.get().key()));
        if m.msg_entry.text().as_str() != app.presence_msg.borrow().as_str() {
            m.msg_entry.set_text(&app.presence_msg.borrow());
        }
        m.updating.set(false);
    }
}

fn presence_dot(p: ContactPresence) -> String {
    let status = match p {
        ContactPresence::Online => Some(UiStatus::Online),
        ContactPresence::Away => Some(UiStatus::Away),
        ContactPresence::Busy => Some(UiStatus::Busy),
        ContactPresence::Offline => Some(UiStatus::Offline),
        ContactPresence::Unknown => None,
    };
    match status {
        Some(s) => {
            let (r, g, b) = status_rgb(s);
            format!("<span foreground='#{r:02x}{g:02x}{b:02x}'>●</span> ")
        }
        // Presence unavailable is not the same as offline: hollow marker.
        None => "<span foreground='#aaaaaa'>○</span> ".to_owned(),
    }
}

pub fn show(app: &Rc<App>, me: &str) {
    if let Some(m) = app.main_ui.borrow().as_ref() {
        m.name
            .set_markup(&format!("<b>{}</b>", gtk::glib::markup_escape_text(me)));
        m.mxid.set_text(me);
        m.status.set_text("Connecting…");
    }
    app.stack.set_visible_child_name("main");
    refresh(app);
}

pub fn refresh(app: &Rc<App>) {
    let ui = app.main_ui.borrow();
    let Some(ui) = ui.as_ref() else { return };
    let query = ui.search.text().to_string();
    let view = view_signature(app, &query);
    if *ui.last_view.borrow() == view {
        return;
    }
    *ui.last_view.borrow_mut() = view;
    for child in ui.list.children() {
        ui.list.remove(&child);
    }
    let mgr = app.mgr.borrow();
    let matches: Vec<Contact> = mgr.search(&query).into_iter().cloned().collect();
    add_invites(app, &ui.list);
    let favourites: Vec<&Contact> = matches.iter().filter(|c| c.favorite).collect();
    if !favourites.is_empty() {
        add_group(app, &ui.list, "Favourites", &favourites, true);
    }
    for g in mgr.document().groups.clone() {
        let members: Vec<&Contact> = matches.iter().filter(|c| c.group == g).collect();
        if members.is_empty() && !query.is_empty() {
            continue;
        }
        add_group(app, &ui.list, &g, &members, false);
    }
    add_rooms(app, &ui.list, &query);
    ui.list.show_all();
}

/// Everything the list displays, as one string, to detect real changes.
fn view_signature(app: &Rc<App>, query: &str) -> String {
    use std::fmt::Write;
    let mut sig = format!("q={query}\n");
    let mgr = app.mgr.borrow();
    let _ = writeln!(sig, "groups={:?}", mgr.document().groups);
    let pres = app.contact_presence.borrow();
    for c in mgr.search(query) {
        let _ = writeln!(
            sig,
            "c|{}|{}|{}|{}|{}|{:?}",
            c.mxid,
            mgr.resolver().resolve(&c.mxid, None),
            c.group,
            c.favorite,
            app.unread_for(&c.mxid),
            pres.get(&c.mxid)
        );
    }
    for i in app.invites.borrow().iter() {
        let who = i
            .inviter
            .as_deref()
            .map(|m| mgr.resolver().resolve(m, None));
        let _ = writeln!(sig, "i|{}|{}|{:?}|{}", i.room_id, i.name, who, i.is_direct);
    }
    if let Some(client) = app.client.borrow().as_ref() {
        for r in msm::matrix::rooms::group_rooms(client) {
            let _ = writeln!(
                sig,
                "r|{}|{}|{}",
                r.room_id(),
                msm::matrix::rooms::cached_title(&r),
                msm::matrix::rooms::unread_count(&r)
            );
        }
    }
    sig
}

fn add_invites(app: &Rc<App>, list: &gtk::Box) {
    let invites = app.invites.borrow().clone();
    if invites.is_empty() {
        return;
    }
    let exp = gtk::Expander::new(Some(&format!("Invitations ({})", invites.len())));
    exp.set_expanded(true);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for inv in invites {
        let who = inv
            .inviter
            .as_deref()
            .map(|m| app.mgr.borrow().resolver().resolve(m, None))
            .unwrap_or_else(|| "Someone".to_owned());
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_line_wrap(true);
        l.set_markup(&format!(
            "<b>{}</b>\n<small>{} invited you{}</small>",
            gtk::glib::markup_escape_text(&inv.name),
            gtk::glib::markup_escape_text(&who),
            if inv.is_direct { " to a chat" } else { "" }
        ));
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let (yes, no) = (
            gtk::Button::with_label("Accept"),
            gtk::Button::with_label("Decline"),
        );
        buttons.pack_start(&yes, false, false, 0);
        buttons.pack_start(&no, false, false, 0);
        let (a, id) = (app.clone(), inv.room_id.clone());
        yes.connect_clicked(move |_| a.answer_invite(&id, true));
        let (a, id) = (app.clone(), inv.room_id.clone());
        no.connect_clicked(move |_| a.answer_invite(&id, false));
        row.pack_start(&l, false, false, 0);
        row.pack_start(&buttons, false, false, 0);
        row.set_margin_start(16);
        inner.pack_start(&row, false, false, 0);
    }
    exp.add(&inner);
    list.pack_start(&exp, false, false, 0);
}

fn add_rooms(app: &Rc<App>, list: &gtk::Box, query: &str) {
    let Some(client) = app.client.borrow().clone() else {
        return;
    };
    let q = query.trim().to_lowercase();
    let mut rooms_list: Vec<_> = msm::matrix::rooms::group_rooms(&client)
        .into_iter()
        .map(|r| (msm::matrix::rooms::cached_title(&r), r))
        .filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q))
        .collect();
    rooms_list.sort_by_key(|(n, _)| n.to_lowercase());
    if rooms_list.is_empty() && !q.is_empty() {
        return;
    }
    let exp = gtk::Expander::new(Some(&format!("Rooms ({})", rooms_list.len())));
    exp.set_expanded(true);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (name, room) in rooms_list {
        let unread = msm::matrix::rooms::unread_count(&room);
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_margin_start(16);
        l.set_markup(&format!(
            "{}{}",
            gtk::glib::markup_escape_text(&name),
            if unread > 0 {
                format!("  <b><span foreground='#d33'>({unread})</span></b>")
            } else {
                String::new()
            }
        ));
        let ev = gtk::EventBox::new();
        ev.add(&l);
        let (a, id) = (app.clone(), room.room_id().to_string());
        ev.connect_button_press_event(move |_, e| {
            if e.button() == 1 && e.event_type() == gtk::gdk::EventType::DoubleButtonPress {
                a.open_room(&id);
            } else if e.button() == 3 {
                let menu = gtk::Menu::new();
                let (a1, id1) = (a.clone(), id.clone());
                menu.append(&item("Open", move || a1.open_room(&id1)));
                let (a2, id2) = (a.clone(), id.clone());
                menu.append(&item("Leave room", move || a2.answer_invite(&id2, false)));
                menu.show_all();
                menu.popup_at_pointer(Some(e));
            }
            gtk::glib::Propagation::Proceed
        });
        inner.pack_start(&ev, false, false, 0);
    }
    exp.add(&inner);
    list.pack_start(&exp, false, false, 0);
}

fn add_group(app: &Rc<App>, list: &gtk::Box, name: &str, members: &[&Contact], favourites: bool) {
    let exp = gtk::Expander::new(Some(&format!("{name} ({})", members.len())));
    exp.set_expanded(true);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for c in members {
        inner.pack_start(&contact_row(app, c), false, false, 0);
    }
    exp.add(&inner);
    if !favourites {
        let gname = name.to_owned();
        let a = app.clone();
        let ev = gtk::EventBox::new();
        ev.add(&exp);
        ev.connect_button_press_event(move |_, e| {
            if e.button() == 3 && e.event_type() == gtk::gdk::EventType::ButtonPress {
                group_menu(&a, &gname).popup_at_pointer(Some(e));
            }
            gtk::glib::Propagation::Proceed
        });
        list.pack_start(&ev, false, false, 0);
    } else {
        list.pack_start(&exp, false, false, 0);
    }
}

fn contact_row(app: &Rc<App>, c: &Contact) -> gtk::Widget {
    let (display, show_id) = {
        let m = app.mgr.borrow();
        (m.resolver().resolve(&c.mxid, None), c.mxid.clone())
    };
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let unread = app.unread_for(&c.mxid);
    let (presence, status_msg) = app
        .contact_presence
        .borrow()
        .get(&c.mxid)
        .cloned()
        .unwrap_or((ContactPresence::Unknown, None));
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_markup(&format!(
        "{}{}{}{}",
        presence_dot(presence),
        if c.favorite { "★ " } else { "" },
        gtk::glib::markup_escape_text(&display),
        if unread > 0 {
            format!("  <b><span foreground='#d33'>({unread})</span></b>")
        } else {
            String::new()
        }
    ));
    let sub_text = match status_msg {
        Some(m) => format!("{show_id} - {m}"),
        None => show_id,
    };
    let sub = gtk::Label::new(Some(&sub_text));
    sub.set_xalign(0.0);
    sub.style_context().add_class("dim-label");
    row.pack_start(&l, false, false, 0);
    row.pack_start(&sub, false, false, 0);
    row.set_margin_start(16);

    let ev = gtk::EventBox::new();
    ev.add(&row);
    let a = app.clone();
    let mxid = c.mxid.clone();
    ev.connect_button_press_event(move |_, e| {
        if e.button() == 3 {
            contact_menu(&a, &mxid).popup_at_pointer(Some(e));
        } else if e.button() == 1 && e.event_type() == gtk::gdk::EventType::DoubleButtonPress {
            a.open_conversation(&mxid);
        }
        gtk::glib::Propagation::Proceed
    });
    ev.upcast()
}

fn item(label: &str, f: impl Fn() + 'static) -> gtk::MenuItem {
    let i = gtk::MenuItem::with_label(label);
    i.connect_activate(move |_| f());
    i
}

fn contact_menu(app: &Rc<App>, mxid: &str) -> gtk::Menu {
    let menu = gtk::Menu::new();
    let (fav, groups) = {
        let m = app.mgr.borrow();
        (
            m.get(mxid).map(|c| c.favorite).unwrap_or(false),
            m.document().groups.clone(),
        )
    };

    let (a, id) = (app.clone(), mxid.to_owned());
    menu.append(&item("Send message", move || a.open_conversation(&id)));
    let (a, id) = (app.clone(), mxid.to_owned());
    menu.append(&item("Rename…", move || {
        let current = a
            .mgr
            .borrow()
            .resolver()
            .nickname(&id)
            .unwrap_or("")
            .to_owned();
        if let Some(n) = prompt(
            &a,
            "Rename contact (blank resets to public name)",
            "Nickname",
            &current,
        ) {
            let _ = a.mgr.borrow_mut().set_nickname(&id, Some(&n), now_ms());
            a.persist_and_sync();
            a.refresh();
        }
    }));
    let (a, id) = (app.clone(), mxid.to_owned());
    menu.append(&item("Reset nickname", move || {
        let _ = a.mgr.borrow_mut().set_nickname(&id, None, now_ms());
        a.persist_and_sync();
        a.refresh();
    }));
    let (a, id) = (app.clone(), mxid.to_owned());
    menu.append(&item(
        if fav {
            "Remove from favourites"
        } else {
            "Add to favourites"
        },
        move || {
            let _ = a.mgr.borrow_mut().set_favorite(&id, !fav, now_ms());
            a.persist_and_sync();
            a.refresh();
        },
    ));

    let sub = gtk::Menu::new();
    for g in groups {
        let (a, id, g2) = (app.clone(), mxid.to_owned(), g.clone());
        sub.append(&item(&g, move || {
            let _ = a.mgr.borrow_mut().move_to_group(&id, &g2, now_ms());
            a.persist_and_sync();
            a.refresh();
        }));
    }
    let mv = gtk::MenuItem::with_label("Move to group");
    mv.set_submenu(Some(&sub));
    menu.append(&mv);

    let (a, id) = (app.clone(), mxid.to_owned());
    menu.append(&item("Remove contact", move || {
        let _ = a.mgr.borrow_mut().remove(&id, now_ms());
        a.persist_and_sync();
        a.refresh();
    }));
    menu.show_all();
    menu
}

fn group_menu(app: &Rc<App>, group: &str) -> gtk::Menu {
    let menu = gtk::Menu::new();
    let (a, g) = (app.clone(), group.to_owned());
    menu.append(&item("Rename group…", move || {
        if let Some(n) = prompt(&a, "Rename group", "Group name", &g) {
            let r = a.mgr.borrow_mut().rename_group(&g, &n, now_ms());
            match r {
                Ok(()) => {
                    a.persist_and_sync();
                    a.refresh();
                }
                Err(e) => error_dialog(&a, &e.to_string()),
            }
        }
    }));
    let (a, g) = (app.clone(), group.to_owned());
    menu.append(&item("Move group up", move || {
        let pos = a
            .mgr
            .borrow()
            .document()
            .groups
            .iter()
            .position(|x| *x == g)
            .unwrap_or(0);
        let _ = a.mgr.borrow_mut().move_group(&g, pos.saturating_sub(1));
        a.persist_and_sync();
        a.refresh();
    }));
    let (a, g) = (app.clone(), group.to_owned());
    menu.append(&item("Delete group (contacts move to Other)", move || {
        let r = a.mgr.borrow_mut().delete_group(&g, "Other", now_ms());
        match r {
            Ok(()) => {
                a.persist_and_sync();
                a.refresh();
            }
            Err(e) => error_dialog(&a, &e.to_string()),
        }
    }));
    menu.show_all();
    menu
}

/// Modal single-line prompt. Returns `None` if cancelled.
pub fn prompt(app: &Rc<App>, title: &str, placeholder: &str, initial: &str) -> Option<String> {
    let d = gtk::Dialog::with_buttons(
        Some(title),
        Some(&app.window),
        gtk::DialogFlags::MODAL,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("OK", gtk::ResponseType::Ok),
        ],
    );
    d.set_default_response(gtk::ResponseType::Ok);
    let e = gtk::Entry::new();
    e.set_placeholder_text(Some(placeholder));
    e.set_text(initial);
    e.set_activates_default(true);
    e.set_margin(12);
    d.content_area().add(&e);
    d.show_all();
    let r = d.run();
    let text = e.text().to_string();
    d.close();
    (r == gtk::ResponseType::Ok).then_some(text)
}

fn error_dialog(app: &Rc<App>, msg: &str) {
    let d = gtk::MessageDialog::new(
        Some(&app.window),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Error,
        gtk::ButtonsType::Close,
        msg,
    );
    d.run();
    d.close();
}
