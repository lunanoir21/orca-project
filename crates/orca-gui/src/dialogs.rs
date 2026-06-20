//! Small reusable dialog helpers built on GTK's `AlertDialog` plus a couple of
//! custom windows (text result, passphrase prompt).
//!
//! All helpers are callback-based: they take an optional parent window and a
//! closure invoked on confirmation, so callers in components can route the
//! result back through their own message channel.

use relm4::gtk;
use relm4::gtk::prelude::*;

use crate::i18n;

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

/// Handle to a still-open passphrase prompt, returned by [`passphrase`] so an
/// async caller (e.g. a vault unlock that has to round-trip through a
/// background task) can report the outcome back into the *same* dialog
/// instead of closing it immediately and popping a second error dialog on
/// failure. The user can then just retry in place.
#[derive(Debug, Clone)]
pub struct PassphrasePrompt {
    window: gtk::Window,
    entry: gtk::PasswordEntry,
    error_label: gtk::Label,
}

impl PassphrasePrompt {
    /// The submission succeeded; close the dialog.
    pub fn close(&self) {
        self.window.close();
    }

    /// The submission failed; show `msg` inline and let the user retry
    /// without reopening anything.
    pub fn show_error(&self, msg: &str) {
        self.error_label.set_label(msg);
        self.error_label.set_visible(true);
        self.entry.set_sensitive(true);
        self.entry.set_text("");
        self.entry.grab_focus();
    }
}

/// Prompt for a passphrase (masked, with a peek-to-reveal icon). `on_submit`
/// receives the entered value every time the user confirms with a non-empty
/// passphrase; the dialog stays open (entry disabled) afterwards rather than
/// closing immediately — the caller drives the returned [`PassphrasePrompt`]
/// handle's `close`/`show_error` once it knows whether the submission
/// actually succeeded, so a wrong passphrase can be retried in place instead
/// of reopening a fresh dialog.
pub fn passphrase(
    parent: Option<&gtk::Window>,
    title: &str,
    on_submit: impl Fn(String) + 'static,
) -> PassphrasePrompt {
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

    let error_label = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["error"])
        .build();
    vbox.append(&error_label);

    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel = gtk::Button::with_label(&i18n::t("common.cancel"));
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| window.close());
    }
    let ok = gtk::Button::builder()
        .label(i18n::t("common.ok"))
        .css_classes(["suggested-action"])
        .build();
    buttons.append(&cancel);
    buttons.append(&ok);
    vbox.append(&buttons);

    let prompt = PassphrasePrompt {
        window: window.clone(),
        entry: entry.clone(),
        error_label: error_label.clone(),
    };

    let submit = {
        let prompt = prompt.clone();
        move || {
            let value = prompt.entry.text().to_string();
            if value.is_empty() {
                return;
            }
            prompt.error_label.set_visible(false);
            prompt.entry.set_sensitive(false);
            on_submit(value);
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
    crate::anim::present_with_fade(&window);
    entry.grab_focus();
    prompt
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
    let status_lbl = gtk::Label::builder().halign(gtk::Align::Start).build();
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
    crate::anim::present_with_fade(&window);
}

/// Find the toplevel [`gtk::Window`] of a widget, if it is realised in one.
#[must_use]
pub fn window_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.root().and_then(|r| r.downcast::<gtk::Window>().ok())
}
