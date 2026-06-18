//! Crate error type for `orca-core`.
//!
//! All fallible operations in this crate return [`Result<T>`], an alias for
//! [`std::result::Result<T, OrcaError>`]. Errors are defined with `thiserror`
//! so callers can match on specific failure modes while still getting a
//! human-readable `Display` string.

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

/// Errors produced by `orca-core` filesystem operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OrcaError {
    /// An underlying I/O error, with the path it occurred on when known.
    #[error("I/O error at {path:?}: {source}")]
    Io {
        /// Path the operation was acting on, if available.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// A bare I/O error with no associated path.
    #[error(transparent)]
    BareIo(#[from] std::io::Error),

    /// The requested path does not exist.
    #[error("path does not exist: {0:?}")]
    NotFound(PathBuf),

    /// The process lacks permission to act on the path.
    #[error("permission denied: {0:?}")]
    PermissionDenied(PathBuf),

    /// A path was expected to be a directory but was not.
    #[error("not a directory: {0:?}")]
    NotADirectory(PathBuf),

    /// A path was expected to be a file but was not.
    #[error("not a file: {0:?}")]
    NotAFile(PathBuf),

    /// The destination already exists and overwrite was not requested.
    #[error("already exists: {0:?}")]
    AlreadyExists(PathBuf),

    /// A path was invalid (e.g. empty, or not valid UTF-8 where required).
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// A symlink cycle was detected during resolution.
    #[error("symlink cycle detected at {0:?}")]
    SymlinkCycle(PathBuf),

    /// The operation was cancelled before completion.
    #[error("operation cancelled")]
    Cancelled,

    /// An operation that does not fit a more specific variant failed.
    #[error("{0}")]
    Other(String),
}

impl OrcaError {
    /// Build an [`OrcaError::Io`] from a path and an [`std::io::Error`],
    /// mapping the common `NotFound`/`PermissionDenied` kinds to their
    /// dedicated variants so callers can match on them directly.
    #[must_use]
    pub fn from_io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        let path = path.into();
        match source.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound(path),
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            _ => Self::Io { path, source },
        }
    }
}

/// Convenience result alias used throughout `orca-core`.
pub type Result<T> = std::result::Result<T, OrcaError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_io_maps_not_found() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err = OrcaError::from_io("/tmp/x", io);
        assert!(matches!(err, OrcaError::NotFound(p) if p == Path::new("/tmp/x")));
    }

    #[test]
    fn from_io_maps_permission_denied() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope");
        let err = OrcaError::from_io("/root/secret", io);
        assert!(matches!(err, OrcaError::PermissionDenied(_)));
    }

    #[test]
    fn from_io_falls_back_to_io_variant() {
        let io = std::io::Error::other("boom");
        let err = OrcaError::from_io("/tmp/y", io);
        assert!(matches!(err, OrcaError::Io { .. }));
    }

    #[test]
    fn display_is_human_readable() {
        let err = OrcaError::NotADirectory(PathBuf::from("/etc/hosts"));
        assert_eq!(err.to_string(), "not a directory: \"/etc/hosts\"");
    }

    #[test]
    fn bare_io_from_conversion() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let err: OrcaError = io.into();
        assert!(matches!(err, OrcaError::BareIo(_)));
    }
}
