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

//! The user's calendar: invitations they accepted or tentatively accepted,
//! plus events they organised.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use msm::calendar::{Repeat, Response, format_start};

use msm::calendar::Invite;
use msm::storage::database::{CalendarEntry, PERSONAL_PREFIX};

use crate::app::App;
use crate::ui::main_window::error_dialog;

/// Ask for a title, local date/time and details; `None` if cancelled. Used for new
/// invitations, edits and personal calendar entries.
pub fn event_form(
    parent: &gtk::Window,
    heading: &str,
    ok_label: &str,
    existing: Option<&Invite>,
) -> Option<Invite> {
    event_form_with(parent, heading, ok_label, existing, None).map(|(inv, _)| inv)
}

/// [`event_form`] plus a "repeats" choice when `repeat` is given (personal entries only).
fn event_form_with(
    parent: &gtk::Window,
    heading: &str,
    ok_label: &str,
    existing: Option<&Invite>,
    repeat: Option<Repeat>,
) -> Option<(Invite, Repeat)> {
    let d = gtk::Dialog::with_buttons(
        Some(heading),
        Some(parent),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            (ok_label, gtk::ResponseType::Accept),
        ],
    );
    d.set_default_response(gtk::ResponseType::Accept);
    let area = d.content_area();
    area.set_spacing(6);
    area.set_margin(12);
    let title = gtk::Entry::new();
    title.set_placeholder_text(Some("Title"));
    title.set_activates_default(true);
    let today = glib::DateTime::now_local()
        .ok()
        .and_then(|t| t.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let date = gtk::Entry::new();
    date.set_text(&today);
    date.set_tooltip_text(Some("Date, YYYY-MM-DD (your local time)"));
    let time = gtk::Entry::new();
    time.set_text("12:00");
    time.set_tooltip_text(Some("Time, HH:MM (24-hour, your local time)"));
    let when = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    when.pack_start(&date, true, true, 0);
    when.pack_start(&time, true, true, 0);
    let desc = gtk::Entry::new();
    desc.set_placeholder_text(Some("Details (optional)"));
    if let Some(inv) = existing {
        title.set_text(&inv.title);
        desc.set_text(&inv.description);
        if let Some((d, t)) = msm::calendar::split_local(inv.start_ms) {
            date.set_text(&d);
            time.set_text(&t);
        }
    }
    let repeats = gtk::ComboBoxText::new();
    for r in Repeat::ALL {
        repeats.append(Some(r.key()), r.label());
    }
    repeats.set_active_id(Some(repeat.unwrap_or_default().key()));
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    if repeat.is_some() {
        area.pack_start(&repeats, false, false, 0);
    }
    for w in [
        title.upcast_ref::<gtk::Widget>(),
        when.upcast_ref(),
        desc.upcast_ref(),
        error.upcast_ref(),
    ] {
        area.pack_start(w, false, false, 0);
    }
    d.show_all();
    let invite = loop {
        if d.run() != gtk::ResponseType::Accept {
            break None;
        }
        let t = title.text().trim().to_owned();
        if t.is_empty() {
            error.set_text("Enter a title.");
            continue;
        }
        let Some(start_ms) = msm::calendar::parse_local(&date.text(), &time.text()) else {
            error.set_text("Use the date as YYYY-MM-DD and the time as HH:MM.");
            continue;
        };
        let chosen = repeats
            .active_id()
            .and_then(|k| Repeat::from_key(&k))
            .unwrap_or_default();
        break Some((
            Invite {
                title: t,
                start_ms,
                description: desc.text().trim().to_owned(),
            },
            chosen,
        ));
    };
    d.close();
    invite
}

/// A fresh ID for a personal entry.
fn new_personal_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    format!(
        "{PERSONAL_PREFIX}{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}

pub fn open(app: &Rc<App>) {
    let d = gtk::Dialog::with_buttons(
        Some("Calendar"),
        Some(&app.window),
        gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Close", gtk::ResponseType::Close)],
    );
    d.set_default_size(400, 420);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.set_margin(12);
    let add = gtk::Button::with_label("Add event…");
    {
        let (app, list) = (app.clone(), list.clone());
        add.connect_clicked(move |_| add_or_edit(&app, &list, None));
    }
    d.content_area().pack_start(&add, false, false, 6);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&list);
    d.content_area().pack_start(&scroll, true, true, 0);
    fill(app, &list);
    d.connect_response(|d, _| d.close());
    d.show_all();
}

