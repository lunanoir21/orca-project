//! Single-file encryption / decryption (security-critical).
//!
//! Implements `encrypt_file` / `decrypt_file` for both [`EncryptionScheme`]s.
//! Every byte is processed in fixed-size chunks: a 4 GiB file never loads into
//! memory. There is **no custom cryptography** — the `age` crate and RustCrypto
//! `chacha20poly1305` provide all primitives.
//!
//! # On-disk container
//! ```text
//! [8]  magic  = b"ORCAVLT1"
//! [1]  scheme = 0 (Age) | 1 (Argon2idXchacha20)
//! payload (scheme-specific)
//! ```
//! For the Argon2id scheme the payload is:
//! ```text
//! [4]  memory_kib  (u32 LE)   ┐ Argon2id parameters, plaintext (not secret);
//! [4]  iterations  (u32 LE)   │ tampering them yields the wrong key, so the
//! [4]  parallelism (u32 LE)   ┘ AEAD tag check fails — corruption is detected.
//! [16] salt                   plaintext, unique per file
//! [19] STREAM nonce prefix    plaintext
//! [..] XChaCha20-Poly1305 STREAM chunks (each carries a 16-byte tag)
//! ```
//! The `age` scheme's payload is a self-contained `age` stream (it embeds its
//! own scrypt salt and authentication).
//!
//! # Original filename preservation
//! The plaintext fed into the cipher is framed as `[u32 LE name_len][name]
//! [file bytes]`. The original filename therefore lives **inside** the
//! encrypted envelope, never in the on-disk `.age` filename. On decrypt the
//! name is recovered and the file is restored under it.
//!
//! # Security
//! - Decryption is authenticated: any single-bit change to ciphertext, salt,
//!   nonce, or KDF parameters causes a clean error, never garbage plaintext.
//! - A malicious container cannot escape the destination directory: the
//!   embedded filename is validated to be a bare file name (no separators, no
//!   `.`/`..`), and the output is created with `create_new` so an existing file
//!   is never overwritten (defends rule 9.6: path traversal / overwrite).
//! - Derived keys are zeroized on drop ([`crate::DerivedKey`]); passphrases are
//!   never logged or echoed in errors.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};

use crate::crypto::{derive_key, DerivedKey, KdfParams};
use crate::{EncryptionScheme, Result, VaultError};

/// Container magic, identifies an Orca single-file encrypted blob.
const MAGIC: &[u8; 8] = b"ORCAVLT1";
/// Plaintext chunk size for streaming (64 KiB).
const CHUNK: usize = 64 * 1024;
/// AEAD tag length appended to every XChaCha20-Poly1305 STREAM chunk.
const TAG_LEN: usize = 16;
/// STREAM nonce-prefix length for XChaCha20-Poly1305 (24-byte nonce − 5 used by
/// the BE32 STREAM construction for counter + last-block flag).
const NONCE_PREFIX_LEN: usize = 19;

fn scheme_byte(scheme: EncryptionScheme) -> u8 {
    match scheme {
        EncryptionScheme::Age => 0,
        EncryptionScheme::Argon2idXchacha20 => 1,
    }
}

fn scheme_from_byte(b: u8) -> Result<EncryptionScheme> {
    match b {
        0 => Ok(EncryptionScheme::Age),
        1 => Ok(EncryptionScheme::Argon2idXchacha20),
        other => Err(VaultError::Corrupt(format!("unknown scheme byte {other}"))),
    }
}

