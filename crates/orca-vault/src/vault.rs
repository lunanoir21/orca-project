//! Vault lifecycle: create, open, lock, passphrase verification
//! (security-critical).
//!
//! A vault is a directory containing:
//! ```text
//! <vault>/
//!   meta.toml   plaintext: format version, scheme, integrity hash, Argon2id
//!               parameters, salt, and a sealed passphrase verifier
//!   index.age   the VaultIndex, sealed with the master key
//!   data/       sealed file blobs (populated by vault file ops, Phase 2.5)
//! ```
//!
//! # Why `meta.toml` is plaintext
//! Deriving the master key needs the salt and Argon2id parameters, so those
//! cannot themselves be stored encrypted-under-that-key (a chicken-and-egg).
//! Salts and KDF parameters are **not secret** (their only job is to defeat
//! precomputation), so storing them in the clear is correct and standard. No
//! key material or passphrase is ever written to `meta.toml`.
//!
//! # Master key
//! The master key is `Argon2id(passphrase, salt, params)`, held only while the
//! vault is unlocked and zeroized on [`Vault::lock`] or drop. Passphrase
//! correctness is checked by sealing a fixed [`VERIFIER_PLAINTEXT`] on create
//! and authenticating it on open — a wrong passphrase fails the AEAD tag and
//! never yields a usable key.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::blob;
use crate::crypto::{derive_key, gen_salt, DerivedKey, KdfParams};
use crate::file_crypto::{xchacha_open_core, xchacha_seal_core};
use crate::util::{from_hex, to_hex, Hasher};
use crate::{
    EncryptionScheme, IntegrityHash, IntegrityReport, Result, VaultConfig, VaultEntry, VaultError,
    VaultIndex, VaultState,
};

/// Current on-disk meta format version.
const META_VERSION: u32 = 1;
/// Meta filename within the vault directory.
const META_FILE: &str = "meta.toml";
/// Sealed-index filename within the vault directory.
const INDEX_FILE: &str = "index.age";
/// Subdirectory holding sealed file blobs.
const DATA_DIR: &str = "data";
/// Fixed plaintext sealed on create and checked on open to verify the
/// passphrase without storing or exposing the key.
const VERIFIER_PLAINTEXT: &[u8] = b"orca-vault-passphrase-verifier-v1";
/// Magic for an encrypted vault backup (`.tar.age`).
const BACKUP_MAGIC: &[u8; 8] = b"ORCABKP1";

/// Plaintext, on-disk vault metadata. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VaultMeta {
    /// Meta format version, for forward compatibility.
    version: u32,
    /// File-content encryption scheme chosen for this vault.
    scheme: EncryptionScheme,
    /// Content integrity hash algorithm.
    integrity_hash: IntegrityHash,
    /// Argon2id cost parameters used to derive the master key.
    kdf: KdfParams,
    /// Hex-encoded KDF salt (public, unique per vault).
    salt_hex: String,
    /// Hex-encoded sealed passphrase verifier.
    verifier_hex: String,
}

/// An Orca vault. Unlocked when it holds a master key, locked otherwise.
pub struct Vault {
    config: VaultConfig,
    meta: VaultMeta,
    /// Master key, present only while unlocked. Zeroized on drop/lock.
    key: Option<DerivedKey>,
    /// Decrypted index, present only while unlocked.
    index: Option<VaultIndex>,
}

impl std::fmt::Debug for Vault {
    /// Redacted debug output: never prints key material or index contents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("name", &self.config.name)
            .field("path", &self.config.path)
            .field("scheme", &self.meta.scheme)
            .field("locked", &self.is_locked())
            .finish_non_exhaustive()
    }
}

impl Vault {
    /// Create a new vault at `config.path` and return it unlocked.
    ///
    /// # Errors
    /// Returns [`VaultError::Config`] for an invalid config,
    /// [`VaultError::AlreadyExists`] if the directory already contains a vault,
    /// or an I/O / crypto error.
    pub fn create(config: VaultConfig, passphrase: &[u8]) -> Result<Self> {
        config.validate()?;
        let dir = config.path.clone();
        let meta_path = dir.join(META_FILE);
        if meta_path.exists() {
            return Err(VaultError::AlreadyExists(meta_path));
        }
        std::fs::create_dir_all(&dir).map_err(|e| VaultError::from_io(&dir, e))?;
        std::fs::create_dir_all(dir.join(DATA_DIR))
            .map_err(|e| VaultError::from_io(dir.join(DATA_DIR), e))?;

        let params = KdfParams::default();
        let salt = gen_salt()?;
        let key = derive_key(passphrase, &salt, params)?;

        let verifier = blob::seal(&key, VERIFIER_PLAINTEXT)?;
        let meta = VaultMeta {
            version: META_VERSION,
            scheme: config.scheme,
            integrity_hash: config.integrity_hash,
            kdf: params,
            salt_hex: to_hex(&salt),
            verifier_hex: to_hex(&verifier),
        };
        write_meta(&meta_path, &meta)?;

        let index = VaultIndex::new();
        write_index(&dir.join(INDEX_FILE), &key, &index)?;

        Ok(Self {
            config,
            meta,
            key: Some(key),
            index: Some(index),
        })
    }

