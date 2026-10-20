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

mod app;
mod idle;
mod tray;
mod ui;

use gtk::prelude::*;
use msm::matrix::authentication;
use msm::storage::database::Database;

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // The SDK logs expected 404s (e.g. no contacts account data yet) at ERROR.
                .unwrap_or_else(|_| "info,matrix_sdk::http_client=off".into()),
        )
        .init();
    let app = gtk::Application::builder()
        .application_id("com.matrixmessenger.Msm")
        .build();
    app.connect_activate(activate);
    app.run()
}

fn activate(gtk_app: &gtk::Application) {
    if let Some(w) = gtk_app.active_window() {
        w.present();
        return;
    }
    if let Some(d) = gtk::gdk::Display::default() {
        use gtk::glib::prelude::ObjectExt;
        tracing::info!("GDK backend: {}", d.type_().name());
    }
    let dir = authentication::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let db = match Database::open(&dir.join("app.db")) {
        Ok(db) => db,
        Err(e) => {
            tracing::error!("cannot open app database: {e}");
            Database::open_in_memory().expect("in-memory database")
        }
    };

    let window = gtk::ApplicationWindow::builder()
        .application(gtk_app)
        .title("Matrix Messenger")
        .default_width(320)
        .default_height(640)
        .build();
    let stack = gtk::Stack::new();
    window.add(&stack);

    let state = app::App::new(window.clone(), stack.clone(), db);
    let s = state.clone();
    gtk_app.connect_shutdown(move |_| s.shutdown());
    state.start_tray();
    state.start_idle_monitor();
    let s = state.clone();
    window.connect_delete_event(move |_, _| {
        if s.hide_to_tray() {
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    stack.add_named(&ui::login_window::build(&state), "login");
    stack.add_named(&ui::main_window::build(&state), "main");
    window.show_all();
    state.update_tray();
    stack.set_visible_child_name("login");

    // Try to resume a saved session before asking for credentials.
    let s = state.clone();
    app::spawn_async(
        async move { authentication::restore(&authentication::data_dir()).await },
        move |res| match res {
            Ok(Some(client)) => s.start_session(client),
            Ok(None) => {}
            Err(e) => s.show_login(Some(&e.to_string())),
        },
    );
}