fn fill(app: &Rc<App>, list: &gtk::Box) {
    for child in list.children() {
        list.remove(&child);
    }
    let entries = match app.db.calendar_entries() {
        Ok(e) => e,
        Err(e) => {
            error_dialog(app, &format!("Could not read the calendar: {e}"));
            return;
        }
    };
    if entries.is_empty() {
        let none = gtk::Label::new(Some(
            "Nothing yet. Add your own event, or accept (or mark tentative) an invitation.",
        ));
        none.set_line_wrap(true);
        none.set_xalign(0.0);
        list.pack_start(&none, false, false, 0);
    }
    let me = app.own_id().unwrap_or_default();
    for e in entries {
        let who = if e.event_id.starts_with(PERSONAL_PREFIX) {
            "personal".to_owned()
        } else if e.sender == me {
            "from you".to_owned()
        } else {
            format!(
                "from {}",
                app.mgr.borrow().resolver().resolve(&e.sender, None)
            )
        };
        let tag = match e.response {
            Response::Tentative => " · <i>tentative</i>",
            _ => "",
        };
        let again = match e.repeat {
            Repeat::Never => String::new(),
            r => format!(" · {}", r.label().to_lowercase()),
        };
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_line_wrap(true);
        let details = if e.description.is_empty() {
            String::new()
        } else {
            format!(
                "\n<small>{}</small>",
                glib::markup_escape_text(&e.description)
            )
        };
        label.set_markup(&format!(
            "<b>{}</b>{tag}\n{} · {}{again}{details}",
            glib::markup_escape_text(&e.title),
            glib::markup_escape_text(&format_start(e.start_ms)),
            glib::markup_escape_text(&who)
        ));
        let personal = e.event_id.starts_with(PERSONAL_PREFIX);
        let remove = gtk::Button::with_label("Remove");
        let (a, l, id) = (app.clone(), list.clone(), e.event_id.clone());
        remove.connect_clicked(move |_| {
            if let Err(err) = a.db.remove_calendar_entry(&id) {
                error_dialog(&a, &format!("Could not remove it: {err}"));
            }
            fill(&a, &l);
        });
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.pack_start(&label, true, true, 0);
        row.pack_end(&remove, false, false, 0);
        if personal {
            let edit = gtk::Button::with_label("Edit…");
            let (a, l, cur) = (app.clone(), list.clone(), e.clone());
            edit.connect_clicked(move |_| add_or_edit(&a, &l, Some(cur.clone())));
            row.pack_end(&edit, false, false, 0);
        }
        list.pack_start(&row, false, false, 0);
    }
    list.show_all();
}

/// Create a personal entry, or change one the user made earlier.
fn add_or_edit(app: &Rc<App>, list: &gtk::Box, existing: Option<CalendarEntry>) {
    let Some(parent) = list
        .toplevel()
        .and_then(|w| w.downcast::<gtk::Window>().ok())
    else {
        return;
    };
    let cur = existing.as_ref().map(|e| Invite {
        title: e.title.clone(),
        start_ms: e.start_ms,
        description: e.description.clone(),
    });
    let (heading, ok) = if existing.is_some() {
        ("Edit event", "Save")
    } else {
        ("New event", "Add")
    };
    let repeat = existing.as_ref().map(|e| e.repeat).unwrap_or_default();
    let Some((inv, repeat)) = event_form_with(&parent, heading, ok, cur.as_ref(), Some(repeat))
    else {
        return;
    };
    let entry = CalendarEntry {
        event_id: existing.map(|e| e.event_id).unwrap_or_else(new_personal_id),
        room_id: String::new(),
        sender: app.own_id().unwrap_or_default(),
        title: inv.title,
        start_ms: inv.start_ms,
        description: inv.description,
        response: Response::Accepted,
        repeat,
    };
    if let Err(e) = app.db.save_personal_entry(&entry) {
        error_dialog(app, &format!("Could not save the event: {e}"));
    }
    fill(app, list);
}
