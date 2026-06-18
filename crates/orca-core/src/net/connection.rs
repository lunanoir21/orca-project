//! Saved network connections, persisted as TOML under the XDG config directory.
//!
//! Only non-secret connection metadata is stored here (host, port, username,
//! remote path). Passwords are never written to this file — they live in the OS
//! keyring via [`crate::net::credentials`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{OrcaError, Result};

/// Application name used for the config subdirectory.
const APP_DIR: &str = "orca";
/// File holding the saved connection list.
const FILE_NAME: &str = "connections.toml";

/// Which protocol a saved connection uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// SSH File Transfer Protocol.
    Sftp,
    /// File Transfer Protocol.
    Ftp,
}

/// A saved remote connection (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedConnection {
    /// Human-friendly label, also the keyring lookup key.
    pub name: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Hostname or IP.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Login username.
    pub username: String,
    /// Initial remote directory (empty for the server default).
    #[serde(default)]
    pub remote_path: String,
}

/// On-disk wrapper so the TOML file reads as `[[connection]]` tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionsFile {
    #[serde(default, rename = "connection")]
    connection: Vec<SavedConnection>,
}

/// Resolve the connections file path under `$XDG_CONFIG_HOME/orca/`.
fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| OrcaError::Other("could not resolve XDG config directory".to_string()))?;
    Ok(base.join(APP_DIR).join(FILE_NAME))
}

/// Load all saved connections. Returns an empty list if the file does not exist.
///
/// # Errors
/// I/O errors reading the file, or [`OrcaError::Other`] if it is malformed.
pub fn load_connections() -> Result<Vec<SavedConnection>> {
    let path = config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OrcaError::from_io(&path, e)),
    };
    let parsed: ConnectionsFile =
        toml::from_str(&text).map_err(|e| OrcaError::Other(format!("invalid {FILE_NAME}: {e}")))?;
    Ok(parsed.connection)
}

/// Persist the full connection list, replacing the file. The parent directory is
/// created if missing.
///
/// # Errors
/// I/O errors writing the file or [`OrcaError::Other`] on serialization failure.
pub fn save_connections(connections: &[SavedConnection]) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OrcaError::from_io(parent, e))?;
    }
    let file = ConnectionsFile {
        connection: connections.to_vec(),
    };
    let text =
        toml::to_string_pretty(&file).map_err(|e| OrcaError::Other(format!("serialize: {e}")))?;
    std::fs::write(&path, text).map_err(|e| OrcaError::from_io(&path, e))
}

/// Insert or replace a connection by `name`, then persist the list.
///
/// # Errors
/// As [`save_connections`].
pub fn upsert_connection(conn: SavedConnection) -> Result<()> {
    let mut all = load_connections()?;
    match all.iter_mut().find(|c| c.name == conn.name) {
        Some(existing) => *existing = conn,
        None => all.push(conn),
    }
    save_connections(&all)
}

/// Remove a connection by `name`. Returns `true` if one was removed.
///
/// # Errors
/// As [`save_connections`].
pub fn remove_connection(name: &str) -> Result<bool> {
    let mut all = load_connections()?;
    let before = all.len();
    all.retain(|c| c.name != name);
    let removed = all.len() != before;
    if removed {
        save_connections(&all)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> SavedConnection {
        SavedConnection {
            name: name.to_string(),
            protocol: Protocol::Sftp,
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            remote_path: "/home/alice".to_string(),
        }
    }

    #[tokio::test]
    async fn save_load_roundtrip() {
        let _guard = crate::util::ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());

        // Empty before anything is written.
        assert!(load_connections().expect("load").is_empty());

        save_connections(&[sample("server-a"), sample("server-b")]).expect("save");
        let loaded = load_connections().expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "server-a");
        assert_eq!(loaded[0].protocol, Protocol::Sftp);

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn upsert_replaces_by_name() {
        let _guard = crate::util::ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());

        upsert_connection(sample("srv")).expect("insert");
        let mut updated = sample("srv");
        updated.host = "newhost.net".to_string();
        upsert_connection(updated).expect("update");

        let all = load_connections().expect("load");
        assert_eq!(all.len(), 1, "upsert must not duplicate");
        assert_eq!(all[0].host, "newhost.net");

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn remove_deletes_entry() {
        let _guard = crate::util::ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());

        save_connections(&[sample("keep"), sample("drop")]).expect("save");
        assert!(remove_connection("drop").expect("remove"));
        assert!(!remove_connection("missing").expect("remove missing"));
        let all = load_connections().expect("load");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "keep");

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn ftp_protocol_serializes_lowercase() {
        let _guard = crate::util::ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());

        let mut c = sample("ftp-srv");
        c.protocol = Protocol::Ftp;
        c.port = 21;
        save_connections(&[c]).expect("save");

        let path = config_path().expect("path");
        let text = std::fs::read_to_string(path).expect("read");
        assert!(text.contains("protocol = \"ftp\""), "got: {text}");

        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
