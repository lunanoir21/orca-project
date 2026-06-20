//! User configuration (Phase 7.1, brought forward for language + appearance).
//!
//! Configuration lives at `$XDG_CONFIG_HOME/orca/config.toml`. Every field has a
//! shipped default (mirrored in `config/default.toml`), so a missing file or a
//! partially-specified file is valid: absent keys keep their defaults. The
//! struct round-trips through `serde`, so the appearance picker on the home page
//! can mutate one field and persist the whole document with [`Config::save`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use orca_vault::VaultConfig;

use crate::i18n::Lang;
use crate::pane::ViewMode;
use crate::toolbar::{self, ToolbarItem};
use crate::xdg::XdgPaths;

/// The full user configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// General behaviour.
    pub general: General,
    /// Appearance and theming.
    pub appearance: Appearance,
    /// Keyboard shortcuts for all application actions.
    pub keybinds: Keybinds,
    /// Embedded terminal settings.
    pub terminal: Terminal,
    /// User-defined bookmarked directories, shown in the Places sidebar.
    pub bookmarks: Vec<PathBuf>,
    /// Configured vaults, each with path and auto-lock settings.
    pub vaults: Vec<VaultConfig>,
    /// Toolbar layout (which items, in what order).
    pub toolbar: ToolbarConfig,
}

/// `[toolbar]` — the customizable toolbar layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarConfig {
    /// Items shown in the toolbar, in display order.
    pub items: Vec<ToolbarItem>,
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self {
            items: toolbar::default_items(),
        }
    }
}

/// `[keybinds]` — keyboard shortcut strings (format: `"Ctrl+T"`, `"F3"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybinds {
    /// Open a new tab.
    pub new_tab: String,
    /// Close the current tab.
    pub close_tab: String,
    /// Duplicate the current tab.
    pub duplicate_tab: String,
    /// Toggle hidden files.
    pub toggle_hidden: String,
    /// Toggle dual-pane mode.
    pub toggle_dual_pane: String,
    /// Toggle the embedded terminal.
    pub open_terminal: String,
    /// Copy selection to the other pane (F5).
    pub copy_to_pane: String,
    /// Move selection to the other pane (F6).
    pub move_to_pane: String,
    /// Rename the selected file.
    pub rename: String,
    /// Toggle the search bar.
    pub search: String,
    /// Toggle the path entry bar.
    pub path_entry: String,
    /// Navigate back.
    pub back: String,
    /// Navigate forward.
    pub forward: String,
    /// Navigate up to parent directory.
    pub up: String,
    /// Switch to list view.
    pub view_list: String,
    /// Switch to icon view.
    pub view_icon: String,
    /// Switch to detail view.
    pub view_detail: String,
    /// Add selected files to vault.
    pub vault_add: String,
    /// Show the Quick Look fullscreen preview overlay for the selected file.
    pub quick_look: String,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            new_tab: "Ctrl+T".to_owned(),
            close_tab: "Ctrl+W".to_owned(),
            duplicate_tab: "Ctrl+D".to_owned(),
            toggle_hidden: "Ctrl+H".to_owned(),
            toggle_dual_pane: "F3".to_owned(),
            open_terminal: "F4".to_owned(),
            copy_to_pane: "F5".to_owned(),
            move_to_pane: "F6".to_owned(),
            rename: "F2".to_owned(),
            search: "Ctrl+F".to_owned(),
            path_entry: "Ctrl+L".to_owned(),
            back: "Alt+Left".to_owned(),
            forward: "Alt+Right".to_owned(),
            up: "Alt+Up".to_owned(),
            view_list: "Ctrl+1".to_owned(),
            view_icon: "Ctrl+2".to_owned(),
            view_detail: "Ctrl+3".to_owned(),
            vault_add: "Ctrl+Shift+V".to_owned(),
            quick_look: "Space".to_owned(),
        }
    }
}

/// `[terminal]` — embedded terminal options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// Shell binary path. Empty string falls back to `$SHELL` or `/bin/sh`.
    pub shell: String,
    /// Terminal font (Pango description).
    pub font: String,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            shell: String::new(),
            font: "JetBrains Mono 11".to_owned(),
        }
    }
}

impl Config {
    /// Add `path` to the bookmarks if not already present. Returns whether it
    /// was added.
    pub fn add_bookmark(&mut self, path: PathBuf) -> bool {
        if self.bookmarks.contains(&path) {
            return false;
        }
        self.bookmarks.push(path);
        true
    }

    /// Remove `path` from the bookmarks. Returns whether anything changed.
    pub fn remove_bookmark(&mut self, path: &Path) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|p| p != path);
        self.bookmarks.len() != before
    }
}

/// `[general]` — behavioural options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Interface language code (`"tr"`, `"en"`).
    pub language: String,
    /// Show dot-prefixed hidden files.
    pub show_hidden: bool,
    /// Open items on a single click.
    pub single_click_open: bool,
    /// Confirm before trashing files.
    pub confirm_delete: bool,
    /// Default view mode for new tabs (`"list"`, `"icon"`, `"detail"`).
    pub default_view: String,
    /// Show the toolbar row.
    pub toolbar_visible: bool,
    /// Show the breadcrumb/path bar.
    pub breadcrumb_visible: bool,
    /// Page shown on launch (`"files"` or `"home"`).
    pub start_page: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: "tr".to_owned(),
            show_hidden: false,
            single_click_open: false,
            confirm_delete: true,
            default_view: "list".to_owned(),
            toolbar_visible: true,
            breadcrumb_visible: true,
            start_page: "files".to_owned(),
        }
    }
}

