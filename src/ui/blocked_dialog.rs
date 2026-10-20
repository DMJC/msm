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

//! The account's block list: unblock anyone (contact or not) or block by Matrix ID.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use msm::matrix::ignore;

use crate::app::{App, spawn_async};
use crate::ui::main_window::error_dialog;

pub fn open(app: &Rc<App>) {
    let Some(client) = app.client.borrow().clone() else {
        return;
    };
    let d = gtk::Dialog::with_buttons(
        Some("Blocked users"),
        Some(&app.window),
        gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Close", gtk::ResponseType::Close)],
    );
    d.set_default_size(360, 360);
    let area = d.content_area();
    area.set_spacing(6);
    area.set_margin(12);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&list);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("@user:server to block"));
    let block = gtk::Button::with_label("Block");
    let add = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    add.pack_start(&entry, true, true, 0);
    add.pack_start(&block, false, false, 0);
    area.pack_start(&scroll, true, true, 0);
    area.pack_start(&add, false, false, 0);

    // `reload` and the row buttons call each other, so it lives behind a cell.
    type Reload = Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>;
    let reload: Reload = Rc::default();
    let run: Rc<dyn Fn()> = {
        let (app, client, list, reload) =
            (app.clone(), client.clone(), list.clone(), reload.clone());
        Rc::new(move || {
            let (app, client, list, reload) =
                (app.clone(), client.clone(), list.clone(), reload.clone());
            let c = client.clone();
            spawn_async(async move { ignore::blocked(&c).await }, move |ids| {
                for child in list.children() {
                    list.remove(&child);
                }
                if ids.is_empty() {
                    let none = gtk::Label::new(Some("Nobody is blocked."));
                    none.set_xalign(0.0);
                    list.pack_start(&none, false, false, 0);
                }
                for id in ids {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    let shown = app.mgr.borrow().resolver().resolve(&id, None);
                    let label = gtk::Label::new(None);
                    label.set_xalign(0.0);
                    label.set_markup(&format!(
                        "{}\n<small>{}</small>",
                        glib::markup_escape_text(&shown),
                        glib::markup_escape_text(&id)
                    ));
                    let unblock = gtk::Button::with_label("Unblock");
                    let (app2, client2, reload2, id2) =
                        (app.clone(), client.clone(), reload.clone(), id.clone());
                    unblock.connect_clicked(move |_| {
                            let (app3, reload3) = (app2.clone(), reload2.clone());
                            let (c, id3) = (client2.clone(), id2.clone());
                            spawn_async(
                                async move { ignore::unblock(&c, &id3).await.map_err(|e| e.to_string()) },
                                move |res| {
                                    match res {
                                        Ok(()) => app3.refresh_blocked(),
                                        Err(e) => error_dialog(&app3, &e),
                                    }
                                    if let Some(f) = reload3.borrow().clone() {
                                        f();
                                    }
                                },
                            );
                        });
                    row.pack_start(&label, true, true, 0);
                    row.pack_end(&unblock, false, false, 0);
                    list.pack_start(&row, false, false, 0);
                }
                list.show_all();
            });
        })
    };
    *reload.borrow_mut() = Some(run.clone());

    {
        let (app, client, entry, reload) = (app.clone(), client, entry.clone(), reload.clone());
        block.connect_clicked(move |_| {
            let id = entry.text().to_string();
            if id.trim().is_empty() {
                return;
            }
            let (app, entry, reload, c) =
                (app.clone(), entry.clone(), reload.clone(), client.clone());
            spawn_async(
                async move { ignore::block(&c, &id).await.map_err(|e| e.to_string()) },
                move |res| {
                    match res {
                        Ok(()) => {
                            entry.set_text("");
                            app.refresh_blocked();
                        }
                        Err(e) => error_dialog(&app, &e),
                    }
                    if let Some(f) = reload.borrow().clone() {
                        f();
                    }
                },
            );
        });
    }

    run();
    d.connect_response(|d, _| d.close());
    d.show_all();
}
