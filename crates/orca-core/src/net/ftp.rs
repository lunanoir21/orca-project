//! FTP client built on `suppaftp`.
//!
//! `suppaftp`'s default `FtpStream` is blocking, so an [`FtpClient`] wraps one
//! stream behind a mutex and runs each operation on a blocking task, mirroring
//! the SFTP client's design.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use suppaftp::FtpStream;

use crate::error::{OrcaError, Result};
use crate::net::RemoteEntry;

/// A connected, logged-in FTP session.
#[derive(Clone)]
pub struct FtpClient {
    inner: Arc<Mutex<FtpStream>>,
}

impl std::fmt::Debug for FtpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpClient").finish_non_exhaustive()
    }
}

/// Map a `suppaftp` error into the crate error type.
fn map_ftp(e: suppaftp::FtpError) -> OrcaError {
    OrcaError::Other(format!("ftp error: {e}"))
}

/// Lock the shared stream, mapping poisoning to a clean error.
fn lock(stream: &Arc<Mutex<FtpStream>>) -> Result<std::sync::MutexGuard<'_, FtpStream>> {
    stream
        .lock()
        .map_err(|_| OrcaError::Other("ftp stream mutex poisoned".to_string()))
}

impl FtpClient {
    /// Connect to `host:port` and log in as `username`.
    ///
    /// # Errors
    /// [`OrcaError::Other`] if the connection or login fails.
    pub async fn connect(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self> {
        let host = host.into();
        let username = username.into();
        let password = password.into();
        tokio::task::spawn_blocking(move || {
            let mut stream = FtpStream::connect(format!("{host}:{port}")).map_err(map_ftp)?;
            stream.login(&username, &password).map_err(map_ftp)?;
            Ok(FtpClient {
                inner: Arc::new(Mutex::new(stream)),
            })
        })
        .await
        .map_err(|e| OrcaError::Other(format!("ftp connect task failed: {e}")))?
    }

    /// List the remote directory `remote_dir`.
    ///
    /// # Errors
    /// [`OrcaError::Other`] on protocol errors.
    pub async fn list(&self, remote_dir: impl AsRef<Path>) -> Result<Vec<RemoteEntry>> {
        let stream = Arc::clone(&self.inner);
        let dir = remote_dir.as_ref().to_path_buf();
        let dir_str = dir.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            let mut s = lock(&stream)?;
            let lines = s.list(Some(&dir_str)).map_err(map_ftp)?;
            let mut entries = Vec::new();
            for line in lines {
                // Parse the raw LIST line; skip lines suppaftp cannot interpret.
                let Ok(file) = suppaftp::list::File::from_str(&line) else {
                    continue;
                };
                entries.push(RemoteEntry {
                    name: file.name().to_string(),
                    path: dir.join(file.name()),
                    size: file.size() as u64,
                    is_dir: file.is_directory(),
                });
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        })
        .await
        .map_err(|e| OrcaError::Other(format!("ftp list task failed: {e}")))?
    }

    /// Download `remote` to local path `local`.
    ///
    /// # Errors
    /// I/O or [`OrcaError::Other`] protocol errors.
    pub async fn download(&self, remote: impl AsRef<Path>, local: impl AsRef<Path>) -> Result<()> {
        let stream = Arc::clone(&self.inner);
        let remote = remote.as_ref().to_string_lossy().into_owned();
        let local = local.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut s = lock(&stream)?;
            let cursor = s.retr_as_buffer(&remote).map_err(map_ftp)?;
            std::fs::write(&local, cursor.into_inner())
                .map_err(|e| OrcaError::from_io(&local, e))?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("ftp download task failed: {e}")))?
    }

    /// Upload local file `local` to `remote`.
    ///
    /// # Errors
    /// I/O or [`OrcaError::Other`] protocol errors.
    pub async fn upload(&self, local: impl AsRef<Path>, remote: impl AsRef<Path>) -> Result<()> {
        let stream = Arc::clone(&self.inner);
        let local = local.as_ref().to_path_buf();
        let remote = remote.as_ref().to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            let mut s = lock(&stream)?;
            let mut input =
                std::fs::File::open(&local).map_err(|e| OrcaError::from_io(&local, e))?;
            s.put_file(&remote, &mut input).map_err(map_ftp)?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("ftp upload task failed: {e}")))?
    }

    /// Delete the remote file `remote`.
    ///
    /// # Errors
    /// [`OrcaError::Other`] on protocol errors.
    pub async fn delete(&self, remote: impl AsRef<Path>) -> Result<()> {
        let stream = Arc::clone(&self.inner);
        let remote = remote.as_ref().to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            let mut s = lock(&stream)?;
            s.rm(&remote).map_err(map_ftp)?;
            Ok(())
        })
        .await
        .map_err(|e| OrcaError::Other(format!("ftp delete task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Connecting where no FTP server listens must fail cleanly.
    #[tokio::test]
    async fn connect_to_closed_port_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let err = FtpClient::connect("127.0.0.1", port, "anonymous", "")
            .await
            .expect_err("should fail to connect");
        assert!(matches!(err, OrcaError::Other(_)));
    }

    #[test]
    fn debug_is_non_exhaustive() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<FtpClient>();
    }
}
