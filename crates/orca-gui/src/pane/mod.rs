//! Single-pane file browser component (Phases 3.2–3.3).
//!
//! [`FilePane`] lists one directory and offers three view modes (list, icon,
//! detail) over a `GtkStack`: list/detail share a `GtkColumnView` (detail simply
//! reveals the extra metadata columns) and icon uses a `GtkGridView`, both via
//! relm4's typed views. It supports multi-select with rubber-band, single/double
//! click activation, keyboard navigation (arrows native, Backspace to parent,
//! Enter to activate, Ctrl+1/2/3 view modes, Ctrl+H hidden toggle) and inline
//! rename (F2). Listing runs off the UI thread against [`orca_core::list_dir`].

mod item;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::gio;
use relm4::gtk::prelude::*;
use relm4::typed_view::column::{LabelColumn, TypedColumnView};
use relm4::typed_view::grid::TypedGridView;
use relm4::typed_view::TypedListItem;
use relm4::{Component, ComponentParts, ComponentSender};

use std::collections::HashMap as StdHashMap;

use orca_core::{
    search_by_content, search_by_name, ArchiveFormat, CancelToken, FileEntry, FilterOptions,
    GitStatus, SearchOptions, SortKey,
};

use self::item::{
    DragSelection, FileItem, KindColumn, ModifiedColumn, NameColumn, PermissionsColumn,
    RenameRegistry, SizeColumn,
};
use crate::dialogs;
use crate::i18n;
use crate::properties::Algo;

/// How many streamed search matches to accumulate before pushing them into the
/// view as one batch (bounds per-message overhead without feeling un-live).
const SEARCH_BATCH: usize = 64;

/// How directory entries are presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Column view showing name, size and modified time.
    #[default]
    List,
    /// Grid of large icons with names.
    Icon,
    /// Column view showing all metadata columns.
    Detail,
}

/// Initial configuration for a pane.
#[derive(Debug, Clone)]
pub struct PaneInit {
    /// Directory the pane opens on.
    pub dir: PathBuf,
    /// Whether dot-files are shown.
    pub show_hidden: bool,
    /// Activate (open) entries on a single click rather than a double click.
    pub single_click: bool,
    /// Confirm before moving files to the trash.
    pub confirm_delete: bool,
    /// Initial sort key.
    pub sort: SortKey,
    /// Initial view mode.
    pub mode: ViewMode,
}

/// Messages the pane handles.
#[derive(Debug)]
pub enum PaneInput {
    /// Change directory and reload (records history).
    Navigate(PathBuf),
    /// Re-read the current directory.
    Reload,
    /// Navigate to the parent directory (Backspace / Up).
    GoUp,
    /// Go to the previous directory in history.
    Back,
    /// Go to the next directory in history.
    Forward,
    /// A row at the given view position was activated (open).
    Activate(u32),
    /// Toggle hidden-file visibility and reload.
    ToggleHidden,
    /// Set hidden-file visibility to an explicit value and reload (settings).
    SetHidden(bool),
    /// Set single-click activation on both views (settings).
    SetSingleClick(bool),
    /// Set whether trashing files asks for confirmation (settings).
    SetConfirmDelete(bool),
    /// Change the sort key and reload. Driven by the view-mode/sort controls
    /// added in later phases; no in-crate caller exists yet.
    #[allow(dead_code)]
    SetSort(SortKey),
    /// Switch view mode (remembered per directory).
    SetViewMode(ViewMode),
    /// Create a new folder in the current directory and reload.
    NewFolder,
    /// Create a new empty file in the current directory and reload.
    NewFile,
    /// Show/hide the live name-filter search bar.
    ToggleSearch,
    /// Set the live name filter (`None` clears it) and reload.
    SetNameFilter(Option<String>),
    /// Run a recursive search from the current directory. `content` greps file
    /// contents; otherwise entry names are matched. An empty query restores the
    /// normal directory listing.
    RunSearch {
        /// The query string (name substring or content regex).
        query: String,
        /// Whether to search file contents rather than names.
        content: bool,
    },
    /// Begin inline rename of the single selected row (F2).
    BeginRename,
    /// Context: open the selected entries.
    ContextOpen,
    /// Context: open the selected entry with a chosen application.
    ContextOpenWith,
    /// Context: hold the selection for paste; `cut` means move on paste.
    ContextClip {
        /// Whether this is a cut (move) rather than a copy.
        cut: bool,
    },
    /// Context: paste the held clipboard into the current directory.
    ContextPaste,
    /// Context: copy the full path(s) of the selection to the clipboard.
    ContextCopyPath,
    /// Context: copy the file name(s) of the selection to the clipboard.
    ContextCopyName,
    /// Context: move the selection to the trash.
    ContextTrash,
    /// Context: permanently delete the selection (after confirmation).
    ContextDelete,
    /// Context: compute a checksum of the single selected file.
    ContextChecksum(Algo),
    /// Context: compress the selection into an archive of the given format.
    ContextCompress(ArchiveFormat),
    /// Context: extract the selected archive(s) into the current directory.
    ContextExtract,
    /// Context: extract the selected archive(s) into a user-chosen directory.
    ContextDiskUsage,
    ContextExtractTo,
    /// Context: open a browse-archive read-only view of the focused archive.
    ContextBrowseArchive,
    /// Context: encrypt the selected file(s) to `.age` (prompts for passphrase).
    ContextEncrypt,
    /// Context: show git status for the current directory.
    ContextGitStatus,
    /// Context: request the properties dialog for the focused selection.
    RequestProperties,
    /// Context: open the permissions editor for the focused file/directory.
    RequestPermissions,
    /// Context: request adding the selection to a vault.
    RequestAddToVault,
    /// Context: request bookmarking the focused directory (or current dir).
    RequestAddBookmark,
    /// Context: open the bulk-rename dialog for the current selection (2+ files).
    RequestBulkRename,
    /// Make `path` the sole context target if it is not already part of the
    /// current selection (used so a right-click acts on the row under the
    /// pointer even when nothing was selected first).
    ContextTarget(PathBuf),
    /// Copy the selection into the given directory (e.g. the other pane).
    CopyTo(PathBuf),
    /// Move the selection into the given directory (e.g. the other pane).
    MoveTo(PathBuf),
    /// Files were dropped onto this pane (drag-and-drop from any source).
    DropFiles {
        /// Paths of the dragged files.
        paths: Vec<PathBuf>,
        /// Whether the source requested a move (`MOVE` action) rather than copy.
        move_it: bool,
    },
    /// An inline edit finished with a changed name; perform the rename.
    RenameCommitted {
        /// Path of the entry being renamed.
        path: PathBuf,
        /// The new file name (final component only).
        new_name: String,
    },
    /// The selection changed; recompute and report the summary.
    SelectionChanged,
    /// Re-emit current directory, history, view mode and selection (used when a
    /// tab becomes active so the chrome can resync without re-listing).
    Announce,
    /// Apply plugin-provided badges to matching file items.
    UpdatePluginBadges(HashMap<PathBuf, (String, String)>),
    /// Populate the plugin context-menu section.
    SetPluginContextItems(Vec<(String, String)>),
    /// A plugin context item was activated; fire the action with selected paths.
    ContextPluginItem(String),
}

