//! The row item and column definitions for the file browser's
//! [`relm4::typed_view::column::TypedColumnView`].
//!
//! Each [`FileItem`] wraps one [`orca_core::FileEntry`] plus the wiring needed
//! for inline rename: a clone of the pane's input [`relm4::Sender`] and a shared
//! registry of the per-row [`gtk::EditableLabel`] widgets (so the pane can start
//! editing the selected row in response to F2).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::gio;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::typed_view::column::{LabelColumn, RelmColumn};
use relm4::typed_view::grid::RelmGridItem;

use orca_core::{FileEntry, FileKind, GitStatus};

use super::PaneInput;
use crate::format;

/// Thumbnail edge length (px) for the name-column icon (list/detail views).
const NAME_THUMBNAIL_SIZE: i32 = 24;
/// Thumbnail edge length (px) for an icon-view grid cell.
const GRID_THUMBNAIL_SIZE: i32 = 48;

/// Shared map from entry path to its currently-bound name-cell editable label.
///
/// Cells are recycled by `GtkColumnView`, so this always reflects the rows that
/// are presently realised. Used to trigger inline rename on the selected row.
pub type RenameRegistry = Rc<RefCell<HashMap<PathBuf, glib::WeakRef<gtk::EditableLabel>>>>;

/// Shared list of currently-selected paths, used to build drag payloads
/// without querying the selection model from a cell callback.
pub type DragSelection = Rc<RefCell<Vec<PathBuf>>>;

/// One row in the file browser.
#[derive(Clone)]
pub struct FileItem {
    /// The underlying filesystem entry.
    pub entry: FileEntry,
    /// Pane input channel, used to report a committed inline rename.
    sender: relm4::Sender<PaneInput>,
    /// Registry the name cell registers its editable label into on bind.
    registry: RenameRegistry,
    /// Shared selection snapshot for drag payloads.
    pub drag_selection: DragSelection,
    /// Git working-tree status, populated asynchronously after listing.
    pub git_status: Option<GitStatus>,
    /// Plugin badge `(text, color)` set by the plugin system.
    pub plugin_badge: Option<(String, String)>,
}

impl FileItem {
    /// Build a row item bound to the given pane sender and rename registry.
    #[must_use]
    pub fn new(
        entry: FileEntry,
        sender: relm4::Sender<PaneInput>,
        registry: RenameRegistry,
        drag_selection: DragSelection,
    ) -> Self {
        Self {
            entry,
            sender,
            registry,
            drag_selection,
            git_status: None,
            plugin_badge: None,
        }
    }
}

/// Per-cell state for the name column, updated on every `bind` so the
/// editing-finished closure (connected once in `setup`) acts on the row the
/// cell currently displays rather than the one it was created for.
struct NameCellState {
    path: PathBuf,
    original: String,
    sender: Option<relm4::Sender<PaneInput>>,
    /// Shared selection snapshot; set in `bind` for DragSource payloads.
    drag_sel: Option<DragSelection>,
}

/// Widgets owned by a name cell: the type icon, the editable name label, and
/// the git-status badge and plugin badge (hidden when not set).
pub struct NameWidgets {
    icon: gtk::Image,
    label: gtk::EditableLabel,
    git_badge: gtk::Label,
    plugin_badge_lbl: gtk::Label,
    state: Rc<RefCell<NameCellState>>,
}

/// The name column: a type icon plus an inline-editable label.
pub struct NameColumn;

impl RelmColumn for NameColumn {
    type Root = gtk::Box;
    type Widgets = NameWidgets;
    type Item = FileItem;

    const COLUMN_NAME: &'static str = "Name";
    const ENABLE_RESIZE: bool = true;
    const ENABLE_EXPAND: bool = true;

