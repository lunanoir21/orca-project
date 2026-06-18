//! Application shell: the top-level window, the Places sidebar, the toolbar,
//! navigation bar, the content stack and the status bar.
//!
//! The content area is a `GtkStack` with three pages: the home page (quick-access
//! cards over the background image, shown on launch), the dual-pane file browser,
//! and the settings page. The Places sidebar ([`crate::places`]) and the home
//! cards switch between them. The dual-pane browser hosts two independent
//! [`crate::side::TabbedPane`] sides (each a notebook of
//! [`crate::pane::FilePane`] tabs) in a `GtkPaned`; one side is "active" and the
//! chrome reflects its visible tab. F3 reveals the second side, F5/F6 copy/move
//! the active selection into the other side, and Ctrl+T/Ctrl+W manage tabs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use orca_core::FileEntry;

use orca_vault::VaultManager;

use crate::archive_view::ArchiveView;
use crate::disk_usage::{DiskUsageDialog, DiskUsageOutput};
use crate::mount_manager::MountManager;
use crate::network::NetworkBrowser;
use crate::bulk_rename::{BulkRenameDialog, BulkRenameOutput};
use crate::config::Config;
use crate::keybind::{Action, ActionMap};
use crate::permissions::{PermissionsDialog, PermissionsInit};
use crate::dialogs;
use crate::format;
use crate::home::{HomeInput, HomeOutput, HomePage};
use crate::i18n::{self, Lang};
use crate::nav::{NavBar, NavInput, NavOutput};
use crate::pane::{PaneInput, PaneOutput, ViewMode};
use crate::places;
use crate::preview::{PreviewInput, PreviewPanel};
use crate::properties::Properties;
use crate::settings::{SettingsInit, SettingsOutput, SettingsPage};
use crate::side::{SideInit, SideInput, SideOutput, TabbedPane};
use crate::terminal::{TerminalInput, TerminalPanel};
use crate::toolbar::{self, ToolbarHandles};
use crate::vault_ui::{VaultPanel, VaultPanelInit, VaultPanelInput, VaultPanelOutput};
use crate::xdg::XdgPaths;

use orca_plugin::PluginManager;

/// Application initialisation payload.
pub struct AppInit {
    /// Resolved XDG directory layout.
    pub paths: XdgPaths,
    /// Loaded user configuration.
    pub config: Config,
    /// Path to the built-in `plugins/` directory shipped with Orca.
    pub builtin_plugins: std::path::PathBuf,
}

/// Which content page is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppPage {
    /// The home / start page.
    Home,
    /// The dual-pane file browser.
    Files,
    /// The settings page.
    Settings,
}

impl AppPage {
    /// The `GtkStack` child name for this page.
    fn name(self) -> &'static str {
        match self {
            AppPage::Home => "home",
            AppPage::Files => "files",
            AppPage::Settings => "settings",
        }
    }
}

/// Cached UI-relevant state for one pane, used to refresh the chrome when the
/// active pane changes without re-querying the pane.
#[derive(Debug, Clone)]
struct PaneUiState {
    dir: PathBuf,
    status: String,
    can_back: bool,
    can_forward: bool,
    mode: ViewMode,
}

impl Default for PaneUiState {
    fn default() -> Self {
        Self {
            dir: PathBuf::new(),
            status: i18n::tf("status.items", &[("n", "0")]),
            can_back: false,
            can_forward: false,
            mode: ViewMode::List,
        }
    }
}

/// Top-level application model.
pub struct AppModel {
    /// Resolved XDG directory layout.
    paths: XdgPaths,
    /// Loaded user configuration (mutated by the appearance picker, persisted).
    config: Config,
    /// Shared vault manager (unlocked vaults, timers). Arc<Mutex> so it can
    /// be passed to spawn_blocking inside VaultPanel without holding the GTK lock.
    #[allow(dead_code)]
    vault_mgr: Arc<Mutex<VaultManager>>,
    /// The two tabbed browser sides (index 0 = left, 1 = right). Each side is a
    /// notebook of independent [`crate::pane::FilePane`] tabs.
    panes: [Controller<TabbedPane>; 2],
    /// The home / start page.
    home: Controller<HomePage>,
    /// The settings page. Held only to keep the controller (and thus its widget
    /// in the content stack) alive; the app sends it no messages.
    #[allow(dead_code)]
    settings: Controller<SettingsPage>,
    /// The vault sidebar panel (Phase 6).
    vault_panel: Controller<VaultPanel>,
    /// Concise vault status string shown in the status bar.
    vault_status: String,
    /// The places part of the sidebar (cleared/repopulated on bookmark changes).
    places_box: gtk::Box,
    /// The file-preview side panel (Phase 4.4).
    preview: Controller<PreviewPanel>,
    /// Whether the preview panel is shown.
    preview_visible: bool,
    /// The embedded terminal panel (Phase 4.7).
    terminal: Controller<TerminalPanel>,
    /// Whether the terminal panel is shown.
    terminal_visible: bool,
    /// The currently visible content page.
    page: AppPage,
    /// Index of the active pane.
    active: usize,
    /// Whether the second pane is shown.
    dual: bool,
    /// Cached chrome state per pane.
    state: [PaneUiState; 2],
    /// The navigation bar (breadcrumb / path entry), reflecting the active pane.
    nav: Controller<NavBar>,
    /// Window title (active directory name).
    title: String,
    /// Status-bar text for the active pane.
    status: String,
    /// Active directory path shown (truncated) in the status bar.
    current_path: String,
    /// Handles to stateful toolbar buttons (history sensitivity, view toggles).
    toolbar: ToolbarHandles,
    /// The currently-open properties dialog, kept alive while shown.
    properties: Option<Controller<Properties>>,
    /// The currently-open bulk rename dialog, kept alive while shown.
    bulk_rename: Option<Controller<BulkRenameDialog>>,
    /// The currently-open permissions editor, kept alive while shown.
    permissions: Option<Controller<PermissionsDialog>>,
    /// The currently-open archive browser window, kept alive while shown.
    archive_view: Option<Controller<ArchiveView>>,
    /// The currently-open mount manager dialog, kept alive while shown.
    mount_manager: Option<Controller<MountManager>>,
    /// The currently-open disk usage dialog, kept alive while shown.
    disk_usage: Option<Controller<DiskUsageDialog>>,
    /// The currently-open network browser dialog, kept alive while shown.
    network_browser: Option<Controller<NetworkBrowser>>,
    /// Active scroll-sync binding between the two panes, when enabled.
    sync_binding: Option<gtk::glib::Binding>,
    /// Parsed keybind → action lookup table (shared with the key handler closure).
    action_map: Arc<Mutex<ActionMap>>,
    /// Lua plugin manager — lives on the GTK main thread (mlua::Lua is !Send).
    plugin_mgr: PluginManager,
}