/// Results of background operations delivered back to the component.
#[derive(Debug)]
pub enum PaneCmd {
    /// A directory listing finished.
    Listed(Result<Vec<FileEntry>, String>),
    /// A recursive search began: clear the view to receive streamed results.
    SearchCleared,
    /// A batch of streamed search results to append.
    SearchBatch(Vec<FileEntry>),
    /// A recursive search finished (cancelled or exhausted).
    SearchDone,
    /// A recursive search could not start (e.g. invalid regex/glob).
    SearchError(String),
    /// A rename finished.
    Renamed(Result<(), String>),
    /// A new file/folder creation finished.
    Created(Result<(), String>),
    /// A context filesystem operation finished; reload on success.
    OpDone(Result<(), String>),
    /// A checksum computation finished; show the result.
    Checksum {
        /// The algorithm label (e.g. "SHA-256").
        label: String,
        /// The file the checksum is for.
        path: PathBuf,
        /// The hex digest, or an error message.
        result: Result<String, String>,
    },
    /// An informational result to show in a dialog (e.g. git status).
    Info {
        /// Dialog heading.
        title: String,
        /// Dialog body text.
        body: String,
    },
    /// Git status for the current directory, loaded async after listing.
    GitStatus(StdHashMap<std::path::PathBuf, GitStatus>),
    /// An archive compress/extract progress update.
    Progress {
        /// Fraction complete in [0.0, 1.0]; 0.0 if total is unknown.
        fraction: f64,
        /// Short label of the entry currently being processed.
        current: String,
    },
}

/// Messages the pane emits to its parent.
#[derive(Debug)]
pub enum PaneOutput {
    /// The pane navigated to a new directory.
    DirChanged(PathBuf),
    /// The selection summary changed: `(selected, selected_bytes, total)`.
    Selection {
        /// Number of selected entries.
        selected: usize,
        /// Combined size of the selected entries, in bytes.
        bytes: u64,
        /// Total number of entries in the directory.
        total: usize,
    },
    /// The active view mode changed.
    ViewModeChanged(ViewMode),
    /// The single-selection preview target changed: `Some(entry)` when exactly
    /// one entry is selected, `None` otherwise.
    Preview(Option<FileEntry>),
    /// History availability changed: whether Back/Forward are possible.
    History {
        /// Whether a Back navigation is available.
        can_back: bool,
        /// Whether a Forward navigation is available.
        can_forward: bool,
    },
    /// A non-directory entry was activated and should be opened.
    OpenFile(PathBuf),
    /// Show the properties dialog for the given entry.
    ShowProperties(FileEntry),
    /// Open the permissions editor for the given entry.
    ShowPermissions(FileEntry),
    /// The user asked to add these paths to a vault.
    AddToVault(Vec<PathBuf>),
    /// The user asked to bookmark this directory.
    AddBookmark(PathBuf),
    /// Open the bulk-rename dialog for these paths.
    BulkRename(Vec<PathBuf>),
    /// The user chose "Extract To" — provide a destination picker.
    /// The user chose "Disk Usage" for a directory.
    DiskUsage(PathBuf),
    ExtractTo(Vec<PathBuf>),
    /// The user chose to browse an archive's contents.
    BrowseArchive(PathBuf),
    /// A user-facing error occurred (e.g. listing or rename failed).
    Error(String),
    /// A plugin context item was activated; fire the named action with selected paths.
    PluginContextItem {
        /// The plugin action id.
        action_id: String,
        /// The currently selected paths at activation time.
        paths: Vec<PathBuf>,
    },
}

/// The single-pane file browser.
pub struct FilePane {
    list_view: TypedColumnView<FileItem, gtk::MultiSelection>,
    grid_view: TypedGridView<FileItem, gtk::MultiSelection>,
    stack: gtk::Stack,
    /// The collapsible search bar holding the live name-filter entry.
    search_bar: gtk::SearchBar,
    dir: PathBuf,
    /// Directories visited before the current one (most recent last).
    back: Vec<PathBuf>,
    /// Directories navigated away from via Back (most recent last).
    forward: Vec<PathBuf>,
    filter: FilterOptions,
    sort: SortKey,
    mode: ViewMode,
    /// Remembered view mode per directory (session-scoped; disk persistence is
    /// added with the config system in Phase 7).
    dir_modes: HashMap<PathBuf, ViewMode>,
    /// Registry of realised name-cell editable labels, for F2 rename.
    registry: RenameRegistry,
    /// Paths held for a pending paste, and whether the operation is a move.
    clipboard: Vec<PathBuf>,
    /// Whether the clipboard holds a cut (move) rather than a copy.
    clip_cut: bool,
    /// Whether trashing files asks for confirmation first.
    confirm_delete: bool,
    /// Whether the view currently shows recursive search results rather than the
    /// directory listing.
    searching: bool,
    /// Cancellation handle for the in-flight recursive search, if any.
    search_cancel: Option<CancelToken>,
    /// Currently selected paths, kept in sync so DragSource cells can build a
    /// complete payload without querying the selection model directly.
    drag_selection: DragSelection,
    /// Progress bar shown during compress/extract operations; `None` when idle.
    progress_bar: Option<gtk::ProgressBar>,
    /// Window holding the progress bar; `None` when idle.
    progress_win: Option<gtk::Window>,
    /// Dynamic context-menu section for plugin-contributed items.
    plugin_items_menu: gio::Menu,
}

#[relm4::component(pub)]
impl Component for FilePane {
    type Init = PaneInit;
    type Input = PaneInput;
    type Output = PaneOutput;
    type CommandOutput = PaneCmd;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_vexpand: true,
            set_hexpand: true,

            // The search bar's child is built imperatively in `init` (entry plus
            // a content-search toggle), so the toggle state can be read when the
            // entry is activated.
            #[name = "search_bar"]
            gtk::SearchBar {},

