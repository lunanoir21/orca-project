//! Read-only archive content browser (Phase 5.3).
//!
//! Shows the entries inside a supported archive file (zip, tar.gz, tar.xz,
//! tar.bz2) in a simple scrolled list. The listing runs off the UI thread via
//! [`orca_core::list_archive`].

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::ArchiveEntry;

use crate::format;
use crate::i18n;

/// Archive browser model.
pub struct ArchiveView {
    path: PathBuf,
    list_box: gtk::ListBox,
}

/// Input messages.
#[derive(Debug)]
pub enum ArchiveViewInput {
    /// Reload the archive listing.
    Reload,
}

/// Background results.
#[derive(Debug)]
pub enum ArchiveViewCmd {
    Listed(Result<Vec<ArchiveEntry>, String>),
}

#[relm4::component(pub)]
impl Component for ArchiveView {
    type Init = PathBuf;
    type Input = ArchiveViewInput;
    type Output = ();
    type CommandOutput = ArchiveViewCmd;

    view! {
        #[root]
        gtk::Window {
            set_modal: true,
            set_default_width: 540,
            set_default_height: 420,
        }
    }

    fn init(
        path: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        root.set_title(Some(&title));

        let widgets = view_output!();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::None);
        list_box.add_css_class("boxed-list");

        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(300)
            .child(&list_box)
            .build();
        outer.append(&scroll);

        let close = gtk::Button::with_label(&i18n::t("common.close"));
        close.set_halign(gtk::Align::End);
        {
            let root = root.clone();
            close.connect_clicked(move |_| root.close());
        }
        outer.append(&close);

        root.set_child(Some(&outer));

        let model = ArchiveView { path, list_box };

        sender.input(ArchiveViewInput::Reload);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            ArchiveViewInput::Reload => {
                let path = self.path.clone();
                sender.oneshot_command(async move {
                    let result = orca_core::list_archive(path)
                        .await
                        .map_err(|e| e.to_string());
                    ArchiveViewCmd::Listed(result)
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ArchiveViewCmd::Listed(Ok(entries)) => {
                while let Some(child) = self.list_box.first_child() {
                    self.list_box.remove(&child);
                }
                for entry in &entries {
                    let row = gtk::ListBoxRow::new();
                    let inner = gtk::Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .spacing(12)
                        .margin_top(4)
                        .margin_bottom(4)
                        .margin_start(6)
                        .margin_end(6)
                        .build();
                    let name_lbl = gtk::Label::builder()
                        .label(&entry.path)
                        .halign(gtk::Align::Start)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::Middle)
                        .build();
                    let size_lbl = gtk::Label::builder()
                        .label(format::size(entry.size))
                        .halign(gtk::Align::End)
                        .build();
                    size_lbl.add_css_class("dim-label");
                    inner.append(&name_lbl);
                    inner.append(&size_lbl);
                    row.set_child(Some(&inner));
                    self.list_box.append(&row);
                }
            }
            ArchiveViewCmd::Listed(Err(e)) => {
                tracing::warn!(error = %e, "archive listing failed");
            }
        }
    }
}
