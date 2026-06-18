//! The Places sidebar (Phase 4.3) and the shared location enumeration used by
//! both the sidebar and the home page.
//!
//! The sidebar lists quick navigation targets — a Home entry that returns to the
//! start page, the XDG user directories, mounted drives — and a Settings entry
//! at the bottom. Rows drive the application directly via [`AppMsg`].

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::ComponentSender;

use crate::app::{AppModel, AppMsg};
use crate::i18n;

/// The XDG user directories that exist, with localized labels and icons.
#[must_use]
pub fn user_directories() -> Vec<(String, String, PathBuf)> {
    let mut out: Vec<(String, String, PathBuf)> = Vec::new();
    let mut push = |key: &str, icon: &str, dir: Option<PathBuf>, suffix: Option<String>| {
        if let Some(dir) = dir {
            if dir.is_dir() {
                let mut label = i18n::t(key);
                if let Some(s) = suffix {
                    label = format!("{label} · {s}");
                }
                out.push((label, icon.to_owned(), dir));
            }
        }
    };

    let user = std::env::var("USER").ok();
    push("dir.home", "user-home-symbolic", dirs::home_dir(), user);
    push(
        "dir.desktop",
        "user-desktop-symbolic",
        dirs::desktop_dir(),
        None,
    );
    push(
        "dir.downloads",
        "folder-download-symbolic",
        dirs::download_dir(),
        None,
    );
    push(
        "dir.documents",
        "folder-documents-symbolic",
        dirs::document_dir(),
        None,
    );
    push(
        "dir.pictures",
        "folder-pictures-symbolic",
        dirs::picture_dir(),
        None,
    );
    push(
        "dir.videos",
        "folder-videos-symbolic",
        dirs::video_dir(),
        None,
    );
    push(
        "dir.music",
        "folder-music-symbolic",
        dirs::audio_dir(),
        None,
    );
    let screenshots = dirs::picture_dir().map(|p| p.join("Screenshots"));
    push(
        "dir.screenshots",
        "camera-photo-symbolic",
        screenshots,
        None,
    );
    out
}

/// Mount points worth showing: the root filesystem first, then every
/// user-visible mount reported by the volume monitor, de-duplicated by path.
#[must_use]
pub fn mounted_drives() -> Vec<(String, PathBuf)> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out: Vec<(String, PathBuf)> = Vec::new();

    let root = PathBuf::from("/");
    out.push((i18n::t("drive.system"), root.clone()));
    seen.push(root);

    let monitor = gtk::gio::VolumeMonitor::get();
    for mount in monitor.mounts() {
        let Some(path) = mount.default_location().path() else {
            continue;
        };
        if seen.contains(&path) {
            continue;
        }
        out.push((mount.name().to_string(), path.clone()));
        seen.push(path);
    }
    out
}

/// Build the Places sidebar section (XDG dirs, drives, bookmarks, network) and fill it.
/// Width and CSS class are set by the caller's outer sidebar container.
#[must_use]
pub fn build(sender: &ComponentSender<AppModel>, bookmarks: &[PathBuf]) -> gtk::Box {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    populate(&panel, sender, bookmarks);
    panel
}

/// Clear and (re)fill the Places sidebar — used on startup and whenever the
/// bookmark set changes.
pub fn populate(panel: &gtk::Box, sender: &ComponentSender<AppModel>, bookmarks: &[PathBuf]) {
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }

    panel.append(&action_row(
        "go-home-symbolic",
        &i18n::t("sidebar.home"),
        sender,
        AppMsg::ShowHome,
    ));

    panel.append(&section_label(&i18n::t("home.user_dirs")));
    for (label, icon, path) in user_directories() {
        panel.append(&nav_row(&icon, &label, path, sender));
    }

    if !bookmarks.is_empty() {
        panel.append(&section_label(&i18n::t("home.bookmarks")));
        for path in bookmarks {
            panel.append(&bookmark_row(path, sender));
        }
    }

    panel.append(&section_label(&i18n::t("home.drives")));
    for (name, path) in mounted_drives() {
        panel.append(&nav_row("drive-harddisk-symbolic", &name, path, sender));
    }

    // Network locations section: button that opens the network browser dialog.
    panel.append(&section_label(&i18n::t("sidebar.network")));
    panel.append(&action_row(
        "network-server-symbolic",
        &i18n::t("net.connections"),
        sender,
        AppMsg::OpenNetworkBrowser,
    ));

    panel.append(&gtk::Box::builder().vexpand(true).build());
    panel.append(&action_row(
        "emblem-system-symbolic",
        &i18n::t("sidebar.settings"),
        sender,
        AppMsg::ShowSettings,
    ));
}

/// A bookmark row: navigates on click, with a trailing remove button.
fn bookmark_row(path: &std::path::Path, sender: &ComponentSender<AppModel>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["orca-bookmark-row"])
        .build();

    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let nav = nav_row(
        "user-bookmarks-symbolic",
        &label,
        path.to_path_buf(),
        sender,
    );
    nav.set_hexpand(true);
    nav.set_tooltip_text(Some(&path.to_string_lossy()));
    row.append(&nav);

    let remove = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .has_frame(false)
        .tooltip_text(i18n::t("home.remove_bookmark"))
        .css_classes(["orca-bookmark-remove"])
        .build();
    let sender = sender.clone();
    let path = path.to_path_buf();
    remove.connect_clicked(move |_| sender.input(AppMsg::RemoveBookmark(path.clone())));
    row.append(&remove);
    row
}

/// A section heading label inside the sidebar.
fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .css_classes(["orca-section-label"])
        .build()
}

/// A sidebar row that navigates to `path`.
fn nav_row(
    icon: &str,
    label: &str,
    path: PathBuf,
    sender: &ComponentSender<AppModel>,
) -> gtk::Button {
    let button = row_button(icon, label);
    let sender = sender.clone();
    button.connect_clicked(move |_| sender.input(AppMsg::NavigateTo(path.clone())));
    button
}

/// A sidebar row that emits a fixed message (Home / Settings).
fn action_row(
    icon: &str,
    label: &str,
    sender: &ComponentSender<AppModel>,
    msg: AppMsg,
) -> gtk::Button {
    let button = row_button(icon, label);
    let sender = sender.clone();
    button.connect_clicked(move |_| sender.input(msg.clone()));
    button
}

/// Build the flat icon+label button shared by all sidebar rows.
fn row_button(icon: &str, label: &str) -> gtk::Button {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(&gtk::Image::builder().icon_name(icon).pixel_size(18).build());
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    gtk::Button::builder()
        .child(&row)
        .has_frame(false)
        .css_classes(["orca-place"])
        .build()
}
