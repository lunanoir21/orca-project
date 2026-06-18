//! Bulk rename dialog (Phase 5.1).
//!
//! Opened when the user right-clicks two or more files and chooses
//! "Bulk Rename…". Shows a live preview of old→new names, highlights
//! conflicts in red, and applies the renames off the UI thread.
//!
//! Template variables (passed through to [`orca_core::BulkRenameRule`]):
//! - `{name}` — file stem
//! - `{ext}`  — extension without dot
//! - `{n}`    — counter, zero-padded to `counter_width`
//! - `{date}` — today as `YYYY-MM-DD`

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::{BulkRenameRule, RenameConflict, RenamePreview};

use crate::i18n;

/// Emitted to the parent when the dialog is done.
#[derive(Debug)]
pub enum BulkRenameOutput {
    /// Renames were applied; parent should reload the listing.
    Done,
}

/// Dialog model.
pub struct BulkRenameDialog {
    paths: Vec<PathBuf>,
    rule: BulkRenameRule,
    /// Reference to the preview list to rebuild it in-place.
    list_box: gtk::ListBox,
    /// Apply button — disabled when any preview row has a conflict.
    apply_btn: gtk::Button,
}

/// Input messages.
#[derive(Debug)]
pub enum BulkRenameInput {
    FindChanged(String),
    ReplaceChanged(String),
    CaseToggled(bool),
    CounterStartChanged(u32),
    CounterStepChanged(u32),
    CounterWidthChanged(u32),
    /// Trigger a preview re-compute (fired automatically by field changes).
    Refresh,
    Apply,
}

/// Background task results.
#[derive(Debug)]
pub enum BulkRenameCmd {
    /// Preview computation finished.
    Previewed(Result<Vec<RenamePreview>, String>),
    /// Apply finished.
    Applied(Result<usize, String>),
}

#[relm4::component(pub)]
impl Component for BulkRenameDialog {
    type Init = Vec<PathBuf>;
    type Input = BulkRenameInput;
    type Output = BulkRenameOutput;
    type CommandOutput = BulkRenameCmd;

    view! {
        #[root]
        gtk::Window {
            set_title: Some(&i18n::t("bulk.title")),
            set_modal: true,
            set_default_width: 680,
            set_default_height: 540,
        }
    }

    fn init(
        paths: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // --- Build content ---
        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(14)
            .margin_bottom(14)
            .margin_start(14)
            .margin_end(14)
            .build();

        // --- Field rows ---
        let fields = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .build();

        let find_lbl = gtk::Label::builder()
            .label(i18n::t("bulk.find"))
            .halign(gtk::Align::End)
            .build();
        let find_entry = gtk::Entry::builder().hexpand(true).build();
        {
            let s = sender.clone();
            find_entry.connect_changed(move |e| {
                s.input(BulkRenameInput::FindChanged(e.text().to_string()));
            });
        }
        fields.attach(&find_lbl, 0, 0, 1, 1);
        fields.attach(&find_entry, 1, 0, 3, 1);

        let replace_lbl = gtk::Label::builder()
            .label(i18n::t("bulk.replace"))
            .halign(gtk::Align::End)
            .build();
        let replace_entry = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("{name}.{ext}")
            .build();
        {
            let s = sender.clone();
            replace_entry.connect_changed(move |e| {
                s.input(BulkRenameInput::ReplaceChanged(e.text().to_string()));
            });
        }
        fields.attach(&replace_lbl, 0, 1, 1, 1);
        fields.attach(&replace_entry, 1, 1, 3, 1);

        let case_chk = gtk::CheckButton::with_label(&i18n::t("bulk.case"));
        {
            let s = sender.clone();
            case_chk.connect_toggled(move |b| {
                s.input(BulkRenameInput::CaseToggled(b.is_active()));
            });
        }
        fields.attach(&case_chk, 0, 2, 4, 1);

        // Counter row
        let ctr_lbl = gtk::Label::builder()
            .label(i18n::t("bulk.counter"))
            .halign(gtk::Align::End)
            .build();
        let start_spin = gtk::SpinButton::with_range(0.0, 99999.0, 1.0);
        start_spin.set_value(1.0);
        {
            let s = sender.clone();
            start_spin.connect_value_changed(move |b| {
                s.input(BulkRenameInput::CounterStartChanged(b.value() as u32));
            });
        }
        let step_lbl = gtk::Label::new(Some(&i18n::t("bulk.step")));
        let step_spin = gtk::SpinButton::with_range(1.0, 100.0, 1.0);
        step_spin.set_value(1.0);
        {
            let s = sender.clone();
            step_spin.connect_value_changed(move |b| {
                s.input(BulkRenameInput::CounterStepChanged(b.value() as u32));
            });
        }
        let width_lbl = gtk::Label::new(Some(&i18n::t("bulk.width")));
        let width_spin = gtk::SpinButton::with_range(1.0, 10.0, 1.0);
        width_spin.set_value(1.0);
        {
            let s = sender.clone();
            width_spin.connect_value_changed(move |b| {
                s.input(BulkRenameInput::CounterWidthChanged(b.value() as u32));
            });
        }
        let ctr_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        ctr_row.append(&start_spin);
        ctr_row.append(&step_lbl);
        ctr_row.append(&step_spin);
        ctr_row.append(&width_lbl);
        ctr_row.append(&width_spin);
        fields.attach(&ctr_lbl, 0, 3, 1, 1);
        fields.attach(&ctr_row, 1, 3, 3, 1);

        outer.append(&fields);

        // --- Preview list ---
        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::None);
        list_box.add_css_class("boxed-list");
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(200)
            .child(&list_box)
            .build();
        outer.append(&scroll);

