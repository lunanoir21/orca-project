//! Plugin discovery, metadata parsing, and single-plugin loading.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::api::{register_api, ApiState};
use crate::error::PluginError;
use crate::sandbox::create_sandbox;
use crate::types::{BadgeMap, PluginMeta, PluginState};

/// A loaded plugin: its Lua state, hook registry, metadata and runtime status.
pub struct LoadedPlugin {
    /// Parsed header metadata.
    pub meta: PluginMeta,
    /// The sandboxed Lua state (not `Send` — always on GTK main thread).
    pub lua: mlua::Lua,
    /// Hook keys and log lines accumulated during and after load.
    pub state: Rc<RefCell<ApiState>>,
    /// Whether the plugin is currently enabled.
    pub plugin_state: PluginState,
}

/// Parse plugin metadata from the leading comment block of a `.lua` file.
///
/// Expected format (any order, `@` prefix):
/// ```lua
/// -- @name Git Status
/// -- @version 1.0.0
/// -- @author orca
/// -- @description Sets git status badges on files in repositories
/// ```
///
/// # Errors
/// Returns `PluginError::Meta` if a required field is missing.
pub fn parse_meta(path: &Path, source: &str) -> Result<PluginMeta, PluginError> {
    let mut name = None;
    let mut version = None;
    let mut author = None;
    let mut description = None;

    for line in source.lines() {
        let line = line.trim();
        if !line.starts_with("--") {
            // Stop at the first non-comment line.
            break;
        }
        let content = line.trim_start_matches('-').trim();
        if let Some(val) = content.strip_prefix("@name") {
            name = Some(val.trim().to_owned());
        } else if let Some(val) = content.strip_prefix("@version") {
            version = Some(val.trim().to_owned());
        } else if let Some(val) = content.strip_prefix("@author") {
            author = Some(val.trim().to_owned());
        } else if let Some(val) = content.strip_prefix("@description") {
            description = Some(val.trim().to_owned());
        }
    }

    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(PluginMeta {
        id,
        name: name.ok_or_else(|| PluginError::Meta("name".into()))?,
        version: version.unwrap_or_else(|| "0.0.0".into()),
        author: author.unwrap_or_else(|| "unknown".into()),
        description: description.unwrap_or_default(),
        path: path.to_path_buf(),
    })
}

/// Load a single plugin from `path`. On success returns a `LoadedPlugin` ready
/// to receive event dispatches. On failure returns a `LoadedPlugin` with
/// `PluginState::Error`.
pub fn load_plugin(path: &Path, badge_map: Arc<Mutex<BadgeMap>>) -> LoadedPlugin {
    match try_load_plugin(path, badge_map) {
        Ok(p) => p,
        Err(e) => {
            let err_str = e.to_string();
            tracing::warn!(plugin = %path.display(), error = %err_str, "plugin failed to load");
            let meta = PluginMeta {
                id: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                name: path.display().to_string(),
                version: "?".into(),
                author: "?".into(),
                description: String::new(),
                path: path.to_path_buf(),
            };
            LoadedPlugin {
                meta,
                lua: mlua::Lua::new(),
                state: Rc::new(RefCell::new(ApiState::default())),
                plugin_state: PluginState::Error(err_str),
            }
        }
    }
}

fn try_load_plugin(path: &Path, badge_map: Arc<Mutex<BadgeMap>>) -> Result<LoadedPlugin, PluginError> {
    let source = std::fs::read_to_string(path)?;
    let meta = parse_meta(path, &source)?;
    let lua = create_sandbox()?;
    let api_state: Rc<RefCell<ApiState>> = Rc::new(RefCell::new(ApiState::default()));

    register_api(&lua, api_state.clone(), badge_map, meta.id.clone())?;

    // Execute the plugin source. This runs the top-level registration calls
    // (orca.on_dir_change, orca.register_action, etc.) but does not yet fire
    // any event hooks.
    lua.load(&source)
        .set_name(&meta.id)
        .exec()
        .map_err(PluginError::Lua)?;

    tracing::info!(
        plugin = %meta.id,
        version = %meta.version,
        "loaded plugin"
    );

    Ok(LoadedPlugin {
        meta,
        lua,
        state: api_state,
        plugin_state: PluginState::Enabled,
    })
}

/// Discover all `.lua` plugin files in `dir`. Returns sorted paths.
pub fn discover(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "lua")
        })
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_meta_full_header() {
        let src = "-- @name  My Plugin\n-- @version 2.0.0\n-- @author Alice\n-- @description Does stuff\nreturn\n";
        let meta = parse_meta(Path::new("/tmp/my-plugin.lua"), src).unwrap();
        assert_eq!(meta.id, "my-plugin");
        assert_eq!(meta.name, "My Plugin");
        assert_eq!(meta.version, "2.0.0");
        assert_eq!(meta.author, "Alice");
        assert_eq!(meta.description, "Does stuff");
    }

    #[test]
    fn parse_meta_missing_name_errors() {
        let src = "-- @version 1.0\n";
        assert!(parse_meta(Path::new("/tmp/p.lua"), src).is_err());
    }

    #[test]
    fn parse_meta_defaults_version_author() {
        let src = "-- @name Minimal\n";
        let meta = parse_meta(Path::new("/tmp/minimal.lua"), src).unwrap();
        assert_eq!(meta.version, "0.0.0");
        assert_eq!(meta.author, "unknown");
    }
}