/// Encrypt `src` into a new `<src>.age` file alongside it, using `scheme`.
///
/// The original filename is preserved inside the encrypted envelope. Returns the
/// path of the created `.age` file.
///
/// # Errors
/// Returns [`VaultError`] on I/O failure, if the destination already exists, if
/// the passphrase is not valid UTF-8 (required by `age`), or on a cryptographic
/// failure.
pub fn encrypt_file(src: &Path, passphrase: &[u8], scheme: EncryptionScheme) -> Result<PathBuf> {
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| VaultError::Crypto("source has no valid UTF-8 filename".into()))?;

    let input = File::open(src).map_err(|e| VaultError::from_io(src, e))?;

    // Frame: [u32 LE name_len][name][file bytes], streamed via a chained reader
    // so the file contents are never fully buffered.
    let mut header = Vec::with_capacity(4 + name.len());
    let name_len =
        u32::try_from(name.len()).map_err(|_| VaultError::Crypto("filename too long".into()))?;
    header.extend_from_slice(&name_len.to_le_bytes());
    header.extend_from_slice(name.as_bytes());
    let plaintext = io::Cursor::new(header).chain(input);

    let mut dest_os: OsString = src.as_os_str().to_owned();
    dest_os.push(".age");
    let dest = PathBuf::from(dest_os);

    // create_new: never clobber an existing file.
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest)
        .map_err(|e| VaultError::from_io(&dest, e))?;

    out.write_all(MAGIC)
        .and_then(|()| out.write_all(&[scheme_byte(scheme)]))
        .map_err(|e| VaultError::from_io(&dest, e))?;

    let result = match scheme {
        EncryptionScheme::Age => encrypt_age(plaintext, &mut out, passphrase),
        EncryptionScheme::Argon2idXchacha20 => encrypt_xchacha(plaintext, &mut out, passphrase),
    };

    if let Err(e) = result.and_then(|()| out.flush().map_err(VaultError::BareIo)) {
        // Clean up the partial output so a failed encrypt leaves no half file.
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    Ok(dest)
}

/// Decrypt an Orca `.age` container `src` into `dest_dir`, restoring the
/// original filename embedded in the envelope. Returns the restored file path.
///
/// # Errors
/// Returns [`VaultError::WrongPassphrase`] if the passphrase does not
/// authenticate, [`VaultError::Corrupt`] if the container is malformed,
/// [`VaultError::AlreadyExists`] if the restored file already exists, or an I/O
/// error.
pub fn decrypt_file(src: &Path, passphrase: &[u8], dest_dir: &Path) -> Result<PathBuf> {
    let input = File::open(src).map_err(|e| VaultError::from_io(src, e))?;
    let mut sink = FrameWriter::new(dest_dir);
    decrypt_container(input, passphrase, &mut sink)?;
    sink.finish()
}

/// Read the container magic + scheme byte and decrypt the payload into `sink`,
/// which receives the *framed* plaintext (`[u32 name_len][name][bytes]`). The
/// caller decides how to interpret the frame (write to disk, parse in memory).
///
/// Exposed within the crate so the in-memory fuzz/inspection helpers can run the
/// exact same parser the on-disk path uses.
///
/// # Errors
/// [`VaultError::Corrupt`] for a malformed container, [`VaultError::WrongPassphrase`]
/// on authentication failure, or an I/O error.
pub(crate) fn decrypt_container<R: Read, W: Write>(
    mut input: R,
    passphrase: &[u8],
    sink: &mut W,
) -> Result<()> {
    let mut magic = [0u8; 8];
    read_exact_or_corrupt(&mut input, &mut magic, "missing magic")?;
    if &magic != MAGIC {
        return Err(VaultError::Corrupt(
            "bad magic; not an Orca container".into(),
        ));
    }
    let mut sb = [0u8; 1];
    read_exact_or_corrupt(&mut input, &mut sb, "missing scheme byte")?;
    match scheme_from_byte(sb[0])? {
        EncryptionScheme::Age => decrypt_age(input, sink, passphrase),
        EncryptionScheme::Argon2idXchacha20 => decrypt_xchacha(input, sink, passphrase),
    }
}

// --- age scheme ----------------------------------------------------------

fn passphrase_str(passphrase: &[u8]) -> Result<age::secrecy::SecretString> {
    let s = std::str::from_utf8(passphrase)
        .map_err(|_| VaultError::Crypto("passphrase is not valid UTF-8".into()))?;
    Ok(age::secrecy::SecretString::from(s.to_owned()))
}

fn encrypt_age(mut plaintext: impl Read, out: &mut impl Write, passphrase: &[u8]) -> Result<()> {
    let enc = age::Encryptor::with_user_passphrase(passphrase_str(passphrase)?);
    let mut writer = enc
        .wrap_output(out)
        .map_err(|e| VaultError::Crypto(format!("age init failed: {e}")))?;
    io::copy(&mut plaintext, &mut writer).map_err(VaultError::BareIo)?;
    writer
        .finish()
        .map_err(|e| VaultError::Crypto(format!("age finalize failed: {e}")))?;
    Ok(())
}

fn decrypt_age(input: impl Read, sink: &mut impl Write, passphrase: &[u8]) -> Result<()> {
    let decryptor = age::Decryptor::new(input)
        .map_err(|e| VaultError::Corrupt(format!("age header invalid: {e}")))?;
    let identity = age::scrypt::Identity::new(passphrase_str(passphrase)?);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| VaultError::WrongPassphrase)?;
    io::copy(&mut reader, sink).map_err(map_sink_err)?;
    Ok(())
}

