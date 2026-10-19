use std::rc::Rc;

use gtk::prelude::*;
use msm::contacts::contact::Contact;

use crate::app::{App, now_ms};

pub struct MainUi {
    name: gtk::Label,
    mxid: gtk::Label,
    status: gtk::Label,
    search: gtk::SearchEntry,
    list: gtk::Box,
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
    mxid.style_context().add_class("dim-label");
    let status = gtk::Label::new(Some("Connecting…"));
    status.set_xalign(0.0);
    status.style_context().add_class("dim-label");
    for w in [&name, &mxid, &status] {
        root.pack_start(w, false, false, 0);
    }

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
    let logout = gtk::Button::with_label("Sign out");
    for b in [&add, &group, &logout] {
        bar.pack_start(b, true, true, 0);
    }
    root.pack_start(&bar, false, false, 0);

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
    });
    root.upcast()
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
    for child in ui.list.children() {
        ui.list.remove(&child);
    }
    let query = ui.search.text().to_string();
    let mgr = app.mgr.borrow();
    let matches: Vec<Contact> = mgr.search(&query).into_iter().cloned().collect();
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
    ui.list.show_all();
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
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    l.set_markup(&format!(
        "{}{}",
        if c.favorite { "★ " } else { "" },
        gtk::glib::markup_escape_text(&display)
    ));
    let sub = gtk::Label::new(Some(&show_id));
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
fn prompt(app: &Rc<App>, title: &str, placeholder: &str, initial: &str) -> Option<String> {
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
