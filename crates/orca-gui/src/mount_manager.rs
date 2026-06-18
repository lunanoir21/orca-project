//! Mount manager dialog (Phase 5.5).
//!
//! Lists all UDisks2-managed block devices visible via [`orca_core::list_mounts`]
//! and lets the user mount, unmount, or eject them. The list auto-refreshes
//! when [`orca_core::mount_events`] fires (plug/unplug).

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, Sender};

use orca_core::MountedVolume;

use crate::format;
use crate::i18n;

/// Mount manager model.
pub struct MountManager {
    volumes: Vec<VolumeRow>,
    list_box: gtk::ListBox,
}

/// One row in the list: the volume data plus widget handles kept alive as long
/// as the row is visible (GTK ref-counts, but the Vec owns the strong refs).
struct VolumeRow {
    volume: MountedVolume,
    // These widget handles are stored solely to keep the GTK objects alive;
    // they are never individually read after construction.
    #[allow(dead_code)]
    row: gtk::ListBoxRow,
    #[allow(dead_code)]
    mount_btn: gtk::Button,
    #[allow(dead_code)]
    unmount_btn: gtk::Button,
    #[allow(dead_code)]
    eject_btn: gtk::Button,
}

/// Input messages.
#[derive(Debug)]
pub enum MountManagerInput {
    /// Re-query the mount list from UDisks2.
    Refresh,
    /// User pressed Mount on the device at the given list index.
    Mount(usize),
    /// User pressed Unmount on the volume at the given list index.
    Unmount(usize),
    /// User pressed Eject on the volume at the given list index.
    Eject(usize),
}

/// Background command results.
#[derive(Debug)]
pub enum MountManagerCmd {
    Listed(Result<Vec<MountedVolume>, String>),
    OpDone(Result<(), String>),
}

#[relm4::component(pub)]
impl Component for MountManager {
    type Init = ();
    type Input = MountManagerInput;
    type Output = ();
    type CommandOutput = MountManagerCmd;

    view! {
        #[root]
        gtk::Window {
            set_title: Some(&i18n::t("mnt.title")),
            set_modal: true,
            set_default_width: 620,
            set_default_height: 420,
        }
    }