    /// Open an existing vault, verifying the passphrase, and return it unlocked.
    ///
    /// # Errors
    /// Returns [`VaultError::InvalidVault`] if no vault is present,
    /// [`VaultError::WrongPassphrase`] if the passphrase does not authenticate,
    /// or an I/O / parse error.
    pub fn open(config: VaultConfig, passphrase: &[u8]) -> Result<Self> {
        config.validate()?;
        let dir = config.path.clone();
        let meta_path = dir.join(META_FILE);
        if !meta_path.exists() {
            return Err(VaultError::InvalidVault(dir));
        }
        let meta = read_meta(&meta_path)?;
        let key = derive_key_from_meta(passphrase, &meta)?;

        // Authenticate the passphrase against the sealed verifier.
        let verifier = from_hex(&meta.verifier_hex)?;
        let opened = blob::open(&key, &verifier)?;
        if opened != VERIFIER_PLAINTEXT {
            // Should be unreachable (AEAD would have failed first), but never
            // proceed with an unverified key.
            return Err(VaultError::WrongPassphrase);
        }

        let index = read_index(&dir.join(INDEX_FILE), &key)?;

        Ok(Self {
            config,
            meta,
            key: Some(key),
            index: Some(index),
        })
    }

    /// Lock the vault: drop the master key (zeroized) and decrypted index.
    ///
    /// # Errors
    /// Infallible today; returns [`Result`] so future flush logic can report
    /// errors without an API break.
    pub fn lock(&mut self) -> Result<()> {
        // Dropping DerivedKey zeroizes the key bytes (see crypto::DerivedKey).
        self.key = None;
        self.index = None;
        Ok(())
    }

