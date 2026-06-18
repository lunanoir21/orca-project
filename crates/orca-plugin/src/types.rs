//! Core types shared across the plugin system.

use std::collections::HashMap;
use std::path::PathBuf;

/// Metadata parsed from a plugin's header comment block.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    /// Stable identifier — the filename without `.lua`.
    pub id: String,
    /// Human-readable name (`@name`).
    pub name: String,
    /// Semver-ish version string (`@version`).
    pub version: String,
    /// Author name or contact (`@author`).
    pub author: String,
    /// One-line description (`@description`).
    pub description: String,
    /// Absolute path to the `.lua` file.
    pub path: PathBuf,
}

/// A badge set on a specific file path by a plugin via `orca.set_badge`.
#[derive(Debug, Clone)]
pub struct Badge {
    /// Short text shown on the badge (e.g. `"M"`, `"?"`, `"✓"`).
    pub text: String,
    /// CSS color string (e.g. `"#f5a97f"`, `"red"`).
    pub color: String,
}

/// Map from file path to the current badge for that path.
///
/// Written by plugins via `orca.set_badge`; read by the GUI when binding
/// list/grid cells.
pub type BadgeMap = HashMap<PathBuf, Badge>;

/// Runtime state of a loaded plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Running normally.
    Enabled,
    /// Manually disabled by the user (persisted in config).
    Disabled,
    /// Failed to load or panicked; message holds the diagnostic.
    Error(String),
}