            #[name = "stack"]
            gtk::Stack {
                set_vexpand: true,
                set_hexpand: true,
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let registry: RenameRegistry = Rc::new(RefCell::new(HashMap::new()));

        let mut list_view: TypedColumnView<FileItem, gtk::MultiSelection> = TypedColumnView::new();
        list_view.append_column::<NameColumn>();
        list_view.append_column::<SizeColumn>();
        list_view.append_column::<ModifiedColumn>();
        list_view.append_column::<KindColumn>();
        list_view.append_column::<PermissionsColumn>();
        list_view.view.set_enable_rubberband(true);
        list_view.view.set_single_click_activate(init.single_click);

        let grid_view: TypedGridView<FileItem, gtk::MultiSelection> = TypedGridView::new();
        grid_view.view.set_enable_rubberband(true);
        grid_view.view.set_single_click_activate(init.single_click);

        wire_activate(&list_view.view, &sender);
        wire_grid_activate(&grid_view.view, &sender);
        wire_selection(&list_view.selection_model, &sender);
        wire_selection(&grid_view.selection_model, &sender);
        wire_keys(&list_view.view, &sender);
        wire_keys(&grid_view.view, &sender);

        let list_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&list_view.view)
            .build();
        list_scroll.add_css_class("orca-content");
        let grid_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&grid_view.view)
            .build();
        grid_scroll.add_css_class("orca-content");

        wire_drop_target(&list_scroll, &sender);
        wire_drop_target(&grid_scroll, &sender);

        let widgets = view_output!();
        let stack = widgets.stack.clone();
        stack.add_named(&list_scroll, Some("columns"));
        stack.add_named(&grid_scroll, Some("grid"));

        let filter = FilterOptions {
            show_hidden: init.show_hidden,
            ..Default::default()
        };
        let search_bar = widgets.search_bar.clone();
        search_bar.set_key_capture_widget(Some(&list_scroll));
        build_search_box(&search_bar, &sender);

        let drag_selection: DragSelection = Rc::new(RefCell::new(Vec::new()));

        let model = FilePane {
            list_view,
            grid_view,
            stack,
            search_bar,
            dir: init.dir.clone(),
            back: Vec::new(),
            forward: Vec::new(),
            filter,
            sort: init.sort,
            mode: init.mode,
            dir_modes: HashMap::new(),
            registry,
            clipboard: Vec::new(),
            clip_cut: false,
            confirm_delete: init.confirm_delete,
            searching: false,
            search_cancel: None,
            drag_selection,
            progress_bar: None,
            progress_win: None,
            plugin_items_menu: gio::Menu::new(),
        };
        model.apply_mode();

        setup_context_menu(
            &root,
            &model.list_view.view,
            &model.grid_view.view,
            &model.plugin_items_menu,
            &sender,
        );

        sender.input(PaneInput::Navigate(init.dir));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PaneInput::Navigate(dir) => {
                if dir != self.dir {
                    self.back
                        .push(std::mem::replace(&mut self.dir, dir.clone()));
                    self.forward.clear();
                }
                self.load(dir, &sender);
            }
            PaneInput::Reload => self.spawn_list(&sender),
            PaneInput::GoUp => {
                if let Some(parent) = self.dir.parent() {
                    sender.input(PaneInput::Navigate(parent.to_path_buf()));
                }
            }
            PaneInput::Back => {
                if let Some(prev) = self.back.pop() {
                    self.forward
                        .push(std::mem::replace(&mut self.dir, prev.clone()));
                    self.load(prev, &sender);
                }
            }
            PaneInput::Forward => {
                if let Some(next) = self.forward.pop() {
                    self.back
                        .push(std::mem::replace(&mut self.dir, next.clone()));
                    self.load(next, &sender);
                }
            }
            PaneInput::Activate(pos) => {
                if let Some(item) = self.active_get(pos) {
                    let entry = item.borrow();
                    if entry.entry.kind.is_dir() {
                        let path = entry.entry.path.clone();
                        drop(entry);
                        sender.input(PaneInput::Navigate(path));
                    } else {
                        sender
                            .output(PaneOutput::OpenFile(entry.entry.path.clone()))
                            .ok();
                    }
                }
            }
            PaneInput::ToggleHidden => {
                self.filter.show_hidden = !self.filter.show_hidden;
                self.spawn_list(&sender);
            }
            PaneInput::SetHidden(show) => {
                if self.filter.show_hidden != show {
                    self.filter.show_hidden = show;
                    self.spawn_list(&sender);
                }
            }
            PaneInput::SetSingleClick(single) => {
                self.list_view.view.set_single_click_activate(single);
                self.grid_view.view.set_single_click_activate(single);
            }
            PaneInput::SetConfirmDelete(confirm) => {
                self.confirm_delete = confirm;
            }
            PaneInput::SetSort(sort) => {
                self.sort = sort;
                self.spawn_list(&sender);
            }
            PaneInput::SetViewMode(mode) => {
                self.mode = mode;
                self.dir_modes.insert(self.dir.clone(), mode);
                self.apply_mode();
                self.report_selection(&sender);
                sender.output(PaneOutput::ViewModeChanged(mode)).ok();
            }
            PaneInput::NewFolder => self.spawn_create(true, &sender),
            PaneInput::NewFile => self.spawn_create(false, &sender),
            PaneInput::ToggleSearch => {
                let on = !self.search_bar.is_search_mode();
                self.search_bar.set_search_mode(on);
                if !on && (self.filter.name_pattern.is_some() || self.searching) {
                    self.filter.name_pattern = None;
                    self.restore_listing(&sender);
                }
            }
            PaneInput::SetNameFilter(pattern) => {
                // A live local filter only narrows the current listing; ignore it
                // while showing recursive search results.
                if !self.searching {
                    self.filter.name_pattern = pattern;
                    self.spawn_list(&sender);
                }
            }
            PaneInput::RunSearch { query, content } => {
                if query.trim().is_empty() {
                    self.restore_listing(&sender);
                } else {
                    self.spawn_search(query, content, &sender);
                }
            }
            PaneInput::BeginRename => self.begin_rename(),
            PaneInput::ContextOpen => self.context_open(&sender),
            PaneInput::ContextOpenWith => self.context_open_with(),
            PaneInput::ContextClip { cut } => {
                self.clipboard = self.selected_paths();
                self.clip_cut = cut;
            }
            PaneInput::ContextPaste => self.context_paste(&sender),
            PaneInput::ContextCopyPath => self.copy_to_clipboard(true),
            PaneInput::ContextCopyName => self.copy_to_clipboard(false),
            PaneInput::ContextTrash => self.context_delete(false, &sender),
            PaneInput::ContextDelete => self.context_delete(true, &sender),
            PaneInput::ContextChecksum(algo) => self.context_checksum(algo, &sender),
            PaneInput::ContextCompress(format) => self.context_compress(format, &sender),
            PaneInput::ContextExtract => self.context_extract(&sender),
            PaneInput::ContextDiskUsage => {
                let path = self
                    .focused_entry()
                    .filter(|e| e.kind.is_dir())
                    .map(|e| e.path.clone())
                    .unwrap_or_else(|| self.dir.clone());
                sender.output(PaneOutput::DiskUsage(path)).ok();
            }
            PaneInput::ContextExtractTo => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    sender.output(PaneOutput::ExtractTo(paths)).ok();
                }
            }
            PaneInput::ContextBrowseArchive => {
                if let Some(entry) = self.focused_entry() {
                    sender.output(PaneOutput::BrowseArchive(entry.path)).ok();
                }
            }
            PaneInput::ContextEncrypt => self.context_encrypt(&sender),
            PaneInput::ContextGitStatus => self.context_git_status(&sender),
            PaneInput::RequestProperties => {
                if let Some(entry) = self.focused_entry() {
                    sender.output(PaneOutput::ShowProperties(entry)).ok();
                }
            }
            PaneInput::RequestPermissions => {
                if let Some(entry) = self.focused_entry() {
                    sender.output(PaneOutput::ShowPermissions(entry)).ok();
                }
            }
            PaneInput::RequestAddToVault => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    sender.output(PaneOutput::AddToVault(paths)).ok();
                }
            }
            PaneInput::RequestBulkRename => {
                let paths = self.selected_paths();
                if paths.len() >= 2 {
                    sender.output(PaneOutput::BulkRename(paths)).ok();
                }
            }
            PaneInput::RequestAddBookmark => {
                // Bookmark the focused entry if it is a directory, else the
                // current directory.
                let target = self
                    .focused_entry()
                    .filter(|e| e.kind.is_dir())
                    .map(|e| e.path)
                    .unwrap_or_else(|| self.dir.clone());
                sender.output(PaneOutput::AddBookmark(target)).ok();
            }
            PaneInput::ContextTarget(path) => self.select_context_target(&path),
            PaneInput::DropFiles { paths, move_it } => {
                self.transfer_paths(paths, self.dir.clone(), move_it, &sender);
            }
            PaneInput::CopyTo(dest) => self.transfer(dest, false, &sender),
            PaneInput::MoveTo(dest) => self.transfer(dest, true, &sender),
            PaneInput::RenameCommitted { path, new_name } => {
                sender.oneshot_command(async move {
                    let result = orca_core::rename(&path, &new_name)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    PaneCmd::Renamed(result)
                });
            }
            PaneInput::SelectionChanged => self.report_selection(&sender),
            PaneInput::Announce => {
                sender.output(PaneOutput::DirChanged(self.dir.clone())).ok();
                sender
                    .output(PaneOutput::History {
                        can_back: !self.back.is_empty(),
                        can_forward: !self.forward.is_empty(),
                    })
                    .ok();
                sender.output(PaneOutput::ViewModeChanged(self.mode)).ok();
                self.report_selection(&sender);
            }
            PaneInput::ContextPluginItem(action_id) => {
                let paths = self.selected_paths();
                sender
                    .output(PaneOutput::PluginContextItem { action_id, paths })
                    .ok();
            }
            PaneInput::SetPluginContextItems(items) => {
                self.plugin_items_menu.remove_all();
                for (action_id, label) in &items {
                    self.plugin_items_menu.append(
                        Some(label.as_str()),
                        Some(&format!("ctx.plugin-item::{action_id}")),
                    );
                }
            }
            PaneInput::UpdatePluginBadges(badges) => {
                let n = self.list_view.len();
                for i in 0..n {
                    if let Some(item) = self.list_view.get(i) {
                        let badge = badges.get(&item.borrow().entry.path).cloned();
                        item.borrow_mut().plugin_badge = badge;
                    }
                }
                let gn = self.grid_view.len();
                for i in 0..gn {
                    if let Some(item) = self.grid_view.get(i) {
                        let badge = badges.get(&item.borrow().entry.path).cloned();
                        item.borrow_mut().plugin_badge = badge;
                    }
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            PaneCmd::Listed(Ok(entries)) => {
                self.registry.borrow_mut().clear();
                self.list_view.clear();
                self.grid_view.clear();
                let input = sender.input_sender().clone();
                for entry in entries {
                    self.list_view.append(FileItem::new(
                        entry.clone(),
                        input.clone(),
                        self.registry.clone(),
                        self.drag_selection.clone(),
                    ));
                    self.grid_view.append(FileItem::new(
                        entry,
                        input.clone(),
                        self.registry.clone(),
                        self.drag_selection.clone(),
                    ));
                }
                self.report_selection(&sender);
                // Kick off async git status after listing, no listing delay.
                let dir = self.dir.clone();
                sender.oneshot_command(async move {
                    let statuses = orca_core::git_status(&dir)
                        .await
                        .unwrap_or_default();
                    PaneCmd::GitStatus(statuses)
                });
            }
            PaneCmd::Listed(Err(e)) => {
                tracing::warn!(dir = %self.dir.display(), error = %e, "directory listing failed");
                sender.output(PaneOutput::Error(e)).ok();
            }
            PaneCmd::SearchCleared => {
                self.registry.borrow_mut().clear();
                self.list_view.clear();
                self.grid_view.clear();
                self.report_selection(&sender);
            }
            PaneCmd::SearchBatch(entries) => {
                let input = sender.input_sender().clone();
                for entry in entries {
                    self.list_view.append(FileItem::new(
                        entry.clone(),
                        input.clone(),
                        self.registry.clone(),
                        self.drag_selection.clone(),
                    ));
                    self.grid_view.append(FileItem::new(
                        entry,
                        input.clone(),
                        self.registry.clone(),
                        self.drag_selection.clone(),
                    ));
                }
                self.report_selection(&sender);
            }
            PaneCmd::SearchDone => self.report_selection(&sender),
            PaneCmd::SearchError(e) => {
                self.searching = false;
                tracing::warn!(error = %e, "search failed");
                sender.output(PaneOutput::Error(e)).ok();
            }
            PaneCmd::Renamed(Ok(())) => self.spawn_list(&sender),
            PaneCmd::Renamed(Err(e)) => {
                tracing::warn!(error = %e, "rename failed");
                sender.output(PaneOutput::Error(e)).ok();
            }
            PaneCmd::Created(Ok(())) => self.spawn_list(&sender),
            PaneCmd::Created(Err(e)) => {
                tracing::warn!(error = %e, "create failed");
                sender.output(PaneOutput::Error(e)).ok();
            }
            PaneCmd::OpDone(result) => {
                if let Some(win) = self.progress_win.take() {
                    win.close();
                }
                self.progress_bar = None;
                match result {
                    Ok(()) => self.spawn_list(&sender),
                    Err(e) => {
                        tracing::warn!(error = %e, "operation failed");
                        sender.output(PaneOutput::Error(e)).ok();
                    }
                }
            }
            PaneCmd::GitStatus(statuses) => {
                if statuses.is_empty() {
                    return; // Not a git repo — no badges needed.
                }
                let n = self.list_view.len();
                for i in 0..n {
                    if let Some(item) = self.list_view.get(i) {
                        let status = statuses.get(&item.borrow().entry.path).copied();
                        item.borrow_mut().git_status = status;
                    }
                }
                let gn = self.grid_view.len();
                for i in 0..gn {
                    if let Some(item) = self.grid_view.get(i) {
                        let status = statuses.get(&item.borrow().entry.path).copied();
                        item.borrow_mut().git_status = status;
                    }
                }
            }
            PaneCmd::Progress { fraction, current } => {
                if self.progress_win.is_none() {
                    let parent = crate::dialogs::window_of(root);
                    let pb = gtk::ProgressBar::new();
                    pb.set_show_text(true);
                    pb.set_hexpand(true);
                    let bx = gtk::Box::builder()
                        .orientation(gtk::Orientation::Vertical)
                        .spacing(8)
                        .margin_top(16)
                        .margin_bottom(16)
                        .margin_start(16)
                        .margin_end(16)
                        .build();
                    bx.append(&pb);
                    let win = gtk::Window::builder()
                        .title(i18n::t("arc.progress").as_str())
                        .modal(true)
                        .default_width(360)
                        .child(&bx)
                        .build();
                    if let Some(p) = parent.as_ref() {
                        win.set_transient_for(Some(p));
                    }
                    win.present();
                    self.progress_bar = Some(pb);
                    self.progress_win = Some(win);
                }
                if let Some(pb) = &self.progress_bar {
                    pb.set_fraction(fraction.clamp(0.0, 1.0));
                    if !current.is_empty() {
                        pb.set_text(Some(&current));
                    }
                }
            }
            PaneCmd::Checksum {
                label,
                path,
                result,
            } => match result {
                Ok(digest) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let title = i18n::tf("dlg.checksum", &[("algo", &label), ("name", &name)]);
                    dialogs::checksum(self.window().as_ref(), &title, &digest);
                }
                Err(e) => sender.output(PaneOutput::Error(e)).ok().unwrap_or(()),
            },
            PaneCmd::Info { title, body } => {
                dialogs::info(self.window().as_ref(), &title, &body);
            }
        }
    }
}

