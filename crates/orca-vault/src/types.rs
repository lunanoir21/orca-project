//! Core data types for `orca-vault`.
//!
//! These types describe a vault's static configuration ([`VaultConfig`]), its
//! runtime lock state ([`VaultState`]) and the in-memory representation of the
//! encrypted file index ([`VaultIndex`], [`VaultEntry`]). They are plain data;
//! the cryptography and lifecycle logic that operates on them lives in the
//! `crypto`, `vault` and `index` modules.
//!
//! # Security
//! None of these types hold key material. [`VaultIndex`] holds the *original*
//! plaintext filenames, sizes and hashes of vault contents — this struct only
//! ever exists in memory while the vault is unlocked, and is persisted to disk
//! exclusively in encrypted form (see the `index` module). It must never be
//! written to disk, logged, or serialized to a plaintext location directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Selectable encryption scheme for a vault's file contents.
///
/// # Security
/// Both variants use only audited, third-party cryptography crates — there is
/// **no custom cipher or KDF code** anywhere (project rule 9.2). The default is
/// the most stable, widely-deployed option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncryptionScheme {
    /// `age` native passphrase encryption (ChaCha20-Poly1305 + scrypt KDF).
    ///
    /// Default: the reference `age` format — extensively fuzzed and deployed,
    /// the most stable choice. Authenticated, so any ciphertext corruption is
    /// detected on decrypt rather than producing garbage plaintext.
    #[default]
    Age,
    /// Argon2id key derivation (OWASP parameters) feeding XChaCha20-Poly1305
    /// authenticated encryption (RustCrypto).
    ///
    /// Satisfies the project requirement for Argon2id passphrase derivation.
    Argon2idXchacha20,
}

/// Selectable content-integrity hash stored alongside each vault entry.
///
/// This is **not** encryption: it is a checksum used to detect corruption or
/// bit-rot of decrypted contents, complementing the AEAD authentication tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IntegrityHash {
    /// Blake3 (default): fast, modern, 256-bit.
    #[default]
    Blake3,
    /// SHA-256: slower but ubiquitous and FIPS-standard.
    Sha256,
}

/// Static, on-disk-configurable description of a single vault.
///
/// Serialized into the user's `config.toml` `[[vaults]]` array. Contains no
/// secrets: the passphrase is supplied at unlock time and never stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Human-readable vault name, unique within the user's configuration.
    pub name: String,
    /// Absolute path to the vault directory (contains `meta.age`/`index.age`).
    pub path: PathBuf,
    /// Idle minutes before auto-lock fires. `None` disables auto-lock.
    #[serde(default)]
    pub auto_lock_minutes: Option<u32>,
    /// Encryption scheme for file contents. Defaults to [`EncryptionScheme::Age`].
    #[serde(default)]
    pub scheme: EncryptionScheme,
    /// Content-integrity hash algorithm. Defaults to [`IntegrityHash::Blake3`].
    #[serde(default)]
    pub integrity_hash: IntegrityHash,
}

impl VaultConfig {
    /// Validate the configuration: non-empty name and an absolute vault path.
    ///
    /// Returns [`crate::VaultError::Config`] describing the first problem found.
    ///
    /// # Errors
    /// Fails if `name` is empty/whitespace, or if `path` is not absolute.
    pub fn validate(&self) -> crate::Result<()> {
        if self.name.trim().is_empty() {
            return Err(crate::VaultError::Config("vault name is empty".into()));
        }
        if !self.path.is_absolute() {
            return Err(crate::VaultError::Config(format!(
                "vault path must be absolute: {:?}",
                self.path
            )));
        }
        Ok(())
    }

    /// Returns `true` if auto-lock is enabled with a non-zero duration.
    #[must_use]
    pub fn auto_lock_enabled(&self) -> bool {
        matches!(self.auto_lock_minutes, Some(m) if m > 0)
    }
}

/// Runtime lock state of a vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// Keys are not in memory; vault contents are inaccessible.
    Locked,
    /// Keys are in memory; vault contents are accessible.
    Unlocked,
    /// Unlocked, but the auto-lock timer has fired and a lock is in progress
    /// (keys are being zeroized). Treated as inaccessible by callers.
    AutoLocking,
}

impl VaultState {
    /// Returns `true` only when the vault is fully [`VaultState::Unlocked`].
    ///
    /// [`VaultState::AutoLocking`] returns `false` so that no operation races a
    /// lock that is already clearing key material.
    #[must_use]
    pub fn is_unlocked(self) -> bool {
        matches!(self, VaultState::Unlocked)
    }
}

/// A single entry in the encrypted vault index.
///
/// Maps an opaque on-disk ciphertext filename ([`stored_name`]) back to the
/// original plaintext metadata of the file it encrypts. The `stored_name` is a
/// random identifier so that the encrypted directory listing leaks nothing
/// about the original filenames.
///
/// [`stored_name`]: VaultEntry::stored_name
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultEntry {
    /// Original plaintext filename (without any directory component).
    pub original_name: String,
    /// Opaque on-disk ciphertext filename within the vault directory, e.g.
    /// `"3f9a…d2.age"`. Never derived from `original_name`.
    pub stored_name: String,
    /// Size in bytes of the original plaintext file.
    pub size: u64,
    /// Time the file was added, as seconds since the Unix epoch (UTC).
    pub added_unix: i64,
    /// Lowercase hex content hash of the original plaintext, used for integrity
    /// verification on extract. The algorithm is the vault's configured
    /// [`IntegrityHash`] (Blake3 or SHA-256).
    pub content_hash: String,
}

