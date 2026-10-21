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

//! The voice-call window: ringing (incoming or outgoing), connecting, in call (mute, hang
//! up, elapsed time) and a short "call ended" note.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use gtk::glib;
use gtk::prelude::*;
use msm::calls::session::CallEvent;

use crate::app::{App, spawn_async};

pub struct CallWindow {
    window: gtk::Window,
    heading: gtk::Label,
    status: gtk::Label,
    answer: gtk::Button,
    hang_up: gtk::Button,
    mute: gtk::ToggleButton,
    started: Cell<Option<Instant>>,
    timer: RefCell<Option<glib::SourceId>>,
    /// Who the call is with (already resolved to a display name).
    who: RefCell<String>,
}

fn minutes(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

pub fn build(app: &Rc<App>) -> Rc<CallWindow> {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Voice call");
    window.set_default_size(300, 160);
    window.set_resizable(false);
    window.set_transient_for(Some(&app.window));
    window.set_position(gtk::WindowPosition::Center);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin(16);
    let heading = gtk::Label::new(None);
    heading.set_line_wrap(true);
    let status = gtk::Label::new(None);
    status.style_context().add_class("dim-label");
    let answer = gtk::Button::with_label("Answer");
    answer.style_context().add_class("suggested-action");
    let hang_up = gtk::Button::with_label("Hang up");
    hang_up.style_context().add_class("destructive-action");
    let mute = gtk::ToggleButton::with_label("Mute");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Center);
    buttons.pack_start(&answer, false, false, 0);
    buttons.pack_start(&mute, false, false, 0);
    buttons.pack_start(&hang_up, false, false, 0);
    root.pack_start(&heading, false, false, 0);
    root.pack_start(&status, false, false, 0);
    root.pack_start(&buttons, false, false, 8);
    window.add(&root);
    root.show_all();

    let cw = Rc::new(CallWindow {
        window,
        heading,
        status,
        answer,
        hang_up,
        mute,
        started: Cell::new(None),
        timer: RefCell::new(None),
        who: RefCell::new(String::new()),
    });

    let a = app.clone();
    cw.answer.connect_clicked(move |b| {
        b.set_sensitive(false);
        let Some(calls) = a.calls.borrow().clone() else {
            return;
        };
        let a2 = a.clone();
        spawn_async(async move { calls.answer().await }, move |res| {
            if let Err(e) = res {
                crate::ui::main_window::error_dialog(&a2, &e);
            }
        });
    });
    let a = app.clone();
    cw.hang_up.connect_clicked(move |_| {
        if let Some(calls) = a.calls.borrow().clone() {
            spawn_async(async move { calls.hang_up().await }, |_| {});
        }
    });
    let a = app.clone();
    cw.mute.connect_toggled(move |b| {
        let muted = b.is_active();
        b.set_label(if muted { "Unmute" } else { "Mute" });
        if let Some(calls) = a.calls.borrow().clone() {
            spawn_async(async move { calls.set_muted(muted).await }, |_| {});
        }
    });
    // Closing the window hangs up (a call must not continue unseen).
    let a = app.clone();
    cw.window.connect_delete_event(move |w, _| {
        if let Some(calls) = a.calls.borrow().clone() {
            spawn_async(async move { calls.hang_up().await }, |_| {});
        }
        w.hide();
        glib::Propagation::Stop
    });
    cw
}

impl CallWindow {
    fn stop_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    /// Show the window in the state `ev` describes. `who` is the other person's name.
    pub fn show(self: &Rc<Self>, ev: &CallEvent, who: Option<&str>) {
        if let Some(w) = who {
            *self.who.borrow_mut() = w.to_owned();
        }
        let who = self.who.borrow().clone();
        let (answer, mute, hang_label) = match ev {
            CallEvent::Incoming { .. } => {
                self.heading.set_markup(&format!(
                    "<big><b>{}</b></big>\nis calling you",
                    glib::markup_escape_text(&who)
                ));
                self.status.set_text("Ringing…");
                (true, false, "Decline")
            }
            CallEvent::Ringing { .. } => {
                self.heading.set_markup(&format!(
                    "<big><b>{}</b></big>",
                    glib::markup_escape_text(&who)
                ));
                self.status.set_text("Calling…");
                (false, false, "Cancel")
            }
            CallEvent::Connecting => {
                self.heading.set_markup(&format!(
                    "<big><b>{}</b></big>",
                    glib::markup_escape_text(&who)
                ));
                self.status.set_text("Connecting…");
                (false, false, "Hang up")
            }
            CallEvent::Active => {
                self.heading.set_markup(&format!(
                    "<big><b>{}</b></big>",
                    glib::markup_escape_text(&who)
                ));
                self.started.set(Some(Instant::now()));
                self.status.set_text("00:00");
                self.stop_timer();
                let me = self.clone();
                *self.timer.borrow_mut() = Some(glib::timeout_add_seconds_local(1, move || {
                    if let Some(t) = me.started.get() {
                        me.status.set_text(&minutes(t.elapsed().as_secs()));
                    }
                    glib::ControlFlow::Continue
                }));
                (false, true, "Hang up")
            }
            CallEvent::Ended { reason } => {
                self.stop_timer();
                self.started.set(None);
                self.status.set_text(reason);
                self.answer.hide();
                self.mute.hide();
                self.hang_up.hide();
                let w = self.window.clone();
                glib::timeout_add_seconds_local_once(2, move || w.hide());
                self.window.show();
                return;
            }
        };
        self.mute.set_active(false);
        self.answer.set_sensitive(true);
        self.answer.set_visible(answer);
        self.mute.set_visible(mute);
        self.hang_up.set_label(hang_label);
        self.hang_up.show();
        self.window.show();
        self.window.present();
    }
}
