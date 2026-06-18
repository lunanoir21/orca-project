//! Orca encryption engine (security-critical).
//!
//! `orca-vault` implements Orca's encrypted vault system: `age`-based file
//! encryption, Argon2id passphrase key derivation, vault lifecycle
//! (create/open/lock), an encrypted file index (original filenames are never
//! stored in plaintext), auto-lock timers, multiple vaults, backup/export and
//! integrity verification.
//!
//! # Security invariants
//! - Key material is wiped from memory (`zeroize`) as soon as it is no longer
//!   needed.
//! - Passphrases and derived keys never appear in logs, error messages or debug
//!   output.
//! - No custom cryptography: all primitives come from audited crates.
//!
//! Functionality is implemented phase by phase per `TASKS.md` Phase 2.
#![forbid(unsafe_code)]

mod autolock;
mod blob;
mod crypto;
mod error;
mod file_crypto;
mod manager;
mod types;
mod util;
mod vault;

pub use autolock::{run_auto_lock, AutoLockTimer, LockEvent, LockReason};
pub use crypto::{derive_key, gen_salt, DerivedKey, KdfParams, SALT_LEN};
pub use error::{Result, VaultError};
pub use file_crypto::{decrypt_file, encrypt_file};
pub use manager::VaultManager;
pub use types::{
    EncryptionScheme, IntegrityHash, IntegrityReport, VaultConfig, VaultEntry, VaultIndex,
    VaultState,
};
pub use vault::Vault;

#[doc(hidden)]
pub mod fuzz {
    //! Fuzzing entry points (Phase 2.11), used by the `orca-fuzz` harness.
    //!
    //! These are `#[doc(hidden)]` and not part of the stable API. Every function
    //! must return without panicking on arbitrary input — only `Result::Err` is
    //! an acceptable outcome, never a panic.
    use std::io::Cursor;

    use crate::crypto::{derive_key, KdfParams};

    /// Feed arbitrary bytes as a single-file encrypted container. Exercises the
    /// magic/scheme parser and the AEAD stream decryptor.
    pub fn decrypt_container(data: &[u8]) {
        let mut sink = Vec::new();
        let _ = crate::file_crypto::decrypt_container(Cursor::new(data), b"fuzz-pass", &mut sink);
    }

    /// Feed arbitrary bytes as a sealed index blob. Exercises the blob opener and
    /// the index TOML parser.
    pub fn parse_index(data: &[u8]) {
        let Ok(key) = derive_key(b"fuzz-pass", &[0u8; 16], KdfParams::default()) else {
            return;
        };
        if let Ok(plain) = crate::blob::open(&key, data) {
            if let Ok(text) = std::str::from_utf8(&plain) {
                let _ = toml::from_str::<crate::VaultIndex>(text);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        //! Stable-toolchain smoke tests: the fuzz entry points must not panic on
        //! garbage or partially-valid input. (Full coverage runs under
        //! `cargo +nightly fuzz`.)
        use super::*;

        #[test]
        fn decrypt_container_never_panics() {
            decrypt_container(b"");
            decrypt_container(b"random garbage bytes that are not a container");
            // Valid magic + scheme byte, then truncated payload.
            let mut crafted = b"ORCAVLT1".to_vec();
            crafted.push(1);
            crafted.extend_from_slice(&[0u8; 8]);
            decrypt_container(&crafted);
            // Valid magic + age scheme, garbage age stream.
            let mut age_like = b"ORCAVLT1".to_vec();
            age_like.push(0);
            age_like.extend_from_slice(b"not a real age header");
            decrypt_container(&age_like);
            for n in 0..64usize {
                decrypt_container(&vec![n as u8; n]);
            }
        }

        #[test]
        fn parse_index_never_panics() {
            parse_index(b"");
            parse_index(b"short");
            parse_index(&[0u8; 40]);
            parse_index(&vec![0xabu8; 4096]);
        }
    }
}