/// Messages the shell handles.
#[derive(Debug, Clone)]
pub enum AppMsg {
    /// Show the home page.
    ShowHome,
    /// Show the settings page.
    ShowSettings,
    /// Mark a pane active (focus/click).
    Activate(usize),
    /// Navigate the active pane to its parent directory.
    Up,
    /// Active pane: go back in history.
    Back,
    /// Active pane: go forward in history.
    Forward,
    /// Navigate the active pane to a specific path (and show the file browser).
    NavigateTo(PathBuf),
    /// Toggle the path entry (Ctrl+L).
    TogglePathEntry,
    /// Reload the active pane.
    Reload,
    /// Create a new folder in the active pane's directory.
    NewFolder,
    /// Create a new file in the active pane's directory.
    NewFile,
    /// Toggle the active pane's search bar (and show the file browser).
    ToggleSearch,
    /// Open a new tab in the active side.
    NewTab,
    /// Close the active tab in the active side.
    CloseTab,
    /// Switch to the next tab in the active side.
    NextTab,
    /// Switch to the previous tab in the active side.
    PrevTab,
    /// Duplicate the active tab in the active side.
    DuplicateTab,
    /// A pane's single-selection preview target changed.
    Preview(usize, Option<FileEntry>),
    /// Toggle the file-preview side panel (F7).
    TogglePreview,
    /// Toggle the embedded terminal panel (F4).
    ToggleTerminal,
    /// Set the active pane's view mode.
    SetViewMode(ViewMode),
    /// Toggle the dual-pane layout (F3).
    ToggleDual,
    /// Toggle synchronized scrolling between panes.
    ToggleSync,
    /// Transfer the active pane's selection to the other pane (F5 copy/F6 move).
    Transfer {
        /// Whether to move rather than copy.
        move_it: bool,
    },
    /// Set (or clear) the window background image; persisted to config.
    SetBackground(Option<PathBuf>),
    /// Change the color scheme; persisted and applied live.
    SetScheme(String),
    /// Change the accent color; persisted and applied live.
    SetAccent(String),
    /// Change the background scrim strength; persisted and applied live.
    SetDim(f64),
    /// Change the UI / list font; persisted and applied live.
    SetFont(String),
    /// Toggle hidden-file visibility; persisted and applied to both panes.
    SetShowHidden(bool),
    /// Toggle single-click activation; persisted and applied to both panes.
    SetSingleClick(bool),
    /// Toggle trash confirmation; persisted and applied to both panes.
    SetConfirmDelete(bool),
    /// Change the default view for new panes; persisted (applies to new panes).
    SetDefaultView(String),
    /// Change the interface language; persisted to config (applies next launch).
    SetLanguage(Lang),
    /// A pane changed directory.
    DirChanged(usize, PathBuf),
    /// A pane's history availability changed.
    History {
        /// Pane index.
        idx: usize,
        /// Whether Back is possible.
        can_back: bool,
        /// Whether Forward is possible.
        can_forward: bool,
    },
    /// A pane's selection summary changed.
    Selection {
        /// Pane index.
        idx: usize,
        /// Selected entry count.
        selected: usize,
        /// Combined size of the selection in bytes.
        bytes: u64,
        /// Total entries in the directory.
        total: usize,
    },
    /// A pane reported its active view mode.
    ViewModeChanged(usize, ViewMode),
    /// Open a non-directory file with the system default handler.
    OpenFile(PathBuf),
    /// Show the properties dialog for an entry.
    ShowProperties(FileEntry),
    /// The user asked to add paths to a vault.
    AddToVault(Vec<PathBuf>),
    /// Add a directory to the bookmarks (persisted, shown in Places).
    AddBookmark(PathBuf),
    /// Remove a bookmark.
    RemoveBookmark(PathBuf),
    /// Open the bulk rename dialog for these paths.
    BulkRename(Vec<PathBuf>),
    /// Open the permissions editor for an entry.
    ShowPermissions(FileEntry),
    /// Prompt for a destination folder then extract the given archive paths there.
    ExtractTo(Vec<PathBuf>),
    /// Open the archive content browser for the given archive file.
    BrowseArchive(PathBuf),
    /// Open the mount manager dialog.
    OpenMountManager,
    /// Open the disk usage visualizer for a path.
    OpenDiskUsage(PathBuf),
    /// Open the network browser dialog.
    OpenNetworkBrowser,
    /// Surface an error from a pane.
    Error(String),
    /// Navigate to a path (vault panel output).
    VaultNavigate(PathBuf),
    /// A vault auto-locked; update status bar.
    VaultAutoLocked(String),
    /// Update the vault status bar text.
    VaultStatus(String),
    /// Vault configs changed; persist them.
    VaultConfigsChanged(Vec<orca_vault::VaultConfig>),
    /// Begin renaming the focused file in the active pane (F2).
    Rename,
    /// Toggle hidden files on the active pane.
    ToggleHidden,
    /// Update one keybind in config and rebuild the ActionMap.
    SetKeybind(String, String),
    /// Update the terminal shell path in config.
    SetTerminalShell(String),
    /// Update the terminal font in config.
    SetTerminalFont(String),
    /// Reset all config to defaults and apply.
    ResetDefaults,
    /// Config file changed on disk; reload and apply.
    ReloadConfig,
}

