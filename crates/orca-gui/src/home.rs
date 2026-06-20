//! The home / start page.
//!
//! Shown on launch and whenever the user presses the Home action. It greets
//! the user, offers quick actions (new folder/file, connect to a server),
//! quick-access cards for the XDG user directories, and a recently-used list
//! (`orca_core::get_recent`) — all over the configurable window background
//! image. Mounted drives live in the Places sidebar, not here, to avoid
//! showing the same information twice.
//!
//! The page owns no configuration: navigation and the settings request are
//! emitted as [`HomeOutput`] values for the application to act on.

use std::path::{Path, PathBuf};

use orca_core::RecentEntry;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

use crate::i18n;

/// How many recently-used entries to show.
const RECENT_LIMIT: usize = 8;

/// Messages the home page handles.
#[derive(Debug)]
pub enum HomeInput {
    /// Re-query the recently-used list and rebuild it.
    Refresh,
    /// The async recent-files query finished; rebuild the recent list.
    SetRecent(Vec<RecentEntry>),
}

/// Messages the home page emits to the application.
#[derive(Debug, Clone)]
pub enum HomeOutput {
    /// A card was activated; navigate the active pane here.
    Navigate(PathBuf),
    /// Open the settings page.
    OpenSettings,
    /// "New Folder" quick action.
    NewFolder,
    /// "New File" quick action.
    NewFile,
    /// "Connect to Server" quick action.
    ConnectServer,
}

/// The home page component.
pub struct HomePage {
    /// Container for the recent-items rows, rebuilt on refresh.
    recent_box: gtk::Box,
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

        content.append(&section_label(&i18n::t("home.quick_actions")));
        content.append(&quick_actions_flow(&sender));

        content.append(&section_label(&i18n::t("home.user_dirs")));
        content.append(&user_dir_grid(&sender));

        content.append(&section_label(&i18n::t("home.recent")));
        let recent_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        content.append(&recent_box);

        root.set_child(Some(&content));

        fetch_recent(&sender);

        ComponentParts {
            model: HomePage { recent_box },
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            HomeInput::Refresh => fetch_recent(&sender),
            HomeInput::SetRecent(entries) => {
                while let Some(child) = self.recent_box.first_child() {
                    self.recent_box.remove(&child);
                }
                if entries.is_empty() {
                    let lbl = gtk::Label::builder()
                        .label(i18n::t("home.no_recent"))
                        .halign(gtk::Align::Start)
                        .css_classes(["dim-label"])
                        .build();
                    self.recent_box.append(&lbl);
                } else {
                    for entry in entries {
                        self.recent_box.append(&recent_row(&entry, &sender));
                    }
                }
            }
        }
    }
}

/// Kick off the async recently-used query and feed the result back as a message.
fn fetch_recent(sender: &ComponentSender<HomePage>) {
    let sender = sender.clone();
    relm4::spawn(async move {
        let entries = orca_core::get_recent(RECENT_LIMIT)
            .await
            .unwrap_or_default();
        sender.input(HomeInput::SetRecent(entries));
    });
}

/// Build the header row: personalized greeting + subtitle, refresh and settings buttons.
fn build_header(sender: &ComponentSender<HomePage>) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["orca-home-header"])
        .build();

    let titles = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();

    let greeting = match std::env::var("USER").ok() {
        Some(user) => i18n::tf("home.welcome", &[("user", &user)]),
        None => i18n::t("home.welcome_fallback"),
    };
    titles.append(
        &gtk::Label::builder()
            .label(greeting)
            .halign(gtk::Align::Start)
            .css_classes(["orca-home-title"])
            .build(),
    );
    titles.append(
        &gtk::Label::builder()
            .label(i18n::t("home.subtitle"))
            .halign(gtk::Align::Start)
            .css_classes(["dim-label"])
            .build(),
    );
    header.append(&titles);

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

/// The three fixed quick-action cards: new folder, new file, connect to server.
fn quick_actions_flow(sender: &ComponentSender<HomePage>) -> gtk::FlowBox {
    let flow = card_flow(3);
    flow.append(&quick_action_card(
        "folder-new-symbolic",
        &i18n::t("tb.new_folder"),
        HomeOutput::NewFolder,
        sender,
    ));
    flow.append(&quick_action_card(
        "document-new-symbolic",
        &i18n::t("tb.new_file"),
        HomeOutput::NewFile,
        sender,
    ));
    flow.append(&quick_action_card(
        "network-server-symbolic",
        &i18n::t("home.connect_server"),
        HomeOutput::ConnectServer,
        sender,
    ));
    flow
}

/// A quick-action card: centered icon over centered label.
fn quick_action_card(
    icon: &str,
    label: &str,
    output: HomeOutput,
    sender: &ComponentSender<HomePage>,
) -> gtk::Button {
    let col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    col.append(
        &gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(32)
            .css_classes(["orca-card-icon"])
            .build(),
    );
    col.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Center)
            .build(),
    );

    let button = gtk::Button::builder()
        .child(&col)
        .css_classes(["orca-card"])
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        sender.output(output.clone()).ok();
    });
    button
}

/// Build the responsive grid of XDG user-directory cards.
fn user_dir_grid(sender: &ComponentSender<HomePage>) -> gtk::FlowBox {
    let flow = card_flow(2);
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
fn card_flow(max_per_line: u32) -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(max_per_line)
        .min_children_per_line(1)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .css_classes(["orca-card-grid"])
        .build()
}

/// A recently-used row: icon, name, and parent directory, navigates on click.
fn recent_row(entry: &RecentEntry, sender: &ComponentSender<HomePage>) -> gtk::Button {
    let is_dir = entry.path.is_dir();
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.path.display().to_string());
    let parent = entry
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.append(
        &gtk::Image::builder()
            .icon_name(if is_dir {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            })
            .pixel_size(22)
            .build(),
    );

    let titles = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    titles.append(
        &gtk::Label::builder()
            .label(&name)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    titles.append(
        &gtk::Label::builder()
            .label(&parent)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "orca-card-subtitle"])
            .build(),
    );
    row.append(&titles);

    let button = gtk::Button::builder()
        .child(&row)
        .css_classes(["orca-card"])
        .build();
    let sender = sender.clone();
    // Navigate to the item itself if it's a directory, otherwise to its
    // parent so the file is visible and selectable in the pane.
    let target = if is_dir {
        entry.path.clone()
    } else {
        entry
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(entry.path.clone())
    };
    button.connect_clicked(move |_| {
        sender.output(HomeOutput::Navigate(target.clone())).ok();
    });
    button
}