    /// Returns `true` if the vault is locked (no master key in memory).
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.key.is_none()
    }

    /// Current [`VaultState`].
    #[must_use]
    pub fn state(&self) -> VaultState {
        if self.key.is_some() {
            VaultState::Unlocked
        } else {
            VaultState::Locked
        }
    }

    /// Check whether `passphrase` would unlock this vault, without changing its
    /// lock state or retaining the derived key.
    ///
    /// # Errors
    /// Returns an error only for malformed meta or a KDF failure; an incorrect
    /// passphrase yields `Ok(false)`.
    pub fn verify_passphrase(&self, passphrase: &[u8]) -> Result<bool> {
        let key = derive_key_from_meta(passphrase, &self.meta)?;
        let verifier = from_hex(&self.meta.verifier_hex)?;
        match blob::open(&key, &verifier) {
            Ok(p) if p == VERIFIER_PLAINTEXT => Ok(true),
            Ok(_) | Err(VaultError::WrongPassphrase) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The vault's configuration.
    #[must_use]
    pub fn config(&self) -> &VaultConfig {
        &self.config
    }

    /// The vault's encryption scheme.
    #[must_use]
    pub fn scheme(&self) -> EncryptionScheme {
        self.meta.scheme
    }

    /// Borrow the decrypted index. Returns [`VaultError::Locked`] when locked.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if the vault is locked.
    pub fn index(&self) -> Result<&VaultIndex> {
        self.index.as_ref().ok_or(VaultError::Locked)
    }

    /// List the files currently stored in the vault (clones of the index
    /// entries).
    ///
    /// # Errors
    /// [`VaultError::Locked`] if the vault is locked.
    pub fn list_files(&self) -> Result<Vec<VaultEntry>> {
        Ok(self.index()?.entries.clone())
    }

    fn key(&self) -> Result<&DerivedKey> {
        self.key.as_ref().ok_or(VaultError::Locked)
    }

    fn data_dir(&self) -> PathBuf {
        self.config.path.join(DATA_DIR)
    }

    fn index_path(&self) -> PathBuf {
        self.config.path.join(INDEX_FILE)
    }

    /// Encrypt `src` into the vault and record it in the index. The original
    /// file is left untouched (the GUI deletes it after a successful add).
    ///
    /// Streams the source through the configured integrity hasher and the master
    /// key, so large files never load fully into memory.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, [`VaultError::AlreadyExists`] if a file
    /// of the same original name is already in the vault, or an I/O / crypto
    /// error.
    pub fn add_file(&mut self, src: &Path) -> Result<()> {
        let name = bare_name(src)?;
        if self.index()?.find_by_original(&name).is_some() {
            return Err(VaultError::AlreadyExists(PathBuf::from(name)));
        }

        let stored = random_stored_name()?;
        let dest = self.data_dir().join(&stored);
        let input = File::open(src).map_err(|e| VaultError::from_io(src, e))?;

        // Stream: hash + count plaintext while sealing it to the data file.
        let mut hasher = Hasher::new(self.meta.integrity_hash);
        let mut size: u64 = 0;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|e| VaultError::from_io(&dest, e))?;

        let seal = {
            let reader = HashingReader {
                inner: input,
                hasher: &mut hasher,
                count: &mut size,
            };
            xchacha_seal_core(reader, &mut out, self.key()?)
        };
        if let Err(e) = seal.and_then(|()| {
            use std::io::Write as _;
            out.flush().map_err(VaultError::BareIo)
        }) {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }

        let entry = VaultEntry {
            original_name: name,
            stored_name: stored,
            size,
            added_unix: now_unix(),
            content_hash: hasher.finish_hex(),
        };
        let path = self.index_path();
        let key = self.key.as_ref().ok_or(VaultError::Locked)?;
        let index = self.index.as_mut().ok_or(VaultError::Locked)?;
        index.entries.push(entry);
        let result = write_index(&path, key, index);
        if result.is_err() {
            // Roll back the in-memory push and the data file so disk and memory
            // stay consistent on a failed index write.
            index.entries.pop();
            let _ = std::fs::remove_file(&dest);
        }
        result
    }

    /// Extract a vault file by its original name into `dest_dir`, verifying its
    /// content hash. Returns the restored file path.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, [`VaultError::NotFound`] if the name is
    /// not in the vault, [`VaultError::IntegrityFailure`] if the decrypted
    /// content does not match the stored hash, or an I/O / crypto error.
    pub fn extract_file(&self, original_name: &str, dest_dir: &Path) -> Result<PathBuf> {
        let entry = self
            .index()?
            .find_by_original(original_name)
            .ok_or_else(|| VaultError::NotFound(PathBuf::from(original_name)))?
            .clone();

        let src = self.data_dir().join(&entry.stored_name);
        let input = File::open(&src).map_err(|e| VaultError::from_io(&src, e))?;
        let dest = dest_dir.join(&entry.original_name);
        let out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|e| VaultError::from_io(&dest, e))?;

        // Hash the plaintext as it is written, then compare to the index.
        let mut hasher = Hasher::new(self.meta.integrity_hash);
        let mut writer = HashingWriter {
            inner: out,
            hasher: &mut hasher,
        };
        let result = xchacha_open_core(input, &mut writer, self.key()?);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
        if hasher.finish_hex() != entry.content_hash {
            let _ = std::fs::remove_file(&dest);
            return Err(VaultError::IntegrityFailure(format!(
                "content hash mismatch for {original_name}"
            )));
        }
        Ok(dest)
    }

    /// Remove a file from the vault by its original name.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, [`VaultError::NotFound`] if absent, or
    /// an I/O error.
    pub fn remove_file(&mut self, original_name: &str) -> Result<()> {
        let stored = {
            let entry = self
                .index()?
                .find_by_original(original_name)
                .ok_or_else(|| VaultError::NotFound(PathBuf::from(original_name)))?;
            entry.stored_name.clone()
        };
        let data = self.data_dir().join(&stored);
        // Remove the ciphertext first; a missing data file is tolerated so the
        // index can still be cleaned up.
        match std::fs::remove_file(&data) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(VaultError::from_io(&data, e)),
        }
        let path = self.index_path();
        let key = self.key.as_ref().ok_or(VaultError::Locked)?;
        let index = self.index.as_mut().ok_or(VaultError::Locked)?;
        index.entries.retain(|e| e.original_name != original_name);
        write_index(&path, key, index)
    }

    /// Rename a file within the vault. The ciphertext on disk is unchanged; only
    /// the index entry's original name is updated.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, [`VaultError::NotFound`] if absent,
    /// [`VaultError::AlreadyExists`] if `new_name` is taken, or
    /// [`VaultError::Config`] if `new_name` is not a bare filename.
    pub fn move_file(&mut self, original_name: &str, new_name: &str) -> Result<()> {
        validate_bare_name(new_name)?;
        if self.index()?.find_by_original(new_name).is_some() {
            return Err(VaultError::AlreadyExists(PathBuf::from(new_name)));
        }
        let path = self.index_path();
        let key = self.key.as_ref().ok_or(VaultError::Locked)?;
        let index = self.index.as_mut().ok_or(VaultError::Locked)?;
        let entry = index
            .entries
            .iter_mut()
            .find(|e| e.original_name == original_name)
            .ok_or_else(|| VaultError::NotFound(PathBuf::from(original_name)))?;
        entry.original_name = new_name.to_owned();
        write_index(&path, key, index)
    }

    /// Verify every vault file against the index: confirm each data blob
    /// decrypts and its content hash matches, and detect missing or orphaned
    /// blobs. The vault must be unlocked.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, or an unexpected I/O error. A failed
    /// hash check or AEAD authentication is reported in the returned
    /// [`IntegrityReport`], not as an error.
    pub fn verify_integrity(&self) -> Result<IntegrityReport> {
        let key = self.key()?;
        let index = self.index()?;
        let data_dir = self.data_dir();
        let mut report = IntegrityReport::default();
        let mut referenced = std::collections::HashSet::new();

        for entry in &index.entries {
            referenced.insert(entry.stored_name.clone());
            let path = data_dir.join(&entry.stored_name);
            if !path.exists() {
                report.missing.push(entry.original_name.clone());
                continue;
            }
            match verify_one(&path, key, self.meta.integrity_hash, &entry.content_hash) {
                Ok(true) => report.ok.push(entry.original_name.clone()),
                // Hash mismatch, or AEAD/format failure ⇒ tampered/corrupt blob.
                Ok(false) | Err(VaultError::WrongPassphrase) | Err(VaultError::Corrupt(_)) => {
                    report.tampered.push(entry.original_name.clone());
                }
                Err(e) => return Err(e),
            }
        }

        // Any data file not referenced by the index is orphaned.
        if let Ok(rd) = std::fs::read_dir(&data_dir) {
            for ent in rd.flatten() {
                if let Some(n) = ent.file_name().to_str() {
                    if !referenced.contains(n) {
                        report.orphaned.push(n.to_owned());
                    }
                }
            }
        }
        Ok(report)
    }

    /// Export the whole vault directory as an encrypted `.tar.age` backup at
    /// `dest`. The vault must be unlocked.
    ///
    /// The archive is sealed with the vault's master key; the backup header
    /// records the (non-secret) Argon2id salt/params so the same passphrase can
    /// re-derive the key on import. The AEAD tag authenticates the whole stream,
    /// providing integrity verification.
    ///
    /// # Errors
    /// [`VaultError::Locked`] if locked, or an I/O / crypto error.
    pub fn export_backup(&self, dest: &Path) -> Result<()> {
        let key = self.key()?;

        // 1. Tar the vault directory to a temporary plaintext archive. It holds
        //    only already-encrypted blobs + the non-secret meta, but it is
        //    removed promptly regardless.
        let tar_tmp = temp_sibling(dest, "tar");
        let tar_result = (|| -> Result<()> {
            let file = File::create(&tar_tmp).map_err(|e| VaultError::from_io(&tar_tmp, e))?;
            let mut builder = tar::Builder::new(file);
            builder
                .append_dir_all(".", &self.config.path)
                .map_err(|e| VaultError::Other(format!("tar build failed: {e}")))?;
            builder
                .finish()
                .map_err(|e| VaultError::Other(format!("tar finalize failed: {e}")))?;
            Ok(())
        })();
        if let Err(e) = tar_result {
            let _ = std::fs::remove_file(&tar_tmp);
            return Err(e);
        }

        // 2. Seal the archive to `dest`, prefixed by the backup header.
        let seal = (|| -> Result<()> {
            let tar_in = File::open(&tar_tmp).map_err(|e| VaultError::from_io(&tar_tmp, e))?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dest)
                .map_err(|e| VaultError::from_io(dest, e))?;
            write_backup_header(&mut out, &self.meta)?;
            xchacha_seal_core(tar_in, &mut out, key)?;
            use std::io::Write as _;
            out.flush().map_err(VaultError::BareIo)
        })();

        let _ = std::fs::remove_file(&tar_tmp);
        if seal.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        seal
    }

    /// Import an encrypted backup created by [`export_backup`] into `dest_dir`
    /// (which becomes the restored vault directory), and open it under `name`.
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] for a malformed backup, [`VaultError::WrongPassphrase`]
    /// if the passphrase does not authenticate the archive or the restored vault,
    /// or an I/O error.
    pub fn import_backup(
        src: &Path,
        passphrase: &[u8],
        dest_dir: &Path,
        name: &str,
    ) -> Result<Self> {
        let mut input = File::open(src).map_err(|e| VaultError::from_io(src, e))?;
        let (params, salt) = read_backup_header(&mut input)?;
        let key = derive_key(passphrase, &salt, params)?;

        // Decrypt to a temporary archive (AEAD authenticates integrity here).
        let tar_tmp = temp_sibling(dest_dir, "tar");
        let decrypt = (|| -> Result<()> {
            let mut out = File::create(&tar_tmp).map_err(|e| VaultError::from_io(&tar_tmp, e))?;
            xchacha_open_core(input, &mut out, &key)?;
            use std::io::Write as _;
            out.flush().map_err(VaultError::BareIo)
        })();
        if let Err(e) = decrypt {
            let _ = std::fs::remove_file(&tar_tmp);
            return Err(e);
        }

        // Unpack into the destination. `tar`'s unpack rejects `..` traversal.
        let unpack = (|| -> Result<()> {
            std::fs::create_dir_all(dest_dir).map_err(|e| VaultError::from_io(dest_dir, e))?;
            let file = File::open(&tar_tmp).map_err(|e| VaultError::from_io(&tar_tmp, e))?;
            let mut archive = tar::Archive::new(file);
            archive
                .unpack(dest_dir)
                .map_err(|e| VaultError::Corrupt(format!("backup unpack failed: {e}")))
        })();
        let _ = std::fs::remove_file(&tar_tmp);
        unpack?;

        // Open the restored vault, which re-verifies the passphrase via the meta
        // verifier — a second integrity check.
        let config = VaultConfig {
            name: name.to_owned(),
            path: dest_dir.to_path_buf(),
            auto_lock_minutes: None,
            scheme: EncryptionScheme::default(),
            integrity_hash: IntegrityHash::default(),
        };
        Vault::open(config, passphrase)
    }
}