#[relm4::component(pub)]
impl Component for AppModel {
    type Init = AppInit;
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        #[root]
        gtk::ApplicationWindow {
            set_title: Some("Orca"),
            set_default_size: (1180, 760),
            add_css_class: "orca-root",

            #[wrap(Some)]
            set_titlebar = &gtk::HeaderBar {
                #[wrap(Some)]
                set_title_widget = &gtk::Label {
                    #[watch]
                    set_label: &model.title,
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                // --- Slim icon sidebar rail ---
                #[local_ref]
                sidebar -> gtk::Box {},

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    // Toolbar and nav bar are hidden on the home page for a
                    // clean, full-bleed start screen.
                    #[local_ref]
                    toolbar_row -> gtk::Box {
                        #[watch]
                        set_visible: model.page == AppPage::Files,
                    },

                    #[local_ref]
                    nav_widget -> gtk::Stack {
                        #[watch]
                        set_visible: model.page == AppPage::Files,
                    },

                    // --- Content stack: home page or dual-pane browser ---
                    #[local_ref]
                    content_stack -> gtk::Stack {
                        set_vexpand: true,
                        set_hexpand: true,
                        #[watch]
                        set_visible_child_name: model.page.name(),
                    },

                    // --- Embedded terminal (toggled with F4) ---
                    #[local_ref]
                    terminal_widget -> gtk::ScrolledWindow {},

                    // --- Status bar ---
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        add_css_class: "orca-statusbar",

                        gtk::Label {
                            #[watch]
                            set_label: &model.status,
                        },

                        gtk::Label {
                            set_hexpand: true,
                            set_halign: gtk::Align::Start,
                            set_ellipsize: gtk::pango::EllipsizeMode::Middle,
                            add_css_class: "dim-label",
                            #[watch]
                            set_label: &model.current_path,
                            #[watch]
                            set_tooltip_text: Some(&model.current_path),
                        },

                        // Vault lock status indicator.
                        gtk::Label {
                            add_css_class: "dim-label",
                            #[watch]
                            set_label: &model.vault_status,
                            #[watch]
                            set_tooltip_text: Some(&model.vault_status),
                            #[watch]
                            set_visible: !model.vault_status.is_empty() && model.vault_status != i18n::t("vault.none"),
                        },
                    },
                },
            }
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let AppInit { paths, config, builtin_plugins } = init;

        // CSS must be loaded after GTK is up (a display exists by init time).
        crate::theme::load(&paths.themes);
        crate::theme::apply(
            &config.appearance.scheme,
            Some(&config.appearance.accent),
            config.background(),
            config.appearance.background_dim,
            &config.appearance.font,
        );

        let start_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