        // --- Button row ---
        let btn_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        let cancel_btn = gtk::Button::with_label(&i18n::t("common.cancel"));
        {
            let root = root.clone();
            cancel_btn.connect_clicked(move |_| root.close());
        }
        let apply_btn = gtk::Button::with_label(&i18n::t("bulk.apply"));
        apply_btn.set_sensitive(false);
        apply_btn.add_css_class("suggested-action");
        {
            let s = sender.clone();
            apply_btn.connect_clicked(move |_| s.input(BulkRenameInput::Apply));
        }
        btn_row.append(&cancel_btn);
        btn_row.append(&apply_btn);
        outer.append(&btn_row);

        root.set_child(Some(&outer));

        let rule = BulkRenameRule::default();

        let model = BulkRenameDialog {
            paths,
            rule,
            list_box,
            apply_btn,
        };

        // Trigger initial preview (empty rule shows current names unchanged).
        sender.input(BulkRenameInput::Refresh);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            BulkRenameInput::FindChanged(s) => {
                self.rule.find = if s.is_empty() { None } else { Some(s) };
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::ReplaceChanged(s) => {
                self.rule.replace = s;
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::CaseToggled(v) => {
                self.rule.case_insensitive = v;
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::CounterStartChanged(v) => {
                self.rule.counter_start = v as usize;
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::CounterStepChanged(v) => {
                self.rule.counter_step = v as usize;
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::CounterWidthChanged(v) => {
                self.rule.counter_width = v as usize;
                sender.input(BulkRenameInput::Refresh);
            }
            BulkRenameInput::Refresh => {
                let paths = self.paths.clone();
                let rule = self.rule.clone();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        orca_core::preview_rename(&paths, &rule)
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    BulkRenameCmd::Previewed(result)
                });
            }
            BulkRenameInput::Apply => {
                let paths = self.paths.clone();
                let rule = self.rule.clone();
                sender.oneshot_command(async move {
                    let result = orca_core::apply_rename(&paths, &rule)
                        .await
                        .map(|v| v.len())
                        .map_err(|e| e.to_string());
                    BulkRenameCmd::Applied(result)
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            BulkRenameCmd::Previewed(Ok(previews)) => {
                self.rebuild_list(&previews);
            }
            BulkRenameCmd::Previewed(Err(e)) => {
                // Invalid regex — clear list, disable Apply, show nothing.
                while let Some(child) = self.list_box.first_child() {
                    self.list_box.remove(&child);
                }
                self.apply_btn.set_sensitive(false);
                tracing::debug!("bulk rename preview error: {e}");
            }
            BulkRenameCmd::Applied(Ok(n)) => {
                let msg = i18n::tf("bulk.done", &[("n", &n.to_string())]);
                let win = crate::dialogs::window_of(root);
                crate::dialogs::info(win.as_ref(), &i18n::t("bulk.title"), &msg);
                sender.output(BulkRenameOutput::Done).ok();
                root.close();
            }
            BulkRenameCmd::Applied(Err(e)) => {
                let msg = i18n::tf("bulk.error", &[("msg", &e)]);
                let win = crate::dialogs::window_of(root);
                crate::dialogs::info(win.as_ref(), &i18n::t("bulk.title"), &msg);
            }
        }
    }
}

impl BulkRenameDialog {
    /// Clear and rebuild the preview `ListBox` from `previews`. Also updates
    /// the Apply button sensitivity (disabled if any conflict is detected).
    fn rebuild_list(&self, previews: &[RenamePreview]) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        let has_conflict = previews.iter().any(|p| p.conflict.is_some());
        self.apply_btn.set_sensitive(!has_conflict);

        for p in previews {
            let row = gtk::ListBoxRow::new();
            let inner = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(6)
                .margin_end(6)
                .build();

            let old_name = p
                .original
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let old_lbl = gtk::Label::builder()
                .label(&old_name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            let arrow = gtk::Label::new(Some("→"));
            arrow.add_css_class("dim-label");
            let new_lbl = gtk::Label::builder()
                .label(&p.new_name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();

            if let Some(conflict) = &p.conflict {
                let tag = match conflict {
                    RenameConflict::Empty => "!empty",
                    RenameConflict::InvalidName => "!invalid",
                    RenameConflict::DuplicateTarget => "!dup",
                    RenameConflict::TargetExists => "!exists",
                };
                let badge = gtk::Label::builder()
                    .label(tag)
                    .build();
                badge.add_css_class("error");
                inner.append(&old_lbl);
                inner.append(&arrow);
                inner.append(&new_lbl);
                inner.append(&badge);
                row.add_css_class("error");
            } else {
                inner.append(&old_lbl);
                inner.append(&arrow);
                inner.append(&new_lbl);
            }

            row.set_child(Some(&inner));
            self.list_box.append(&row);
        }
    }
}