impl FilePane {
    /// Make `dir` the current directory: restore its remembered view mode, kick
    /// off a listing and report the new directory and history state. History
    /// stacks are managed by the caller.
    fn load(&mut self, dir: PathBuf, sender: &ComponentSender<Self>) {
        // Navigating away ends any active search.
        if let Some(cancel) = self.search_cancel.take() {
            cancel.cancel();
        }
        self.searching = false;
        self.dir = dir.clone();
        if let Some(mode) = self.dir_modes.get(&dir).copied() {
            self.mode = mode;
            self.apply_mode();
            sender.output(PaneOutput::ViewModeChanged(mode)).ok();
        }
        self.spawn_list(sender);
        sender.output(PaneOutput::DirChanged(dir)).ok();
        sender
            .output(PaneOutput::History {
                can_back: !self.back.is_empty(),
                can_forward: !self.forward.is_empty(),
            })
            .ok();
    }

    /// Create a new folder (or empty file) with a unique name in the current
    /// directory, then reload. The unique-name search and creation run off the
    /// UI thread.
    fn spawn_create(&self, is_dir: bool, sender: &ComponentSender<Self>) {
        let dir = self.dir.clone();
        sender.oneshot_command(async move {
            let base = if is_dir { "New Folder" } else { "New File" };
            let target = unique_path(&dir, base).await;
            let result = if is_dir {
                orca_core::create_dir(&target).await
            } else {
                orca_core::create_file(&target).await
            };
            PaneCmd::Created(result.map_err(|e| e.to_string()))
        });
    }