/// Result of a vault integrity verification pass.
///
/// Each file is classified into exactly one category. A vault is healthy when
/// `tampered`, `missing` and `orphaned` are all empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrityReport {
    /// Original names of files whose decrypted content matches the stored hash.
    pub ok: Vec<String>,
    /// Original names whose content failed authentication or did not match the
    /// stored hash (corruption or tampering).
    pub tampered: Vec<String>,
    /// Original names present in the index but with no data file on disk.
    pub missing: Vec<String>,
    /// On-disk data filenames not referenced by any index entry.
    pub orphaned: Vec<String>,
}

impl IntegrityReport {
    /// Returns `true` if no problems were found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.tampered.is_empty() && self.missing.is_empty() && self.orphaned.is_empty()
    }
}

/// In-memory representation of a vault's encrypted file index.
///
/// Loaded (decrypted) on unlock and written back (encrypted) on every change.
/// Only ever persisted via the `index` module's encrypted writer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultIndex {
    /// All files currently stored in the vault.
    #[serde(default)]
    pub entries: Vec<VaultEntry>,
}

impl VaultIndex {
    /// Create an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of files tracked by the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the index tracks no files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Find an entry by its original plaintext filename.
    #[must_use]
    pub fn find_by_original(&self, name: &str) -> Option<&VaultEntry> {
        self.entries.iter().find(|e| e.original_name == name)
    }

    /// Find an entry by its opaque on-disk ciphertext filename.
    #[must_use]
    pub fn find_by_stored(&self, stored: &str) -> Option<&VaultEntry> {
        self.entries.iter().find(|e| e.stored_name == stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, path: &str) -> VaultConfig {
        VaultConfig {
            name: name.into(),
            path: PathBuf::from(path),
            auto_lock_minutes: Some(5),
            scheme: EncryptionScheme::default(),
            integrity_hash: IntegrityHash::default(),
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(cfg("Personal", "/home/u/.vaults/personal")
            .validate()
            .is_ok());
    }

    #[test]
    fn empty_name_rejected() {
        assert!(matches!(
            cfg("   ", "/abs/path").validate(),
            Err(crate::VaultError::Config(_))
        ));
    }

    #[test]
    fn relative_path_rejected() {
        assert!(matches!(
            cfg("V", "relative/path").validate(),
            Err(crate::VaultError::Config(_))
        ));
    }

    #[test]
    fn auto_lock_enabled_logic() {
        let mut c = cfg("V", "/abs");
        assert!(c.auto_lock_enabled());
        c.auto_lock_minutes = Some(0);
        assert!(!c.auto_lock_enabled());
        c.auto_lock_minutes = None;
        assert!(!c.auto_lock_enabled());
    }

    #[test]
    fn scheme_and_hash_defaults_are_stable_choices() {
        assert_eq!(EncryptionScheme::default(), EncryptionScheme::Age);
        assert_eq!(IntegrityHash::default(), IntegrityHash::Blake3);
    }

    #[test]
    fn config_without_scheme_fields_uses_defaults() {
        // Older configs omit scheme/integrity_hash; serde defaults must apply.
        let s = r#"
            name = "Old"
            path = "/abs/old"
        "#;
        let c: VaultConfig = toml::from_str(s).expect("deserialize");
        assert_eq!(c.scheme, EncryptionScheme::Age);
        assert_eq!(c.integrity_hash, IntegrityHash::Blake3);
        assert_eq!(c.auto_lock_minutes, None);
    }

    #[test]
    fn scheme_serializes_kebab_case() {
        let c = VaultConfig {
            name: "V".into(),
            path: "/abs".into(),
            auto_lock_minutes: None,
            scheme: EncryptionScheme::Argon2idXchacha20,
            integrity_hash: IntegrityHash::Sha256,
        };
        let s = toml::to_string(&c).expect("serialize");
        assert!(s.contains("argon2id-xchacha20"), "got: {s}");
        assert!(s.contains("sha256"), "got: {s}");
        let back: VaultConfig = toml::from_str(&s).expect("deserialize");
        assert_eq!(c, back);
    }

    #[test]
    fn state_only_unlocked_is_accessible() {
        assert!(VaultState::Unlocked.is_unlocked());
        assert!(!VaultState::Locked.is_unlocked());
        assert!(!VaultState::AutoLocking.is_unlocked());
    }

    #[test]
    fn index_lookup_by_original_and_stored() {
        let mut idx = VaultIndex::new();
        assert!(idx.is_empty());
        idx.entries.push(VaultEntry {
            original_name: "secret.txt".into(),
            stored_name: "abcd.age".into(),
            size: 42,
            added_unix: 1_700_000_000,
            content_hash: "deadbeef".into(),
        });
        assert_eq!(idx.len(), 1);
        assert!(idx.find_by_original("secret.txt").is_some());
        assert!(idx.find_by_stored("abcd.age").is_some());
        assert!(idx.find_by_original("missing").is_none());
    }

    #[test]
    fn config_toml_roundtrip() {
        let c = cfg("Personal", "/home/u/.vaults/personal");
        let s = toml::to_string(&c).expect("serialize");
        let back: VaultConfig = toml::from_str(&s).expect("deserialize");
        assert_eq!(c, back);
    }

    #[test]
    fn index_serde_roundtrip() {
        let mut idx = VaultIndex::new();
        idx.entries.push(VaultEntry {
            original_name: "a.bin".into(),
            stored_name: "x.age".into(),
            size: 7,
            added_unix: 1,
            content_hash: "00".into(),
        });
        let s = toml::to_string(&idx).expect("serialize");
        let back: VaultIndex = toml::from_str(&s).expect("deserialize");
        assert_eq!(idx, back);
    }
}
