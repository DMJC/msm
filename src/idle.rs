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

//! Idle detection for automatic Away, via the Wayland `ext-idle-notify-v1`
//! protocol (supported by Wayfire, sway, KDE, recent GNOME and others).
//!
//! It runs on its own thread with its own Wayland connection and only talks to
//! the GTK thread through channels. `get_idle_notification` respects idle
//! inhibitors, so watching a video does not count as being away. Where the
//! protocol is missing (X11, older compositors) the monitor reports
//! `Unavailable` and automatic Away is simply off.

use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleEvent {
    Idle,
    Resumed,
    /// No usable idle protocol; automatic Away cannot work.
    Unavailable(String),
}

/// `Some(duration)` enables detection after that much inactivity; `None` disables it.
pub struct IdleHandle {
    cmd: mpsc::Sender<Option<Duration>>,
}

impl IdleHandle {
    pub fn set_timeout(&self, t: Option<Duration>) {
        let _ = self.cmd.send(t);
    }
}

struct State {
    tx: async_channel::Sender<IdleEvent>,
}

macro_rules! ignore_events {
    ($($t:ty => $d:ty),* $(,)?) => {$(
        impl Dispatch<$t, $d> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &$d, _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(
    wl_registry::WlRegistry => GlobalListContents,
    wl_seat::WlSeat => (),
    ExtIdleNotifierV1 => (),
);

impl Dispatch<ExtIdleNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let ev = match event {
            ext_idle_notification_v1::Event::Idled => IdleEvent::Idle,
            ext_idle_notification_v1::Event::Resumed => IdleEvent::Resumed,
            _ => return,
        };
        let _ = state.tx.send_blocking(ev);
    }
}

pub fn start(events: async_channel::Sender<IdleEvent>) -> IdleHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("idle-monitor".into())
        .spawn(move || {
            if let Err(e) = run(events.clone(), cmd_rx) {
                tracing::info!("automatic Away unavailable: {e}");
                let _ = events.send_blocking(IdleEvent::Unavailable(e));
            }
        })
        .ok();
    IdleHandle { cmd: cmd_tx }
}

fn run(
    events: async_channel::Sender<IdleEvent>,
    cmds: mpsc::Receiver<Option<Duration>>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland connection: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let notifier: ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|_| "compositor does not support ext-idle-notify-v1".to_owned())?;
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("no seat: {e}"))?;
    let mut state = State { tx: events };
    let mut current: Option<ExtIdleNotificationV1> = None;

    loop {
        // Apply timeout changes from the GTK side.
        while let Ok(t) = cmds.try_recv() {
            if let Some(old) = current.take() {
                old.destroy();
            }
            if let Some(t) = t {
                current = Some(notifier.get_idle_notification(
                    t.as_millis().min(u32::MAX as u128) as u32,
                    &seat,
                    &qh,
                    (),
                ));
            }
        }
        conn.flush().map_err(|e| e.to_string())?;
        queue
            .dispatch_pending(&mut state)
            .map_err(|e| e.to_string())?;

        // Wait for compositor events, but wake regularly to look for commands.
        if let Some(guard) = queue.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `pfd` is a valid pollfd for one descriptor, owned by the live guard.
            let ready = unsafe { libc::poll(&mut pfd, 1, 500) };
            if ready > 0 {
                guard.read().map_err(|e| e.to_string())?;
            } else {
                drop(guard);
            }
            queue
                .dispatch_pending(&mut state)
                .map_err(|e| e.to_string())?;
        }
    }
}
