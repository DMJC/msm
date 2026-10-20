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
use msm::calendar::{Response, format_start};

use crate::app::App;
use crate::ui::main_window::error_dialog;

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
            "Nothing yet. Invitations appear here once you accept them (or mark them tentative).",
        ));
        none.set_line_wrap(true);
        none.set_xalign(0.0);
        list.pack_start(&none, false, false, 0);
    }
    let me = app.own_id().unwrap_or_default();
    for e in entries {
        let who = if e.sender == me {
            "you".to_owned()
        } else {
            app.mgr.borrow().resolver().resolve(&e.sender, None)
        };
        let tag = match e.response {
            Response::Tentative => " · <i>tentative</i>",
            _ => "",
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
            "<b>{}</b>{tag}\n{} · from {}{details}",
            glib::markup_escape_text(&e.title),
            glib::markup_escape_text(&format_start(e.start_ms)),
            glib::markup_escape_text(&who)
        ));
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
        list.pack_start(&row, false, false, 0);
    }
    list.show_all();
}
