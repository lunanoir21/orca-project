//! File permissions editor dialog (Phase 5.2).
//!
//! Displays the nine standard Unix permission bits (owner/group/others ×
//! read/write/execute) as toggles plus an octal display that stays in sync.
//! For directories, a "apply recursively" option is available.
//! Changes are applied via [`orca_core::set_permissions`].

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::FileKind;

use crate::i18n;

/// Dialog init: the entry to edit.
pub struct PermissionsInit {
    /// Path of the file/directory.
    pub path: PathBuf,
    /// Current permission bits (Unix mode, low 12 bits used).
    pub mode: u32,
    /// Whether the target is a directory (enables recursive option).
    pub is_dir: bool,
    /// Current owner UID (shown in the ownership section).
    pub uid: u32,
    /// Current group GID (shown in the ownership section).
    pub gid: u32,
}

/// Dialog model.
pub struct PermissionsDialog {
    path: PathBuf,
    is_dir: bool,
    /// The nine permission bits as `Rc<Cell<bool>>`, in the order
    /// owner-r, owner-w, owner-x, group-r, group-w, group-x, other-r, other-w,
    /// other-x.
    bits: [Rc<Cell<bool>>; 9],
    /// Octal display label.
    octal_lbl: gtk::Label,
    /// Recursive toggle.
    recursive: Rc<Cell<bool>>,
    /// UID entry (read-only display of the file owner).
    #[allow(dead_code)]
    uid_entry: gtk::Entry,
    /// GID entry (read-only display of the file group).
    #[allow(dead_code)]
    gid_entry: gtk::Entry,
}

/// Input messages.
#[derive(Debug)]
pub enum PermsInput {
    /// A bit toggled — recompute and display octal.
    BitChanged,
    /// Apply the changes.
    Apply,
}

/// Background cmd result.
#[derive(Debug)]
pub enum PermsCmd {
    Done(Result<(), String>),
}

#[relm4::component(pub)]
impl Component for PermissionsDialog {
    type Init = PermissionsInit;
    type Input = PermsInput;
    type Output = ();
    type CommandOutput = PermsCmd;