/// `[appearance]` — look and feel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Color scheme id (see [`crate::theme::SCHEMES`]).
    pub scheme: String,
    /// Optional accent color override (hex `#rrggbb`); empty uses the scheme's
    /// own accent.
    pub accent: String,
    /// UI / list font (Pango font description).
    pub font: String,
    /// Icon size in pixels.
    pub icon_size: u32,
    /// Optional background image shown behind the whole window. `None`/empty
    /// disables the image and uses the flat theme background.
    pub background_image: Option<PathBuf>,
    /// Darkening applied over the background image, 0.0 (none) – 1.0 (black).
    pub background_dim: f64,
    /// Multiplier applied to every panel/chrome background alpha (sidebar,
    /// toolbar, preview, status bar, ...), 0.3 (very see-through) – 1.0 (as
    /// designed). See [`crate::theme::apply`].
    pub panel_opacity: f64,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            scheme: "graphite".to_owned(),
            accent: String::new(),
            font: "JetBrains Mono 11".to_owned(),
            icon_size: 32,
            background_image: None,
            background_dim: 0.45,
            panel_opacity: 1.0,
        }
    }
}

impl Config {
    /// Load the user config, falling back to defaults for a missing file or any
    /// field it omits. A malformed file is logged and treated as absent so the
    /// application always starts.
    #[must_use]
    pub fn load(paths: &XdgPaths) -> Self {
        let file = paths.config.join("config.toml");
        match std::fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(path = %file.display(), error = %e, "invalid config; using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Persist the configuration to `config.toml`, creating the config directory
    /// if needed.
    ///
    /// # Errors
    /// Returns an I/O error if the directory cannot be created or the file
    /// cannot be written.
    pub fn save(&self, paths: &XdgPaths) -> std::io::Result<()> {
        std::fs::create_dir_all(&paths.config)?;
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(paths.config.join("config.toml"), text)
    }

    /// The resolved interface language.
    #[must_use]
    pub fn language(&self) -> Lang {
        Lang::from_code(&self.general.language)
    }

    /// The default view mode for new panes.
    #[must_use]
    pub fn default_view(&self) -> ViewMode {
        match self.general.default_view.as_str() {
            "icon" => ViewMode::Icon,
            "detail" => ViewMode::Detail,
            _ => ViewMode::List,
        }
    }

    /// The configured background image, if it exists on disk.
    #[must_use]
    pub fn background(&self) -> Option<&Path> {
        self.appearance
            .background_image
            .as_deref()
            .filter(|p| p.exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_default_toml_matches_schema() {
        // config/default.toml is documentation, not something Orca reads at
        // runtime (see the module doc comment) — but it drifts silently if
        // nobody checks it against the real `Config` shape. Parse the real
        // shipped file here so a renamed/removed field fails CI instead of
        // just misleading whoever reads the file next.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/default.toml");
        let text = std::fs::read_to_string(&path).expect("read config/default.toml");
        let cfg: Config = toml::from_str(&text).expect("config/default.toml must parse as Config");
        assert_eq!(cfg.appearance.scheme, "graphite");
        assert_eq!(cfg.keybinds.quick_look, "Space");
    }

    #[test]
    fn defaults_are_turkish_list() {
        let cfg = Config::default();
        assert_eq!(cfg.language(), Lang::Turkish);
        assert_eq!(cfg.default_view(), ViewMode::List);
        assert!(cfg.background().is_none());
    }

    #[test]
    fn partial_toml_keeps_defaults() {
        let cfg: Config = toml::from_str("[general]\nlanguage = \"en\"\n").unwrap();
        assert_eq!(cfg.language(), Lang::English);
        // Unspecified appearance falls back to the default dim.
        assert!((cfg.appearance.background_dim - 0.45).abs() < f64::EPSILON);
        assert!(cfg.general.confirm_delete);
        // Unspecified toolbar falls back to the shipped default layout.
        assert_eq!(cfg.toolbar.items, toolbar::default_items());
        assert!(cfg.general.toolbar_visible);
        assert!(cfg.general.breadcrumb_visible);
    }

    #[test]
    fn toolbar_items_roundtrip_through_config() {
        let mut cfg = Config::default();
        cfg.toolbar.items = vec![ToolbarItem::Back, ToolbarItem::Search];
        cfg.general.toolbar_visible = false;
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(
            back.toolbar.items,
            vec![ToolbarItem::Back, ToolbarItem::Search]
        );
        assert!(!back.general.toolbar_visible);
    }

    #[test]
    fn start_page_defaults_to_files_and_roundtrips() {
        let cfg = Config::default();
        assert_eq!(cfg.general.start_page, "files");

        let mut cfg = Config::default();
        cfg.general.start_page = "home".to_owned();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.general.start_page, "home");
    }

    #[test]
    fn roundtrips_through_toml() {
        let mut cfg = Config::default();
        cfg.appearance.background_image = Some(PathBuf::from("/tmp/bg.jpg"));
        cfg.general.language = "en".to_owned();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.general.language, "en");
        assert_eq!(
            back.appearance.background_image,
            Some(PathBuf::from("/tmp/bg.jpg"))
        );
    }
}
