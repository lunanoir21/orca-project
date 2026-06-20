//! Sandboxed Lua 5.4 environment creation.
//!
//! Only the safe subset of the Lua standard library is made available:
//! `string`, `table`, and `math`. The `io`, `os`, `package` and `debug`
//! libraries are excluded so plugins cannot touch the filesystem, environment,
//! or module system directly. `require` is replaced with a stub that always
//! raises an error.

use mlua::{Lua, LuaOptions, StdLib};

/// Hard cap on a plugin's Lua heap. mlua's default is unbounded, so without
/// this a runaway or malicious plugin (e.g. an unbounded table-growth loop)
/// could exhaust host memory and take Orca down with it. 64 MiB is generous
/// for the badge/menu/exec work plugins actually do.
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Create a new sandboxed Lua 5.4 state.
///
/// # Errors
/// Returns an error if the Lua runtime cannot be initialised (extremely rare;
/// typically only in out-of-memory conditions).
pub fn create_sandbox() -> mlua::Result<Lua> {
    // Allowed: safe pure-Lua standard libraries only.
    let allowed = StdLib::STRING | StdLib::TABLE | StdLib::MATH;
    let lua = Lua::new_with(allowed, LuaOptions::default())?;
    lua.set_memory_limit(MEMORY_LIMIT_BYTES)?;

    // Override `require` so plugins cannot load arbitrary modules.
    let blocked_require = lua.create_function(|_, name: String| {
        Err::<mlua::Value, _>(mlua::Error::RuntimeError(format!(
            "sandbox: require('{name}') is blocked — use the orca.* API"
        )))
    })?;
    lua.globals().set("require", blocked_require)?;

    // Remove any remaining references to the blocked libraries that mlua
    // might have placed in globals under non-standard names.
    for blocked in &["io", "os", "package", "debug"] {
        lua.globals().set(*blocked, mlua::Nil)?;
    }

    Ok(lua)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_blocks_io() {
        let lua = create_sandbox().unwrap();
        // `io` global must not exist.
        let io: mlua::Value = lua.globals().get("io").unwrap();
        assert!(matches!(io, mlua::Value::Nil));
    }

    #[test]
    fn sandbox_blocks_os() {
        let lua = create_sandbox().unwrap();
        let os: mlua::Value = lua.globals().get("os").unwrap();
        assert!(matches!(os, mlua::Value::Nil));
    }

    #[test]
    fn sandbox_blocks_debug() {
        let lua = create_sandbox().unwrap();
        let dbg: mlua::Value = lua.globals().get("debug").unwrap();
        assert!(matches!(dbg, mlua::Value::Nil));
    }

    #[test]
    fn sandbox_blocks_require() {
        let lua = create_sandbox().unwrap();
        let err = lua.load("require('os')").exec().unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn sandbox_enforces_memory_limit() {
        let lua = create_sandbox().unwrap();
        // Try to grow a table well past the 64 MiB cap with long strings;
        // mlua must abort the script with a MemoryError before it succeeds.
        let err = lua
            .load(
                r#"
                local t = {}
                for i = 1, 1000000 do
                    t[i] = string.rep("x", 1024)
                end
                "#,
            )
            .exec()
            .unwrap_err();
        assert!(matches!(err, mlua::Error::MemoryError(_)));
    }

    #[test]
    fn sandbox_allows_string_table_math() {
        let lua = create_sandbox().unwrap();
        // string.len, table.insert, math.floor must all work.
        lua.load("assert(string.len('hi') == 2)").exec().unwrap();
        lua.load("local t = {}; table.insert(t, 1); assert(#t == 1)")
            .exec()
            .unwrap();
        lua.load("assert(math.floor(1.9) == 1)").exec().unwrap();
    }
}