        let panes = [
            build_pane(0, &start_dir, &config, &sender),
            build_pane(1, &start_dir, &config, &sender),
        ];

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(panes[0].widget()));
        paned.set_end_child(Some(panes[1].widget()));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        // Start single-pane: hide the right child.
        panes[1].widget().set_visible(false);

        // Clicking inside a pane makes it active.
        attach_focus_gesture(panes[0].widget(), 0, &sender);
        attach_focus_gesture(panes[1].widget(), 1, &sender);

        let home = HomePage::builder()
            .launch(())
            .forward(sender.input_sender(), |out| match out {
                HomeOutput::Navigate(p) => AppMsg::NavigateTo(p),
                HomeOutput::OpenSettings => AppMsg::ShowSettings,
            });

        let settings = SettingsPage::builder()
            .launch(SettingsInit {
                scheme: config.appearance.scheme.clone(),
                accent: config.appearance.accent.clone(),
                dim: config.appearance.background_dim,
                font: config.appearance.font.clone(),
                show_hidden: config.general.show_hidden,
                single_click: config.general.single_click_open,
                confirm_delete: config.general.confirm_delete,
                default_view: config.general.default_view.clone(),
                language: config.language(),
                keybinds: config.keybinds.clone(),
                terminal_shell: config.terminal.shell.clone(),
                terminal_font: config.terminal.font.clone(),
            })
            .forward(sender.input_sender(), |out| match out {
                SettingsOutput::SetScheme(s) => AppMsg::SetScheme(s),
                SettingsOutput::SetAccent(a) => AppMsg::SetAccent(a),
                SettingsOutput::SetBackground(b) => AppMsg::SetBackground(b),
                SettingsOutput::SetDim(d) => AppMsg::SetDim(d),
                SettingsOutput::SetFont(f) => AppMsg::SetFont(f),
                SettingsOutput::SetShowHidden(v) => AppMsg::SetShowHidden(v),
                SettingsOutput::SetSingleClick(v) => AppMsg::SetSingleClick(v),
                SettingsOutput::SetConfirmDelete(v) => AppMsg::SetConfirmDelete(v),
                SettingsOutput::SetDefaultView(v) => AppMsg::SetDefaultView(v),
                SettingsOutput::SetLanguage(l) => AppMsg::SetLanguage(l),
                SettingsOutput::SetKeybind(k, b) => AppMsg::SetKeybind(k, b),
                SettingsOutput::SetTerminalShell(s) => AppMsg::SetTerminalShell(s),
                SettingsOutput::SetTerminalFont(f) => AppMsg::SetTerminalFont(f),
                SettingsOutput::ResetDefaults => AppMsg::ResetDefaults,
            });

        let preview = PreviewPanel::builder().launch(()).detach();
        preview.widget().set_visible(false);

        let terminal = TerminalPanel::builder().launch(start_dir.clone()).detach();
        terminal.widget().set_visible(false);

        // The file browser page: the dual-pane area beside the preview panel.
        paned.set_hexpand(true);
        let files_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        files_box.append(&paned);
        files_box.append(preview.widget());

        // Build the content stack imperatively (the view! macro is awkward with
        // `GtkStack` named children). No transition: page switches are instant.
        let content_stack = gtk::Stack::new();
        content_stack.set_transition_type(gtk::StackTransitionType::None);
        content_stack.add_named(home.widget(), Some(AppPage::Home.name()));
        content_stack.add_named(&files_box, Some(AppPage::Files.name()));
        content_stack.add_named(settings.widget(), Some(AppPage::Settings.name()));

        let nav =
            NavBar::builder()
                .launch(start_dir.clone())
                .forward(sender.input_sender(), |out| match out {
                    NavOutput::Navigate(p) => AppMsg::NavigateTo(p),
                });

        let action_map = Arc::new(Mutex::new(ActionMap::from_keybinds(&config.keybinds)));
        install_shortcuts(&root, &sender, action_map.clone());

        // Watch the config file for external edits; reload on change.
        {
            let config_path = paths.config.join("config.toml");
            let watcher_sender = sender.clone();
            relm4::spawn(async move {
                match orca_core::watch(config_path) {
                    Ok(mut watcher) => {
                        while watcher.recv().await.is_some() {
                            watcher_sender.input(AppMsg::ReloadConfig);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "config file watcher could not start");
                    }
                }
            });
        }

        let (toolbar_row, toolbar) =
            toolbar::build(&toolbar::default_items(), sender.input_sender());

        // Build the vault manager from configured vaults.
        let vault_mgr = Arc::new(Mutex::new(
            VaultManager::new(config.vaults.clone()).unwrap_or_default(),
        ));

        // Places sidebar section (gets cleared/repopulated on bookmark changes).
        let places_box = places::build(&sender, &config.bookmarks);

        // Vault panel section (persistent; not cleared with bookmarks).
        let vault_panel = VaultPanel::builder()
            .launch(VaultPanelInit { mgr: vault_mgr.clone() })
            .forward(sender.input_sender(), |out| match out {
                VaultPanelOutput::Navigate(p) => AppMsg::VaultNavigate(p),
                VaultPanelOutput::AutoLocked(n) => AppMsg::VaultAutoLocked(n),
                VaultPanelOutput::StatusText(t) => AppMsg::VaultStatus(t),
                VaultPanelOutput::ConfigsChanged(c) => AppMsg::VaultConfigsChanged(c),
            });

        // Outer sidebar container: places on top, vault panel below.
        let sidebar_outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_outer.set_width_request(216);
        sidebar_outer.add_css_class("orca-places");
        sidebar_outer.append(&places_box);
        sidebar_outer.append(vault_panel.widget());

        let mut state: [PaneUiState; 2] = Default::default();
        state[0].dir = start_dir.clone();
        state[1].dir = start_dir.clone();

        let mut plugin_mgr = PluginManager::new();
        plugin_mgr.discover_and_load(&paths.plugins, &builtin_plugins);
        tracing::info!(count = plugin_mgr.list().len(), "plugins loaded");

        let model = AppModel {
            paths,
            config,
            vault_mgr,
            panes,
            home,
            settings,
            vault_panel,
            vault_status: i18n::t("vault.none"),
            places_box: places_box.clone(),
            preview,
            preview_visible: false,
            terminal,
            terminal_visible: false,
            page: AppPage::Home,
            active: 0,
            dual: false,
            state,
            nav,
            title: i18n::t("home.title"),
            status: i18n::tf("status.items", &[("n", "0")]),
            current_path: start_dir.to_string_lossy().into_owned(),
            toolbar,
            properties: None,
            bulk_rename: None,
            permissions: None,
            archive_view: None,
            mount_manager: None,
            disk_usage: None,
            network_browser: None,
            sync_binding: None,
            action_map,
            plugin_mgr,
        };

        let nav_widget = model.nav.widget();
        let terminal_widget = model.terminal.widget();
        let toolbar_row = &toolbar_row;
        let content_stack = &content_stack;
        let sidebar = &sidebar_outer;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AppMsg::ShowHome => {
                self.page = AppPage::Home;
                self.title = i18n::t("home.title");
                self.home.emit(HomeInput::Refresh);
            }
            AppMsg::ShowSettings => {
                self.page = AppPage::Settings;
                self.title = i18n::t("settings.title");
            }
            AppMsg::Activate(idx) => {
                if idx != self.active && idx < self.panes.len() {
                    self.active = idx;
                    self.refresh_chrome();
                }
            }
            AppMsg::Up => self.active_pane().emit(SideInput::Forward(PaneInput::GoUp)),
            AppMsg::Back => self.active_pane().emit(SideInput::Forward(PaneInput::Back)),
            AppMsg::Forward => self
                .active_pane()
                .emit(SideInput::Forward(PaneInput::Forward)),
            AppMsg::NavigateTo(path) => {
                self.page = AppPage::Files;
                self.active_pane()
                    .emit(SideInput::Forward(PaneInput::Navigate(path)));
            }
            AppMsg::TogglePathEntry => self.nav.emit(NavInput::ToggleEntry),
            AppMsg::Reload => self
                .active_pane()
                .emit(SideInput::Forward(PaneInput::Reload)),
            AppMsg::NewFolder => self
                .active_pane()
                .emit(SideInput::Forward(PaneInput::NewFolder)),
            AppMsg::NewFile => self
                .active_pane()
                .emit(SideInput::Forward(PaneInput::NewFile)),
            AppMsg::ToggleSearch => {
                self.page = AppPage::Files;
                self.active_pane()
                    .emit(SideInput::Forward(PaneInput::ToggleSearch));
            }
            AppMsg::SetViewMode(mode) => self
                .active_pane()
                .emit(SideInput::Forward(PaneInput::SetViewMode(mode))),
            AppMsg::NewTab => self.active_pane().emit(SideInput::NewTab),
            AppMsg::CloseTab => self.active_pane().emit(SideInput::CloseCurrent),
            AppMsg::NextTab => self.active_pane().emit(SideInput::Next),
            AppMsg::PrevTab => self.active_pane().emit(SideInput::Prev),
            AppMsg::DuplicateTab => self.active_pane().emit(SideInput::Duplicate),
            AppMsg::Preview(idx, entry) => {
                // Only the active pane drives the preview, so a background pane's
                // selection changes do not fight the foreground one.
                if idx == self.active {
                    if let Some(e) = &entry {
                        self.plugin_mgr.fire_file_select(&e.path);
                    }
                    self.preview.emit(PreviewInput::Show(entry));
                }
            }
            AppMsg::TogglePreview => {
                self.preview_visible = !self.preview_visible;
                self.preview.widget().set_visible(self.preview_visible);
            }
            AppMsg::ToggleTerminal => {
                self.terminal_visible = !self.terminal_visible;
                self.terminal.widget().set_visible(self.terminal_visible);
                if self.terminal_visible {
                    // Sync the shell to the active directory on reveal.
                    self.terminal
                        .emit(TerminalInput::Cd(self.state[self.active].dir.clone()));
                }
            }
            AppMsg::ToggleDual => self.toggle_dual(),
            AppMsg::ToggleSync => self.toggle_sync(),
            AppMsg::Transfer { move_it } => {
                let dest = self.state[self.other()].dir.clone();
                let msg = if move_it {
                    PaneInput::MoveTo(dest)
                } else {
                    PaneInput::CopyTo(dest)
                };
                self.active_pane().emit(SideInput::Forward(msg));
            }
            AppMsg::SetBackground(image) => self.set_background(image),
            AppMsg::SetScheme(scheme) => self.set_scheme(scheme),
            AppMsg::SetAccent(accent) => self.set_accent(accent),
            AppMsg::SetDim(dim) => self.set_dim(dim),
            AppMsg::SetFont(font) => self.set_font(font),
            AppMsg::SetShowHidden(v) => self.set_show_hidden(v),
            AppMsg::SetSingleClick(v) => self.set_single_click(v),
            AppMsg::SetConfirmDelete(v) => self.set_confirm_delete(v),
            AppMsg::SetDefaultView(v) => self.set_default_view(v),
            AppMsg::SetLanguage(lang) => self.set_language(lang),
            AppMsg::DirChanged(idx, path) => {
                self.state[idx].dir = path.clone();
                // Fire plugin hook on dir change and push updated badges to both panes.
                self.plugin_mgr.fire_dir_change(&path);
                self.push_plugin_badges();
                if idx == self.active {
                    self.current_path = path.to_string_lossy().into_owned();
                    self.nav.emit(NavInput::SetPath(path.clone()));
                    // The home page keeps its own title; only the file browser
                    // reflects the directory name in the window title.
                    if self.page == AppPage::Files {
                        self.title = dir_title(&path);
                    }
                    // Keep a visible terminal's shell in step with the browser.
                    if self.terminal_visible {
                        self.terminal.emit(TerminalInput::Cd(path.clone()));
                    }
                }
            }
            AppMsg::History {
                idx,
                can_back,
                can_forward,
            } => {
                self.state[idx].can_back = can_back;
                self.state[idx].can_forward = can_forward;
                if idx == self.active {
                    self.toolbar.set_history(can_back, can_forward);
                }
            }
            AppMsg::Selection {
                idx,
                selected,
                bytes,
                total,
            } => {
                let text = if selected > 0 {
                    i18n::tf(
                        "status.selected",
                        &[("n", &selected.to_string()), ("size", &format::size(bytes))],
                    )
                } else {
                    i18n::tf("status.items", &[("n", &total.to_string())])
                };
                self.state[idx].status = text.clone();
                if idx == self.active {
                    self.status = text;
                }
            }
            AppMsg::ViewModeChanged(idx, mode) => {
                self.state[idx].mode = mode;
                if idx == self.active {
                    self.toolbar.set_mode(mode);
                }
            }
            AppMsg::OpenFile(path) => open_default(&path),
            AppMsg::ShowProperties(entry) => {
                let controller = Properties::builder().launch(entry).detach();
                controller.widget().present();
                self.properties = Some(controller);
            }
            AppMsg::ShowPermissions(entry) => {
                use std::os::unix::fs::MetadataExt;
                let (mode, uid, gid) = std::fs::metadata(&entry.path)
                    .map(|m| (m.mode() & 0o777, m.uid(), m.gid()))
                    .unwrap_or((0o644, 0, 0));
                let is_dir = entry.kind.is_dir();
                let ctrl = PermissionsDialog::builder()
                    .launch(PermissionsInit {
                        path: entry.path,
                        mode,
                        is_dir,
                        uid,
                        gid,
                    })
                    .detach();
                ctrl.widget().present();
                self.permissions = Some(ctrl);
            }
            AppMsg::BulkRename(paths) => {
                let pane_widget = self.active_pane().widget().clone();
                let sender_clone = sender.clone();
                let mut builder = BulkRenameDialog::builder();
                if let Some(win) = dialogs::window_of(&pane_widget) {
                    builder = builder.transient_for(&win);
                }
                let ctrl = builder
                    .launch(paths)
                    .forward(sender_clone.input_sender(), move |out| match out {
                        BulkRenameOutput::Done => AppMsg::Reload,
                    });
                ctrl.widget().present();
                self.bulk_rename = Some(ctrl);
            }
            AppMsg::ExtractTo(paths) => {
                let window = dialogs::window_of(self.active_pane().widget());
                let dialog = gtk::FileDialog::builder()
                    .title(i18n::t("ctx.extract_to"))
                    .modal(true)
                    .build();
                // spin off extract after the user picks a folder
                dialog.select_folder(
                    window.as_ref(),
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(folder) = result {
                            if let Some(dest) = folder.path() {
                                for archive in &paths {
                                    let archive = archive.clone();
                                    let dest = dest.clone();
                                    relm4::spawn(async move {
                                        if let Err(e) =
                                            orca_core::extract(archive, dest, None).await
                                        {
                                            tracing::warn!(error = %e, "extract failed");
                                        }
                                    });
                                }
                            }
                        }
                    },
                );
            }
            AppMsg::BrowseArchive(path) => {
                let ctrl = ArchiveView::builder().launch(path).detach();
                ctrl.widget().present();
                self.archive_view = Some(ctrl);
            }
            AppMsg::OpenDiskUsage(path) => {
                let sender_clone = sender.clone();
                let ctrl = DiskUsageDialog::builder()
                    .launch(path)
                    .forward(sender_clone.input_sender(), |out| match out {
                        DiskUsageOutput::Navigate(p) => AppMsg::NavigateTo(p),
                    });
                ctrl.widget().present();
                self.disk_usage = Some(ctrl);
            }
            AppMsg::OpenMountManager => {
                let ctrl = MountManager::builder().launch(()).detach();
                // Start background event listener so plug/unplug auto-refreshes.
                let event_sender = ctrl.sender().clone();
                relm4::spawn(crate::mount_manager::start_event_listener(event_sender));
                ctrl.widget().present();
                self.mount_manager = Some(ctrl);
            }
            AppMsg::OpenNetworkBrowser => {
                let ctrl = NetworkBrowser::builder().launch(()).detach();
                ctrl.widget().present();
                self.network_browser = Some(ctrl);
            }
            AppMsg::AddToVault(paths) => {
                // Delegate to the vault panel which knows which vault is active.
                self.vault_panel.emit(VaultPanelInput::AddFiles(paths));
            }
            AppMsg::VaultNavigate(path) => {
                self.page = AppPage::Files;
                self.active_pane()
                    .emit(SideInput::Forward(PaneInput::Navigate(path)));
            }
            AppMsg::VaultAutoLocked(name) => {
                self.plugin_mgr.fire_vault_lock(&name);
                self.vault_status = i18n::tf("vault.status_locked_sb", &[("name", &name)]);
                tracing::info!(vault = %name, "vault auto-locked");
            }
            AppMsg::VaultStatus(text) => {
                self.vault_status = text;
            }
            AppMsg::VaultConfigsChanged(configs) => {
                self.config.vaults = configs;
                self.persist();
            }
            AppMsg::AddBookmark(path) => {
                if self.config.add_bookmark(path) {
                    self.persist();
                    self.refresh_sidebar(&sender);
                }
            }
            AppMsg::RemoveBookmark(path) => {
                if self.config.remove_bookmark(&path) {
                    self.persist();
                    self.refresh_sidebar(&sender);
                }
            }
            AppMsg::Error(e) => self.status = e,
            AppMsg::Rename => {
                self.active_pane()
                    .emit(SideInput::Forward(PaneInput::BeginRename));
            }
            AppMsg::ToggleHidden => {
                let show = !self.config.general.show_hidden;
                self.set_show_hidden(show);
            }
            AppMsg::SetKeybind(key, binding) => {
                self.set_keybind(key, binding);
            }
            AppMsg::SetTerminalShell(shell) => {
                self.config.terminal.shell = shell;
                self.persist();
            }
            AppMsg::SetTerminalFont(font) => {
                self.config.terminal.font = font;
                self.persist();
            }
            AppMsg::ResetDefaults => {
                self.config = Config::default();
                self.apply_theme();
                self.persist();
                *self.action_map.lock().unwrap() =
                    ActionMap::from_keybinds(&self.config.keybinds);
            }
            AppMsg::ReloadConfig => {
                if let Ok(new_cfg) =
                    std::fs::read_to_string(self.paths.config.join("config.toml"))
                        .and_then(|t| toml::from_str::<Config>(&t).map_err(|e| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
                        }))
                {
                    self.config = new_cfg;
                    self.apply_theme();
                    *self.action_map.lock().unwrap() =
                        ActionMap::from_keybinds(&self.config.keybinds);
                    tracing::info!("config reloaded from disk");
                }
            }
        }
    }
}