/// Build a unique temp path next to `near` with the given extension tag.
fn temp_sibling(near: &Path, tag: &str) -> PathBuf {
    let parent = near.parent().unwrap_or_else(|| Path::new("."));
    let unique = format!(".orca-backup-{}-{}.{}", std::process::id(), now_unix(), tag);
    parent.join(unique)
}

/// Write the plaintext backup header: magic + Argon2id params + salt.
fn write_backup_header(out: &mut impl io::Write, meta: &VaultMeta) -> Result<()> {
    let salt = from_hex(&meta.salt_hex)?;
    out.write_all(BACKUP_MAGIC)
        .and_then(|()| out.write_all(&meta.kdf.memory_kib.to_le_bytes()))
        .and_then(|()| out.write_all(&meta.kdf.iterations.to_le_bytes()))
        .and_then(|()| out.write_all(&meta.kdf.parallelism.to_le_bytes()))
        .and_then(|()| out.write_all(&salt))
        .map_err(VaultError::BareIo)
}

/// Read and validate the backup header, returning the KDF params and salt.
fn read_backup_header(input: &mut impl Read) -> Result<(KdfParams, Vec<u8>)> {
    let mut magic = [0u8; 8];
    read_into(input, &mut magic, "missing backup magic")?;
    if &magic != BACKUP_MAGIC {
        return Err(VaultError::Corrupt("not an Orca backup".into()));
    }
    let mut m = [0u8; 4];
    let mut t = [0u8; 4];
    let mut p = [0u8; 4];
    read_into(input, &mut m, "missing kdf memory")?;
    read_into(input, &mut t, "missing kdf iterations")?;
    read_into(input, &mut p, "missing kdf parallelism")?;
    let params = KdfParams {
        memory_kib: u32::from_le_bytes(m),
        iterations: u32::from_le_bytes(t),
        parallelism: u32::from_le_bytes(p),
    };
    let mut salt = vec![0u8; crate::SALT_LEN];
    read_into(input, &mut salt, "missing salt")?;
    Ok((params, salt))
}

