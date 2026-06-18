//! Small internal helpers shared across modules (percent-encoding for XDG
//! `.trashinfo` paths and `file://` URIs).

use std::path::Path;

/// Percent-encode a filesystem path per RFC 3986 / the freedesktop conventions:
/// leave the unreserved set (`A-Z a-z 0-9 - _ . ~`) and the `/` separator
/// literal; encode every other byte as uppercase `%XX`.
pub(crate) fn percent_encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
        }
    }
    out
}

/// Decode a percent-encoded string into a (lossy UTF-8) [`String`]. Invalid
/// `%XX` sequences are passed through literally.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Map a 4-bit value to its uppercase hex digit.
fn hex_digit(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        _ => (b'A' + (v - 10)) as char,
    }
}

/// Map an ASCII hex digit to its 4-bit value, or `None` if not hex.
fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Crate-wide serialization lock for tests that mutate the process environment
/// (notably `XDG_DATA_HOME`). Shared across modules so trash and recent tests
/// never clobber each other's temporary data directory under cargo's parallel
/// runner. Async-aware because guards are held across `.await`.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn roundtrip_spaces_and_unicode() {
        let p = PathBuf::from("/home/u/My Docs/café.txt");
        let enc = percent_encode_path(&p);
        assert!(enc.contains("%20"));
        assert!(enc.contains('/'));
        assert_eq!(PathBuf::from(percent_decode(&enc)), p);
    }

    #[test]
    fn decode_passes_through_invalid_escape() {
        assert_eq!(percent_decode("100%done"), "100%done");
    }
}
