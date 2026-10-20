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

use std::rc::Rc;

use gtk::prelude::*;
use msm::matrix::authentication;

use crate::app::{App, spawn_async};

pub struct LoginUi {
    homeserver: gtk::Entry,
    user: gtk::Entry,
    password: gtk::Entry,
    error: gtk::Label,
    button: gtk::Button,
}

impl LoginUi {
    pub fn reset(&self, message: Option<&str>) {
        self.password.set_text("");
        self.button.set_sensitive(true);
        self.error.set_text(message.unwrap_or(""));
    }
}

pub fn build(app: &Rc<App>) -> gtk::Widget {
    let grid = gtk::Box::new(gtk::Orientation::Vertical, 8);
    grid.set_margin(24);
    grid.set_valign(gtk::Align::Center);

    let title = gtk::Label::new(None);
    title.set_markup("<big><b>Matrix Messenger</b></big>");
    grid.pack_start(&title, false, false, 12);

    let entry = |placeholder: &str, secret: bool| {
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(placeholder));
        e.set_visibility(!secret);
        e.set_activates_default(true);
        e
    };
    let homeserver = entry("Homeserver (e.g. matrix.org)", false);
    let user = entry("Username", false);
    let password = entry("Password", true);
    let error = gtk::Label::new(None);
    error.set_line_wrap(true);
    error.set_widget_name("login-error");
    let button = gtk::Button::with_label("Sign in");
    button.set_can_default(true);

    for w in [&homeserver, &user, &password] {
        grid.pack_start(w, false, false, 0);
    }
    grid.pack_start(&error, false, false, 0);
    grid.pack_start(&button, false, false, 4);

    let ui = LoginUi {
        homeserver: homeserver.clone(),
        user: user.clone(),
        password: password.clone(),
        error: error.clone(),
        button: button.clone(),
    };

    let a = app.clone();
    button.connect_clicked(move |_| {
        let (hs, u, p) = {
            let l = a.login_ui.borrow();
            let Some(l) = l.as_ref() else { return };
            l.button.set_sensitive(false);
            l.error.set_text("Signing in…");
            (
                l.homeserver.text().to_string(),
                l.user.text().to_string(),
                l.password.text().to_string(),
            )
        };
        let a2 = a.clone();
        spawn_async(
            async move { authentication::login(&hs, &u, &p, &authentication::data_dir()).await },
            move |res| match res {
                Ok(client) => a2.start_session(client),
                Err(e) => {
                    if let Some(l) = a2.login_ui.borrow().as_ref() {
                        l.button.set_sensitive(true);
                        l.error.set_text(&e.to_string());
                    }
                }
            },
        );
    });
    *app.login_ui.borrow_mut() = Some(ui);
    grid.upcast()
}