fn read_into(r: &mut impl Read, buf: &mut [u8], what: &str) -> Result<()> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(VaultError::BareIo(e)),
        }
    }
    if n != buf.len() {
        return Err(VaultError::Corrupt(format!("truncated backup: {what}")));
    }
    Ok(())
}

fn derive_key_from_meta(passphrase: &[u8], meta: &VaultMeta) -> Result<DerivedKey> {
    let salt = from_hex(&meta.salt_hex)?;
    derive_key(passphrase, &salt, meta.kdf)
}

fn write_meta(path: &Path, meta: &VaultMeta) -> Result<()> {
    let text =
        toml::to_string(meta).map_err(|e| VaultError::Other(format!("serialize meta: {e}")))?;
    std::fs::write(path, text).map_err(|e| VaultError::from_io(path, e))
}

fn read_meta(path: &Path) -> Result<VaultMeta> {
    let text = std::fs::read_to_string(path).map_err(|e| VaultError::from_io(path, e))?;
    let meta: VaultMeta =
        toml::from_str(&text).map_err(|e| VaultError::Corrupt(format!("parse meta: {e}")))?;
    if meta.version != META_VERSION {
        return Err(VaultError::Corrupt(format!(
            "unsupported meta version {}",
            meta.version
        )));
    }
    Ok(meta)
}

/// Seal and write the index. `pub(crate)` so Phase 2.6 index ops can persist.
pub(crate) fn write_index(path: &Path, key: &DerivedKey, index: &VaultIndex) -> Result<()> {
    let text =
        toml::to_string(index).map_err(|e| VaultError::Index(format!("serialize index: {e}")))?;
    let sealed = blob::seal(key, text.as_bytes())?;
    // Atomic write: temp file then rename, so a crash never leaves a torn index.
    let tmp = path.with_extension("age.tmp");
    std::fs::write(&tmp, &sealed).map_err(|e| VaultError::from_io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| VaultError::from_io(path, e))
}

pub(crate) fn read_index(path: &Path, key: &DerivedKey) -> Result<VaultIndex> {
    let sealed = std::fs::read(path).map_err(|e| VaultError::from_io(path, e))?;
    let plain = blob::open(key, &sealed)?;
    let text = std::str::from_utf8(&plain)
        .map_err(|_| VaultError::Index("index is not valid UTF-8".into()))?;
    toml::from_str(text).map_err(|e| VaultError::Index(format!("parse index: {e}")))
}

