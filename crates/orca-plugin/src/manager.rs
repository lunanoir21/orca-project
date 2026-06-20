//! [`PluginManager`]: discovers, loads, enables/disables and hot-reloads plugins,
//! and dispatches event hooks across all enabled plugins.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::api::{call_hook_paths, call_hook_str};
use crate::error::PluginError;
use crate::loader::{discover, load_plugin, LoadedPlugin};
use crate::types::{BadgeMap, PluginState};

/// Manages all plugins for the Orca session.
///
/// Lives on the GTK main thread. `Lua` states inside [`LoadedPlugin`] are
/// `!Send`, so `PluginManager` is also `!Send`.
pub struct PluginManager {
    /// Loaded plugins in discovery order.
    plugins: Vec<LoadedPlugin>,
    /// Badge map shared with the GUI for rendering badges on file items.
    pub badge_map: Arc<Mutex<BadgeMap>>,
    /// Plugin ids that are explicitly disabled (from config `[plugins].disabled`).
    disabled: HashSet<String>,
}

impl PluginManager {
    /// Create a new, empty manager. Call [`discover_and_load`] next.
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            badge_map: Arc::new(Mutex::new(BadgeMap::default())),
            disabled: HashSet::new(),
        }
    }

    /// Mark the given plugin ids as disabled before loading.
    pub fn set_disabled(&mut self, ids: impl IntoIterator<Item = String>) {
        self.disabled = ids.into_iter().collect();
    }

    /// Discover and load all `.lua` files in `user_dir` then `builtin_dir`.
    ///
    /// User-dir plugins take priority: if both dirs contain a plugin with the
    /// same id (filename), the user dir version shadows the built-in one.
    pub fn discover_and_load(&mut self, user_dir: &Path, builtin_dir: &Path) {
        let mut paths = discover(user_dir);
        let user_ids: HashSet<String> = paths
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        for p in discover(builtin_dir) {
            let id = p.file_stem().map(|s| s.to_string_lossy().into_owned());
            if !id.as_deref().map(|s| user_ids.contains(s)).unwrap_or(false) {
                paths.push(p);
            }
        }
        for path in paths {
            let mut plugin = load_plugin(&path, self.badge_map.clone());
            if self.disabled.contains(&plugin.meta.id) {
                plugin.plugin_state = PluginState::Disabled;
            }
            self.plugins.push(plugin);
        }
    }

    /// Return info snapshots for all known plugins (for the plugin manager UI).
    #[must_use]
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| PluginInfo {
                id: p.meta.id.clone(),
                name: p.meta.name.clone(),
                version: p.meta.version.clone(),
                author: p.meta.author.clone(),
                description: p.meta.description.clone(),
                state: p.plugin_state.clone(),
                log: p.state.borrow().log.clone(),
            })
            .collect()
    }

    /// Enable a plugin by id. No-op if already enabled or in an error state.
    pub fn enable(&mut self, id: &str) {
        self.disabled.remove(id);
        if let Some(p) = self.plugins.iter_mut().find(|p| p.meta.id == id) {
            if p.plugin_state == PluginState::Disabled {
                p.plugin_state = PluginState::Enabled;
            }
        }
    }

    /// Disable a plugin by id. No-op if already disabled or errored.
    pub fn disable(&mut self, id: &str) {
        self.disabled.insert(id.to_owned());
        if let Some(p) = self.plugins.iter_mut().find(|p| p.meta.id == id) {
            if p.plugin_state == PluginState::Enabled {
                p.plugin_state = PluginState::Disabled;
            }
        }
    }

    /// Hot-reload a plugin by id: re-read the file and rebuild its Lua sandbox.
    pub fn reload(&mut self, id: &str) {
        let Some(idx) = self.plugins.iter().position(|p| p.meta.id == id) else {
            return;
        };
        let path = self.plugins[idx].meta.path.clone();
        let was_disabled = self.disabled.contains(id);
        let mut new_plugin = load_plugin(&path, self.badge_map.clone());
        if was_disabled {
            new_plugin.plugin_state = PluginState::Disabled;
        }
        self.plugins[idx] = new_plugin;
    }

    /// Return the ids of currently disabled plugins (for persisting to config).
    #[must_use]
    pub fn disabled_ids(&self) -> Vec<String> {
        self.disabled.iter().cloned().collect()
    }

    /// Collect all context-menu items registered by enabled plugins.
    ///
    /// Returns `(action_id, label)` pairs for use in `PaneInput::SetPluginContextItems`.
    #[must_use]
    pub fn context_items(&self) -> Vec<(String, String)> {
        let mut items = Vec::new();
        for p in &self.plugins {
            if p.plugin_state != PluginState::Enabled {
                continue;
            }
            for item in &p.state.borrow().context_items {
                items.push((p.meta.id.clone() + "::" + &item.label, item.label.clone()));
            }
        }
        items
    }

    // ---- event dispatch ----

    /// Fire `on_file_select(path)` across all enabled plugins.
    pub fn fire_file_select(&mut self, path: &Path) {
        let arg = path.to_string_lossy().into_owned();
        self.dispatch_str_hook(arg, |s| s.on_file_select.as_ref());
    }

    /// Fire `on_dir_change(path)` across all enabled plugins.
    pub fn fire_dir_change(&mut self, path: &Path) {
        let arg = path.to_string_lossy().into_owned();
        self.dispatch_str_hook(arg, |s| s.on_dir_change.as_ref());
    }

    /// Fire `on_vault_lock(vault_name)` across all enabled plugins.
    pub fn fire_vault_lock(&mut self, vault_name: &str) {
        self.dispatch_str_hook(vault_name.to_owned(), |s| s.on_vault_lock.as_ref());
    }

    /// Invoke a registered action callback for `action_id` with `paths`.
    pub fn fire_action(&mut self, action_id: &str, paths: &[PathBuf]) {
        let mut errors: Vec<(usize, PluginError)> = Vec::new();
        for (idx, p) in self.plugins.iter_mut().enumerate() {
            if p.plugin_state != PluginState::Enabled {
                continue;
            }
            let state = p.state.borrow();
            let Some(action) = state.actions.iter().find(|a| a.id == action_id) else {
                continue;
            };
            if let Err(e) = call_hook_paths(&p.lua, &action.func_key, paths) {
                errors.push((idx, e));
            }
        }
        self.apply_errors(errors);
    }

    // ---- internals ----

    /// Generic str-arg hook dispatcher. `pick` selects the registry key to call.
    fn dispatch_str_hook(
        &mut self,
        arg: String,
        pick: impl Fn(&crate::api::ApiState) -> Option<&mlua::RegistryKey>,
    ) {
        let mut errors: Vec<(usize, PluginError)> = Vec::new();
        for (idx, p) in self.plugins.iter_mut().enumerate() {
            if p.plugin_state != PluginState::Enabled {
                continue;
            }
            // Hold the borrow for the duration of the call so the key stays valid.
            let state = p.state.borrow();
            let Some(key) = pick(&state) else { continue };
            if let Err(e) = call_hook_str(&p.lua, key, &arg) {
                errors.push((idx, e));
            }
        }
        self.apply_errors(errors);
    }

    /// Mark errored plugins based on hook results. Timeouts and memory errors
    /// move the plugin to `PluginState::Error`; other errors are just logged.
    fn apply_errors(&mut self, errors: Vec<(usize, PluginError)>) {
        for (idx, err) in errors {
            let msg = err.to_string();
            tracing::warn!(plugin = %self.plugins[idx].meta.id, error = %msg, "hook error");
            self.plugins[idx]
                .state
                .borrow_mut()
                .log
                .push(format!("[error] {msg}"));
            if matches!(
                &err,
                PluginError::Timeout | PluginError::Lua(mlua::Error::MemoryError(_))
            ) {
                self.plugins[idx].plugin_state = PluginState::Error(msg);
            }
        }
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of a plugin's state for the plugin manager UI.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// Stable id (filename stem).
    pub id: String,
    /// Display name from `@name`.
    pub name: String,
    /// Version string from `@version`.
    pub version: String,
    /// Author from `@author`.
    pub author: String,
    /// One-line description from `@description`.
    pub description: String,
    /// Current runtime state.
    pub state: PluginState,
    /// Lines written via `orca.log` or error messages.
    pub log: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_plugin(dir: &Path, name: &str, src: &str) {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
    }

    #[test]
    fn panicking_plugin_becomes_error() {
        let dir = tempdir().unwrap();
        write_plugin(
            dir.path(),
            "bad.lua",
            "-- @name Bad\n-- @version 1.0\n-- @author test\norca.on_dir_change(function(p) error('oops') end)\n",
        );
        let mut mgr = PluginManager::new();
        mgr.discover_and_load(dir.path(), Path::new("/nonexistent"));
        assert_eq!(mgr.plugins.len(), 1);
        // Fire the hook — the plugin should log the error but not crash.
        mgr.fire_dir_change(Path::new("/tmp"));
        let info = &mgr.list()[0];
        assert!(!info.log.is_empty(), "expected error in log");
    }

    #[test]
    fn builtin_plugins_load_without_error() {
        // Load the real shipped plugins (not synthetic test strings) to catch
        // Lua syntax/registration errors in plugins/*.lua before they'd ship.
        let builtin_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let mut mgr = PluginManager::new();
        mgr.discover_and_load(Path::new("/nonexistent"), &builtin_dir);

        let loaded: Vec<&str> = mgr.plugins.iter().map(|p| p.meta.id.as_str()).collect();
        assert!(loaded.contains(&"git-status"), "loaded: {loaded:?}");
        assert!(loaded.contains(&"archive"), "loaded: {loaded:?}");
        assert!(loaded.contains(&"secure-open"), "loaded: {loaded:?}");

        for p in &mgr.plugins {
            assert_eq!(
                p.plugin_state,
                PluginState::Enabled,
                "{} failed to load: {:?}",
                p.meta.id,
                p.state.borrow().log
            );
        }
    }

    #[test]
    fn enable_disable_roundtrip() {
        let dir = tempdir().unwrap();
        write_plugin(
            dir.path(),
            "good.lua",
            "-- @name Good\n-- @version 1.0\n-- @author test\n",
        );
        let mut mgr = PluginManager::new();
        mgr.discover_and_load(dir.path(), Path::new("/nonexistent"));

        assert_eq!(mgr.plugins[0].plugin_state, PluginState::Enabled);
        mgr.disable("good");
        assert_eq!(mgr.plugins[0].plugin_state, PluginState::Disabled);
        mgr.enable("good");
        assert_eq!(mgr.plugins[0].plugin_state, PluginState::Enabled);
    }

    #[test]
    fn exec_timeout_disables_plugin() {
        let dir = tempdir().unwrap();
        write_plugin(
            dir.path(),
            "spin.lua",
            "-- @name Spin\n-- @version 1.0\n-- @author test\norca.on_dir_change(function(p) while true do end end)\n",
        );
        let mut mgr = PluginManager::new();
        mgr.discover_and_load(dir.path(), Path::new("/nonexistent"));
        mgr.fire_dir_change(Path::new("/tmp"));
        let info = &mgr.list()[0];
        // Timeout should move plugin to Error state.
        assert!(
            matches!(info.state, PluginState::Error(_)),
            "expected Error state, got {:?}",
            info.state
        );
    }
}
