//! Display formatting helpers for file metadata.
//!
//! These convert raw [`orca_core`] values (byte counts, [`SystemTime`],
//! permission bits, [`FileKind`]) into the human-readable strings the file
//! browser shows in its columns. Kept pure and free of GTK so they can be
//! unit-tested directly.

use std::time::SystemTime;

use orca_core::FileKind;

/// Format a byte count as a human-readable size using binary (1024) units.
///
/// Directories are conventionally shown without a size by the caller; this
/// function always formats the number it is given.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Format a modification time as `YYYY-MM-DD HH:MM` in local time.
///
/// Returns an empty string when the timestamp is absent or predates the Unix
/// epoch (which the platform should never report for a real file).
#[must_use]
pub fn modified(time: Option<SystemTime>) -> String {
    let Some(time) = time else {
        return String::new();
    };
    let Ok(dur) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::new();
    };
    let secs = dur.as_secs() as i64;
    let dt = match time::OffsetDateTime::from_unix_timestamp(secs) {
        Ok(dt) => dt.to_offset(local_offset()),
        Err(_) => return String::new(),
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
    )
}

/// The local UTC offset, falling back to UTC if it cannot be determined (which
/// happens in some multi-threaded contexts the `time` crate refuses to query).
fn local_offset() -> time::UtcOffset {
    time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC)
}

/// Render Unix permission bits as a symbolic `rwxr-xr-x`-style string.
#[must_use]
pub fn permissions(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0b111;
        out.push(if bits & 0b100 != 0 { 'r' } else { '-' });
        out.push(if bits & 0b010 != 0 { 'w' } else { '-' });
        out.push(if bits & 0b001 != 0 { 'x' } else { '-' });
    }
    out
}

/// A short human label for a [`FileKind`].
#[must_use]
pub fn kind(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "File",
        FileKind::Directory => "Folder",
        FileKind::Symlink => "Link",
        FileKind::BlockDevice => "Block device",
        FileKind::CharDevice => "Character device",
        FileKind::Fifo => "Pipe",
        FileKind::Socket => "Socket",
        FileKind::Unknown => "Unknown",
    }
}

/// The themed icon name to display for an entry of the given kind.
#[must_use]
pub fn icon_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Directory => "folder-symbolic",
        FileKind::Symlink => "emblem-symbolic-link",
        FileKind::BlockDevice | FileKind::CharDevice => "drive-harddisk-symbolic",
        FileKind::Fifo | FileKind::Socket => "utilities-terminal-symbolic",
        FileKind::File | FileKind::Unknown => "text-x-generic-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn size_below_one_kib_is_bytes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1023), "1023 B");
    }

    #[test]
    fn size_scales_to_binary_units() {
        assert_eq!(size(1024), "1.0 KiB");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(1024 * 1024), "1.0 MiB");
        assert_eq!(size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn modified_none_is_empty() {
        assert_eq!(modified(None), "");
    }

    #[test]
    fn modified_formats_epoch_region() {
        // 2021-01-01T00:00:00Z; only assert the date prefix to stay offset-robust.
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_609_459_200);
        let s = modified(Some(t));
        assert_eq!(s.len(), "YYYY-MM-DD HH:MM".len());
        assert!(s.starts_with("202"), "unexpected: {s}");
    }

    #[test]
    fn permissions_symbolic() {
        assert_eq!(permissions(0o644), "rw-r--r--");
        assert_eq!(permissions(0o755), "rwxr-xr-x");
        assert_eq!(permissions(0o000), "---------");
        assert_eq!(permissions(0o777), "rwxrwxrwx");
    }

    #[test]
    fn kind_labels() {
        assert_eq!(kind(FileKind::Directory), "Folder");
        assert_eq!(kind(FileKind::File), "File");
    }

    #[test]
    fn icon_names_present() {
        assert!(icon_name(FileKind::Directory).contains("folder"));
        assert!(!icon_name(FileKind::File).is_empty());
    }
}
