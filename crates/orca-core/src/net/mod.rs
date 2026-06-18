//! Network file locations: SFTP and FTP clients, saved connections and keyring
//! credential storage.
//!
//! - [`sftp::SftpClient`] / [`ftp::FtpClient`] — connect, list, download, upload
//!   and delete over SFTP and FTP.
//! - [`connection`] — persist and load [`connection::SavedConnection`] entries in
//!   the XDG config directory.
//! - [`credentials`] — store passwords in the OS keyring (Secret Service), never
//!   on disk in plaintext.
//!
//! The SFTP/FTP backends (`ssh2`, `suppaftp`) are blocking, so each network call
//! is dispatched onto a blocking task and the public API stays async.

pub mod connection;
pub mod credentials;
pub mod ftp;
pub mod sftp;

use std::path::PathBuf;

/// One remote directory entry returned by a network listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// Final path component.
    pub name: String,
    /// Full remote path.
    pub path: PathBuf,
    /// Size in bytes (best-effort; 0 if the server did not report one).
    pub size: u64,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}
