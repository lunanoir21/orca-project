//! Data-driven toolbar (Phase 3.5).
//!
//! The toolbar is built from an ordered list of [`ToolbarItem`] values rather
//! than hard-coded widgets, so the set and order can be driven from the
//! `[toolbar]` section of the user config. The config *file* is wired in with
//! the config system (Phase 7.1); until then [`default_items`] supplies the
//! shipped layout. [`build`] realises a list into a widget row plus handles to
//! the stateful buttons (Back/Forward sensitivity, view-mode toggles).

use relm4::gtk;
use relm4::gtk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::AppMsg;
use crate::i18n;
use crate::pane::ViewMode;

/// A single configurable toolbar element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolbarItem {
    /// Navigate back in history.
    Back,
    /// Navigate forward in history.
    Forward,
    /// Navigate to the parent directory.
    Up,
    /// Reload the current directory.
    Reload,
    /// Create a new folder.
    NewFolder,
    /// Create a new empty file.
    NewFile,
    /// Switch to list view.
    ViewList,
    /// Switch to icon view.
    ViewIcon,
    /// Switch to detail view.
    ViewDetail,
    /// Toggle the search bar.
    Search,
    /// Toggle the editable path entry.
    EditPath,
    /// Toggle the dual-pane layout.
    DualPane,
    /// Toggle synchronized scrolling between panes.
    SyncScroll,
    /// Toggle the file-preview side panel.
    Preview,
    /// Toggle the embedded terminal panel.
    Terminal,
    /// Open the mount manager dialog.
    MountManager,
    /// A vertical separator.
    Separator,
}

/// The shipped default toolbar layout.
#[must_use]
pub fn default_items() -> Vec<ToolbarItem> {
    use ToolbarItem::{
        Back, DualPane, EditPath, Forward, MountManager, NewFile, NewFolder, Preview, Reload,
        Search, Separator, SyncScroll, Terminal, Up, ViewDetail, ViewIcon, ViewList,
    };
    vec![
        Back,
        Forward,
        Up,
        Separator,
        Reload,
        Separator,
        NewFolder,
        NewFile,
        Separator,
        ViewList,
        ViewIcon,
        ViewDetail,
        Separator,
        DualPane,
        SyncScroll,
        Preview,
        Terminal,
        MountManager,
        Separator,
        Search,
        EditPath,
    ]
}

/// Handles to the toolbar buttons whose state tracks the application model.
#[derive(Debug, Default)]
pub struct ToolbarHandles {
    /// The Back button (sensitivity follows history).
    pub back: Option<gtk::Button>,
    /// The Forward button (sensitivity follows history).
    pub forward: Option<gtk::Button>,
    /// List/icon/detail view-mode toggles.
    pub view_toggles: Vec<(ViewMode, gtk::ToggleButton)>,
    /// The Mount Manager button — anchor point for its popover (it has no
    /// other entry point, so a popover, not a separate window, keeps mount
    /// management inside the single main window).
    pub mount_btn: Option<gtk::Button>,
}

impl ToolbarHandles {
    /// Update Back/Forward sensitivity.
    pub fn set_history(&self, can_back: bool, can_forward: bool) {
        if let Some(b) = &self.back {
            b.set_sensitive(can_back);
        }
        if let Some(f) = &self.forward {
            f.set_sensitive(can_forward);
        }
    }

    /// Reflect the active view mode in the toggle buttons (no signal feedback:
    /// only the user's `clicked` emits a message, not programmatic `set_active`).
    pub fn set_mode(&self, mode: ViewMode) {
        for (m, btn) in &self.view_toggles {
            btn.set_active(*m == mode);
        }
    }
}

/// All real (non-separator) toolbar items, in their shipped default order —
/// the universe of choices the toolbar customization UI offers.
#[must_use]
pub fn all_items() -> &'static [ToolbarItem] {
    use ToolbarItem::{
        Back, DualPane, EditPath, Forward, MountManager, NewFile, NewFolder, Preview, Reload,
        Search, SyncScroll, Terminal, Up, ViewDetail, ViewIcon, ViewList,
    };
    &[
        Back,
        Forward,
        Up,
        Reload,
        NewFolder,
        NewFile,
        ViewList,
        ViewIcon,
        ViewDetail,
        DualPane,
        SyncScroll,
        Preview,
        Terminal,
        MountManager,
        Search,
        EditPath,
    ]
}