// --- argon2id + XChaCha20-Poly1305 scheme --------------------------------

fn encrypt_xchacha(plaintext: impl Read, out: &mut impl Write, passphrase: &[u8]) -> Result<()> {
    let params = KdfParams::default();
    let salt = crate::crypto::gen_salt()?;
    let key = derive_key(passphrase, &salt, params)?;

    // Header: KDF params + salt (plaintext; authenticated implicitly because a
    // changed value derives a different key and fails the tag check).
    out.write_all(&params.memory_kib.to_le_bytes())
        .and_then(|()| out.write_all(&params.iterations.to_le_bytes()))
        .and_then(|()| out.write_all(&params.parallelism.to_le_bytes()))
        .and_then(|()| out.write_all(&salt))
        .map_err(VaultError::BareIo)?;

    xchacha_seal_core(plaintext, out, &key)
}

fn decrypt_xchacha(mut input: impl Read, sink: &mut impl Write, passphrase: &[u8]) -> Result<()> {
    let mut m = [0u8; 4];
    let mut t = [0u8; 4];
    let mut p = [0u8; 4];
    read_exact_or_corrupt(&mut input, &mut m, "missing kdf memory")?;
    read_exact_or_corrupt(&mut input, &mut t, "missing kdf iterations")?;
    read_exact_or_corrupt(&mut input, &mut p, "missing kdf parallelism")?;
    let params = KdfParams {
        memory_kib: u32::from_le_bytes(m),
        iterations: u32::from_le_bytes(t),
        parallelism: u32::from_le_bytes(p),
    };
    let mut salt = [0u8; crate::SALT_LEN];
    read_exact_or_corrupt(&mut input, &mut salt, "missing salt")?;

    let key = derive_key(passphrase, &salt, params)?;
    xchacha_open_core(input, sink, &key)
}

/// Encrypt a plaintext stream under an already-derived `key`, writing
/// `[19-byte nonce prefix][STREAM chunks]`. No KDF header is written: the caller
/// owns the key. Used for vault file blobs, where the master key is reused
/// across files (re-running Argon2id per file would be wasteful).
pub(crate) fn xchacha_seal_core(
    mut plaintext: impl Read,
    out: &mut impl Write,
    key: &DerivedKey,
) -> Result<()> {
    let mut prefix = [0u8; NONCE_PREFIX_LEN];
    getrandom::getrandom(&mut prefix)
        .map_err(|e| VaultError::Crypto(format!("CSPRNG unavailable: {e}")))?;
    out.write_all(&prefix).map_err(VaultError::BareIo)?;

    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|e| VaultError::Crypto(format!("key setup failed: {e}")))?;
    let mut enc = EncryptorBE32::from_aead(cipher, GenericArray::from_slice(&prefix));

    // One-chunk lookahead so the final chunk is sealed with `encrypt_last`,
    // which marks the end of the STREAM and prevents truncation attacks.
    let mut cur = vec![0u8; CHUNK];
    let mut cur_len = read_full(&mut plaintext, &mut cur).map_err(VaultError::BareIo)?;
    loop {
        let mut next = vec![0u8; CHUNK];
        let next_len = read_full(&mut plaintext, &mut next).map_err(VaultError::BareIo)?;
        if next_len == 0 {
            let ct = enc
                .encrypt_last(&cur[..cur_len])
                .map_err(|e| VaultError::Crypto(format!("encrypt failed: {e}")))?;
            out.write_all(&ct).map_err(VaultError::BareIo)?;
            break;
        }
        let ct = enc
            .encrypt_next(&cur[..cur_len])
            .map_err(|e| VaultError::Crypto(format!("encrypt failed: {e}")))?;
        out.write_all(&ct).map_err(VaultError::BareIo)?;
        cur = next;
        cur_len = next_len;
    }
    Ok(())
}

