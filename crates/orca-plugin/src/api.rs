//! `orca.*` Lua API: hook registration and utility functions.
//!
//! [`register_api`] installs the `orca` global table in a sandboxed Lua state.
//! Hooks are stored as Lua registry keys so they can be retrieved for later
//! invocation without holding a lifetime-tied `mlua::Function` reference.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mlua::Lua;

use crate::types::{Badge, BadgeMap};

/// Per-plugin hook registry and log, owned by `LoadedPlugin`.
#[derive(Default)]
pub struct ApiState {
    /// Key for the `orca.on_file_select` callback (if registered).
    pub on_file_select: Option<mlua::RegistryKey>,
    /// Key for the `orca.on_dir_change` callback (if registered).
    pub on_dir_change: Option<mlua::RegistryKey>,
    /// Key for the `orca.on_vault_lock` callback (if registered).
    pub on_vault_lock: Option<mlua::RegistryKey>,
    /// Actions registered via `orca.register_action`.
    pub actions: Vec<RegisteredAction>,
    /// Context menu items registered via `orca.add_context_item`.
    pub context_items: Vec<ContextItem>,
    /// Per-plugin log lines written via `orca.log`.
    pub log: Vec<String>,
}

/// An action registered via `orca.register_action(id, label, fn)`.
pub struct RegisteredAction {
    /// Stable action identifier.
    pub id: String,
    /// Display label shown in the toolbar / menu.
    pub label: String,
    /// Lua registry key for the callback.
    pub func_key: mlua::RegistryKey,
}

/// A context-menu item registered via `orca.add_context_item(label, fn)`.
pub struct ContextItem {
    /// Display label.
    pub label: String,
    /// Lua registry key for the callback.
    pub func_key: mlua::RegistryKey,
}

/// Install the `orca.*` global table in `lua`.
///
/// `state` accumulates hook keys and log lines.
/// `badge_map` receives `orca.set_badge` updates (shared with the GUI).
/// `plugin_id` is used in log output to identify the source plugin.
pub fn register_api(
    lua: &Lua,
    state: Rc<RefCell<ApiState>>,
    badge_map: Arc<Mutex<BadgeMap>>,
    plugin_id: String,
) -> mlua::Result<()> {
    let orca = lua.create_table()?;

    // orca.on_file_select(fn)
    {
        let st = state.clone();
        let f = lua.create_function(move |lua, func: mlua::Function| {
            let key = lua.create_registry_value(func)?;
            st.borrow_mut().on_file_select = Some(key);
            Ok(())
        })?;
        orca.set("on_file_select", f)?;
    }

    // orca.on_dir_change(fn)
    {
        let st = state.clone();
        let f = lua.create_function(move |lua, func: mlua::Function| {
            let key = lua.create_registry_value(func)?;
            st.borrow_mut().on_dir_change = Some(key);
            Ok(())
        })?;
        orca.set("on_dir_change", f)?;
    }

    // orca.on_vault_lock(fn)
    {
        let st = state.clone();
        let f = lua.create_function(move |lua, func: mlua::Function| {
            let key = lua.create_registry_value(func)?;
            st.borrow_mut().on_vault_lock = Some(key);
            Ok(())
        })?;
        orca.set("on_vault_lock", f)?;
    }

    // orca.register_action(id, label, fn)
    {
        let st = state.clone();
        let f = lua.create_function(
            move |lua, (id, label, func): (String, String, mlua::Function)| {
                let func_key = lua.create_registry_value(func)?;
                st.borrow_mut().actions.push(RegisteredAction {
                    id,
                    label,
                    func_key,
                });
                Ok(())
            },
        )?;
        orca.set("register_action", f)?;
    }

    // orca.add_context_item(label, fn)
    {
        let st = state.clone();
        let f = lua.create_function(
            move |lua, (label, func): (String, mlua::Function)| {
                let func_key = lua.create_registry_value(func)?;
                st.borrow_mut().context_items.push(ContextItem {
                    label,
                    func_key,
                });
                Ok(())
            },
        )?;
        orca.set("add_context_item", f)?;
    }

    // orca.set_badge(path, text, color)
    {
        let bm = badge_map.clone();
        let f = lua.create_function(
            move |_, (path, text, color): (String, String, String)| {
                let mut map = bm.lock().unwrap();
                map.insert(PathBuf::from(&path), Badge { text, color });
                Ok(())
            },
        )?;
        orca.set("set_badge", f)?;
    }

    // orca.exec(cmd) → string
    {
        let f = lua.create_function(move |_, cmd: String| {
            exec_with_timeout(&cmd, Duration::from_secs(5))
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
        })?;
        orca.set("exec", f)?;
    }

    // orca.notify(title, body)
    {
        let f = lua.create_function(move |_, (title, body): (String, String)| {
            // Best-effort: spawn notify-send. If not installed, silently skip.
            let _ = std::process::Command::new("notify-send")
                .args([&title, &body])
                .spawn();
            Ok(())
        })?;
        orca.set("notify", f)?;
    }

    // orca.open(path)
    {
        let f = lua.create_function(move |_, path: String| {
            let _ = std::process::Command::new("xdg-open")
                .arg(&path)
                .spawn();
            Ok(())
        })?;
        orca.set("open", f)?;
    }

    // orca.log(msg)
    {
        let st = state.clone();
        let id = plugin_id.clone();
        let f = lua.create_function(move |_, msg: String| {
            tracing::info!(plugin = %id, "{}", msg);
            st.borrow_mut().log.push(msg);
            Ok(())
        })?;
        orca.set("log", f)?;
    }

    lua.globals().set("orca", orca)?;
    Ok(())
}

