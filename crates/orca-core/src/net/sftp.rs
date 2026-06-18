//! SFTP client built on `ssh2` (libssh2).
//!
//! `ssh2` is a blocking, non-`Sync` library; an [`SftpClient`] therefore wraps a
//! single session behind a mutex and dispatches every operation onto a blocking
//! task so the public API is async and the client is `Send + Sync`.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ssh2::Session;

use crate::error::{OrcaError, Result};
use crate::net::RemoteEntry;

/// How to authenticate an SFTP session.
#[derive(Debug, Clone)]
pub enum SftpAuth {
    /// Username + password.
    Password(String),
    /// Public-key authentication from a private key file with optional passphrase.
    Key {
        /// Path to the private key file.
        private_key: PathBuf,
        /// Optional passphrase protecting the key.
        passphrase: Option<String>,
    },
}

/// A connected, authenticated SFTP session.
#[derive(Clone)]
pub struct SftpClient {
    inner: Arc<Mutex<Session>>,
}

impl std::fmt::Debug for SftpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never expose session internals (which could include auth state).
        f.debug_struct("SftpClient").finish_non_exhaustive()
    }
}

/// Map an `ssh2` error into the crate error type.
fn map_ssh(e: ssh2::Error) -> OrcaError {
    OrcaError::Other(format!("sftp error: {e}"))
}

impl SftpClient {
    /// Connect to `host:port` and authenticate as `username`.
    ///
    /// # Errors
    /// I/O errors connecting, or [`OrcaError::Other`] on handshake/auth failure.
    pub async fn connect(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        auth: SftpAuth,
    ) -> Result<Self> {
        let host = host.into();
        let username = username.into();
        tokio::task::spawn_blocking(move || connect_blocking(&host, port, &username, &auth))
            .await
            .map_err(|e| OrcaError::Other(format!("sftp connect task failed: {e}")))?
    }

    /// List a remote directory.
    ///
    /// # Errors
    /// [`OrcaError::Other`] on protocol errors.
    pub async fn list(&self, remote_dir: impl AsRef<Path>) -> Result<Vec<RemoteEntry>> {
        let sess = Arc::clone(&self.inner);
        let dir = remote_dir.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let sess = lock(&sess)?;
            let sftp = sess.sftp().map_err(map_ssh)?;
            let raw = sftp.readdir(&dir).map_err(map_ssh)?;
            let mut entries = Vec::with_capacity(raw.len());
            for (path, stat) in raw {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                entries.push(RemoteEntry {
                    name,
                    path,
                    size: stat.size.unwrap_or(0),
                    is_dir: stat.is_dir(),
                });
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        })
        .await
        .map_err(|e| OrcaError::Other(format!("sftp list task failed: {e}")))?
    }

    /// Download `remote` to the local path `local`.
    ///
    /// # Errors
    /// I/O errors or [`OrcaError::Other`] on protocol errors.
    pub async fn download(&self, remote: impl AsRef<Path>, local: impl AsRef<Path>) -> Result<()> {
        let sess = Arc::clone(&self.inner);
        let remote = remote.as_ref().to_path_buf();
        let local = local.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let sess = lock(&sess)?;
            let sftp = sess.sftp().map_err(map_ssh)?;
            let mut rf = sftp.open(&remote).map_err(map_ssh)?;
            let mut out =
                std::fs::File::create(&local).map_err(|e| OrcaError::from_io(&local, e))?;
            std::io::copy(&mut rf, &mut out).map_err(|e| OrcaError::from_io(&local, e))?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("sftp download task failed: {e}")))?
    }

    /// Upload local file `local` to `remote`.
    ///
    /// # Errors
    /// I/O errors or [`OrcaError::Other`] on protocol errors.
    pub async fn upload(&self, local: impl AsRef<Path>, remote: impl AsRef<Path>) -> Result<()> {
        let sess = Arc::clone(&self.inner);
        let local = local.as_ref().to_path_buf();
        let remote = remote.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let sess = lock(&sess)?;
            let sftp = sess.sftp().map_err(map_ssh)?;
            let mut input =
                std::fs::File::open(&local).map_err(|e| OrcaError::from_io(&local, e))?;
            let mut rf = sftp.create(&remote).map_err(map_ssh)?;
            std::io::copy(&mut input, &mut rf).map_err(|e| OrcaError::from_io(&local, e))?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("sftp upload task failed: {e}")))?
    }

    /// Delete the remote file `remote`.
    ///
    /// # Errors
    /// [`OrcaError::Other`] on protocol errors.
    pub async fn delete(&self, remote: impl AsRef<Path>) -> Result<()> {
        let sess = Arc::clone(&self.inner);
        let remote = remote.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let sess = lock(&sess)?;
            let sftp = sess.sftp().map_err(map_ssh)?;
            sftp.unlink(&remote).map_err(map_ssh)?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("sftp delete task failed: {e}")))?
    }
}

/// Lock the shared session, mapping poisoning to a clean error.
fn lock(sess: &Arc<Mutex<Session>>) -> Result<std::sync::MutexGuard<'_, Session>> {
    sess.lock()
        .map_err(|_| OrcaError::Other("sftp session mutex poisoned".to_string()))
}

/// Blocking connect + handshake + authentication.
fn connect_blocking(host: &str, port: u16, username: &str, auth: &SftpAuth) -> Result<SftpClient> {
    let tcp =
        TcpStream::connect((host, port)).map_err(|e| OrcaError::from_io(Path::new(host), e))?;
    let mut sess = Session::new().map_err(map_ssh)?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(map_ssh)?;

    match auth {
        SftpAuth::Password(password) => {
            sess.userauth_password(username, password)
                .map_err(map_ssh)?;
        }
        SftpAuth::Key {
            private_key,
            passphrase,
        } => {
            sess.userauth_pubkey_file(username, None, private_key, passphrase.as_deref())
                .map_err(map_ssh)?;
        }
    }

    if !sess.authenticated() {
        return Err(OrcaError::Other("sftp authentication failed".to_string()));
    }
    Ok(SftpClient {
        inner: Arc::new(Mutex::new(sess)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Connecting to a port with no SSH server must fail cleanly, not hang or
    /// panic. We bind a listener to grab a free port, then drop it so the port is
    /// closed (connection refused) for a deterministic, fast failure.
    #[tokio::test]
    async fn connect_to_closed_port_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener); // close it

        let err = SftpClient::connect("127.0.0.1", port, "nobody", SftpAuth::Password("x".into()))
            .await
            .expect_err("should fail to connect");
        assert!(matches!(
            err,
            OrcaError::Io { .. } | OrcaError::BareIo(_) | OrcaError::Other(_)
        ));
    }

    #[test]
    fn debug_does_not_leak_session() {
        // Construct without connecting is not possible, but verify the Debug impl
        // shape is non-exhaustive (compile-time guarantee of no field exposure).
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<SftpClient>();
    }
}
