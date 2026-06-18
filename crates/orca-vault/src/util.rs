//! Small internal helpers with no cryptographic logic of their own.

use crate::{IntegrityHash, Result, VaultError};

/// Streaming content hasher selectable between Blake3 and SHA-256.
///
/// Used to fingerprint vault file contents for the index and integrity checks.
/// This is a checksum, not encryption: it provides corruption detection on top
/// of the AEAD authentication tag.
pub(crate) enum Hasher {
    /// Blake3 256-bit hasher. Boxed: `blake3::Hasher` is far larger than the
    /// SHA-256 state, and boxing keeps the enum small.
    Blake3(Box<blake3::Hasher>),
    /// SHA-256 hasher.
    Sha256(Box<sha2::Sha256>),
}

impl Hasher {
    /// Create a hasher for the given algorithm.
    pub(crate) fn new(algo: IntegrityHash) -> Self {
        match algo {
            IntegrityHash::Blake3 => Hasher::Blake3(Box::new(blake3::Hasher::new())),
            IntegrityHash::Sha256 => {
                Hasher::Sha256(Box::new(<sha2::Sha256 as sha2::Digest>::new()))
            }
        }
    }

    /// Feed more bytes into the hash.
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Blake3(h) => {
                h.update(bytes);
            }
            Hasher::Sha256(h) => sha2::Digest::update(h.as_mut(), bytes),
        }
    }

    /// Finalize and return the lowercase hex digest.
    pub(crate) fn finish_hex(self) -> String {
        match self {
            Hasher::Blake3(h) => to_hex(h.finalize().as_bytes()),
            Hasher::Sha256(h) => to_hex(&sha2::Digest::finalize(*h)),
        }
    }
}

/// Encode bytes as a lowercase hex string.
#[must_use]
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Two lowercase hex digits per byte.
        s.push(char::from_digit((b >> 4) as u32, 16).expect("nibble < 16"));
        s.push(char::from_digit((b & 0x0f) as u32, 16).expect("nibble < 16"));
    }
    s
}

/// Decode a lowercase/uppercase hex string into bytes.
///
/// # Errors
/// Returns [`VaultError::Corrupt`] if the input has odd length or a non-hex
/// digit.
pub(crate) fn from_hex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(VaultError::Corrupt("hex string has odd length".into()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char)
            .to_digit(16)
            .ok_or_else(|| VaultError::Corrupt("invalid hex digit".into()))?;
        let lo = (bytes[i + 1] as char)
            .to_digit(16)
            .ok_or_else(|| VaultError::Corrupt("invalid hex digit".into()))?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data = [0x00u8, 0x0f, 0xa5, 0xff, 0x10];
        let s = to_hex(&data);
        assert_eq!(s, "000fa5ff10");
        assert_eq!(from_hex(&s).unwrap(), data);
    }

    #[test]
    fn from_hex_rejects_odd_length() {
        assert!(matches!(from_hex("abc"), Err(VaultError::Corrupt(_))));
    }

    #[test]
    fn from_hex_rejects_non_hex() {
        assert!(matches!(from_hex("zz"), Err(VaultError::Corrupt(_))));
    }

    #[test]
    fn from_hex_accepts_uppercase() {
        assert_eq!(from_hex("FF00").unwrap(), vec![0xff, 0x00]);
    }

    #[test]
    fn blake3_known_vector() {
        let mut h = Hasher::new(IntegrityHash::Blake3);
        h.update(b"abc");
        assert_eq!(
            h.finish_hex(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn sha256_known_vector() {
        let mut h = Hasher::new(IntegrityHash::Sha256);
        h.update(b"abc");
        assert_eq!(
            h.finish_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_is_chunk_boundary_independent() {
        let mut a = Hasher::new(IntegrityHash::Blake3);
        a.update(b"hello world");
        let mut b = Hasher::new(IntegrityHash::Blake3);
        b.update(b"hello ");
        b.update(b"world");
        assert_eq!(a.finish_hex(), b.finish_hex());
    }
}