/// Run a shell command with a 5-second timeout. Returns stdout as a `String`.
fn exec_with_timeout(cmd: &str, timeout: Duration) -> Result<String, crate::error::PluginError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let cmd = cmd.to_owned();
    std::thread::spawn(move || {
        let result = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(Err(e)) => Err(crate::error::PluginError::Exec(e.to_string())),
        Err(_) => Err(crate::error::PluginError::Timeout),
    }
}

/// Look up a registry key in `lua` as a `mlua::Function` and call it with one
/// string argument. Uses a 500 ms Lua hook to time-out runaway plugins.
pub fn call_hook_str(
    lua: &Lua,
    key: &mlua::RegistryKey,
    arg: &str,
) -> Result<(), crate::error::PluginError> {
    call_with_timeout(lua, Duration::from_millis(500), || {
        let func: mlua::Function = lua.registry_value(key)?;
        func.call::<()>(arg.to_owned())?;
        Ok(())
    })
}

/// Call `func()` inside the Lua state with a Lua-level execution timeout.
///
/// The timeout is enforced via `mlua`'s instruction-count hook: every 100
/// instructions the hook checks the elapsed wall-clock time and interrupts Lua
/// execution with a runtime error if the budget is exceeded.
fn call_with_timeout<F>(
    lua: &Lua,
    timeout: Duration,
    func: F,
) -> Result<(), crate::error::PluginError>
where
    F: FnOnce() -> mlua::Result<()>,
{
    use std::sync::atomic::{AtomicBool, Ordering};

    let timed_out = Arc::new(AtomicBool::new(false));
    let to_flag = timed_out.clone();
    let deadline = std::time::Instant::now() + timeout;

    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(100),
        move |_lua, _debug| {
            if std::time::Instant::now() > deadline {
                to_flag.store(true, Ordering::Relaxed);
                Err(mlua::Error::RuntimeError("plugin timeout".into()))
            } else {
                Ok(mlua::VmState::Continue)
            }
        },
    );

    let result = func();
    lua.remove_hook();

    if timed_out.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(crate::error::PluginError::Timeout);
    }
    result.map_err(crate::error::PluginError::Lua)
}

/// Call a hook that receives a Lua table of file paths (for action/context item
/// callbacks which receive `files: string[]`).
pub fn call_hook_paths(
    lua: &Lua,
    key: &mlua::RegistryKey,
    paths: &[PathBuf],
) -> Result<(), crate::error::PluginError> {
    call_with_timeout(lua, Duration::from_millis(500), || {
        let func: mlua::Function = lua.registry_value(key)?;
        let tbl = lua.create_table()?;
        for (i, p) in paths.iter().enumerate() {
            tbl.set(i + 1, p.to_string_lossy().into_owned())?;
        }
        func.call::<()>(tbl)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::create_sandbox;
    use std::sync::{Arc, Mutex};

    fn make_state() -> Rc<RefCell<ApiState>> {
        Rc::new(RefCell::new(ApiState::default()))
    }

    #[test]
    fn on_dir_change_hook_fires() {
        let lua = create_sandbox().unwrap();
        let state = make_state();
        let badges = Arc::new(Mutex::new(BadgeMap::default()));
        register_api(&lua, state.clone(), badges, "test".into()).unwrap();

        lua.load(r#"
            orca.on_dir_change(function(path)
                orca.log("dir=" .. path)
            end)
        "#)
        .exec()
        .unwrap();

        let key = state.borrow().on_dir_change.as_ref().map(|k| {
            lua.registry_value::<mlua::Function>(k)
                .is_ok()
        });
        assert_eq!(key, Some(true));
    }

    #[test]
    fn set_badge_updates_map() {
        let lua = create_sandbox().unwrap();
        let state = make_state();
        let badges: Arc<Mutex<BadgeMap>> = Arc::new(Mutex::new(BadgeMap::default()));
        register_api(&lua, state, badges.clone(), "test".into()).unwrap();

        lua.load(r##"orca.set_badge("/tmp/foo.rs", "M", "#f00")"##)
            .exec()
            .unwrap();

        let map = badges.lock().unwrap();
        let badge = map.get(&PathBuf::from("/tmp/foo.rs")).unwrap();
        assert_eq!(badge.text, "M");
        assert_eq!(badge.color, "#f00");
    }

    #[test]
    fn hook_timeout_fires() {
        let lua = create_sandbox().unwrap();
        let state = make_state();
        let badges = Arc::new(Mutex::new(BadgeMap::default()));
        register_api(&lua, state.clone(), badges, "test".into()).unwrap();

        lua.load("orca.on_dir_change(function(p) while true do end end)")
            .exec()
            .unwrap();

        let key_ref = state.borrow();
        let key = key_ref.on_dir_change.as_ref().unwrap();
        let result = call_hook_str(&lua, key, "/tmp");
        assert!(
            matches!(result, Err(crate::error::PluginError::Timeout)),
            "expected timeout, got: {result:?}"
        );
    }
}