    /// Cancel any running search, leave search mode and reload the directory.
    fn restore_listing(&mut self, sender: &ComponentSender<Self>) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.cancel();
        }
        self.searching = false;
        self.spawn_list(sender);
    }

    /// Start a recursive search from the current directory, streaming matches
    /// into the view via batched [`PaneCmd::SearchBatch`] messages.
    fn spawn_search(&mut self, query: String, content: bool, sender: &ComponentSender<Self>) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.cancel();
        }
        let cancel = CancelToken::new();
        self.search_cancel = Some(cancel.clone());
        self.searching = true;
        let root = self.dir.clone();

        sender.command(move |out, shutdown| {
            shutdown
                .register(async move {
                    let opts = SearchOptions::default();
                    let started = if content {
                        search_by_content(&root, &query, &opts, cancel)
                    } else {
                        search_by_name(&root, &query, &opts, cancel)
                    };
                    let mut rx = match started {
                        Ok(rx) => {
                            out.send(PaneCmd::SearchCleared).ok();
                            rx
                        }
                        Err(e) => {
                            out.send(PaneCmd::SearchError(e.to_string())).ok();
                            return;
                        }
                    };
                    let mut batch: Vec<FileEntry> = Vec::with_capacity(SEARCH_BATCH);
                    while let Some(entry) = rx.recv().await {
                        batch.push(entry);
                        if batch.len() >= SEARCH_BATCH {
                            out.send(PaneCmd::SearchBatch(std::mem::take(&mut batch)))
                                .ok();
                        }
                    }
                    if !batch.is_empty() {
                        out.send(PaneCmd::SearchBatch(batch)).ok();
                    }
                    out.send(PaneCmd::SearchDone).ok();
                })
                .drop_on_shutdown()
        });
    }

    /// Spawn a background listing of the current directory with current
    /// filter/sort, delivering the result via [`PaneCmd::Listed`].
    fn spawn_list(&self, sender: &ComponentSender<Self>) {
        let dir = self.dir.clone();
        let filter = self.filter.clone();
        let sort = self.sort;
        sender.oneshot_command(async move {
            let result = orca_core::list_dir(&dir, &filter, sort)
                .await
                .map_err(|e| e.to_string());
            PaneCmd::Listed(result)
        });
    }

    /// Show the stack page and column visibility appropriate to the current
    /// mode.
    fn apply_mode(&self) {
        let detail = matches!(self.mode, ViewMode::Detail);
        let columns = self.list_view.get_columns();
        if let Some(col) = columns.get(KindColumn::COLUMN_NAME) {
            col.set_visible(detail);
        }
        if let Some(col) = columns.get(PermissionsColumn::COLUMN_NAME) {
            col.set_visible(detail);
        }
        let page = match self.mode {
            ViewMode::Icon => "grid",
            ViewMode::List | ViewMode::Detail => "columns",
        };
        self.stack.set_visible_child_name(page);
    }

    /// The selection model of the currently displayed view.
    fn active_selection(&self) -> gtk::MultiSelection {
        match self.mode {
            ViewMode::Icon => self.grid_view.selection_model.clone(),
            ViewMode::List | ViewMode::Detail => self.list_view.selection_model.clone(),
        }
    }

    /// Number of entries in the active view.
    fn active_len(&self) -> u32 {
        match self.mode {
            ViewMode::Icon => self.grid_view.len(),
            ViewMode::List | ViewMode::Detail => self.list_view.len(),
        }
    }

    /// Get the item at a position in the active view.
    fn active_get(&self, pos: u32) -> Option<TypedListItem<FileItem>> {
        match self.mode {
            ViewMode::Icon => self.grid_view.get(pos),
            ViewMode::List | ViewMode::Detail => self.list_view.get(pos),
        }
    }

    /// Compute and emit the current selection summary, and update the shared
    /// drag-selection snapshot used by DragSource cells.
    fn report_selection(&self, sender: &ComponentSender<Self>) {
        let total = self.active_len() as usize;
        let set = self.active_selection().selection();
        let mut positions = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            positions.push(first);
            positions.extend(iter);
        }
        let selected = positions.len();
        let mut bytes = 0u64;
        let mut single: Option<FileEntry> = None;
        let mut sel_paths: Vec<PathBuf> = Vec::with_capacity(selected);
        for pos in &positions {
            if let Some(item) = self.active_get(*pos) {
                let entry = item.borrow();
                bytes = bytes.saturating_add(entry.entry.size);
                sel_paths.push(entry.entry.path.clone());
            }
        }
        if selected == 1 {
            if let Some(item) = self.active_get(positions[0]) {
                single = Some(item.borrow().entry.clone());
            }
        }
        *self.drag_selection.borrow_mut() = sel_paths;
        sender
            .output(PaneOutput::Selection {
                selected,
                bytes,
                total,
            })
            .ok();
        sender.output(PaneOutput::Preview(single)).ok();
    }

    /// Start inline editing on the name cell of the single selected row, if
    /// exactly one row is selected in a column view and its label is realised.
    fn begin_rename(&self) {
        if matches!(self.mode, ViewMode::Icon) {
            return;
        }
        let set = self.active_selection().selection();
        if set.size() != 1 {
            return;
        }
        let pos = set.nth(0);
        let Some(item) = self.active_get(pos) else {
            return;
        };
        let path = item.borrow().entry.path.clone();
        if let Some(weak) = self.registry.borrow().get(&path) {
            if let Some(label) = weak.upgrade() {
                label.start_editing();
            }
        }
    }

    // --- Context menu operations ---------------------------------------------

    /// The entries currently selected in the active view.
    fn selected_entries(&self) -> Vec<FileEntry> {
        let set = self.active_selection().selection();
        let mut out = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            for pos in std::iter::once(first).chain(iter) {
                if let Some(item) = self.active_get(pos) {
                    out.push(item.borrow().entry.clone());
                }
            }
        }
        out
    }

    /// The paths currently selected in the active view.
    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    /// The first selected entry, treated as the focused one for single-target
    /// operations (properties, checksum, open-with).
    fn focused_entry(&self) -> Option<FileEntry> {
        self.selected_entries().into_iter().next()
    }

    /// Ensure a right-clicked row becomes the operative target: if `path` is not
    /// already part of the selection, select only it. A right-click on an
    /// already-selected row leaves a multi-selection intact.
    fn select_context_target(&self, path: &std::path::Path) {
        let len = self.active_len();
        let selection = self.active_selection();
        for pos in 0..len {
            let Some(item) = self.active_get(pos) else {
                continue;
            };
            if item.borrow().entry.path == path {
                if !selection.is_selected(pos) {
                    selection.select_item(pos, true);
                }
                return;
            }
        }
    }

    /// The toplevel window hosting the pane, if realised.
    fn window(&self) -> Option<gtk::Window> {
        dialogs::window_of(&self.stack)
    }

    /// Open the selected entries (directories navigate, files open externally).
    fn context_open(&self, sender: &ComponentSender<Self>) {
        for entry in self.selected_entries() {
            if entry.kind.is_dir() {
                sender.input(PaneInput::Navigate(entry.path));
            } else {
                sender.output(PaneOutput::OpenFile(entry.path)).ok();
            }
        }
    }

    /// Open the focused file with a user-chosen application.
    fn context_open_with(&self) {
        if let Some(entry) = self.focused_entry() {
            let file = gio::File::for_path(&entry.path);
            let launcher = gtk::FileLauncher::new(Some(&file));
            launcher.set_always_ask(true);
            launcher.launch(
                self.window().as_ref(),
                gio::Cancellable::NONE,
                |_res: Result<(), gtk::glib::Error>| {},
            );
        }
    }

    /// Copy or move the current selection into `dest` (used by F5/F6 between
    /// panes).
    fn transfer(&self, dest: PathBuf, move_it: bool, sender: &ComponentSender<Self>) {
        self.transfer_paths(self.selected_paths(), dest, move_it, sender);
    }

    /// Copy or move an explicit list of `paths` into `dest`.  Used by both
    /// F5/F6 inter-pane transfers and drag-and-drop drops.
    fn transfer_paths(
        &self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        move_it: bool,
        sender: &ComponentSender<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        sender.oneshot_command(async move {
            for src in paths {
                let Some(name) = src.file_name() else {
                    continue;
                };
                let dst = dest.join(name);
                // Skip no-op (e.g. dropped onto own directory).
                if src == dst {
                    continue;
                }
                let result = if move_it {
                    orca_core::move_entry(&src, &dst).await
                } else {
                    orca_core::copy(&src, &dst, false, None).await
                };
                if let Err(e) = result {
                    return PaneCmd::OpDone(Err(e.to_string()));
                }
            }
            PaneCmd::OpDone(Ok(()))
        });
    }

    /// Paste the clipboard into the current directory (copy, or move if cut).
    fn context_paste(&mut self, sender: &ComponentSender<Self>) {
        if self.clipboard.is_empty() {
            return;
        }
        let items = self.clipboard.clone();
        let cut = self.clip_cut;
        let dir = self.dir.clone();
        if cut {
            self.clipboard.clear();
        }
        sender.oneshot_command(async move {
            for src in items {
                let Some(name) = src.file_name() else {
                    continue;
                };
                let dst = dir.join(name);
                let result = if cut {
                    orca_core::move_entry(&src, &dst).await
                } else {
                    orca_core::copy(&src, &dst, false, None).await
                };
                if let Err(e) = result {
                    return PaneCmd::OpDone(Err(e.to_string()));
                }
            }
            PaneCmd::OpDone(Ok(()))
        });
    }

    /// Copy selected paths (or names) to the system clipboard, newline-joined.
    fn copy_to_clipboard(&self, full_path: bool) {
        let entries = self.selected_entries();
        if entries.is_empty() {
            return;
        }
        let text = entries
            .iter()
            .map(|e| {
                if full_path {
                    e.path.to_string_lossy().into_owned()
                } else {
                    e.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
    }

    /// Delete the selection, to trash or permanently (the latter is confirmed).
    fn context_delete(&self, permanent: bool, sender: &ComponentSender<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let run = {
            let sender = sender.clone();
            move || {
                let paths = paths.clone();
                sender.oneshot_command(async move {
                    for p in paths {
                        if let Err(e) = orca_core::delete(&p, !permanent).await {
                            return PaneCmd::OpDone(Err(e.to_string()));
                        }
                    }
                    PaneCmd::OpDone(Ok(()))
                });
            }
        };
        if permanent {
            let body = i18n::tf("dlg.delete_body", &[("n", &count.to_string())]);
            dialogs::confirm(
                self.window().as_ref(),
                &i18n::t("dlg.delete_title"),
                &body,
                &i18n::t("dlg.delete_confirm"),
                run,
            );
        } else if self.confirm_delete {
            let body = i18n::tf("dlg.trash_body", &[("n", &count.to_string())]);
            dialogs::confirm(
                self.window().as_ref(),
                &i18n::t("dlg.trash_title"),
                &body,
                &i18n::t("dlg.trash_confirm"),
                run,
            );
        } else {
            run();
        }
    }

    /// Compute a checksum of the focused file off the UI thread.
    fn context_checksum(&self, algo: Algo, sender: &ComponentSender<Self>) {
        let Some(entry) = self.focused_entry() else {
            return;
        };
        if entry.kind.is_dir() {
            return;
        }
        let path = entry.path.clone();
        let label = match algo {
            Algo::Sha256 => "SHA-256",
            Algo::Md5 => "MD5",
            Algo::Blake3 => "Blake3",
        }
        .to_owned();
        sender.oneshot_command(async move {
            let result = match algo {
                Algo::Sha256 => orca_core::checksum_sha256(&path).await,
                Algo::Md5 => orca_core::checksum_md5(&path).await,
                Algo::Blake3 => orca_core::checksum_blake3(&path).await,
            };
            PaneCmd::Checksum {
                label,
                path,
                result: result.map_err(|e| e.to_string()),
            }
        });
    }

    /// Compress the selection into a uniquely-named archive in the current dir.
    fn context_compress(&self, format: ArchiveFormat, sender: &ComponentSender<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let ext = match format {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::Tar => "tar",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::TarBz2 => "tar.bz2",
        };
        let base = if paths.len() == 1 {
            paths[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "archive".to_owned())
        } else {
            "archive".to_owned()
        };
        let mut dest = self.dir.join(format!("{base}.{ext}"));
        let mut n = 2u32;
        while dest.exists() {
            dest = self.dir.join(format!("{base} {n}.{ext}"));
            n = n.saturating_add(1);
        }
        sender.command(move |out, shutdown| {
            shutdown
                .register(async move {
                    let (tx, mut rx) =
                        tokio::sync::mpsc::channel::<orca_core::ArchiveProgress>(32);
                    let handle =
                        tokio::spawn(orca_core::compress(paths, dest, format, Some(tx)));
                    while let Some(p) = rx.recv().await {
                        let fraction = if p.total > 0 {
                            p.processed as f64 / p.total as f64
                        } else {
                            0.0
                        };
                        out.send(PaneCmd::Progress {
                            fraction,
                            current: p.current.to_string_lossy().into_owned(),
                        })
                        .ok();
                    }
                    let result = match handle.await {
                        Ok(r) => r.map_err(|e| e.to_string()),
                        Err(e) => Err(format!("compress task panicked: {e}")),
                    };
                    out.send(PaneCmd::OpDone(result)).ok();
                })
                .drop_on_shutdown()
        });
    }

    /// Extract the selected archive(s) into the current directory.
    fn context_extract(&self, sender: &ComponentSender<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dir = self.dir.clone();
        sender.command(|out, shutdown| {
            shutdown
                .register(async move {
                    for archive in paths {
                        let (tx, mut rx) =
                            tokio::sync::mpsc::channel::<orca_core::ArchiveProgress>(32);
                        let dest = dir.clone();
                        let arc = archive.clone();
                        let handle =
                            tokio::spawn(orca_core::extract(arc, dest, Some(tx)));
                        while let Some(p) = rx.recv().await {
                            let fraction = if p.total > 0 {
                                p.processed as f64 / p.total as f64
                            } else {
                                0.0
                            };
                            out.send(PaneCmd::Progress {
                                fraction,
                                current: p.current.to_string_lossy().into_owned(),
                            })
                            .ok();
                        }
                        let result = match handle.await {
                            Ok(r) => r.map_err(|e| e.to_string()),
                            Err(e) => Err(format!("extract task panicked: {e}")),
                        };
                        if let Err(e) = result {
                            out.send(PaneCmd::OpDone(Err(e))).ok();
                            return;
                        }
                    }
                    out.send(PaneCmd::OpDone(Ok(()))).ok();
                })
                .drop_on_shutdown()
        });
    }

    /// Encrypt the selected file(s) to `.age`, prompting for a passphrase.
    fn context_encrypt(&self, sender: &ComponentSender<Self>) {
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .filter(|e| !e.kind.is_dir())
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let sender = sender.clone();
        let title = i18n::t("dlg.encrypt_title");
        dialogs::passphrase(self.window().as_ref(), &title, move |pass| {
            let paths = paths.clone();
            sender.oneshot_command(async move {
                for path in paths {
                    let pass = pass.clone();
                    let outcome = relm4::spawn_blocking(move || {
                        orca_vault::encrypt_file(
                            &path,
                            pass.as_bytes(),
                            orca_vault::EncryptionScheme::Age,
                        )
                    })
                    .await;
                    match outcome {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => return PaneCmd::OpDone(Err(e.to_string())),
                        Err(e) => return PaneCmd::OpDone(Err(e.to_string())),
                    }
                }
                PaneCmd::OpDone(Ok(()))
            });
        });
    }

    /// Show git status for the current directory / selected files.
    fn context_git_status(&self, sender: &ComponentSender<Self>) {
        let dir = self.dir.clone();
        let paths = self.selected_paths();
        sender.oneshot_command(async move {
            match orca_core::git_status(&dir).await {
                Ok(map) if map.is_empty() => PaneCmd::Info {
                    title: i18n::t("dlg.git_title"),
                    body: i18n::t("dlg.git_none"),
                },
                Ok(map) => {
                    let mut body = String::new();
                    let targets: Vec<&PathBuf> = if paths.is_empty() {
                        map.keys().collect()
                    } else {
                        paths.iter().collect()
                    };
                    for p in targets {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.to_string_lossy().into_owned());
                        match map.get(p) {
                            Some(status) => body.push_str(&format!("{name}: {status:?}\n")),
                            None => body.push_str(&format!("{name}: Unmodified\n")),
                        }
                    }
                    PaneCmd::Info {
                        title: i18n::t("dlg.git_title"),
                        body,
                    }
                }
                Err(e) => PaneCmd::Info {
                    title: i18n::t("dlg.git_title"),
                    body: e.to_string(),
                },
            }
        });
    }
}

/// Find a non-existent path in `dir` based on `base`, appending ` 2`, ` 3`, …
/// until a free name is found.
async fn unique_path(dir: &std::path::Path, base: &str) -> PathBuf {
    let mut candidate = dir.join(base);
    let mut n = 2u32;
    while candidate.try_exists().unwrap_or(false) {
        candidate = dir.join(format!("{base} {n}"));
        n = n.saturating_add(1);
    }
    candidate
}

/// Build the context-menu action group + popover and attach right-click
/// gestures to both views.
fn setup_context_menu(
    root: &gtk::Box,
    list: &gtk::ColumnView,
    grid: &gtk::GridView,
    plugin_section: &gio::Menu,
    sender: &ComponentSender<FilePane>,
) {
    let group = gio::SimpleActionGroup::new();
    simple_action(&group, "open", sender, || PaneInput::ContextOpen);
    simple_action(&group, "open-with", sender, || PaneInput::ContextOpenWith);
    simple_action(&group, "cut", sender, || PaneInput::ContextClip {
        cut: true,
    });
    simple_action(&group, "copy", sender, || PaneInput::ContextClip {
        cut: false,
    });
    simple_action(&group, "paste", sender, || PaneInput::ContextPaste);
    simple_action(&group, "rename", sender, || PaneInput::BeginRename);
    simple_action(&group, "trash", sender, || PaneInput::ContextTrash);
    simple_action(&group, "delete", sender, || PaneInput::ContextDelete);
    simple_action(&group, "copy-path", sender, || PaneInput::ContextCopyPath);
    simple_action(&group, "copy-name", sender, || PaneInput::ContextCopyName);
    simple_action(&group, "add-vault", sender, || PaneInput::RequestAddToVault);
    simple_action(&group, "add-bookmark", sender, || {
        PaneInput::RequestAddBookmark
    });
    simple_action(&group, "bulk-rename", sender, || {
        PaneInput::RequestBulkRename
    });
    simple_action(&group, "encrypt", sender, || PaneInput::ContextEncrypt);
    simple_action(&group, "extract", sender, || PaneInput::ContextExtract);
    simple_action(&group, "extract-to", sender, || PaneInput::ContextExtractTo);
    simple_action(&group, "browse-archive", sender, || {
        PaneInput::ContextBrowseArchive
    });
    simple_action(&group, "disk-usage", sender, || PaneInput::ContextDiskUsage);
    simple_action(&group, "git-status", sender, || PaneInput::ContextGitStatus);
    simple_action(&group, "properties", sender, || {
        PaneInput::RequestProperties
    });
    simple_action(&group, "permissions", sender, || {
        PaneInput::RequestPermissions
    });

    let checksum = gio::SimpleAction::new("checksum", Some(gtk::glib::VariantTy::STRING));
    {
        let sender = sender.clone();
        checksum.connect_activate(move |_, param| {
            if let Some(s) = param.and_then(gtk::glib::Variant::str) {
                let algo = match s {
                    "sha256" => Algo::Sha256,
                    "md5" => Algo::Md5,
                    _ => Algo::Blake3,
                };
                sender.input(PaneInput::ContextChecksum(algo));
            }
        });
    }
    group.add_action(&checksum);

    let compress = gio::SimpleAction::new("compress", Some(gtk::glib::VariantTy::STRING));
    {
        let sender = sender.clone();
        compress.connect_activate(move |_, param| {
            if let Some(s) = param.and_then(gtk::glib::Variant::str) {
                let format = match s {
                    "zip" => ArchiveFormat::Zip,
                    "tar" => ArchiveFormat::Tar,
                    "tarxz" => ArchiveFormat::TarXz,
                    "tarbz2" => ArchiveFormat::TarBz2,
                    _ => ArchiveFormat::TarGz,
                };
                sender.input(PaneInput::ContextCompress(format));
            }
        });
    }
    group.add_action(&compress);

    // Plugin context items: a single parameterised action receives the action_id.
    let plugin_item = gio::SimpleAction::new("plugin-item", Some(gtk::glib::VariantTy::STRING));
    {
        let sender = sender.clone();
        plugin_item.connect_activate(move |_, param| {
            if let Some(action_id) = param.and_then(gtk::glib::Variant::str) {
                sender.input(PaneInput::ContextPluginItem(action_id.to_owned()));
            }
        });
    }
    group.add_action(&plugin_item);

    root.insert_action_group("ctx", Some(&group));

    let popover = gtk::PopoverMenu::from_model(Some(&context_menu_model(plugin_section)));
    popover.set_has_arrow(false);
    popover.set_parent(root);

    attach_ctx_gesture(list, root, &popover);
    attach_ctx_gesture(grid, root, &popover);
}

/// Register a parameterless context action that emits `make()` on activation.
fn simple_action(
    group: &gio::SimpleActionGroup,
    name: &str,
    sender: &ComponentSender<FilePane>,
    make: impl Fn() -> PaneInput + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    let sender = sender.clone();
    action.connect_activate(move |_, _| sender.input(make()));
    group.add_action(&action);
}

/// Attach a secondary-click gesture that pops the context menu at the pointer.
fn attach_ctx_gesture<W: IsA<gtk::Widget> + Clone>(
    view: &W,
    root: &gtk::Box,
    popover: &gtk::PopoverMenu,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
    let cb_view = view.clone();
    let root = root.clone();
    let popover = popover.clone();
    gesture.connect_pressed(move |_, _, x, y| {
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        let (tx, ty) = cb_view
            .compute_point(&root, &point)
            .map_or((x, y), |p| (f64::from(p.x()), f64::from(p.y())));
        let rect = gtk::gdk::Rectangle::new(tx as i32, ty as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        popover.popup();
    });
    view.add_controller(gesture);
}

/// Build the context-menu `gio::Menu` model (labels localized).
fn context_menu_model(plugin_section: &gio::Menu) -> gio::Menu {
    let menu = gio::Menu::new();

    let open = gio::Menu::new();
    open.append(Some(&i18n::t("ctx.open")), Some("ctx.open"));
    open.append(Some(&i18n::t("ctx.open_with")), Some("ctx.open-with"));
    menu.append_section(None, &open);

    let clip = gio::Menu::new();
    clip.append(Some(&i18n::t("ctx.cut")), Some("ctx.cut"));
    clip.append(Some(&i18n::t("ctx.copy")), Some("ctx.copy"));
    clip.append(Some(&i18n::t("ctx.paste")), Some("ctx.paste"));
    menu.append_section(None, &clip);

    let edit = gio::Menu::new();
    edit.append(Some(&i18n::t("ctx.rename")), Some("ctx.rename"));
    edit.append(Some(&i18n::t("ctx.bulk_rename")), Some("ctx.bulk-rename"));
    edit.append(Some(&i18n::t("ctx.trash")), Some("ctx.trash"));
    edit.append(Some(&i18n::t("ctx.delete")), Some("ctx.delete"));
    menu.append_section(None, &edit);

    let paths = gio::Menu::new();
    paths.append(Some(&i18n::t("ctx.copy_path")), Some("ctx.copy-path"));
    paths.append(Some(&i18n::t("ctx.copy_name")), Some("ctx.copy-name"));
    paths.append(Some(&i18n::t("ctx.add_bookmark")), Some("ctx.add-bookmark"));
    menu.append_section(None, &paths);

    let vault = gio::Menu::new();
    vault.append(Some(&i18n::t("ctx.add_vault")), Some("ctx.add-vault"));
    vault.append(Some(&i18n::t("ctx.encrypt")), Some("ctx.encrypt"));
    menu.append_section(None, &vault);

    let tools = gio::Menu::new();
    let checksum = gio::Menu::new();
    checksum.append(Some("SHA-256"), Some("ctx.checksum::sha256"));
    checksum.append(Some("MD5"), Some("ctx.checksum::md5"));
    checksum.append(Some("Blake3"), Some("ctx.checksum::blake3"));
    tools.append_submenu(Some(&i18n::t("ctx.checksum")), &checksum);
    let compress = gio::Menu::new();
    compress.append(Some("ZIP"), Some("ctx.compress::zip"));
    compress.append(Some("tar.gz"), Some("ctx.compress::targz"));
    compress.append(Some("tar.xz"), Some("ctx.compress::tarxz"));
    compress.append(Some("tar.bz2"), Some("ctx.compress::tarbz2"));
    tools.append_submenu(Some(&i18n::t("ctx.compress")), &compress);
    tools.append(Some(&i18n::t("ctx.extract")), Some("ctx.extract"));
    tools.append(Some(&i18n::t("ctx.extract_to")), Some("ctx.extract-to"));
    tools.append(
        Some(&i18n::t("ctx.browse_archive")),
        Some("ctx.browse-archive"),
    );
    menu.append_section(None, &tools);

    let meta = gio::Menu::new();
    meta.append(Some(&i18n::t("ctx.disk_usage")), Some("ctx.disk-usage"));
    meta.append(Some(&i18n::t("ctx.git_status")), Some("ctx.git-status"));
    meta.append(
        Some(&i18n::t("ctx.permissions")),
        Some("ctx.permissions"),
    );
    meta.append(Some(&i18n::t("ctx.properties")), Some("ctx.properties"));
    menu.append_section(None, &meta);

    // Plugin-contributed items (populated dynamically via PaneInput::SetPluginContextItems).
    menu.append_section(None, plugin_section);

    menu
}

/// Route a column view's `activate` signal to [`PaneInput::Activate`].
fn wire_activate(view: &gtk::ColumnView, sender: &ComponentSender<FilePane>) {
    let sender = sender.clone();
    view.connect_activate(move |_, pos| sender.input(PaneInput::Activate(pos)));
}

/// Route a grid view's `activate` signal to [`PaneInput::Activate`].
fn wire_grid_activate(view: &gtk::GridView, sender: &ComponentSender<FilePane>) {
    let sender = sender.clone();
    view.connect_activate(move |_, pos| sender.input(PaneInput::Activate(pos)));
}

/// Route a selection model's `selection-changed` to [`PaneInput::SelectionChanged`].
fn wire_selection(model: &gtk::MultiSelection, sender: &ComponentSender<FilePane>) {
    let sender = sender.clone();
    model.connect_selection_changed(move |_, _, _| sender.input(PaneInput::SelectionChanged));
}

/// Build and install the search bar's child: a search entry that live-filters
/// the current listing as the user types and runs a recursive search on Enter,
/// plus a toggle that switches the recursive search between name and content.
fn build_search_box(bar: &gtk::SearchBar, sender: &ComponentSender<FilePane>) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();

    let entry = gtk::SearchEntry::builder()
        .hexpand(true)
        .placeholder_text(i18n::t("search.placeholder"))
        .build();
    let content = gtk::ToggleButton::builder()
        .icon_name("text-x-generic-symbolic")
        .tooltip_text(i18n::t("search.content"))
        .build();

    // Live, in-directory name filter as the user types.
    {
        let sender = sender.clone();
        entry.connect_search_changed(move |e| {
            let text = e.text().to_string();
            sender.input(PaneInput::SetNameFilter((!text.is_empty()).then_some(text)));
        });
    }
    // Enter runs a recursive search from the current directory.
    {
        let sender = sender.clone();
        let toggle = content.clone();
        entry.connect_activate(move |e| {
            sender.input(PaneInput::RunSearch {
                query: e.text().to_string(),
                content: toggle.is_active(),
            });
        });
    }
    // Flipping name/content re-runs the current query.
    {
        let sender = sender.clone();
        let entry = entry.clone();
        content.connect_toggled(move |b| {
            let query = entry.text().to_string();
            if !query.trim().is_empty() {
                sender.input(PaneInput::RunSearch {
                    query,
                    content: b.is_active(),
                });
            }
        });
    }

    row.append(&entry);
    row.append(&content);
    bar.set_child(Some(&row));
    bar.connect_entry(&entry);
}

/// Attach a `gtk::DropTarget` that accepts `gdk::FileList` drops and emits
/// [`PaneInput::DropFiles`].
///
/// Copy vs. move is tracked by `connect_enter` / `connect_motion`, which
/// return the negotiated action; that result is stored in `pending_action` and
/// read in `connect_drop`.  GTK4 has no `gdk::Drop::selected_action`; the
/// negotiated action is only known from the enter/motion callbacks.
fn wire_drop_target(widget: &impl IsA<gtk::Widget>, sender: &ComponentSender<FilePane>) {
    let drop = gtk::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );

    // Track the last action negotiated by enter/motion so connect_drop can
    // decide copy vs. move.
    let pending_action = Rc::new(RefCell::new(gdk::DragAction::COPY));

    {
        let pa = pending_action.clone();
        drop.connect_enter(move |target, _x, _y| {
            // Prefer MOVE when that is the ONLY action offered by the source
            // (e.g. a Shift+drag from the same pane); otherwise default COPY.
            let offered = target
                .current_drop()
                .map(|d| d.actions())
                .unwrap_or(gdk::DragAction::COPY);
            let action = if offered == gdk::DragAction::MOVE {
                gdk::DragAction::MOVE
            } else {
                gdk::DragAction::COPY
            };
            *pa.borrow_mut() = action;
            action
        });
    }
    {
        let pa = pending_action.clone();
        drop.connect_motion(move |target, _x, _y| {
            let offered = target
                .current_drop()
                .map(|d| d.actions())
                .unwrap_or(gdk::DragAction::COPY);
            let action = if offered == gdk::DragAction::MOVE {
                gdk::DragAction::MOVE
            } else {
                gdk::DragAction::COPY
            };
            *pa.borrow_mut() = action;
            action
        });
    }

    let sender = sender.clone();
    drop.connect_drop(move |_target, value, _x, _y| {
        let Ok(file_list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths: Vec<PathBuf> = file_list
            .files()
            .into_iter()
            .filter_map(|f| f.path())
            .collect();
        if paths.is_empty() {
            return false;
        }
        let move_it = *pending_action.borrow() == gdk::DragAction::MOVE;
        sender.input(PaneInput::DropFiles { paths, move_it });
        true
    });
    widget.add_controller(drop);
}

/// Attach the pane key bindings to a view widget.
fn wire_keys(view: &impl IsA<gtk::Widget>, sender: &ComponentSender<FilePane>) {
    let sender = sender.clone();
    let key = gtk::EventControllerKey::new();
    key.connect_key_pressed(move |_, keyval, _, state| {
        let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        match keyval {
            gtk::gdk::Key::BackSpace => {
                sender.input(PaneInput::GoUp);
                gtk::glib::Propagation::Stop
            }
            gtk::gdk::Key::F2 => {
                sender.input(PaneInput::BeginRename);
                gtk::glib::Propagation::Stop
            }
            gtk::gdk::Key::h | gtk::gdk::Key::H if ctrl => {
                sender.input(PaneInput::ToggleHidden);
                gtk::glib::Propagation::Stop
            }
            gtk::gdk::Key::_1 if ctrl => {
                sender.input(PaneInput::SetViewMode(ViewMode::List));
                gtk::glib::Propagation::Stop
            }
            gtk::gdk::Key::_2 if ctrl => {
                sender.input(PaneInput::SetViewMode(ViewMode::Icon));
                gtk::glib::Propagation::Stop
            }
            gtk::gdk::Key::_3 if ctrl => {
                sender.input(PaneInput::SetViewMode(ViewMode::Detail));
                gtk::glib::Propagation::Stop
            }
            _ => gtk::glib::Propagation::Proceed,
        }
    });
    view.add_controller(key);
}