/// Decrypt a stream produced by [`xchacha_seal_core`] under `key`, writing the
/// recovered plaintext to `out`.
pub(crate) fn xchacha_open_core(
    mut input: impl Read,
    out: &mut impl Write,
    key: &DerivedKey,
) -> Result<()> {
    let mut prefix = [0u8; NONCE_PREFIX_LEN];
    read_exact_or_corrupt(&mut input, &mut prefix, "missing nonce prefix")?;

    let cipher = XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|e| VaultError::Crypto(format!("key setup failed: {e}")))?;
    let mut dec = DecryptorBE32::from_aead(cipher, GenericArray::from_slice(&prefix));

    let ct_chunk = CHUNK + TAG_LEN;
    let mut cur = vec![0u8; ct_chunk];
    let mut cur_len = read_full(&mut input, &mut cur).map_err(VaultError::BareIo)?;
    loop {
        let mut next = vec![0u8; ct_chunk];
        let next_len = read_full(&mut input, &mut next).map_err(VaultError::BareIo)?;
        if next_len == 0 {
            let pt = dec
                .decrypt_last(&cur[..cur_len])
                .map_err(|_| VaultError::WrongPassphrase)?;
            out.write_all(&pt).map_err(map_sink_err)?;
            break;
        }
        let pt = dec
            .decrypt_next(&cur[..cur_len])
            .map_err(|_| VaultError::WrongPassphrase)?;
        out.write_all(&pt).map_err(map_sink_err)?;
        cur = next;
        cur_len = next_len;
    }
    Ok(())
}

// --- helpers -------------------------------------------------------------

/// Read until `buf` is full or EOF; returns the number of bytes read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn read_exact_or_corrupt(r: &mut impl Read, buf: &mut [u8], what: &str) -> Result<()> {
    let n = read_full(r, buf).map_err(VaultError::BareIo)?;
    if n != buf.len() {
        return Err(VaultError::Corrupt(format!("truncated container: {what}")));
    }
    Ok(())
}

/// Map an I/O error coming out of [`FrameWriter`] back to a `VaultError`,
/// preserving the structured variants the writer encodes via `io::Error`.
fn map_sink_err(e: io::Error) -> VaultError {
    match e.kind() {
        io::ErrorKind::AlreadyExists => VaultError::AlreadyExists(PathBuf::from(e.to_string())),
        io::ErrorKind::InvalidData => VaultError::Corrupt(e.to_string()),
        _ => VaultError::BareIo(e),
    }
}

/// A [`Write`] sink that consumes the framed plaintext stream, recovers the
/// embedded original filename, and writes the remaining bytes to a freshly
/// created file in the destination directory.
///
/// Security: the embedded name is validated to be a bare filename and the
/// output is created with `create_new`, so a hostile container can neither
/// traverse out of `dest_dir` nor overwrite an existing file.
struct FrameWriter {
    dest_dir: PathBuf,
    header: Vec<u8>,
    name_len: Option<usize>,
    file: Option<File>,
    path: Option<PathBuf>,
}

impl FrameWriter {
    fn new(dest_dir: &Path) -> Self {
        Self {
            dest_dir: dest_dir.to_path_buf(),
            header: Vec::new(),
            name_len: None,
            file: None,
            path: None,
        }
    }

    fn validate_name(name: &str) -> io::Result<&str> {
        if name.is_empty() || name == "." || name == ".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid embedded filename",
            ));
        }
        // Must be a single path component: reject separators and NUL.
        if Path::new(name).components().count() != 1
            || name.contains('/')
            || name.contains('\\')
            || name.contains('\0')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "embedded filename is not a bare name",
            ));
        }
        Ok(name)
    }

    fn open_target(&mut self, leftover: &[u8]) -> io::Result<()> {
        let len = self.name_len.expect("name_len set");
        let name_bytes = &self.header[4..4 + len];
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "filename not UTF-8"))?;
        let name = Self::validate_name(name)?;
        let path = self.dest_dir.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(leftover)?;
        self.file = Some(file);
        self.path = Some(path);
        self.header = Vec::new();
        Ok(())
    }

    fn push(&mut self, buf: &[u8]) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            return file.write_all(buf);
        }
        self.header.extend_from_slice(buf);
        if self.name_len.is_none() {
            if self.header.len() < 4 {
                return Ok(());
            }
            let len = u32::from_le_bytes([
                self.header[0],
                self.header[1],
                self.header[2],
                self.header[3],
            ]) as usize;
            self.name_len = Some(len);
        }
        let len = self.name_len.expect("just set");
        if self.header.len() >= 4 + len {
            let leftover = self.header[4 + len..].to_vec();
            self.open_target(&leftover)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<PathBuf> {
        match self.file.as_mut() {
            Some(file) => {
                file.flush().map_err(VaultError::BareIo)?;
                Ok(self.path.expect("path set when file set"))
            }
            None => Err(VaultError::Corrupt(
                "container ended before filename was complete".into(),
            )),
        }
    }
}

