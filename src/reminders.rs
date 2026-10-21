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

//! Desktop reminders for calendar events: one notification shortly before an event
//! starts (or just after, if the app was not running), once per event.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gtk::glib;
use gtk::prelude::*;
use msm::calendar::{LEAD_SETTING, REMINDER_LEAD_MS, format_start, reminder_due, starts_in};

use crate::app::App;

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Check the calendar every 30 seconds. Safe to call again after signing back in.
pub fn start(app: &Rc<App>) {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    let app = app.clone();
    glib::timeout_add_seconds_local(30, move || {
        check(&app);
        glib::ControlFlow::Continue
    });
}

fn check(app: &Rc<App>) {
    let Some(gapp) = app.window.application() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64;
    let lead_ms = lead_ms(app);
    let Ok(entries) = app.db.calendar_entries() else {
        return;
    };
    for e in entries {
        // Key on the start time too, so a rescheduled event reminds again.
        let key = format!("{}@{}", e.event_id, e.start_ms);
        if !reminder_due(e.start_ms, now, lead_ms)
            || !app.db.mark_reminded(&key, now).unwrap_or(false)
        {
            continue;
        }
        let n = gtk::gio::Notification::new(&format!("📅 {}", e.title));
        n.set_body(Some(&format!(
            "{} · {}",
            format_start(e.start_ms),
            starts_in(e.start_ms, now)
        )));
        gapp.send_notification(Some(&format!("cal-{}", e.event_id)), &n);
    }
}

/// The chosen reminder lead time in ms (0 = reminders off); 10 minutes by default.
pub fn lead_ms(app: &App) -> u64 {
    app.db
        .get_setting(LEAD_SETTING)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|m| m * 60_000)
        .unwrap_or(REMINDER_LEAD_MS)
}