impl AppModel {
    /// Push the current plugin badge map to both pane sides.
    fn push_plugin_badges(&self) {
        use crate::pane::PaneInput;
        use crate::side::SideInput;
        let map = self.plugin_mgr.badge_map.lock().unwrap();
        let badges: std::collections::HashMap<std::path::PathBuf, (String, String)> = map
            .iter()
            .map(|(p, b)| (p.clone(), (b.text.clone(), b.color.clone())))
            .collect();
        drop(map);
        for pane in &self.panes {
            pane.emit(SideInput::Forward(PaneInput::UpdatePluginBadges(badges.clone())));
        }
    }

    /// The controller of the currently active tabbed side.
    fn active_pane(&self) -> &Controller<TabbedPane> {
        &self.panes[self.active]
    }

    /// The index of the inactive pane.
    fn other(&self) -> usize {
        1 - self.active
    }

    /// Refresh the toolbar/nav/status bar from the active pane's cached state.
    fn refresh_chrome(&mut self) {
        let st = self.state[self.active].clone();
        self.title = dir_title(&st.dir);
        self.current_path = st.dir.to_string_lossy().into_owned();
        self.status = st.status;
        self.toolbar.set_history(st.can_back, st.can_forward);
        self.toolbar.set_mode(st.mode);
        self.nav.emit(NavInput::SetPath(st.dir));
    }