impl Write for FrameWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.push(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let mut d = std::env::temp_dir();
        let uniq = format!(
            "orca-fc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        d.push(uniq);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write");
    }

    fn roundtrip(scheme: EncryptionScheme, contents: &[u8]) {
        let dir = tmpdir();
        let src = dir.join("report.dat");
        write_file(&src, contents);

        let enc = encrypt_file(&src, b"hunter2", scheme).expect("encrypt");
        assert_eq!(enc.file_name().unwrap().to_str().unwrap(), "report.dat.age");

        let out_dir = dir.join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let restored = decrypt_file(&enc, b"hunter2", &out_dir).expect("decrypt");
        assert_eq!(
            restored.file_name().unwrap().to_str().unwrap(),
            "report.dat"
        );

        let mut got = Vec::new();
        File::open(&restored)
            .unwrap()
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, contents);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn age_roundtrip_small() {
        roundtrip(EncryptionScheme::Age, b"hello vault");
    }

    #[test]
    fn xchacha_roundtrip_small() {
        roundtrip(EncryptionScheme::Argon2idXchacha20, b"hello vault");
    }

    #[test]
    fn age_roundtrip_multichunk() {
        // > 2 chunks to exercise the streaming lookahead path.
        let data: Vec<u8> = (0..(CHUNK * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        roundtrip(EncryptionScheme::Age, &data);
    }

    #[test]
    fn xchacha_roundtrip_multichunk() {
        let data: Vec<u8> = (0..(CHUNK * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        roundtrip(EncryptionScheme::Argon2idXchacha20, &data);
    }

    #[test]
    fn empty_file_roundtrips() {
        roundtrip(EncryptionScheme::Argon2idXchacha20, b"");
        roundtrip(EncryptionScheme::Age, b"");
    }

    #[test]
    fn wrong_passphrase_fails_cleanly_xchacha() {
        let dir = tmpdir();
        let src = dir.join("s.bin");
        write_file(&src, b"top secret bytes");
        let enc = encrypt_file(&src, b"right", EncryptionScheme::Argon2idXchacha20).unwrap();
        let out = dir.join("o");
        std::fs::create_dir_all(&out).unwrap();
        let err = decrypt_file(&enc, b"wrong", &out).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase));
        // No partial plaintext file must remain.
        assert!(std::fs::read_dir(&out).unwrap().next().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrong_passphrase_fails_cleanly_age() {
        let dir = tmpdir();
        let src = dir.join("s.bin");
        write_file(&src, b"top secret bytes");
        let enc = encrypt_file(&src, b"right", EncryptionScheme::Age).unwrap();
        let out = dir.join("o");
        std::fs::create_dir_all(&out).unwrap();
        let err = decrypt_file(&enc, b"wrong", &out).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tampered_ciphertext_detected() {
        let dir = tmpdir();
        let src = dir.join("s.bin");
        write_file(&src, b"integrity matters here, do not corrupt");
        let enc = encrypt_file(&src, b"pw", EncryptionScheme::Argon2idXchacha20).unwrap();

        // Flip a byte deep in the ciphertext body (past magic+scheme+header).
        let mut bytes = std::fs::read(&enc).unwrap();
        let idx = bytes.len() - 5;
        bytes[idx] ^= 0xff;
        std::fs::write(&enc, &bytes).unwrap();

        let out = dir.join("o");
        std::fs::create_dir_all(&out).unwrap();
        let err = decrypt_file(&enc, b"pw", &out).unwrap_err();
        assert!(matches!(
            err,
            VaultError::WrongPassphrase | VaultError::Corrupt(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encrypt_refuses_to_overwrite_existing_age() {
        let dir = tmpdir();
        let src = dir.join("a.txt");
        write_file(&src, b"x");
        std::fs::write(dir.join("a.txt.age"), b"pre-existing").unwrap();
        let err = encrypt_file(&src, b"pw", EncryptionScheme::Age).unwrap_err();
        assert!(matches!(err, VaultError::AlreadyExists(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_magic_is_corrupt() {
        let dir = tmpdir();
        let bad = dir.join("not.age");
        std::fs::write(&bad, b"NOTORCA1\x00rest").unwrap();
        let out = dir.join("o");
        std::fs::create_dir_all(&out).unwrap();
        let err = decrypt_file(&bad, b"pw", &out).unwrap_err();
        assert!(matches!(err, VaultError::Corrupt(_)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