    fn setup(_item: &gtk::ListItem) -> (Self::Root, Self::Widgets) {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
        let label = gtk::EditableLabel::new("");
        label.set_hexpand(true);
        label.set_halign(gtk::Align::Start);
        let git_badge = gtk::Label::builder()
            .halign(gtk::Align::End)
            .visible(false)
            .build();
        git_badge.add_css_class("git-badge");
        let plugin_badge_lbl = gtk::Label::builder()
            .halign(gtk::Align::End)
            .visible(false)
            .build();
        plugin_badge_lbl.add_css_class("plugin-badge");
        root.append(&icon);
        root.append(&label);
        root.append(&git_badge);
        root.append(&plugin_badge_lbl);

        let state = Rc::new(RefCell::new(NameCellState {
            path: PathBuf::new(),
            original: String::new(),
            sender: None,
            drag_sel: None,
        }));

        // Detect the end of an editing session: when `editing` flips back to
        // false, compare the text and emit a rename if it changed.
        let st = state.clone();
        let lbl = label.clone();
        label.connect_editing_notify(move |l| {
            if l.is_editing() {
                return;
            }
            let new_name = lbl.text().to_string();
            let cell = st.borrow();
            if new_name.is_empty() || new_name == cell.original {
                return;
            }
            if let Some(sender) = &cell.sender {
                sender.emit(PaneInput::RenameCommitted {
                    path: cell.path.clone(),
                    new_name,
                });
            }
        });

        // Right-click selects this row (capture phase, so the view's own
        // context-menu gesture still pops the menu afterwards) and makes it the
        // operative target for single-file actions like checksum.
        let st_ctx = state.clone();
        let ctx = gtk::GestureClick::new();
        ctx.set_button(gtk::gdk::BUTTON_SECONDARY);
        ctx.set_propagation_phase(gtk::PropagationPhase::Capture);
        ctx.connect_pressed(move |_, _, _, _| {
            let cell = st_ctx.borrow();
            if let Some(sender) = &cell.sender {
                sender.emit(PaneInput::ContextTarget(cell.path.clone()));
            }
        });
        root.add_controller(ctx);

        // DragSource: build a gdk::FileList from the current selection when a
        // drag begins. If the cell's own path is in the selection, drag all
        // selected files; otherwise drag just this cell (single-item drag from
        // an unselected row).
        let st_drag = state.clone();
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
        drag.connect_prepare(move |src, _, _| {
            let cell = st_drag.borrow();
            let paths: Vec<PathBuf> = if let Some(sel) = &cell.drag_sel {
                let sel = sel.borrow();
                if sel.contains(&cell.path) {
                    sel.clone()
                } else {
                    vec![cell.path.clone()]
                }
            } else {
                vec![cell.path.clone()]
            };
            if paths.is_empty() {
                return None;
            }
            let gfiles: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
            let file_list = gdk::FileList::from_array(&gfiles);
            src.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
            Some(gdk::ContentProvider::for_value(&file_list.to_value()))
        });
        root.add_controller(drag);

        (
            root,
            NameWidgets {
                icon,
                label,
                git_badge,
                plugin_badge_lbl,
                state,
            },
        )
    }

    fn bind(item: &mut Self::Item, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        widgets
            .icon
            .set_icon_name(Some(format::icon_name(item.entry.kind)));
        widgets.label.set_text(&item.entry.name);
        {
            let mut state = widgets.state.borrow_mut();
            state.path = item.entry.path.clone();
            state.original = item.entry.name.clone();
            state.sender = Some(item.sender.clone());
            state.drag_sel = Some(item.drag_selection.clone());
        }
        // Replace the type icon with an image thumbnail when applicable. The
        // guard re-reads the cell's current path so a recycled cell is not
        // overwritten by a late-arriving decode.
        let st = widgets.state.clone();
        let path = item.entry.path.clone();
        crate::thumbnail::apply(
            &widgets.icon,
            &item.entry.path,
            crate::thumbnail::mtime_secs(item.entry.modified),
            NAME_THUMBNAIL_SIZE,
            item.entry.size,
            move || st.borrow().path == path,
        );
        item.registry
            .borrow_mut()
            .insert(item.entry.path.clone(), widgets.label.downgrade());

        // Git status badge: show only for notable statuses.
        apply_git_badge(&widgets.git_badge, item.git_status);
        apply_plugin_badge(&widgets.plugin_badge_lbl, item.plugin_badge.as_ref());
    }

    fn unbind(item: &mut Self::Item, _widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        item.registry.borrow_mut().remove(&item.entry.path);
    }
}