/// The i18n key used for this item's label/tooltip everywhere (toolbar
/// buttons and the settings customization list).
#[must_use]
pub fn item_label(item: ToolbarItem) -> String {
    let key = match item {
        ToolbarItem::Back => "tb.back",
        ToolbarItem::Forward => "tb.forward",
        ToolbarItem::Up => "tb.up",
        ToolbarItem::Reload => "tb.reload",
        ToolbarItem::NewFolder => "tb.new_folder",
        ToolbarItem::NewFile => "tb.new_file",
        ToolbarItem::ViewList => "tb.view_list",
        ToolbarItem::ViewIcon => "tb.view_icon",
        ToolbarItem::ViewDetail => "tb.view_detail",
        ToolbarItem::Search => "tb.search",
        ToolbarItem::EditPath => "tb.edit_path",
        ToolbarItem::DualPane => "tb.dual",
        ToolbarItem::SyncScroll => "tb.sync",
        ToolbarItem::Preview => "preview.toggle",
        ToolbarItem::Terminal => "tb.terminal",
        ToolbarItem::MountManager => "tb.mounts",
        ToolbarItem::Separator => "tb.separator",
    };
    i18n::t(key)
}

/// Build a toolbar widget row and its stateful handles from an item list.
#[must_use]
pub fn build(items: &[ToolbarItem], sender: &relm4::Sender<AppMsg>) -> (gtk::Box, ToolbarHandles) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .css_classes(["orca-toolbar"])
        .build();
    let handles = populate(&row, items, sender);
    (row, handles)
}

/// Clear and refill `row` from `items`, returning fresh stateful handles.
/// Used both by [`build`] (fresh row) and by the settings page's toolbar
/// customizer (rebuilding the live row in place).
#[must_use]
pub fn populate(
    row: &gtk::Box,
    items: &[ToolbarItem],
    sender: &relm4::Sender<AppMsg>,
) -> ToolbarHandles {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    let mut handles = ToolbarHandles::default();

    for item in items {
        match item {
            ToolbarItem::Separator => {
                row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
            }
            ToolbarItem::Back => {
                let b = action_button(
                    "go-previous-symbolic",
                    &i18n::t("tb.back"),
                    sender,
                    AppMsg::Back,
                );
                b.set_sensitive(false);
                row.append(&b);
                handles.back = Some(b);
            }
            ToolbarItem::Forward => {
                let b = action_button(
                    "go-next-symbolic",
                    &i18n::t("tb.forward"),
                    sender,
                    AppMsg::Forward,
                );
                b.set_sensitive(false);
                row.append(&b);
                handles.forward = Some(b);
            }
            ToolbarItem::Up => {
                row.append(&action_button(
                    "go-up-symbolic",
                    &i18n::t("tb.up"),
                    sender,
                    AppMsg::Up,
                ));
            }
            ToolbarItem::Reload => {
                row.append(&action_button(
                    "view-refresh-symbolic",
                    &i18n::t("tb.reload"),
                    sender,
                    AppMsg::Reload,
                ));
            }
            ToolbarItem::NewFolder => {
                row.append(&action_button(
                    "folder-new-symbolic",
                    &i18n::t("tb.new_folder"),
                    sender,
                    AppMsg::NewFolder,
                ));
            }
            ToolbarItem::NewFile => {
                row.append(&action_button(
                    "document-new-symbolic",
                    &i18n::t("tb.new_file"),
                    sender,
                    AppMsg::NewFile,
                ));
            }
            ToolbarItem::Search => {
                row.append(&action_button(
                    "system-search-symbolic",
                    &i18n::t("tb.search"),
                    sender,
                    AppMsg::ToggleSearch,
                ));
            }
            ToolbarItem::EditPath => {
                row.append(&action_button(
                    "document-edit-symbolic",
                    &i18n::t("tb.edit_path"),
                    sender,
                    AppMsg::TogglePathEntry,
                ));
            }
            ToolbarItem::DualPane => {
                row.append(&toggle_button(
                    "view-dual-symbolic",
                    &i18n::t("tb.dual"),
                    sender,
                    AppMsg::ToggleDual,
                ));
            }
            ToolbarItem::SyncScroll => {
                row.append(&toggle_button(
                    "emblem-synchronizing-symbolic",
                    &i18n::t("tb.sync"),
                    sender,
                    AppMsg::ToggleSync,
                ));
            }
            ToolbarItem::Preview => {
                row.append(&toggle_button(
                    "view-paged-symbolic",
                    &i18n::t("preview.toggle"),
                    sender,
                    AppMsg::TogglePreview,
                ));
            }
            ToolbarItem::Terminal => {
                row.append(&toggle_button(
                    "utilities-terminal-symbolic",
                    &i18n::t("tb.terminal"),
                    sender,
                    AppMsg::ToggleTerminal,
                ));
            }
            ToolbarItem::MountManager => {
                let b = action_button(
                    "drive-removable-media-symbolic",
                    &i18n::t("tb.mounts"),
                    sender,
                    AppMsg::OpenMountManager,
                );
                row.append(&b);
                handles.mount_btn = Some(b);
            }
            ToolbarItem::ViewList => {
                let b = view_toggle(
                    "view-list-symbolic",
                    &i18n::t("tb.view_list"),
                    sender,
                    ViewMode::List,
                );
                row.append(&b);
                handles.view_toggles.push((ViewMode::List, b));
            }
            ToolbarItem::ViewIcon => {
                let b = view_toggle(
                    "view-grid-symbolic",
                    &i18n::t("tb.view_icon"),
                    sender,
                    ViewMode::Icon,
                );
                row.append(&b);
                handles.view_toggles.push((ViewMode::Icon, b));
            }
            ToolbarItem::ViewDetail => {
                let b = view_toggle(
                    "view-more-symbolic",
                    &i18n::t("tb.view_detail"),
                    sender,
                    ViewMode::Detail,
                );
                row.append(&b);
                handles.view_toggles.push((ViewMode::Detail, b));
            }
        }
    }
    // Mark the default mode active.
    handles.set_mode(ViewMode::List);
    handles
}