/// Decrypt a data blob into a discarding hasher and compare to `expected`.
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch; AEAD/format failures
/// surface as `Err` for the caller to classify as tampered.
fn verify_one(path: &Path, key: &DerivedKey, algo: IntegrityHash, expected: &str) -> Result<bool> {
    let input = File::open(path).map_err(|e| VaultError::from_io(path, e))?;
    let mut hasher = Hasher::new(algo);
    let mut sink = HashingWriter {
        inner: io::sink(),
        hasher: &mut hasher,
    };
    xchacha_open_core(input, &mut sink, key)?;
    Ok(hasher.finish_hex() == expected)
}

/// Extract and validate the bare filename of `path`.
fn bare_name(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| VaultError::Config("source has no valid UTF-8 filename".into()))?;
    validate_bare_name(name)?;
    Ok(name.to_owned())
}

/// Reject names that are empty, `.`/`..`, or contain a path separator — a vault
/// entry name must be a single component so extraction cannot traverse paths.
fn validate_bare_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || Path::new(name).components().count() != 1
    {
        return Err(VaultError::Config(format!("not a bare filename: {name:?}")));
    }
    Ok(())
}

/// Generate a random opaque on-disk name (`<32 hex chars>.age`) for a vault blob
/// so the encrypted directory listing leaks nothing about original filenames.
fn random_stored_name() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| VaultError::Crypto(format!("CSPRNG unavailable: {e}")))?;
    Ok(format!("{}.age", to_hex(&bytes)))
}

/// Current time as seconds since the Unix epoch (UTC). Returns 0 if the system
/// clock is set before 1970 (entries simply get an epoch timestamp).
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A [`Read`] adapter that hashes and counts bytes as they flow through, so a
/// file can be hashed in the same streaming pass that encrypts it.
struct HashingReader<'a, R: Read> {
    inner: R,
    hasher: &'a mut Hasher,
    count: &'a mut u64,
}

impl<R: Read> Read for HashingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
            *self.count += n as u64;
        }
        Ok(n)
    }
}

/// A [`Write`] adapter that hashes plaintext as it is written out, so extracted
/// contents can be integrity-checked in the same streaming pass.
struct HashingWriter<'a, W: io::Write> {
    inner: W,
    hasher: &'a mut Hasher,
}

