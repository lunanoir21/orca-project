//! Crate error type for `orca-vault`.
//!
//! All fallible operations in this crate return [`Result<T>`], an alias for
//! [`std::result::Result<T, VaultError>`]. Errors are defined with `thiserror`
//! so callers can match on specific failure modes while still getting a
//! human-readable `Display` string.
//!
//! # Security
//! No `VaultError` variant ever carries a passphrase, a derived key, or any
//! plaintext key material. Cryptographic failures are reported through opaque
//! variants ([`VaultError::WrongPassphrase`], [`VaultError::Crypto`]) whose
//! messages describe *what* failed, never the secret involved. This upholds the
//! project rule that key material must never appear in logs or error messages.

use std::path::PathBuf;

/// Errors produced by `orca-vault` operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VaultError {
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

    /// The supplied passphrase did not decrypt the vault.
    ///
    /// Returned for any authentication failure. It deliberately does **not**
    /// distinguish "wrong passphrase" from "tampered ciphertext" beyond the
    /// dedicated [`VaultError::IntegrityFailure`] variant, and never echoes the
    /// passphrase back to the caller.
    #[error("incorrect passphrase")]
    WrongPassphrase,

    /// Passphrase-to-key derivation (Argon2id) failed.
    ///
    /// The message describes the failure mode (e.g. invalid parameters) and
    /// never contains the passphrase or the derived key.
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    /// An encryption or decryption primitive failed for a non-authentication
    /// reason (e.g. malformed `age` header).
    ///
    /// The message never contains key material.
    #[error("cryptographic operation failed: {0}")]
    Crypto(String),

    /// A vault file (meta or index) is structurally corrupt or unparsable.
    #[error("corrupt vault data: {0}")]
    Corrupt(String),

    /// Integrity verification failed: stored data does not match its expected
    /// hash or authentication tag.
    #[error("integrity verification failed: {0}")]
    IntegrityFailure(String),

    /// An operation required the vault to be unlocked, but it was locked.
    #[error("vault is locked")]
    Locked,

    /// The requested vault path or in-vault entry does not exist.
    #[error("not found: {0:?}")]
    NotFound(PathBuf),

    /// The destination vault or entry already exists.
    #[error("already exists: {0:?}")]
    AlreadyExists(PathBuf),

    /// The vault directory structure is missing or invalid.
    #[error("not a valid vault directory: {0:?}")]
    InvalidVault(PathBuf),

    /// Encrypted-index (de)serialization failed.
    #[error("vault index error: {0}")]
    Index(String),

    /// Vault configuration was invalid (e.g. empty name, relative path).
    #[error("invalid vault configuration: {0}")]
    Config(String),

    /// An operation that does not fit a more specific variant failed.
    #[error("{0}")]
    Other(String),
}

impl VaultError {
    /// Build a [`VaultError::Io`] from a path and an [`std::io::Error`],
    /// mapping the common `NotFound`/`AlreadyExists` kinds to their dedicated
    /// variants so callers can match on them directly.
    #[must_use]
    pub fn from_io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        let path = path.into();
        match source.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound(path),
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            _ => Self::Io { path, source },
        }
    }
}

/// Convenience result alias used throughout `orca-vault`.
pub type Result<T> = std::result::Result<T, VaultError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn from_io_maps_not_found() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err = VaultError::from_io("/tmp/v", io);
        assert!(matches!(err, VaultError::NotFound(p) if p == Path::new("/tmp/v")));
    }

    #[test]
    fn from_io_maps_already_exists() {
        let io = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "there");
        let err = VaultError::from_io("/tmp/v", io);
        assert!(matches!(err, VaultError::AlreadyExists(_)));
    }

    #[test]
    fn from_io_falls_back_to_io_variant() {
        let io = std::io::Error::other("boom");
        let err = VaultError::from_io("/tmp/v", io);
        assert!(matches!(err, VaultError::Io { .. }));
    }

    #[test]
    fn bare_io_from_conversion() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let err: VaultError = io.into();
        assert!(matches!(err, VaultError::BareIo(_)));
    }

    #[test]
    fn wrong_passphrase_message_leaks_no_secret() {
        // The Display string must never echo a passphrase; it is a fixed string.
        assert_eq!(
            VaultError::WrongPassphrase.to_string(),
            "incorrect passphrase"
        );
    }

    #[test]
    fn display_is_human_readable() {
        let err = VaultError::Locked;
        assert_eq!(err.to_string(), "vault is locked");
    }
}