    /// Regenerate the runtime theme from the current appearance config.
    fn apply_theme(&self) {
        crate::theme::apply(
            &self.config.appearance.scheme,
            Some(&self.config.appearance.accent),
            self.config.background(),
            self.config.appearance.background_dim,
            &self.config.appearance.font,
        );
    }

    /// Rebuild the Places section of the sidebar from the current bookmark set.
    fn refresh_sidebar(&self, sender: &ComponentSender<Self>) {
        places::populate(&self.places_box, sender, &self.config.bookmarks);
    }

    /// Persist the current config, logging (not surfacing) any I/O failure.
    fn persist(&self) {
        if let Err(e) = self.config.save(&self.paths) {
            tracing::warn!(error = %e, "failed to persist config");
        }
    }

    /// Apply and persist a new (or cleared) background image.
    fn set_background(&mut self, image: Option<PathBuf>) {
        self.config.appearance.background_image = image;
        self.apply_theme();
        self.persist();
    }

    /// Apply and persist a new color scheme.
    fn set_scheme(&mut self, scheme: String) {
        self.config.appearance.scheme = scheme;
        self.apply_theme();
        self.persist();
    }

    /// Apply and persist a new accent color.
    fn set_accent(&mut self, accent: String) {
        self.config.appearance.accent = accent;
        self.apply_theme();
        self.persist();
    }

