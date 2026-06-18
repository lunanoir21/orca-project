//! File properties dialog (Phase 3.8).
//!
//! Shows name, path, kind, size, timestamps, permissions, owner/group and (for
//! symlinks) the link target, and computes SHA-256/MD5/Blake3 checksums on
//! demand off the UI thread. Built as a transient relm4 component whose root is
//! a `GtkWindow`; the content is assembled imperatively and the checksum value
//! labels are updated in place when results arrive.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::FileEntry;

use crate::format;

/// A checksum algorithm offered in the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    /// SHA-256.
    Sha256,
    /// MD5.
    Md5,
    /// Blake3.
    Blake3,
}

/// Properties dialog model.
pub struct Properties {
    path: PathBuf,
    /// Checksum value labels, updated in place when results arrive.
    sha256: gtk::Label,
    md5: gtk::Label,
    blake3: gtk::Label,
}

/// Messages the dialog handles.
#[derive(Debug)]
pub enum PropInput {
    /// Compute the given checksum.
    Compute(Algo),
}

/// Background checksum results.
#[derive(Debug)]
pub enum PropCmd {
    /// A checksum finished.
    Checksum(Algo, Result<String, String>),
}

#[relm4::component(pub)]
impl Component for Properties {
    type Init = FileEntry;
    type Input = PropInput;
    type Output = ();
    type CommandOutput = PropCmd;

    view! {
        #[root]
        gtk::Window {
            set_title: Some("Properties"),
            set_modal: true,
            set_default_width: 480,
        }
    }

    fn init(
        entry: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let grid = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(12)
            .build();
        let mut row = 0;
        let mut add = |label: &str, value: &str| {
            grid.attach(&row_label(label), 0, row, 1, 1);
            grid.attach(&value_label(value), 1, row, 1, 1);
            row += 1;
        };
        let meta = std::fs::symlink_metadata(&entry.path).ok();
        add("Name", &entry.name);
        add("Location", &entry.path.to_string_lossy());
        add("Type", format::kind(entry.kind));
        add("Size", &format::size(entry.size));
        add(
            "Permissions",
            &format!(
                "{} ({:#o})",
                format::permissions(entry.permissions),
                entry.permissions
            ),
        );
        if let Some(m) = &meta {
            add("Owner", &format!("uid {} / gid {}", m.uid(), m.gid()));
        }
        add("Modified", &format::modified(entry.modified));
        if let Some(m) = &meta {
            add("Accessed", &format::modified(m.accessed().ok()));
            add("Created", &format::modified(m.created().ok()));
        }
        if entry.is_symlink {
            if let Ok(target) = std::fs::read_link(&entry.path) {
                add("Link target", &target.to_string_lossy());
            }
        }
        content.append(&grid);

        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let heading = gtk::Label::builder().xalign(0.0).build();
        heading.set_markup("<b>Checksums</b>");
        content.append(&heading);

        let sha256 = value_label("");
        let md5 = value_label("");
        let blake3 = value_label("");
        content.append(&checksum_row("SHA-256", &sha256, Algo::Sha256, &sender));
        content.append(&checksum_row("MD5", &md5, Algo::Md5, &sender));
        content.append(&checksum_row("Blake3", &blake3, Algo::Blake3, &sender));

        root.set_child(Some(&content));

        let model = Properties {
            path: entry.path.clone(),
            sha256,
            md5,
            blake3,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PropInput::Compute(algo) => {
                self.set_sum(algo, "computing…");
                let path = self.path.clone();
                sender.oneshot_command(async move {
                    let result = match algo {
                        Algo::Sha256 => orca_core::checksum_sha256(&path).await,
                        Algo::Md5 => orca_core::checksum_md5(&path).await,
                        Algo::Blake3 => orca_core::checksum_blake3(&path).await,
                    };
                    PropCmd::Checksum(algo, result.map_err(|e| e.to_string()))
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PropCmd::Checksum(algo, Ok(sum)) => self.set_sum(algo, &sum),
            PropCmd::Checksum(algo, Err(e)) => self.set_sum(algo, &format!("error: {e}")),
        }
    }
}

impl Properties {
    fn set_sum(&self, algo: Algo, value: &str) {
        match algo {
            Algo::Sha256 => self.sha256.set_text(value),
            Algo::Md5 => self.md5.set_text(value),
            Algo::Blake3 => self.blake3.set_text(value),
        }
    }
}

/// A left-column field label.
fn row_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build()
}

/// A right-column selectable value label.
fn value_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .hexpand(true)
        .build()
}

/// Build one checksum row: name, a Compute button, and the value label.
fn checksum_row(
    name: &str,
    value: &gtk::Label,
    algo: Algo,
    sender: &ComponentSender<Properties>,
) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(name)
            .width_chars(8)
            .xalign(0.0)
            .build(),
    );
    let button = gtk::Button::with_label("Compute");
    {
        let sender = sender.clone();
        button.connect_clicked(move |_| sender.input(PropInput::Compute(algo)));
    }
    row.append(&button);
    row.append(value);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use orca_core::FileKind;
    use std::time::SystemTime;

    #[test]
    fn algo_is_copy() {
        let a = Algo::Sha256;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn file_entry_fields_format() {
        // Exercise the pure formatting used by the dialog without GTK.
        let _ = FileKind::File;
        let _ = SystemTime::UNIX_EPOCH;
        assert_eq!(format::size(10), "10 B");
        assert_eq!(format::permissions(0o644), "rw-r--r--");
    }
}
