//! Authenticated encryption of small in-memory blobs (security-critical).
//!
//! Used for the vault's passphrase verifier and the encrypted index — data
//! small enough to live in memory, unlike streamed file contents. The cipher is
//! XChaCha20-Poly1305 (RustCrypto, audited): **no custom cryptography**.
//!
//! # Format
//! ```text
//! [24] random XChaCha20 nonce
//! [..] ciphertext + 16-byte Poly1305 tag
//! ```
//! The nonce is generated fresh per call from the OS CSPRNG. The 192-bit nonce
//! space makes random reuse negligible, so a single derived key can seal many
//! blobs safely.
//!
//! # Security
//! - Authenticated: any tampering (or a wrong key) makes [`open`] fail rather
//!   than return corrupted plaintext.
//! - The key is borrowed as [`DerivedKey`] and never copied into a long-lived
//!   buffer here; no key bytes appear in errors.

use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};

use crate::crypto::DerivedKey;
use crate::{Result, VaultError};

/// XChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 24;
/// Poly1305 authentication tag length in bytes.
const TAG_LEN: usize = 16;

/// Seal `plaintext` under `key`, returning `nonce || ciphertext+tag`.
///
/// # Errors
/// Returns [`VaultError::Crypto`] if the RNG or cipher setup fails.
pub(crate) fn seal(key: &DerivedKey, plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|e| VaultError::Crypto(format!("key setup failed: {e}")))?;

    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce)
        .map_err(|e| VaultError::Crypto(format!("CSPRNG unavailable: {e}")))?;

    let ciphertext = cipher
        .encrypt(GenericArray::from_slice(&nonce), plaintext)
        .map_err(|e| VaultError::Crypto(format!("seal failed: {e}")))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Open a blob produced by [`seal`].
///
/// # Errors
/// Returns [`VaultError::Corrupt`] if the blob is too short, or
/// [`VaultError::WrongPassphrase`] if authentication fails (wrong key or
/// tampered data).
pub(crate) fn open(key: &DerivedKey, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < NONCE_LEN + TAG_LEN {
        return Err(VaultError::Corrupt("sealed blob is too short".into()));
    }
    let (nonce, ciphertext) = data.split_at(NONCE_LEN);

    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|e| VaultError::Crypto(format!("key setup failed: {e}")))?;

    cipher
        .decrypt(GenericArray::from_slice(nonce), ciphertext)
        // Authentication failure is reported opaquely; it could be a wrong
        // passphrase or tampered ciphertext, and we never leak which.
        .map_err(|_| VaultError::WrongPassphrase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{derive_key, KdfParams};

    fn key(pass: &[u8]) -> DerivedKey {
        derive_key(pass, &[3u8; 16], KdfParams::default()).expect("derive")
    }

    #[test]
    fn seal_open_roundtrip() {
        let k = key(b"pw");
        let sealed = seal(&k, b"index contents").expect("seal");
        assert_eq!(open(&k, &sealed).expect("open"), b"index contents");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&key(b"right"), b"data").expect("seal");
        let err = open(&key(b"wrong"), &sealed).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase));
    }

    #[test]
    fn tamper_detected() {
        let k = key(b"pw");
        let mut sealed = seal(&k, b"data").expect("seal");
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(matches!(
            open(&k, &sealed),
            Err(VaultError::WrongPassphrase)
        ));
    }

    #[test]
    fn nonce_is_unique_per_call() {
        let k = key(b"pw");
        let a = seal(&k, b"x").expect("a");
        let b = seal(&k, b"x").expect("b");
        // Same plaintext + key, different nonce → different ciphertext.
        assert_ne!(a, b);
    }

    #[test]
    fn short_blob_is_corrupt() {
        let k = key(b"pw");
        assert!(matches!(
            open(&k, b"too short"),
            Err(VaultError::Corrupt(_))
        ));
    }

    #[test]
    fn empty_plaintext_roundtrips() {
        let k = key(b"pw");
        let sealed = seal(&k, b"").expect("seal");
        assert_eq!(open(&k, &sealed).expect("open"), b"");
    }
}