    /// Apply and persist a new background scrim strength.
    fn set_dim(&mut self, dim: f64) {
        self.config.appearance.background_dim = dim.clamp(0.0, 1.0);
        self.apply_theme();
        self.persist();
    }

    /// Apply and persist a new UI / list font.
    fn set_font(&mut self, font: String) {
        self.config.appearance.font = font;
        self.apply_theme();
        self.persist();
    }

    /// Apply and persist hidden-file visibility across every tab of both sides.
    fn set_show_hidden(&mut self, show: bool) {
        self.config.general.show_hidden = show;
        for pane in &self.panes {
            pane.emit(SideInput::SetAllHidden(show));
        }
        self.persist();
    }

    /// Apply and persist single-click activation across every tab of both sides.
    fn set_single_click(&mut self, single: bool) {
        self.config.general.single_click_open = single;
        for pane in &self.panes {
            pane.emit(SideInput::SetAllSingleClick(single));
        }
        self.persist();
    }

    /// Apply and persist trash-confirmation across every tab of both sides.
    fn set_confirm_delete(&mut self, confirm: bool) {
        self.config.general.confirm_delete = confirm;
        for pane in &self.panes {
            pane.emit(SideInput::SetAllConfirmDelete(confirm));
        }
        self.persist();
    }

    /// Persist the default view for new panes (existing panes are unchanged).
    fn set_default_view(&mut self, view: String) {
        self.config.general.default_view = view;
        self.persist();
    }

    /// Update one keybind by config key name, rebuild the ActionMap, and persist.
    fn set_keybind(&mut self, config_key: String, binding: String) {
        let kb = &mut self.config.keybinds;
        match config_key.as_str() {
            "new_tab" => kb.new_tab = binding,
            "close_tab" => kb.close_tab = binding,
            "duplicate_tab" => kb.duplicate_tab = binding,
            "toggle_hidden" => kb.toggle_hidden = binding,
            "toggle_dual_pane" => kb.toggle_dual_pane = binding,
            "open_terminal" => kb.open_terminal = binding,
            "copy_to_pane" => kb.copy_to_pane = binding,
            "move_to_pane" => kb.move_to_pane = binding,
            "rename" => kb.rename = binding,
            "search" => kb.search = binding,
            "path_entry" => kb.path_entry = binding,
            "back" => kb.back = binding,
            "forward" => kb.forward = binding,
            "up" => kb.up = binding,
            "view_list" => kb.view_list = binding,
            "view_icon" => kb.view_icon = binding,
            "view_detail" => kb.view_detail = binding,
            "vault_add" => kb.vault_add = binding,
            _ => {
                tracing::warn!(key = %config_key, "set_keybind: unknown action key");
                return;
            }
        }
        *self.action_map.lock().unwrap() = ActionMap::from_keybinds(&self.config.keybinds);
        self.persist();
    }

    /// Persist a new interface language (takes effect on the next launch, since
    /// the loaded translation table is process-global).
    fn set_language(&mut self, lang: Lang) {
        if self.config.language() == lang {
            return;
        }
        self.config.general.language = lang.code().to_owned();
        self.persist();
    }

    /// Show or hide the second pane.
    fn toggle_dual(&mut self) {
        self.dual = !self.dual;
        self.panes[1].widget().set_visible(self.dual);
        if !self.dual && self.active == 1 {
            self.active = 0;
            self.refresh_chrome();
        }
    }

    /// Enable or disable synchronized vertical scrolling between the panes.
    fn toggle_sync(&mut self) {
        if let Some(binding) = self.sync_binding.take() {
            binding.unbind();
            return;
        }
        let (Some(a), Some(b)) = (
            pane_vadjustment(self.panes[0].widget()),
            pane_vadjustment(self.panes[1].widget()),
        ) else {
            return;
        };
        let binding = a
            .bind_property("value", &b, "value")
            .bidirectional()
            .sync_create()
            .build();
        self.sync_binding = Some(binding);
    }

    /// The resolved XDG paths (used by later phases for plugins/vault wiring).
    #[allow(dead_code)]
    #[must_use]
    pub fn paths(&self) -> &XdgPaths {
        &self.paths
    }
}

/// Build a tabbed browser side, seeded from config and forwarding the active
/// tab's outputs tagged with the side index.
fn build_pane(
    idx: usize,
    dir: &Path,
    config: &Config,
    sender: &ComponentSender<AppModel>,
) -> Controller<TabbedPane> {
    TabbedPane::builder()
        .launch(SideInit {
            dir: dir.to_path_buf(),
            show_hidden: config.general.show_hidden,
            single_click: config.general.single_click_open,
            confirm_delete: config.general.confirm_delete,
            mode: config.default_view(),
        })
        .forward(sender.input_sender(), move |out| match out {
            SideOutput::Event(out) => map_pane(idx, out),
        })
}

