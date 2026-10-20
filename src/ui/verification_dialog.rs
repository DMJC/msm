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

//! Emoji verification window. The person compares the emoji with the other side
//! (ideally over another channel) and only their click confirms a match.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use msm::matrix::verification::{self, Flow, VerifyCmd, VerifyUi};

use crate::app::{App, spawn_async};
use crate::ui::main_window::error_dialog;

/// Ask `mxid` (our own ID for another of our devices) to verify, then show the flow.
pub fn start(app: &Rc<App>, mxid: &str) {
    let Some(client) = app.client.borrow().clone() else {
        return;
    };
    let id = mxid.to_owned();
    let app2 = app.clone();
    spawn_async(
        async move {
            let req = verification::request(&client, &id)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(verification::run(req))
        },
        move |r| match r {
            Ok(flow) => open(&app2, flow),
            Err(e) => error_dialog(&app2, &e),
        },
    );
}

/// Show the dialog for a flow (ours or incoming).
pub fn open(app: &Rc<App>, flow: Flow) {
    let Flow {
        other_user,
        self_verification,
        mut ui,
        cmd,
    } = flow;
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Verify");
    window.set_transient_for(Some(&app.window));
    window.set_default_size(380, 240);
    let qr_grid: Rc<RefCell<Vec<Vec<bool>>>> = Rc::default();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin(12);
    let who = if self_verification {
        "another of your devices".to_owned()
    } else {
        app.mgr.borrow().resolver().resolve(&other_user, None)
    };
    let heading = gtk::Label::new(None);
    heading.set_markup(&format!("<b>Verify {}</b>", glib::markup_escape_text(&who)));
    heading.set_xalign(0.0);
    let status = gtk::Label::new(Some("Starting…"));
    status.set_xalign(0.0);
    status.set_line_wrap(true);
    status.set_max_width_chars(48);
    let emoji_box = gtk::Grid::new();
    emoji_box.set_column_spacing(10);
    emoji_box.set_row_spacing(4);
    emoji_box.set_halign(gtk::Align::Center);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let accept = gtk::Button::with_label("Accept");
    let matches = gtk::Button::with_label("They match");
    let differ = gtk::Button::with_label("They don't match");
    let scan = gtk::Button::with_label("Scan their QR code");
    let emoji = gtk::Button::with_label("Compare emoji instead");
    let cancel = gtk::Button::with_label("Cancel");
    // Our own code, for the other side's camera.
    let qr_area = gtk::DrawingArea::new();
    qr_area.set_size_request(220, 220);
    qr_area.set_halign(gtk::Align::Center);
    {
        let grid = qr_grid.clone();
        qr_area.connect_draw(move |area, cr| {
            let grid = grid.borrow();
            let n = grid.len();
            if n == 0 {
                return glib::Propagation::Stop;
            }
            let side = f64::from(area.allocated_width().min(area.allocated_height()));
            // The code needs a light quiet zone around it to be read.
            let cell = (side / (n as f64 + 4.0)).floor().max(1.0);
            let total = cell * (n as f64 + 4.0);
            let (ox, oy) = ((f64::from(area.allocated_width()) - total) / 2.0, 0.0);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.rectangle(ox, oy, total, total);
            let _ = cr.fill();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            for (y, row) in grid.iter().enumerate() {
                for (x, dark) in row.iter().enumerate() {
                    if *dark {
                        cr.rectangle(
                            ox + (x as f64 + 2.0) * cell,
                            oy + (y as f64 + 2.0) * cell,
                            cell,
                            cell,
                        );
                    }
                }
            }
            let _ = cr.fill();
            glib::Propagation::Stop
        });
    }
    for b in [&accept, &matches, &differ, &scan, &emoji, &cancel] {
        buttons.pack_start(b, false, false, 0);
    }
    root.pack_start(&heading, false, false, 0);
    root.pack_start(&status, false, false, 0);
    root.pack_start(&qr_area, false, false, 6);
    root.pack_start(&emoji_box, false, false, 6);
    root.pack_end(&buttons, false, false, 0);
    window.add(&root);
    window.show_all();
    for b in [&accept, &matches, &differ, &scan, &emoji] {
        b.hide();
    }
    qr_area.hide();

    for (btn, c) in [
        (&accept, VerifyCmd::Accept),
        (&matches, VerifyCmd::Match),
        (&differ, VerifyCmd::Mismatch),
        (&emoji, VerifyCmd::StartEmoji),
        (&cancel, VerifyCmd::Cancel),
    ] {
        let cmd = cmd.clone();
        btn.connect_clicked(move |_| {
            let _ = cmd.send(c.clone());
        });
    }
    {
        let (cmd, status, scan2) = (cmd.clone(), status.clone(), scan.clone());
        scan.connect_clicked(move |_| {
            scan2.set_sensitive(false);
            status.set_text("Hold their QR code up to the camera…");
            let (cmd, status, scan2) = (cmd.clone(), status.clone(), scan2.clone());
            spawn_async(
                async move { msm::camera::scan_qr(Duration::from_secs(60)).await },
                move |r| match r {
                    Ok(bytes) => {
                        let _ = cmd.send(VerifyCmd::ScanQr(bytes));
                    }
                    Err(e) => {
                        status.set_text(&format!("{e}. Try again, or compare emoji instead."));
                        scan2.set_sensitive(true);
                    }
                },
            );
        });
    }
    // Closing the window abandons the flow.
    let cmd2 = cmd.clone();
    window.connect_delete_event(move |_, _| {
        let _ = cmd2.send(VerifyCmd::Cancel);
        glib::Propagation::Proceed
    });

    let app = app.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Some(state) = ui.recv().await {
            for b in [&accept, &matches, &differ, &scan, &emoji] {
                b.hide();
            }
            qr_area.hide();
            matches.set_label("They match");
            for c in emoji_box.children() {
                emoji_box.remove(&c);
            }
            match state {
                VerifyUi::Waiting if self_verification => status.set_text(
                    "Waiting for your other device to accept… A device that is not yet \
                     verified cannot be asked from here: on that device open Security → \
                     Verify another device… instead.",
                ),
                VerifyUi::Waiting => status.set_text("Waiting for them to accept…"),
                VerifyUi::Requested => {
                    status.set_text("They want to verify with you.");
                    accept.show();
                }
                VerifyUi::Ready { qr } => {
                    emoji.show();
                    if let Some(grid) = qr {
                        status.set_text(
                            "Scan this code with their device, scan theirs, or compare emoji.",
                        );
                        *qr_grid.borrow_mut() = grid;
                        qr_area.show();
                        qr_area.queue_draw();
                    } else {
                        status.set_text("Compare emoji, or scan their QR code.");
                    }
                    scan.set_sensitive(true);
                    scan.show();
                }
                VerifyUi::QrScanned => {
                    status
                        .set_text("They scanned your code. Confirm only if you did this yourself.");
                    matches.set_label("Yes, that was them");
                    matches.show();
                    differ.set_label("No");
                    differ.show();
                }
                VerifyUi::Starting => status.set_text("Starting…"),
                VerifyUi::Emojis(list) => {
                    status.set_text(
                        "Check that these emoji appear, in the same order, on the other side.",
                    );
                    differ.set_label("They don't match");
                    for (i, (sym, name)) in list.iter().enumerate() {
                        let big = gtk::Label::new(None);
                        big.set_markup(&format!(
                            "<span size='xx-large'>{}</span>",
                            glib::markup_escape_text(sym)
                        ));
                        let small = gtk::Label::new(Some(name));
                        // Four to a row keeps the window narrow enough for small screens.
                        let (col, row) = ((i % 4) as i32, (i / 4) as i32 * 2);
                        emoji_box.attach(&big, col, row, 1, 1);
                        emoji_box.attach(&small, col, row + 1, 1, 1);
                    }
                    emoji_box.show_all();
                    matches.show();
                    differ.show();
                }
                VerifyUi::Confirmed => status.set_text("Waiting for the other side to confirm…"),
                VerifyUi::Done => {
                    status.set_text("✅ Verified.");
                    // Trust is recorded just after Done; look again shortly.
                    let app = app.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        app.refresh_trust()
                    });
                    cancel.set_label("Close");
                    cancel.connect_clicked({
                        let w = window.clone();
                        move |_| w.close()
                    });
                }
                VerifyUi::Cancelled(why) => {
                    status.set_text(&format!("Verification was cancelled: {why}"));
                    cancel.set_label("Close");
                    cancel.connect_clicked({
                        let w = window.clone();
                        move |_| w.close()
                    });
                }
            }
        }
    });
}
