//! File checksum generation: SHA-256, MD5 and Blake3.
//!
//! All three functions stream the file through their hasher in fixed-size
//! chunks — the file is never read fully into memory, so hashing a multi-gigabyte
//! file uses a constant, small buffer. Results are returned as lower-case hex
//! strings.

use std::path::Path;

use sha2::Digest;
use tokio::io::AsyncReadExt;

use crate::error::{OrcaError, Result};

/// Streaming read buffer size (64 KiB) — matches the copy engine's chunk size.
const HASH_BUF_SIZE: usize = 64 * 1024;

/// Compute the SHA-256 checksum of `path` as a lower-case hex string.
///
/// # Errors
/// Returns [`OrcaError::NotAFile`] if `path` is not a regular file, or an I/O
/// error if it cannot be read.
pub async fn checksum_sha256(path: impl AsRef<Path>) -> Result<String> {
    digest_file::<sha2::Sha256>(path.as_ref()).await
}

/// Compute the MD5 checksum of `path` as a lower-case hex string.
///
/// MD5 is cryptographically broken; it is offered only for interoperability
/// (verifying downloads that publish MD5 sums). Prefer SHA-256 or Blake3.
///
/// # Errors
/// As [`checksum_sha256`].
pub async fn checksum_md5(path: impl AsRef<Path>) -> Result<String> {
    digest_file::<md5::Md5>(path.as_ref()).await
}

/// Compute the Blake3 checksum of `path` as a lower-case hex string.
///
/// # Errors
/// As [`checksum_sha256`].
pub async fn checksum_blake3(path: impl AsRef<Path>) -> Result<String> {
    let mut file = open_regular_file(path.as_ref()).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| OrcaError::from_io(path.as_ref(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Stream `path` through any RustCrypto [`Digest`] and return lower-case hex.
async fn digest_file<D: Digest>(path: &Path) -> Result<String> {
    let mut file = open_regular_file(path).await?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; HASH_BUF_SIZE];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| OrcaError::from_io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(hasher.finalize().as_slice()))
}

/// Open `path`, rejecting anything that is not a regular file so that opening a
/// directory fails with a clear [`OrcaError::NotAFile`] rather than an opaque
/// read error.
async fn open_regular_file(path: &Path) -> Result<tokio::fs::File> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))?;
    if !meta.is_file() {
        return Err(OrcaError::NotAFile(path.to_path_buf()));
    }
    tokio::fs::File::open(path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))
}

/// Encode bytes as a lower-case hex string without pulling in a hex crate.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known-answer vectors for the empty input and for the ASCII string "abc",
    // taken from each algorithm's published test vectors.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";
    const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
    const EMPTY_BLAKE3: &str = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const ABC_BLAKE3: &str = "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";

    async fn file_with(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data");
        tokio::fs::write(&path, contents).await.expect("write");
        (dir, path)
    }

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(to_hex(&[0x00, 0xff, 0x1a]), "00ff1a");
    }

    #[tokio::test]
    async fn sha256_matches_known_vectors() {
        let (_d, empty) = file_with(b"").await;
        assert_eq!(checksum_sha256(&empty).await.expect("hash"), EMPTY_SHA256);
        let (_d2, abc) = file_with(b"abc").await;
        assert_eq!(checksum_sha256(&abc).await.expect("hash"), ABC_SHA256);
    }

    #[tokio::test]
    async fn md5_matches_known_vectors() {
        let (_d, empty) = file_with(b"").await;
        assert_eq!(checksum_md5(&empty).await.expect("hash"), EMPTY_MD5);
        let (_d2, abc) = file_with(b"abc").await;
        assert_eq!(checksum_md5(&abc).await.expect("hash"), ABC_MD5);
    }

    #[tokio::test]
    async fn blake3_matches_known_vectors() {
        let (_d, empty) = file_with(b"").await;
        assert_eq!(checksum_blake3(&empty).await.expect("hash"), EMPTY_BLAKE3);
        let (_d2, abc) = file_with(b"abc").await;
        assert_eq!(checksum_blake3(&abc).await.expect("hash"), ABC_BLAKE3);
    }

    #[tokio::test]
    async fn hashes_large_multichunk_file_consistently() {
        // Larger than HASH_BUF_SIZE to exercise the streaming loop across reads.
        let payload = vec![0xa5u8; HASH_BUF_SIZE * 3 + 7];
        let (_d, path) = file_with(&payload).await;
        // Cross-check against a one-shot hash of the same bytes.
        let expected = to_hex(sha2::Sha256::digest(&payload).as_slice());
        assert_eq!(checksum_sha256(&path).await.expect("hash"), expected);
    }

    #[tokio::test]
    async fn directory_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = checksum_sha256(dir.path()).await.expect_err("dir");
        assert!(matches!(err, OrcaError::NotAFile(_)));
    }

    #[tokio::test]
    async fn missing_file_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = checksum_blake3(dir.path().join("nope"))
            .await
            .expect_err("missing");
        assert!(matches!(err, OrcaError::NotFound(_) | OrcaError::Io { .. }));
    }
}
