//! The home / start page.
//!
//! Shown on launch and whenever the user presses the Home action. It presents
//! quick-access cards for the XDG user directories and for mounted drives (with
//! a capacity bar), over the configurable window background image. The header
//! carries a refresh button and a settings button that opens the settings page.
//!
//! The page owns no configuration: navigation and the settings request are
//! emitted as [`HomeOutput`] values for the application to act on.

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

use orca_core::filesystem_usage;

use crate::format;
use crate::i18n;

/// Messages the home page handles.
#[derive(Debug)]
pub enum HomeInput {
    /// Re-query drive capacities and rebuild the drive cards.
    Refresh,
}

/// Messages the home page emits to the application.
#[derive(Debug)]
pub enum HomeOutput {
    /// A card was activated; navigate the active pane here.
    Navigate(PathBuf),
    /// Open the settings page.
    OpenSettings,
}

/// The home page component.
pub struct HomePage {
    /// Container for the drive cards, rebuilt on [`HomeInput::Refresh`].
    drives: gtk::FlowBox,
}

impl SimpleComponent for HomePage {
    type Init = ();
    type Input = HomeInput;
    type Output = HomeOutput;
    type Root = gtk::ScrolledWindow;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .css_classes(["orca-home-scroll"])
            .build()
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .css_classes(["orca-home"])
            .build();

        content.append(&build_header(&sender));

        content.append(&section_label(&i18n::t("home.user_dirs")));
        content.append(&user_dir_grid(&sender));

        content.append(&section_label(&i18n::t("home.drives")));
        let drives = drive_flow();
        populate_drives(&drives, &sender);
        content.append(&drives);

        root.set_child(Some(&content));

        ComponentParts {
            model: HomePage { drives },
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            HomeInput::Refresh => {
                while let Some(child) = self.drives.first_child() {
                    self.drives.remove(&child);
                }
                populate_drives(&self.drives, &sender);
            }
        }
    }
}

/// Build the header row: title, refresh button and the settings button.
fn build_header(sender: &ComponentSender<HomePage>) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["orca-home-header"])
        .build();

    header.append(
        &gtk::Label::builder()
            .label(i18n::t("home.title"))
            .halign(gtk::Align::Start)
            .hexpand(true)
            .css_classes(["orca-home-title"])
            .build(),
    );

    let refresh = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(i18n::t("home.refresh"))
        .has_frame(false)
        .build();
    {
        let sender = sender.clone();
        refresh.connect_clicked(move |_| sender.input(HomeInput::Refresh));
    }
    header.append(&refresh);

    let settings = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text(i18n::t("sidebar.settings"))
        .has_frame(false)
        .build();
    {
        let sender = sender.clone();
        settings.connect_clicked(move |_| {
            sender.output(HomeOutput::OpenSettings).ok();
        });
    }
    header.append(&settings);
    header
}

/// A section heading label (e.g. "USER DIRECTORIES").
fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .css_classes(["orca-section-label"])
        .build()
}

/// Build the responsive grid of XDG user-directory cards.
fn user_dir_grid(sender: &ComponentSender<HomePage>) -> gtk::FlowBox {
    let flow = card_flow();
    for (label, icon, path) in crate::places::user_directories() {
        flow.append(&dir_card(&label, &icon, path, sender));
    }
    flow
}

/// A clickable user-directory card.
fn dir_card(
    label: &str,
    icon: &str,
    path: PathBuf,
    sender: &ComponentSender<HomePage>,
) -> gtk::Button {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();

    row.append(
        &gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(28)
            .css_classes(["orca-card-icon"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );

    let button = gtk::Button::builder()
        .child(&row)
        .css_classes(["orca-card"])
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        sender.output(HomeOutput::Navigate(path.clone())).ok();
    });
    button
}

/// A `FlowBox` configured for cards (no selection, wrapping rows).
fn card_flow() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .css_classes(["orca-card-grid"])
        .build()
}

/// A `FlowBox` for the wider drive cards.
fn drive_flow() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .css_classes(["orca-card-grid"])
        .build()
}

/// Fill `flow` with one card per mounted drive (plus the root filesystem).
fn populate_drives(flow: &gtk::FlowBox, sender: &ComponentSender<HomePage>) {
    for (name, path) in crate::places::mounted_drives() {
        flow.append(&drive_card(&name, path, sender));
    }
}

/// A clickable drive card with a capacity bar.
fn drive_card(name: &str, path: PathBuf, sender: &ComponentSender<HomePage>) -> gtk::Button {
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();

    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    top.append(
        &gtk::Image::builder()
            .icon_name("drive-harddisk-symbolic")
            .pixel_size(28)
            .css_classes(["orca-card-icon"])
            .build(),
    );

    let titles = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    titles.append(
        &gtk::Label::builder()
            .label(name)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["orca-card-title"])
            .build(),
    );

    if let Ok(usage) = filesystem_usage(&path) {
        let subtitle = i18n::tf(
            "drive.usage",
            &[
                ("free", &format::size(usage.free)),
                ("total", &format::size(usage.total)),
            ],
        );
        titles.append(
            &gtk::Label::builder()
                .label(subtitle)
                .halign(gtk::Align::Start)
                .css_classes(["dim-label", "orca-card-subtitle"])
                .build(),
        );
        top.append(&titles);
        outer.append(&top);

        let fraction = if usage.total > 0 {
            usage.used as f64 / usage.total as f64
        } else {
            0.0
        };
        outer.append(
            &gtk::ProgressBar::builder()
                .fraction(fraction)
                .css_classes(["orca-drive-bar"])
                .build(),
        );
    } else {
        top.append(&titles);
        outer.append(&top);
    }

    let button = gtk::Button::builder()
        .child(&outer)
        .css_classes(["orca-card", "orca-drive-card"])
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        sender.output(HomeOutput::Navigate(path.clone())).ok();
    });
    button
}