    fn init(
        _: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
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

        // Bottom row: Refresh + Close
        let btn_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        let refresh_btn = gtk::Button::with_label(&i18n::t("mnt.refresh"));
        {
            let s = sender.clone();
            refresh_btn.connect_clicked(move |_| s.input(MountManagerInput::Refresh));
        }
        let close_btn = gtk::Button::with_label(&i18n::t("common.close"));
        close_btn.set_hexpand(true);
        close_btn.set_halign(gtk::Align::End);
        {
            let root = root.clone();
            close_btn.connect_clicked(move |_| root.close());
        }
        btn_row.append(&refresh_btn);
        btn_row.append(&close_btn);
        outer.append(&btn_row);

        root.set_child(Some(&outer));

        let model = MountManager {
            volumes: Vec::new(),
            list_box,
        };

        sender.input(MountManagerInput::Refresh);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            MountManagerInput::Refresh => {
                sender.oneshot_command(async move {
                    MountManagerCmd::Listed(
                        orca_core::list_mounts()
                            .await
                            .map_err(|e| e.to_string()),
                    )
                });
            }
            MountManagerInput::Mount(idx) => {
                if let Some(row) = self.volumes.get(idx) {
                    let device = row.volume.device.clone();
                    sender.oneshot_command(async move {
                        MountManagerCmd::OpDone(
                            orca_core::mount(&device)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
            }
            MountManagerInput::Unmount(idx) => {
                if let Some(row) = self.volumes.get(idx) {
                    let mp = row.volume.mount_point.clone();
                    sender.oneshot_command(async move {
                        MountManagerCmd::OpDone(
                            orca_core::unmount(&mp)
                                .await
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
            }
            MountManagerInput::Eject(idx) => {
                if let Some(row) = self.volumes.get(idx) {
                    let device = row.volume.device.clone();
                    sender.oneshot_command(async move {
                        MountManagerCmd::OpDone(
                            orca_core::eject(&device)
                                .await
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            MountManagerCmd::Listed(Ok(volumes)) => {
                // Rebuild the list box.
                while let Some(child) = self.list_box.first_child() {
                    self.list_box.remove(&child);
                }
                self.volumes.clear();

                for (idx, vol) in volumes.into_iter().enumerate() {
                    let (row, mount_btn, unmount_btn, eject_btn) =
                        build_row(&vol, idx, &sender);
                    self.list_box.append(&row);
                    self.volumes.push(VolumeRow {
                        volume: vol,
                        row,
                        mount_btn,
                        unmount_btn,
                        eject_btn,
                    });
                }

                if self.volumes.is_empty() {
                    let placeholder = gtk::Label::builder()
                        .label(i18n::t("mnt.empty"))
                        .margin_top(24)
                        .margin_bottom(24)
                        .build();
                    placeholder.add_css_class("dim-label");
                    let row = gtk::ListBoxRow::new();
                    row.set_child(Some(&placeholder));
                    self.list_box.append(&row);
                }
            }
            MountManagerCmd::Listed(Err(e)) => {
                tracing::warn!(error = %e, "mount list failed");
            }
            MountManagerCmd::OpDone(Ok(())) => {
                sender.input(MountManagerInput::Refresh);
            }
            MountManagerCmd::OpDone(Err(e)) => {
                tracing::warn!(error = %e, "mount operation failed");
                sender.input(MountManagerInput::Refresh);
            }
        }
    }
}

/// Build one row for a volume. Returns the row widget plus the three action
/// buttons so the model can keep them alive.
fn build_row(
    vol: &MountedVolume,
    idx: usize,
    sender: &ComponentSender<MountManager>,
) -> (gtk::ListBoxRow, gtk::Button, gtk::Button, gtk::Button) {
    let row = gtk::ListBoxRow::new();
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .build();

    // Top line: device + fs_type + mount point
    let info_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let dev_lbl = gtk::Label::builder()
        .label(vol.device.to_string_lossy().as_ref())
        .halign(gtk::Align::Start)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Start)
        .build();
    dev_lbl.add_css_class("monospace");

    let fs_lbl = gtk::Label::builder()
        .label(&vol.fs_type)
        .halign(gtk::Align::End)
        .build();
    fs_lbl.add_css_class("dim-label");

    info_row.append(&dev_lbl);
    info_row.append(&fs_lbl);
    outer.append(&info_row);

    // Mount point
    let mp_str = vol.mount_point.to_string_lossy().into_owned();
    let mounted = !mp_str.is_empty() && mp_str != "/";

    let mp_lbl = gtk::Label::builder()
        .label(&mp_str)
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    mp_lbl.add_css_class("dim-label");
    outer.append(&mp_lbl);

    // Usage bar (only when mounted)
    if mounted {
        if let Ok(usage) = orca_core::filesystem_usage(&vol.mount_point) {
            let bar_row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .build();
            let bar = gtk::ProgressBar::new();
            bar.set_hexpand(true);
            let fraction = if usage.total > 0 {
                usage.used as f64 / usage.total as f64
            } else {
                0.0
            };
            bar.set_fraction(fraction.clamp(0.0, 1.0));
            let usage_lbl = gtk::Label::builder()
                .label(
                    i18n::tf(
                        "drive.usage",
                        &[
                            ("free", &format::size(usage.free)),
                            ("total", &format::size(usage.total)),
                        ],
                    )
                    .as_str(),
                )
                .halign(gtk::Align::End)
                .build();
            usage_lbl.add_css_class("dim-label");
            bar_row.append(&bar);
            bar_row.append(&usage_lbl);
            outer.append(&bar_row);
        }
    }

    // Action buttons
    let btn_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .build();

    let mount_btn = gtk::Button::with_label(&i18n::t("mnt.mount"));
    mount_btn.set_sensitive(!mounted);
    {
        let s = sender.clone();
        mount_btn.connect_clicked(move |_| s.input(MountManagerInput::Mount(idx)));
    }

    let unmount_btn = gtk::Button::with_label(&i18n::t("mnt.unmount"));
    unmount_btn.set_sensitive(mounted);
    {
        let s = sender.clone();
        unmount_btn.connect_clicked(move |_| s.input(MountManagerInput::Unmount(idx)));
    }

    let eject_btn = gtk::Button::with_label(&i18n::t("mnt.eject"));
    {
        let s = sender.clone();
        eject_btn.connect_clicked(move |_| s.input(MountManagerInput::Eject(idx)));
    }

    btn_row.append(&mount_btn);
    btn_row.append(&unmount_btn);
    btn_row.append(&eject_btn);
    outer.append(&btn_row);

    row.set_child(Some(&outer));

    (row, mount_btn, unmount_btn, eject_btn)
}

/// Wire up plug/unplug auto-refresh: subscribes to `orca_core::mount_events`
/// and sends `MountManagerInput::Refresh` whenever the device list changes.
///
/// This should be called once after the component is launched. The returned
/// future must be dropped (or it cancels) when the dialog closes.
pub async fn start_event_listener(sender: Sender<MountManagerInput>) {
    match orca_core::mount_events().await {
        Ok(mut rx) => {
            while rx.recv().await.is_some() {
                sender.emit(MountManagerInput::Refresh);
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "mount event stream failed");
        }
    }
}