    view! {
        #[root]
        gtk::Window {
            set_title: Some(&i18n::t("perms.title")),
            set_modal: true,
            set_default_width: 380,
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(14)
            .margin_bottom(14)
            .margin_start(14)
            .margin_end(14)
            .build();

        // --- Permission grid: owner / group / others × r / w / x ---
        let grid = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(12)
            .build();

        // Header row
        for (col, label) in [
            (1, i18n::t("perms.read")),
            (2, i18n::t("perms.write")),
            (3, i18n::t("perms.exec")),
        ] {
            let h = gtk::Label::builder()
                .label(label)
                .halign(gtk::Align::Center)
                .build();
            h.add_css_class("dim-label");
            grid.attach(&h, col, 0, 1, 1);
        }

        let row_labels = [
            i18n::t("perms.owner"),
            i18n::t("perms.group"),
            i18n::t("perms.others"),
        ];

        // Build 9 bit cells
        let bits: [Rc<Cell<bool>>; 9] = std::array::from_fn(|_| Rc::new(Cell::new(false)));

        for (row, label) in row_labels.iter().enumerate() {
            let lbl = gtk::Label::builder()
                .label(label.as_str())
                .halign(gtk::Align::End)
                .build();
            grid.attach(&lbl, 0, (row + 1) as i32, 1, 1);

            for col in 0..3usize {
                let idx = row * 3 + col;
                // Shift: owner-r = bit 8, down to other-x = bit 0.
                let shift = 8u32.saturating_sub(idx as u32);
                let bit_val = (init.mode >> shift) & 1 == 1;
                bits[idx].set(bit_val);

                let chk = gtk::CheckButton::new();
                chk.set_active(bit_val);
                chk.set_halign(gtk::Align::Center);
                let cell = bits[idx].clone();
                let s = sender.clone();
                chk.connect_toggled(move |b| {
                    cell.set(b.is_active());
                    s.input(PermsInput::BitChanged);
                });
                grid.attach(&chk, (col + 1) as i32, (row + 1) as i32, 1, 1);
            }
        }
        outer.append(&grid);

        // --- Octal display ---
        let octal_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        let octal_lbl_header = gtk::Label::new(Some(&i18n::t("perms.octal")));
        octal_lbl_header.add_css_class("dim-label");
        let octal_lbl = gtk::Label::builder().label(format_octal(&bits)).build();
        octal_lbl.add_css_class("monospace");
        octal_row.append(&octal_lbl_header);
        octal_row.append(&octal_lbl);
        outer.append(&octal_row);

        // --- Ownership display (read-only: uid / gid) ---
        let own_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        let uid_lbl = gtk::Label::builder()
            .label(i18n::t("perms.owner"))
            .halign(gtk::Align::Start)
            .build();
        uid_lbl.add_css_class("dim-label");
        let uid_entry = gtk::Entry::builder()
            .text(init.uid.to_string().as_str())
            .editable(false)
            .width_chars(8)
            .build();
        let gid_lbl = gtk::Label::builder()
            .label(i18n::t("perms.group"))
            .halign(gtk::Align::Start)
            .build();
        gid_lbl.add_css_class("dim-label");
        let gid_entry = gtk::Entry::builder()
            .text(init.gid.to_string().as_str())
            .editable(false)
            .width_chars(8)
            .build();
        own_row.append(&uid_lbl);
        own_row.append(&uid_entry);
        own_row.append(&gid_lbl);
        own_row.append(&gid_entry);
        outer.append(&own_row);

        // --- Recursive option (directories only) ---
        let recursive = Rc::new(Cell::new(false));
        if init.is_dir {
            let rec_chk = gtk::CheckButton::with_label(&i18n::t("perms.recursive"));
            let rc = recursive.clone();
            rec_chk.connect_toggled(move |b| rc.set(b.is_active()));
            outer.append(&rec_chk);
        }

        // --- Buttons ---
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
        let apply_btn = gtk::Button::with_label(&i18n::t("perms.apply"));
        apply_btn.add_css_class("suggested-action");
        {
            let s = sender.clone();
            apply_btn.connect_clicked(move |_| s.input(PermsInput::Apply));
        }
        btn_row.append(&cancel_btn);
        btn_row.append(&apply_btn);
        outer.append(&btn_row);

        root.set_child(Some(&outer));

        let model = PermissionsDialog {
            path: init.path,
            is_dir: init.is_dir,
            bits,
            octal_lbl,
            recursive,
            uid_entry,
            gid_entry,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            PermsInput::BitChanged => {
                self.octal_lbl.set_label(&format_octal(&self.bits));
            }
            PermsInput::Apply => {
                let mode = bits_to_mode(&self.bits);
                let path = self.path.clone();
                let recursive = self.recursive.get();
                let is_dir = self.is_dir;
                sender.oneshot_command(async move {
                    let result = if recursive && is_dir {
                        apply_recursive(&path, mode).await
                    } else {
                        orca_core::set_permissions(&path, mode)
                            .await
                            .map_err(|e| e.to_string())
                    };
                    PermsCmd::Done(result)
                });
                root.close();
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        if let PermsCmd::Done(Err(e)) = msg {
            tracing::warn!(error = %e, "set permissions failed");
        }
    }
}

/// Compute the current permission mode from the nine bit cells.
fn bits_to_mode(bits: &[Rc<Cell<bool>>; 9]) -> u32 {
    let mut mode = 0u32;
    for (idx, bit) in bits.iter().enumerate() {
        if bit.get() {
            let shift = 8u32.saturating_sub(idx as u32);
            mode |= 1 << shift;
        }
    }
    mode
}

/// Format the permission mode as a 4-digit octal string (`0755`).
fn format_octal(bits: &[Rc<Cell<bool>>; 9]) -> String {
    format!("{:04o}", bits_to_mode(bits))
}

/// Apply `mode` recursively to a directory tree.
async fn apply_recursive(root: &std::path::Path, mode: u32) -> Result<(), String> {
    orca_core::set_permissions(root, mode)
        .await
        .map_err(|e| e.to_string())?;
    let entries = orca_core::list_dir(root, &Default::default(), Default::default())
        .await
        .map_err(|e| e.to_string())?;
    for entry in entries {
        if entry.kind == FileKind::Directory {
            Box::pin(apply_recursive(&entry.path, mode)).await?;
        } else {
            orca_core::set_permissions(&entry.path, mode)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_to_mode_identity() {
        let bits: [Rc<Cell<bool>>; 9] = [
            Rc::new(Cell::new(true)),  // owner r
            Rc::new(Cell::new(true)),  // owner w
            Rc::new(Cell::new(true)),  // owner x
            Rc::new(Cell::new(true)),  // group r
            Rc::new(Cell::new(false)), // group w
            Rc::new(Cell::new(true)),  // group x
            Rc::new(Cell::new(true)),  // other r
            Rc::new(Cell::new(false)), // other w
            Rc::new(Cell::new(true)),  // other x
        ];
        // rwxr-xr-x = 0755
        assert_eq!(bits_to_mode(&bits), 0o755);
    }

    #[test]
    fn format_octal_zero() {
        let bits: [Rc<Cell<bool>>; 9] = std::array::from_fn(|_| Rc::new(Cell::new(false)));
        assert_eq!(format_octal(&bits), "0000");
    }
}
