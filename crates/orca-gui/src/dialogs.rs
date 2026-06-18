//! Small reusable dialog helpers built on GTK's `AlertDialog` plus a couple of
//! custom windows (text result, passphrase prompt).
//!
//! All helpers are callback-based: they take an optional parent window and a
//! closure invoked on confirmation, so callers in components can route the
//! result back through their own message channel.

use relm4::gtk;
use relm4::gtk::prelude::*;

/// Show a modal confirmation with a destructive default action; `on_confirm`
/// runs only if the user chooses the confirm button.
pub fn confirm(
    parent: Option<&gtk::Window>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(heading)
        .detail(body)
        .buttons([confirm_label, "Cancel"])
        .cancel_button(1)
        .default_button(1)
        .build();
    dialog.choose(parent, gtk::gio::Cancellable::NONE, move |result| {
        if let Ok(0) = result {
            on_confirm();
        }
    });
}

/// Show a simple informational message.
pub fn info(parent: Option<&gtk::Window>, heading: &str, body: &str) {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(heading)
        .detail(body)
        .buttons(["OK"])
        .build();
    dialog.show(parent);
}

/// Prompt for a passphrase (masked); `on_ok` receives the entered value when the
/// user confirms with a non-empty passphrase.
pub fn passphrase(parent: Option<&gtk::Window>, title: &str, on_ok: impl Fn(String) + 'static) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(380)
        .build();
    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    vbox.append(&entry);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    let ok = gtk::Button::builder()
        .label("OK")
        .css_classes(["suggested-action"])
        .build();
    buttons.append(&cancel);
    buttons.append(&ok);
    vbox.append(&buttons);

    let submit = {
        let entry = entry.clone();
        let window = window.clone();
        move || {
            let value = entry.text().to_string();
            if !value.is_empty() {
                on_ok(value);
                window.close();
            }
        }
    };
    let submit = std::rc::Rc::new(submit);
    {
        let submit = submit.clone();
        ok.connect_clicked(move |_| submit());
    }
    {
        let submit = submit.clone();
        entry.connect_activate(move |_| submit());
    }

    window.set_child(Some(&vbox));
    window.present();
    entry.grab_focus();
}

/// Show a checksum result dialog with a copy-to-clipboard button and a compare
/// field. The compare field turns green/red when the user pastes an expected
/// value so they can verify integrity at a glance.
pub fn checksum(parent: Option<&gtk::Window>, title: &str, digest: &str) {
    use crate::i18n;

    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(560)
        .build();
    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    // --- Computed digest row ---
    let digest_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let digest_entry = gtk::Entry::builder()
        .text(digest)
        .editable(false)
        .hexpand(true)
        .build();
    digest_entry.add_css_class("monospace");
    let copy_btn = gtk::Button::with_label(&i18n::t("chk.copy"));
    {
        let value = digest.to_owned();
        copy_btn.connect_clicked(move |btn| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&value);
            }
            btn.set_label(&i18n::t("chk.copied"));
        });
    }
    digest_row.append(&digest_entry);
    digest_row.append(&copy_btn);
    vbox.append(&digest_row);

    // --- Compare field ---
    let compare_entry = gtk::Entry::builder()
        .placeholder_text(i18n::t("chk.compare").as_str())
        .hexpand(true)
        .build();
    compare_entry.add_css_class("monospace");
    vbox.append(&compare_entry);

    // Match/mismatch status label
    let status_lbl = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .build();
    vbox.append(&status_lbl);

    {
        let digest_lc = digest.to_lowercase();
        let status_lbl = status_lbl.clone();
        compare_entry.connect_changed(move |entry| {
            let val = entry.text().to_string();
            if val.is_empty() {
                status_lbl.set_label("");
                status_lbl.remove_css_class("success");
                status_lbl.remove_css_class("error");
            } else if val.trim().to_lowercase() == digest_lc {
                status_lbl.set_label(&i18n::t("chk.match"));
                status_lbl.add_css_class("success");
                status_lbl.remove_css_class("error");
            } else {
                status_lbl.set_label(&i18n::t("chk.mismatch"));
                status_lbl.add_css_class("error");
                status_lbl.remove_css_class("success");
            }
        });
    }

    // --- Close button ---
    let close = gtk::Button::with_label(&i18n::t("common.close"));
    close.set_halign(gtk::Align::End);
    {
        let window = window.clone();
        close.connect_clicked(move |_| window.close());
    }
    vbox.append(&close);

    window.set_child(Some(&vbox));
    window.present();
}

/// Find the toplevel [`gtk::Window`] of a widget, if it is realised in one.
#[must_use]
pub fn window_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.root().and_then(|r| r.downcast::<gtk::Window>().ok())
}