/// Build a plain action button that emits a fixed message on click.
fn action_button(
    icon: &str,
    tooltip: &str,
    sender: &relm4::Sender<AppMsg>,
    msg: AppMsg,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        sender.emit(msg.clone());
    });
    button
}

/// Build a stateless toggle button that emits a fixed message on each click
/// (the application flips the corresponding state itself).
fn toggle_button(
    icon: &str,
    tooltip: &str,
    sender: &relm4::Sender<AppMsg>,
    msg: AppMsg,
) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |_| sender.emit(msg.clone()));
    button
}

/// Build a view-mode toggle that requests its mode when the user activates it.
fn view_toggle(
    icon: &str,
    tooltip: &str,
    sender: &relm4::Sender<AppMsg>,
    mode: ViewMode,
) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build();
    let sender = sender.clone();
    button.connect_clicked(move |b| {
        if b.is_active() {
            sender.emit(AppMsg::SetViewMode(mode));
        }
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_core_items() {
        let items = default_items();
        assert!(items.contains(&ToolbarItem::Back));
        assert!(items.contains(&ToolbarItem::NewFolder));
        assert!(items.contains(&ToolbarItem::Search));
    }

    #[test]
    fn toolbar_item_roundtrips_kebab_case() {
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            items: Vec<ToolbarItem>,
        }
        let toml = toml::to_string(&Wrap {
            items: vec![ToolbarItem::NewFolder, ToolbarItem::EditPath],
        })
        .unwrap();
        assert!(toml.contains("new-folder"), "got: {toml}");
        assert!(toml.contains("edit-path"), "got: {toml}");

        let parsed: Wrap = toml::from_str("items = [\"back\", \"view-icon\"]\n").unwrap();
        assert_eq!(parsed.items, vec![ToolbarItem::Back, ToolbarItem::ViewIcon]);
    }
}
