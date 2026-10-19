//! Security dialog: encryption status, devices, recovery key setup / entry.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use msm::matrix::encryption::{self, DeviceInfo, SecurityStatus};

use crate::app::{App, spawn_async};
use crate::ui::main_window::{error_dialog, prompt_secret};

pub fn open(app: &Rc<App>) {
    let Some(client) = app.client.borrow().clone() else {
        return;
    };
    let d = gtk::Dialog::with_buttons(
        Some("Security"),
        Some(&app.window),
        gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Close", gtk::ResponseType::Close)],
    );
    d.set_default_size(420, 420);
    let area = d.content_area();
    area.set_spacing(6);
    area.set_margin(12);

    let summary = gtk::Label::new(Some("Checking…"));
    summary.set_xalign(0.0);
    summary.set_line_wrap(true);
    summary.set_selectable(true);
    area.pack_start(&summary, false, false, 0);

    let devices = gtk::Label::new(None);
    devices.set_xalign(0.0);
    devices.set_line_wrap(true);
    area.pack_start(&devices, false, false, 6);

    let note = gtk::Label::new(Some(
        "A recovery key lets a new device read your older encrypted messages. \
         Without it, messages from before a device existed stay unreadable there.",
    ));
    note.set_xalign(0.0);
    note.set_line_wrap(true);
    note.style_context().add_class("dim-label");
    area.pack_start(&note, false, false, 0);

    let setup = gtk::Button::with_label("Set up recovery…");
    let enter = gtk::Button::with_label("Enter recovery key…");
    area.pack_start(&setup, false, false, 0);
    area.pack_start(&enter, false, false, 0);
    let reset = gtk::Button::with_label("Reset recovery key…");
    let disable = gtk::Button::with_label("Turn off recovery…");
    area.pack_start(&reset, false, false, 0);
    area.pack_start(&disable, false, false, 0);
    let verify_other = gtk::Button::with_label("Verify another device…");
    area.pack_start(&verify_other, false, false, 0);
    {
        let app = app.clone();
        verify_other.connect_clicked(move |_| {
            if let Some(me) = app.own_id() {
                crate::ui::verification_dialog::start(&app, &me);
            }
        });
    }

    let reload = {
        let (client, summary, devices) = (client.clone(), summary.clone(), devices.clone());
        let (setup, enter) = (setup.clone(), enter.clone());
        let (reset, disable) = (reset.clone(), disable.clone());
        Rc::new(move || {
            let (c, summary, devices) = (client.clone(), summary.clone(), devices.clone());
            let (setup, enter) = (setup.clone(), enter.clone());
            let (reset, disable) = (reset.clone(), disable.clone());
            spawn_async(
                async move {
                    (
                        encryption::status(&c).await,
                        encryption::own_devices(&c).await,
                    )
                },
                move |(st, devs)| {
                    // Setting up again would fail (a backup exists); entering a key only helps when one is needed.
                    setup.set_sensitive(st.recovery == encryption::Recovery::Disabled);
                    enter.set_sensitive(st.recovery == encryption::Recovery::Incomplete);
                    // Changing or deleting the key needs this device to hold the secrets.
                    reset.set_sensitive(st.recovery == encryption::Recovery::Enabled);
                    disable.set_sensitive(st.recovery == encryption::Recovery::Enabled);
                    summary.set_markup(&summary_markup(&st));
                    devices.set_markup(&devices_markup(devs.as_deref().unwrap_or(&[])));
                },
            );
        })
    };
    reload();

    let (a, c, r) = (app.clone(), client.clone(), reload.clone());
    setup.connect_clicked(move |_| {
        let Some(pw) = prompt_secret(&a, "Confirm your password", "Account password") else {
            return;
        };
        let (a2, r2, c2) = (a.clone(), r.clone(), c.clone());
        spawn_async(
            async move {
                encryption::setup_recovery(&c2, &pw)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| match res {
                Ok(key) => {
                    show_recovery_key(&a2, &key);
                    r2();
                }
                Err(e) => error_dialog(&a2, &e),
            },
        );
    });

    let (a, c, r) = (app.clone(), client.clone(), reload.clone());
    reset.connect_clicked(move |_| {
        if !confirm(
            &a,
            "Replace the recovery key? The old key will stop working, so keep the new one safe.",
        ) {
            return;
        }
        let (a2, r2, c2) = (a.clone(), r.clone(), c.clone());
        spawn_async(
            async move {
                encryption::reset_recovery_key(&c2)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                match res {
                    Ok(key) => show_recovery_key(&a2, &key),
                    Err(e) => error_dialog(&a2, &e),
                }
                r2();
            },
        );
    });

    let (a, c, r) = (app.clone(), client.clone(), reload.clone());
    disable.connect_clicked(move |_| {
        if !confirm(
            &a,
            "Turn off recovery? This deletes your recovery key and key backup from the server. \
             New devices will not be able to read older encrypted messages.",
        ) {
            return;
        }
        let (a2, r2, c2) = (a.clone(), r.clone(), c.clone());
        spawn_async(
            async move {
                encryption::disable_recovery(&c2)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                if let Err(e) = res {
                    error_dialog(&a2, &e);
                }
                r2();
            },
        );
    });

    let (a, c, r) = (app.clone(), client, reload);
    enter.connect_clicked(move |_| {
        let Some(key) = prompt_secret(&a, "Recovery key", "Paste your recovery key") else {
            return;
        };
        let (a2, r2, c2) = (a.clone(), r.clone(), c.clone());
        spawn_async(
            async move {
                encryption::recover(&c2, &key)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |res| {
                if let Err(e) = res {
                    error_dialog(&a2, &e);
                }
                r2();
            },
        );
    });

    d.connect_response(|d, _| d.close());
    d.show_all();
}

/// Ask a yes/no question; the destructive answer is never the default.
fn confirm(app: &Rc<App>, question: &str) -> bool {
    let d = gtk::MessageDialog::new(
        Some(&app.window),
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        question,
    );
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Continue", gtk::ResponseType::Accept);
    d.set_default_response(gtk::ResponseType::Cancel);
    let ok = d.run() == gtk::ResponseType::Accept;
    d.close();
    ok
}

fn summary_markup(s: &SecurityStatus) -> String {
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    format!(
        "<b>This device:</b> {}\n<b>Cross-signing set up:</b> {}\n<b>This device verified:</b> {}\n<b>Recovery:</b> {}\n<b>Key backup:</b> {}",
        glib::markup_escape_text(&s.device_id),
        yes_no(s.cross_signing_ready),
        yes_no(s.this_device_verified),
        s.recovery.label(),
        yes_no(s.backup_enabled),
    )
}

fn devices_markup(devs: &[DeviceInfo]) -> String {
    if devs.is_empty() {
        return String::new();
    }
    let mut out = String::from("<b>Your devices</b>");
    for d in devs {
        out.push_str(&format!(
            "\n{} {} <small>({}){}</small>",
            if d.verified { "✔" } else { "⚠" },
            glib::markup_escape_text(d.name.as_deref().unwrap_or(if d.is_current {
                "This device"
            } else {
                "Unnamed device"
            })),
            glib::markup_escape_text(&d.device_id),
            if d.is_current { " - this device" } else { "" },
        ));
    }
    out
}

/// The recovery key is shown once; the app does not store it.
fn show_recovery_key(app: &Rc<App>, key: &str) {
    let d = gtk::Dialog::with_buttons(
        Some("Your recovery key"),
        Some(&app.window),
        gtk::DialogFlags::MODAL,
        &[("I have saved it", gtk::ResponseType::Ok)],
    );
    d.set_default_size(480, -1);
    let area = d.content_area();
    area.set_margin(12);
    area.set_spacing(8);
    let l = gtk::Label::new(Some(
        "Save this key somewhere safe (a password manager is ideal). It is shown only once \
         and is the only way to read old encrypted messages on a new device.",
    ));
    l.set_line_wrap(true);
    l.set_xalign(0.0);
    let k = gtk::Label::new(None);
    k.set_markup(&format!(
        "<tt><b>{}</b></tt>",
        glib::markup_escape_text(key)
    ));
    k.set_selectable(true);
    k.set_line_wrap(true);
    let copy = gtk::Button::with_label("Copy to clipboard");
    let key = key.to_owned();
    copy.connect_clicked(move |_| {
        gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_text(&key);
    });
    area.pack_start(&l, false, false, 0);
    area.pack_start(&k, false, false, 0);
    area.pack_start(&copy, false, false, 0);
    d.show_all();
    d.run();
    d.close();
}
