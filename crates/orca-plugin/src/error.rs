//! Error type for the plugin system.

/// All errors that can occur when loading or executing a plugin.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The Lua runtime reported an error.
    #[error("lua: {0}")]
    Lua(#[from] mlua::Error),
    /// A hook or exec call exceeded its time budget.
    #[error("plugin timed out")]
    Timeout,
    /// `orca.exec` failed to spawn or the child exited with stderr.
    #[error("exec: {0}")]
    Exec(String),
    /// Standard I/O error (file read, directory scan).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Metadata header was missing or malformed.
    #[error("missing metadata field '{0}' in plugin header")]
    Meta(String),
}
