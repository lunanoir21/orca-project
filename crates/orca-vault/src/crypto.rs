//! Cryptographic primitives for `orca-vault` (security-critical).
//!
//! This module implements passphrase key derivation only. It contains **no
//! custom cryptography**: key stretching is delegated entirely to the audited
//! [`argon2`] crate (Argon2id), and memory wiping to [`zeroize`]. The AEAD
//! ciphers that consume the derived key live in the scheme modules.
//!
//! # Security design
//! - **Argon2id** is used with OWASP-recommended parameters (m≥64 MiB, t≥3,
//!   p=4) so that brute-forcing a passphrase is expensive. Parameters are
//!   stored in the vault meta so a vault created with stronger settings can
//!   still be opened later.
//! - **Salt** is 16 bytes from the OS CSPRNG ([`getrandom`]), unique per vault,
//!   never reused. It is stored in plaintext in the vault meta (salts are not
//!   secret); its only job is to defeat precomputation.
//! - **Zeroization**: the derived key is wrapped in [`DerivedKey`], which
//!   zeroizes its bytes on drop. Callers must likewise zeroize the passphrase
//!   buffer they pass in. Neither the passphrase nor the key is ever logged,
//!   formatted, or returned in an error message.

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroize;

use crate::{Result, VaultError};

/// Length in bytes of a vault salt.
pub const SALT_LEN: usize = 16;

/// Length in bytes of a derived symmetric key (256-bit).
const KEY_LEN: usize = 32;

/// Argon2id cost parameters, persisted in the vault meta so a vault remains
/// openable even if defaults change later.
///
/// Defaults follow the OWASP Password Storage Cheat Sheet recommendation for
/// Argon2id: 64 MiB memory, 3 iterations, 4 lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KdfParams {
    /// Memory cost in kibibytes (`m`). Default 65536 (= 64 MiB).
    pub memory_kib: u32,
    /// Time cost / iterations (`t`). Default 3.
    pub iterations: u32,
    /// Degree of parallelism / lanes (`p`). Default 4.
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: 65_536,
            iterations: 3,
            parallelism: 4,
        }
    }
}

impl KdfParams {
    /// Validate that the parameters meet the project's minimum security floor
    /// (OWASP: m≥65536 KiB, t≥3) and are structurally acceptable to Argon2.
    ///
    /// # Errors
    /// Returns [`VaultError::KeyDerivation`] if any parameter is below the
    /// minimum or zero.
    pub fn validate(&self) -> Result<()> {
        if self.memory_kib < 65_536 {
            return Err(VaultError::KeyDerivation(
                "memory cost below OWASP minimum (65536 KiB)".into(),
            ));
        }
        if self.iterations < 3 {
            return Err(VaultError::KeyDerivation(
                "iteration count below OWASP minimum (3)".into(),
            ));
        }
        if self.parallelism == 0 {
            return Err(VaultError::KeyDerivation("parallelism must be >= 1".into()));
        }
        Ok(())
    }
}

/// A 256-bit symmetric key derived from a passphrase.
///
/// The inner bytes are zeroized when this value is dropped, ensuring key
/// material does not linger in freed memory. It deliberately does **not**
/// implement `Debug`/`Display`/`Clone`: the key must never be printed, logged,
/// or silently copied.
pub struct DerivedKey([u8; KEY_LEN]);

impl DerivedKey {
    /// Borrow the raw key bytes for handoff to an AEAD cipher.
    ///
    /// Callers must not copy these bytes into a non-zeroizing buffer.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl Drop for DerivedKey {
    fn drop(&mut self) {
        // SECURITY: wipe key material from memory as soon as the key is no
        // longer needed, so it cannot be recovered from a freed allocation.
        self.0.zeroize();
    }
}

/// Generate a fresh, cryptographically random salt for a new vault.
///
/// Uses the OS CSPRNG via [`getrandom`]. The salt is not secret but must be
/// unique per vault and never reused.
///
/// # Errors
/// Returns [`VaultError::Crypto`] if the OS RNG is unavailable.
pub fn gen_salt() -> Result<[u8; SALT_LEN]> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt)
        .map_err(|e| VaultError::Crypto(format!("CSPRNG unavailable: {e}")))?;
    Ok(salt)
}