impl<W: io::Write> io::Write for HashingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        if n > 0 {
            self.hasher.update(&buf[..n]);
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_vault_dir() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "orca-vault-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        d
    }

    fn cfg(dir: &Path) -> VaultConfig {
        VaultConfig {
            name: "Test".into(),
            path: dir.to_path_buf(),
            auto_lock_minutes: Some(5),
            scheme: EncryptionScheme::Age,
            integrity_hash: IntegrityHash::Blake3,
        }
    }

    #[test]
    fn create_then_open_roundtrip() {
        let dir = tmp_vault_dir();
        let v = Vault::create(cfg(&dir), b"correct horse battery").expect("create");
        assert!(!v.is_locked());
        assert_eq!(v.index().unwrap().len(), 0);
        drop(v);

        let v2 = Vault::open(cfg(&dir), b"correct horse battery").expect("open");
        assert_eq!(v2.state(), VaultState::Unlocked);
        assert_eq!(v2.index().unwrap().len(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_with_wrong_passphrase_fails() {
        let dir = tmp_vault_dir();
        Vault::create(cfg(&dir), b"right").expect("create");
        let err = Vault::open(cfg(&dir), b"wrong").unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lock_clears_key_and_index() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        v.lock().expect("lock");
        assert!(v.is_locked());
        assert_eq!(v.state(), VaultState::Locked);
        assert!(matches!(v.index(), Err(VaultError::Locked)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_passphrase_does_not_change_state() {
        let dir = tmp_vault_dir();
        let v = Vault::create(cfg(&dir), b"pw").expect("create");
        assert!(v.verify_passphrase(b"pw").expect("verify"));
        assert!(!v.verify_passphrase(b"nope").expect("verify"));
        // State unchanged: still unlocked.
        assert!(!v.is_locked());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_refuses_existing_vault() {
        let dir = tmp_vault_dir();
        Vault::create(cfg(&dir), b"pw").expect("create");
        let err = Vault::create(cfg(&dir), b"pw").unwrap_err();
        assert!(matches!(err, VaultError::AlreadyExists(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_missing_vault_is_invalid() {
        let dir = tmp_vault_dir();
        let err = Vault::open(cfg(&dir), b"pw").unwrap_err();
        assert!(matches!(err, VaultError::InvalidVault(_)));
    }

    #[test]
    fn meta_is_plaintext_and_holds_no_passphrase() {
        let dir = tmp_vault_dir();
        Vault::create(cfg(&dir), b"super-secret-pass").expect("create");
        let meta_text = std::fs::read_to_string(dir.join(META_FILE)).expect("read meta");
        assert!(meta_text.contains("version"));
        assert!(meta_text.contains("salt_hex"));
        // The passphrase must never appear in plaintext meta.
        assert!(!meta_text.contains("super-secret-pass"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn index_file_is_not_plaintext() {
        let dir = tmp_vault_dir();
        Vault::create(cfg(&dir), b"pw").expect("create");
        let raw = std::fs::read(dir.join(INDEX_FILE)).expect("read index");
        // Sealed: must not contain the TOML key name in the clear.
        assert!(!raw.windows(7).any(|w| w == b"entries"));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn cfg_hash(dir: &Path, algo: IntegrityHash) -> VaultConfig {
        let mut c = cfg(dir);
        c.integrity_hash = algo;
        c
    }

    fn make_src(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).expect("write src");
        p
    }

    fn run_add_extract_roundtrip(algo: IntegrityHash, contents: &[u8]) {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg_hash(&dir, algo), b"pw").expect("create");
        let src = make_src(&dir, "secret.txt", contents);

        v.add_file(&src).expect("add");
        let files = v.list_files().expect("list");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].original_name, "secret.txt");
        assert_eq!(files[0].size, contents.len() as u64);
        assert_ne!(files[0].stored_name, "secret.txt");

        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let restored = v.extract_file("secret.txt", &out).expect("extract");
        assert_eq!(std::fs::read(&restored).unwrap(), contents);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_extract_roundtrip_blake3() {
        run_add_extract_roundtrip(IntegrityHash::Blake3, b"classified material");
    }

    #[test]
    fn add_extract_roundtrip_sha256_large() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        run_add_extract_roundtrip(IntegrityHash::Sha256, &data);
    }

    #[test]
    fn data_survives_lock_and_reopen() {
        let dir = tmp_vault_dir();
        {
            let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
            let src = make_src(&dir, "a.bin", b"persisted bytes");
            v.add_file(&src).expect("add");
        }
        let v2 = Vault::open(cfg(&dir), b"pw").expect("reopen");
        assert_eq!(v2.list_files().unwrap().len(), 1);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let restored = v2.extract_file("a.bin", &out).expect("extract");
        assert_eq!(std::fs::read(restored).unwrap(), b"persisted bytes");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_add_is_rejected() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        let src = make_src(&dir, "dup.txt", b"x");
        v.add_file(&src).expect("add");
        assert!(matches!(
            v.add_file(&src),
            Err(VaultError::AlreadyExists(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_file_deletes_data_and_entry() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        let src = make_src(&dir, "r.txt", b"bye");
        v.add_file(&src).expect("add");
        let stored = v.list_files().unwrap()[0].stored_name.clone();
        v.remove_file("r.txt").expect("remove");
        assert_eq!(v.list_files().unwrap().len(), 0);
        assert!(!dir.join(DATA_DIR).join(stored).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn move_file_renames_entry() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        let src = make_src(&dir, "old.txt", b"data");
        v.add_file(&src).expect("add");
        v.move_file("old.txt", "new.txt").expect("move");
        assert!(v.index().unwrap().find_by_original("new.txt").is_some());
        assert!(v.index().unwrap().find_by_original("old.txt").is_none());
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let restored = v.extract_file("new.txt", &out).expect("extract");
        assert_eq!(std::fs::read(restored).unwrap(), b"data");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extract_missing_is_not_found() {
        let dir = tmp_vault_dir();
        let v = Vault::create(cfg(&dir), b"pw").expect("create");
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert!(matches!(
            v.extract_file("nope.txt", &out),
            Err(VaultError::NotFound(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extract_detects_tampered_data() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        let src = make_src(&dir, "t.bin", b"important integrity-checked data");
        v.add_file(&src).expect("add");
        let stored = v.list_files().unwrap()[0].stored_name.clone();

        // Corrupt the ciphertext on disk.
        let data_path = dir.join(DATA_DIR).join(&stored);
        let mut bytes = std::fs::read(&data_path).unwrap();
        let i = bytes.len() - 3;
        bytes[i] ^= 0xff;
        std::fs::write(&data_path, &bytes).unwrap();

        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        // AEAD fails first (WrongPassphrase) or hash mismatch (IntegrityFailure);
        // either way the corruption is caught and no file is left behind.
        let err = v.extract_file("t.bin", &out).unwrap_err();
        assert!(matches!(
            err,
            VaultError::WrongPassphrase | VaultError::IntegrityFailure(_)
        ));
        assert!(std::fs::read_dir(&out).unwrap().next().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ops_fail_when_locked() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        let src = make_src(&dir, "l.txt", b"x");
        v.lock().expect("lock");
        assert!(matches!(v.add_file(&src), Err(VaultError::Locked)));
        assert!(matches!(v.list_files(), Err(VaultError::Locked)));
        assert!(matches!(v.remove_file("l.txt"), Err(VaultError::Locked)));
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        assert!(matches!(
            v.extract_file("l.txt", &out),
            Err(VaultError::Locked)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backup_export_import_roundtrip() {
        let base = tmp_vault_dir();
        let vault_dir = base.join("vault");
        let mut v = Vault::create(cfg(&vault_dir), b"backup-pass").expect("create");
        let src = make_src(&base, "doc.txt", b"backup me please");
        v.add_file(&src).expect("add");

        let backup = base.join("vault.tar.age");
        v.export_backup(&backup).expect("export");
        assert!(backup.exists());

        let restored_dir = base.join("restored");
        let v2 = Vault::import_backup(&backup, b"backup-pass", &restored_dir, "Restored")
            .expect("import");
        assert_eq!(v2.list_files().unwrap().len(), 1);
        let out = base.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let f = v2.extract_file("doc.txt", &out).expect("extract");
        assert_eq!(std::fs::read(f).unwrap(), b"backup me please");

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn import_wrong_passphrase_fails() {
        let base = tmp_vault_dir();
        let v = Vault::create(cfg(&base.join("vault")), b"right").expect("create");
        let backup = base.join("b.tar.age");
        v.export_backup(&backup).expect("export");

        let err = Vault::import_backup(&backup, b"wrong", &base.join("restored"), "R").unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn import_detects_tampered_backup() {
        let base = tmp_vault_dir();
        let mut v = Vault::create(cfg(&base.join("vault")), b"pw").expect("create");
        v.add_file(&make_src(&base, "x.bin", b"data integrity"))
            .expect("add");
        let backup = base.join("b.tar.age");
        v.export_backup(&backup).expect("export");

        let mut bytes = std::fs::read(&backup).unwrap();
        let i = bytes.len() - 4;
        bytes[i] ^= 0xff;
        std::fs::write(&backup, &bytes).unwrap();

        let err = Vault::import_backup(&backup, b"pw", &base.join("restored"), "R").unwrap_err();
        assert!(matches!(
            err,
            VaultError::WrongPassphrase | VaultError::Corrupt(_)
        ));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn export_requires_unlocked() {
        let base = tmp_vault_dir();
        let mut v = Vault::create(cfg(&base.join("v")), b"pw").expect("create");
        v.lock().expect("lock");
        assert!(matches!(
            v.export_backup(&base.join("b.tar.age")),
            Err(VaultError::Locked)
        ));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn import_bad_magic_is_corrupt() {
        let base = tmp_vault_dir();
        std::fs::create_dir_all(&base).unwrap();
        let bad = base.join("bad.tar.age");
        std::fs::write(&bad, b"NOTABACKUP and some more bytes here").unwrap();
        let err = Vault::import_backup(&bad, b"pw", &base.join("r"), "R").unwrap_err();
        assert!(matches!(err, VaultError::Corrupt(_)));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn integrity_clean_vault_is_ok() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        v.add_file(&make_src(&dir, "a.txt", b"aaa")).expect("add a");
        v.add_file(&make_src(&dir, "b.txt", b"bbbbb"))
            .expect("add b");
        let report = v.verify_integrity().expect("verify");
        assert!(report.is_clean());
        assert_eq!(report.ok.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn integrity_detects_tampered() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        v.add_file(&make_src(&dir, "t.txt", b"important"))
            .expect("add");
        let stored = v.list_files().unwrap()[0].stored_name.clone();
        let data = dir.join(DATA_DIR).join(&stored);
        let mut bytes = std::fs::read(&data).unwrap();
        let i = bytes.len() - 2;
        bytes[i] ^= 0xff;
        std::fs::write(&data, &bytes).unwrap();

        let report = v.verify_integrity().expect("verify");
        assert!(!report.is_clean());
        assert_eq!(report.tampered, vec!["t.txt".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn integrity_detects_missing() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        v.add_file(&make_src(&dir, "m.txt", b"gone soon"))
            .expect("add");
        let stored = v.list_files().unwrap()[0].stored_name.clone();
        std::fs::remove_file(dir.join(DATA_DIR).join(&stored)).unwrap();

        let report = v.verify_integrity().expect("verify");
        assert_eq!(report.missing, vec!["m.txt".to_string()]);
        assert!(!report.is_clean());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn integrity_detects_orphaned() {
        let dir = tmp_vault_dir();
        let v = Vault::create(cfg(&dir), b"pw").expect("create");
        // Stray blob not referenced by the index.
        std::fs::write(dir.join(DATA_DIR).join("deadbeef.age"), b"junk").unwrap();
        let report = v.verify_integrity().expect("verify");
        assert_eq!(report.orphaned, vec!["deadbeef.age".to_string()]);
        assert!(!report.is_clean());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn integrity_requires_unlocked() {
        let dir = tmp_vault_dir();
        let mut v = Vault::create(cfg(&dir), b"pw").expect("create");
        v.lock().expect("lock");
        assert!(matches!(v.verify_integrity(), Err(VaultError::Locked)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