/// Map a pane output to the tagged application message.
fn map_pane(idx: usize, out: PaneOutput) -> AppMsg {
    match out {
        PaneOutput::DirChanged(p) => AppMsg::DirChanged(idx, p),
        PaneOutput::Selection {
            selected,
            bytes,
            total,
        } => AppMsg::Selection {
            idx,
            selected,
            bytes,
            total,
        },
        PaneOutput::ViewModeChanged(m) => AppMsg::ViewModeChanged(idx, m),
        PaneOutput::History {
            can_back,
            can_forward,
        } => AppMsg::History {
            idx,
            can_back,
            can_forward,
        },
        PaneOutput::Preview(e) => AppMsg::Preview(idx, e),
        PaneOutput::OpenFile(p) => AppMsg::OpenFile(p),
        PaneOutput::ShowProperties(e) => AppMsg::ShowProperties(e),
        PaneOutput::ShowPermissions(e) => AppMsg::ShowPermissions(e),
        PaneOutput::AddToVault(p) => AppMsg::AddToVault(p),
        PaneOutput::AddBookmark(p) => AppMsg::AddBookmark(p),
        PaneOutput::BulkRename(p) => AppMsg::BulkRename(p),
        PaneOutput::DiskUsage(p) => AppMsg::OpenDiskUsage(p),
        PaneOutput::ExtractTo(p) => AppMsg::ExtractTo(p),
        PaneOutput::BrowseArchive(p) => AppMsg::BrowseArchive(p),
        PaneOutput::Error(e) => AppMsg::Error(e),
    }
}

/// Attach a capture-phase click gesture that marks a pane active on any click.
fn attach_focus_gesture(
    widget: &impl IsA<gtk::Widget>,
    idx: usize,
    sender: &ComponentSender<AppModel>,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(0);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sender = sender.clone();
    gesture.connect_pressed(move |_, _, _, _| sender.input(AppMsg::Activate(idx)));
    widget.add_controller(gesture);
}

/// Install window-level keyboard shortcuts using the config-driven `ActionMap`.
fn install_shortcuts(
    root: &gtk::ApplicationWindow,
    sender: &ComponentSender<AppModel>,
    action_map: Arc<Mutex<ActionMap>>,
) {
    let sender = sender.clone();
    let key = gtk::EventControllerKey::new();
    key.connect_key_pressed(move |_, keyval, _, state| {
        let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        // Ctrl+Tab / Ctrl+Shift+Tab are fixed tab-cycle shortcuts not in the
        // keybind config (they conflict with normal text-box behaviour less).
        if keyval == gtk::gdk::Key::Tab && ctrl {
            sender.input(AppMsg::NextTab);
            return gtk::glib::Propagation::Stop;
        }
        if keyval == gtk::gdk::Key::ISO_Left_Tab && ctrl {
            sender.input(AppMsg::PrevTab);
            return gtk::glib::Propagation::Stop;
        }
        // F7 (preview) is also fixed — not in keybind config.
        if keyval == gtk::gdk::Key::F7 {
            sender.input(AppMsg::TogglePreview);
            return gtk::glib::Propagation::Stop;
        }

        let map = action_map.lock().unwrap();
        if let Some(action) = map.action_for(keyval, state) {
            if let Some(msg) = action_to_msg(action) {
                drop(map);
                sender.input(msg);
                return gtk::glib::Propagation::Stop;
            }
        }
        gtk::glib::Propagation::Proceed
    });
    root.add_controller(key);
}

/// Translate an `Action` to an `AppMsg`. Returns `None` for actions that need
/// richer context (e.g. vault_add requires a file selection).
fn action_to_msg(action: Action) -> Option<AppMsg> {
    let msg = match action {
        Action::NewTab => AppMsg::NewTab,
        Action::CloseTab => AppMsg::CloseTab,
        Action::DuplicateTab => AppMsg::DuplicateTab,
        Action::ToggleHidden => AppMsg::ToggleHidden,
        Action::ToggleDualPane => AppMsg::ToggleDual,
        Action::OpenTerminal => AppMsg::ToggleTerminal,
        Action::CopyToPane => AppMsg::Transfer { move_it: false },
        Action::MoveToPane => AppMsg::Transfer { move_it: true },
        Action::Rename => AppMsg::Rename,
        Action::Search => AppMsg::ToggleSearch,
        Action::PathEntry => AppMsg::TogglePathEntry,
        Action::Back => AppMsg::Back,
        Action::Forward => AppMsg::Forward,
        Action::Up => AppMsg::Up,
        Action::ViewList => AppMsg::SetViewMode(ViewMode::List),
        Action::ViewIcon => AppMsg::SetViewMode(ViewMode::Icon),
        Action::ViewDetail => AppMsg::SetViewMode(ViewMode::Detail),
        Action::VaultAdd => return None, // requires active file selection
    };
    Some(msg)
}

/// Locate the vertical adjustment of the first `ScrolledWindow` inside a pane.
fn pane_vadjustment(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Adjustment> {
    find_scrolled(widget.as_ref()).map(|s| s.vadjustment())
}

/// Depth-first search for the first descendant `ScrolledWindow`.
fn find_scrolled(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    if let Ok(scrolled) = widget.clone().downcast::<gtk::ScrolledWindow>() {
        return Some(scrolled);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_scrolled(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// A window-title string for a directory path: its final component, or the path
/// itself for the filesystem root.
fn dir_title(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// Open a path with the system default application via the GIO launcher.
fn open_default(path: &Path) {
    let file = gtk::gio::File::for_path(path);
    let uri = file.uri();
    if let Err(e) =
        gtk::gio::AppInfo::launch_default_for_uri(&uri, gtk::gio::AppLaunchContext::NONE)
    {
        tracing::warn!(uri = %uri, error = %e, "failed to open file");
    }
}