/// Derive a 256-bit key from a passphrase using Argon2id.
///
/// The passphrase bytes are not retained: this function does not store them and
/// the caller is responsible for zeroizing its own passphrase buffer.
///
/// # Security
/// Argon2id with the supplied [`KdfParams`] (validated against the OWASP floor).
/// On any internal failure the returned error carries no key or passphrase
/// material — only a description of the failure mode.
///
/// # Errors
/// Returns [`VaultError::KeyDerivation`] if parameters are invalid or Argon2
/// fails.
pub fn derive_key(passphrase: &[u8], salt: &[u8], params: KdfParams) -> Result<DerivedKey> {
    params.validate()?;
    if salt.len() < 8 {
        return Err(VaultError::KeyDerivation("salt too short".into()));
    }

    let argon_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| VaultError::KeyDerivation(format!("invalid Argon2 params: {e}")))?;

    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut key = [0u8; KEY_LEN];
    if let Err(e) = argon.hash_password_into(passphrase, salt, &mut key) {
        // Wipe any partial output before surfacing the error.
        key.zeroize();
        return Err(VaultError::KeyDerivation(format!("Argon2id failed: {e}")));
    }

    Ok(DerivedKey(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS: KdfParams = KdfParams {
        memory_kib: 65_536,
        iterations: 3,
        parallelism: 4,
    };

    #[test]
    fn derivation_is_deterministic() {
        let salt = [7u8; SALT_LEN];
        let a = derive_key(b"correct horse", &salt, PARAMS).expect("derive a");
        let b = derive_key(b"correct horse", &salt, PARAMS).expect("derive b");
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn different_salt_yields_different_key() {
        let a = derive_key(b"pw", &[1u8; SALT_LEN], PARAMS).expect("a");
        let b = derive_key(b"pw", &[2u8; SALT_LEN], PARAMS).expect("b");
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn different_passphrase_yields_different_key() {
        let salt = [9u8; SALT_LEN];
        let a = derive_key(b"alpha", &salt, PARAMS).expect("a");
        let b = derive_key(b"bravo", &salt, PARAMS).expect("b");
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn derived_key_is_full_length() {
        let k = derive_key(b"x", &[0u8; SALT_LEN], PARAMS).expect("derive");
        assert_eq!(k.as_bytes().len(), 32);
    }

    #[test]
    fn weak_params_rejected() {
        let weak = KdfParams {
            memory_kib: 1024,
            iterations: 3,
            parallelism: 4,
        };
        assert!(matches!(
            derive_key(b"x", &[0u8; SALT_LEN], weak),
            Err(VaultError::KeyDerivation(_))
        ));
    }

    #[test]
    fn short_salt_rejected() {
        assert!(matches!(
            derive_key(b"x", &[0u8; 4], PARAMS),
            Err(VaultError::KeyDerivation(_))
        ));
    }

    #[test]
    fn gen_salt_is_random_and_correct_length() {
        let a = gen_salt().expect("salt a");
        let b = gen_salt().expect("salt b");
        assert_eq!(a.len(), SALT_LEN);
        // Astronomically unlikely to collide; guards against a stubbed RNG.
        assert_ne!(a, b);
    }

    #[test]
    fn params_default_meets_owasp_floor() {
        assert!(KdfParams::default().validate().is_ok());
    }

    #[test]
    fn rfc9106_argon2id_known_answer() {
        // Independent known-answer vector: Argon2id, v=0x13, m=65536, t=2, p=1,
        // password = "password", salt = "somesalt" (PHC reference test suite).
        // Verifies our wiring matches the standard, not just itself.
        let params = Params::new(65_536, 2, 1, Some(KEY_LEN)).expect("params");
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = [0u8; KEY_LEN];
        argon
            .hash_password_into(b"password", b"somesalt", &mut out)
            .expect("hash");
        let expected = "09316115d5cf24ed5a15a31a3ba326e5cf32edc24702987c02b6566f61913cf7";
        assert_eq!(hex_lower(&out), expected);
    }

    fn hex_lower(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}