/// The size column. Directories are shown blank (their inode size is not
/// meaningful to users; recursive size is a separate feature).
pub struct SizeColumn;

impl LabelColumn for SizeColumn {
    type Item = FileItem;
    type Value = u64;

    const COLUMN_NAME: &'static str = "Size";
    const ENABLE_SORT: bool = false;
    const ENABLE_RESIZE: bool = true;

    fn get_cell_value(item: &Self::Item) -> Self::Value {
        item.entry.size
    }

    fn format_cell_value(value: &Self::Value) -> String {
        format::size(*value)
    }
}

/// The modification-time column.
pub struct ModifiedColumn;

impl LabelColumn for ModifiedColumn {
    type Item = FileItem;
    type Value = String;

    const COLUMN_NAME: &'static str = "Modified";
    const ENABLE_SORT: bool = false;
    const ENABLE_RESIZE: bool = true;

    fn get_cell_value(item: &Self::Item) -> Self::Value {
        format::modified(item.entry.modified)
    }

    fn format_cell_value(value: &Self::Value) -> String {
        value.clone()
    }
}

/// The entry-kind column.
pub struct KindColumn;

impl LabelColumn for KindColumn {
    type Item = FileItem;
    type Value = String;

    const COLUMN_NAME: &'static str = "Kind";
    const ENABLE_SORT: bool = false;
    const ENABLE_RESIZE: bool = true;

    fn get_cell_value(item: &Self::Item) -> Self::Value {
        kind_label(item.entry.kind, item.entry.is_symlink).to_owned()
    }

    fn format_cell_value(value: &Self::Value) -> String {
        value.clone()
    }
}

/// The permission column (symbolic `rwx` form).
pub struct PermissionsColumn;

impl LabelColumn for PermissionsColumn {
    type Item = FileItem;
    type Value = String;

    const COLUMN_NAME: &'static str = "Permissions";
    const ENABLE_SORT: bool = false;
    const ENABLE_RESIZE: bool = true;

    fn get_cell_value(item: &Self::Item) -> Self::Value {
        format::permissions(item.entry.permissions)
    }

    fn format_cell_value(value: &Self::Value) -> String {
        value.clone()
    }
}

/// Per-cell context target for a grid cell, updated on every `bind`.
struct GridCellState {
    path: PathBuf,
    sender: Option<relm4::Sender<PaneInput>>,
    /// Shared selection snapshot for DragSource payloads.
    drag_sel: Option<DragSelection>,
}

/// Widgets owned by an icon-view grid cell.
pub struct GridWidgets {
    icon: gtk::Image,
    label: gtk::Label,
    git_badge: gtk::Label,
    plugin_badge_lbl: gtk::Label,
    state: Rc<RefCell<GridCellState>>,
}

impl RelmGridItem for FileItem {
    type Root = gtk::Box;
    type Widgets = GridWidgets;

    fn setup(_item: &gtk::ListItem) -> (Self::Root, Self::Widgets) {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .width_request(96)
            .build();
        let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
        icon.set_pixel_size(48);
        let label = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(14)
            .justify(gtk::Justification::Center)
            .build();
        let git_badge = gtk::Label::builder()
            .halign(gtk::Align::End)
            .visible(false)
            .build();
        git_badge.add_css_class("git-badge");
        let plugin_badge_lbl = gtk::Label::builder()
            .halign(gtk::Align::End)
            .visible(false)
            .build();
        plugin_badge_lbl.add_css_class("plugin-badge");
        root.append(&icon);
        root.append(&label);
        root.append(&git_badge);
        root.append(&plugin_badge_lbl);

        let state = Rc::new(RefCell::new(GridCellState {
            path: PathBuf::new(),
            sender: None,
            drag_sel: None,
        }));
        // Right-click selects this cell (see the column-view note above).
        let st_ctx = state.clone();
        let ctx = gtk::GestureClick::new();
        ctx.set_button(gtk::gdk::BUTTON_SECONDARY);
        ctx.set_propagation_phase(gtk::PropagationPhase::Capture);
        ctx.connect_pressed(move |_, _, _, _| {
            let cell = st_ctx.borrow();
            if let Some(sender) = &cell.sender {
                sender.emit(PaneInput::ContextTarget(cell.path.clone()));
            }
        });
        root.add_controller(ctx);

        // DragSource for the icon-view grid cell (same logic as NameColumn).
        let st_drag = state.clone();
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
        drag.connect_prepare(move |src, _, _| {
            let cell = st_drag.borrow();
            let paths: Vec<PathBuf> = if let Some(sel) = &cell.drag_sel {
                let sel = sel.borrow();
                if sel.contains(&cell.path) {
                    sel.clone()
                } else {
                    vec![cell.path.clone()]
                }
            } else {
                vec![cell.path.clone()]
            };
            if paths.is_empty() {
                return None;
            }
            let gfiles: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
            let file_list = gdk::FileList::from_array(&gfiles);
            src.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
            Some(gdk::ContentProvider::for_value(&file_list.to_value()))
        });
        root.add_controller(drag);

        (
            root,
            GridWidgets {
                icon,
                label,
                git_badge,
                plugin_badge_lbl,
                state,
            },
        )
    }

    fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        widgets
            .icon
            .set_icon_name(Some(format::icon_name(self.entry.kind)));
        widgets.icon.set_pixel_size(GRID_THUMBNAIL_SIZE);
        widgets.label.set_text(&self.entry.name);
        {
            let mut state = widgets.state.borrow_mut();
            state.path = self.entry.path.clone();
            state.sender = Some(self.sender.clone());
            state.drag_sel = Some(self.drag_selection.clone());
        }
        let st = widgets.state.clone();
        let path = self.entry.path.clone();
        crate::thumbnail::apply(
            &widgets.icon,
            &self.entry.path,
            crate::thumbnail::mtime_secs(self.entry.modified),
            GRID_THUMBNAIL_SIZE,
            self.entry.size,
            move || st.borrow().path == path,
        );
        apply_git_badge(&widgets.git_badge, self.git_status);
        apply_plugin_badge(&widgets.plugin_badge_lbl, self.plugin_badge.as_ref());
    }
}

/// Apply a git-status badge to a label widget.
///
/// The label is shown with a short letter and color class for notable statuses,
/// and hidden for `Unmodified`, `Ignored`, and `None`.
fn apply_git_badge(badge: &gtk::Label, status: Option<GitStatus>) {
    for cls in &[
        "git-modified",
        "git-added",
        "git-deleted",
        "git-renamed",
        "git-untracked",
        "git-conflicted",
    ] {
        badge.remove_css_class(cls);
    }
    let (text, css) = match status {
        Some(GitStatus::Modified) => ("M", "git-modified"),
        Some(GitStatus::Added) => ("A", "git-added"),
        Some(GitStatus::Deleted) => ("D", "git-deleted"),
        Some(GitStatus::Renamed) => ("R", "git-renamed"),
        Some(GitStatus::Untracked) => ("?", "git-untracked"),
        Some(GitStatus::Conflicted) => ("!", "git-conflicted"),
        _ => {
            badge.set_visible(false);
            return;
        }
    };
    badge.set_label(text);
    badge.add_css_class(css);
    badge.set_visible(true);
}

/// Apply a plugin-provided badge to a label widget. `badge` is `(text, color)`.
///
/// The color is set as an inline CSS `color` property on the label.
fn apply_plugin_badge(badge_lbl: &gtk::Label, badge: Option<&(String, String)>) {
    match badge {
        Some((text, color)) => {
            badge_lbl.set_label(text);
            badge_lbl.set_markup(&format!(
                r#"<span foreground="{}">{}</span>"#,
                glib::markup_escape_text(color),
                glib::markup_escape_text(text),
            ));
            badge_lbl.set_visible(true);
        }
        None => {
            badge_lbl.set_visible(false);
        }
    }
}

/// The kind label, distinguishing a symlink from its underlying classification.
fn kind_label(kind: FileKind, is_symlink: bool) -> &'static str {
    if is_symlink {
        "Link"
    } else {
        format::kind(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_label_prefers_symlink() {
        assert_eq!(kind_label(FileKind::File, true), "Link");
        assert_eq!(kind_label(FileKind::Directory, false), "Folder");
    }
}
