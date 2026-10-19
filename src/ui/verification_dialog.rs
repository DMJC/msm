//! Emoji verification window. The person compares the emoji with the other side
//! (ideally over another channel) and only their click confirms a match.

use std::rc::Rc;

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
    let emoji_box = gtk::Grid::new();
    emoji_box.set_column_spacing(10);
    emoji_box.set_row_spacing(4);
    emoji_box.set_halign(gtk::Align::Center);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let accept = gtk::Button::with_label("Accept");
    let matches = gtk::Button::with_label("They match");
    let differ = gtk::Button::with_label("They don't match");
    let cancel = gtk::Button::with_label("Cancel");
    for b in [&accept, &matches, &differ, &cancel] {
        buttons.pack_start(b, false, false, 0);
    }
    root.pack_start(&heading, false, false, 0);
    root.pack_start(&status, false, false, 0);
    root.pack_start(&emoji_box, false, false, 6);
    root.pack_end(&buttons, false, false, 0);
    window.add(&root);
    window.show_all();
    for b in [&accept, &matches, &differ] {
        b.hide();
    }

    for (btn, c) in [
        (&accept, VerifyCmd::Accept),
        (&matches, VerifyCmd::Match),
        (&differ, VerifyCmd::Mismatch),
        (&cancel, VerifyCmd::Cancel),
    ] {
        let cmd = cmd.clone();
        btn.connect_clicked(move |_| {
            let _ = cmd.send(c);
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
            for b in [&accept, &matches, &differ] {
                b.hide();
            }
            for c in emoji_box.children() {
                emoji_box.remove(&c);
            }
            match state {
                VerifyUi::Waiting => status.set_text("Waiting for them to accept…"),
                VerifyUi::Requested => {
                    status.set_text("They want to verify with you.");
                    accept.show();
                }
                VerifyUi::Starting => status.set_text("Starting…"),
                VerifyUi::Emojis(list) => {
                    status.set_text(
                        "Check that these emoji appear, in the same order, on the other side.",
                    );
                    for (i, (sym, name)) in list.iter().enumerate() {
                        let big = gtk::Label::new(None);
                        big.set_markup(&format!(
                            "<span size='xx-large'>{}</span>",
                            glib::markup_escape_text(sym)
                        ));
                        let small = gtk::Label::new(Some(name));
                        emoji_box.attach(&big, i as i32, 0, 1, 1);
                        emoji_box.attach(&small, i as i32, 1, 1, 1);
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
