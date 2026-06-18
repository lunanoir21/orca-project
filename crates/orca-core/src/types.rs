//! Core data types shared across `orca-core`.
//!
//! These types describe filesystem entries ([`FileEntry`], [`FileKind`]) and the
//! options used to list directories: sort order ([`SortKey`]) and filtering
//! ([`FilterOptions`]). They are plain data with a small number of helper
//! methods; the listing logic that produces them lives in the `fs` module.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The kind of a filesystem entry, derived from its file type bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link (the link itself, regardless of its target).
    Symlink,
    /// A block device node.
    BlockDevice,
    /// A character device node.
    CharDevice,
    /// A named pipe (FIFO).
    Fifo,
    /// A unix domain socket.
    Socket,
    /// A type that could not be determined.
    Unknown,
}

impl FileKind {
    /// Returns `true` if this entry is a directory.
    #[must_use]
    pub fn is_dir(self) -> bool {
        matches!(self, FileKind::Directory)
    }

    /// Classify a [`std::fs::FileType`] into a [`FileKind`].
    ///
    /// Symlinks must be classified from the *unfollowed* file type (i.e.
    /// `symlink_metadata`); a followed type will never report `Symlink`.
    #[must_use]
    pub fn from_file_type(ft: std::fs::FileType) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if ft.is_symlink() {
                return FileKind::Symlink;
            }
            if ft.is_dir() {
                return FileKind::Directory;
            }
            if ft.is_file() {
                return FileKind::File;
            }
            if ft.is_block_device() {
                return FileKind::BlockDevice;
            }
            if ft.is_char_device() {
                return FileKind::CharDevice;
            }
            if ft.is_fifo() {
                return FileKind::Fifo;
            }
            if ft.is_socket() {
                return FileKind::Socket;
            }
            FileKind::Unknown
        }
        #[cfg(not(unix))]
        {
            if ft.is_symlink() {
                FileKind::Symlink
            } else if ft.is_dir() {
                FileKind::Directory
            } else if ft.is_file() {
                FileKind::File
            } else {
                FileKind::Unknown
            }
        }
    }
}

/// A single filesystem entry with the metadata Orca displays and operates on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Absolute or listing-relative path to the entry.
    pub path: PathBuf,
    /// The final path component (file name) as a lossy UTF-8 string.
    pub name: String,
    /// Size in bytes. For directories this is the size of the directory inode,
    /// not its recursive content size (see `disk_usage` for that).
    pub size: u64,
    /// Last modification time, if the platform/filesystem reports it.
    pub modified: Option<SystemTime>,
    /// Unix permission bits (the low 12 bits of the mode), e.g. `0o644`.
    pub permissions: u32,
    /// The kind of entry (file, directory, symlink, ...).
    pub kind: FileKind,
    /// Whether the entry is hidden (dot-prefixed name on Linux).
    pub is_hidden: bool,
    /// Whether the entry is a symbolic link.
    pub is_symlink: bool,
}

impl FileEntry {
    /// The file extension (without the leading dot), lowercased, if any.
    ///
    /// Returns `None` for directories, dotfiles with no further extension, and
    /// names without a `.`.
    #[must_use]
    pub fn extension(&self) -> Option<String> {
        Path::new(&self.name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
    }
}

/// The key a directory listing is sorted by. Directories are always grouped
/// before files by the listing logic; this key orders within each group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Sort by name (case-insensitive).
    #[default]
    Name,
    /// Sort by size in bytes.
    Size,
    /// Sort by last modification time.
    Modified,
    /// Sort by file extension, then name.
    Extension,
    /// Sort by entry kind, then name.
    Kind,
}

/// Options controlling which entries a directory listing includes.
///
/// Construct with [`FilterOptions::default`] (shows everything except hidden
/// files) and adjust fields, or use the builder-style setters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterOptions {
    /// Include dot-prefixed hidden files.
    pub show_hidden: bool,
    /// Case-insensitive substring the name must contain to be included.
    pub name_pattern: Option<String>,
    /// Minimum size in bytes (inclusive), if set.
    pub min_size: Option<u64>,
    /// Maximum size in bytes (inclusive), if set.
    pub max_size: Option<u64>,
    /// Inclusive `(start, end)` modification-time range, if set.
    pub date_range: Option<(SystemTime, SystemTime)>,
}

impl FilterOptions {
    /// Returns `true` if the given entry passes all configured filters.
    #[must_use]
    pub fn matches(&self, entry: &FileEntry) -> bool {
        if entry.is_hidden && !self.show_hidden {
            return false;
        }
        if let Some(pattern) = &self.name_pattern {
            if !entry.name.to_lowercase().contains(&pattern.to_lowercase()) {
                return false;
            }
        }
        if let Some(min) = self.min_size {
            if entry.size < min {
                return false;
            }
        }
        if let Some(max) = self.max_size {
            if entry.size > max {
                return false;
            }
        }
        if let Some((start, end)) = self.date_range {
            match entry.modified {
                Some(m) if m >= start && m <= end => {}
                _ => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, size: u64, hidden: bool) -> FileEntry {
        FileEntry {
            path: PathBuf::from("/tmp").join(name),
            name: name.to_string(),
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000)),
            permissions: 0o644,
            kind: FileKind::File,
            is_hidden: hidden,
            is_symlink: false,
        }
    }

    #[test]
    fn file_kind_is_dir() {
        assert!(FileKind::Directory.is_dir());
        assert!(!FileKind::File.is_dir());
    }

    #[test]
    fn extension_lowercased() {
        assert_eq!(
            entry("Photo.JPG", 1, false).extension().as_deref(),
            Some("jpg")
        );
        assert_eq!(entry("README", 1, false).extension(), None);
    }

    #[test]
    fn sort_key_default_is_name() {
        assert_eq!(SortKey::default(), SortKey::Name);
    }

    #[test]
    fn filter_hides_hidden_by_default() {
        let f = FilterOptions::default();
        assert!(!f.matches(&entry(".hidden", 1, true)));
        assert!(f.matches(&entry("visible", 1, false)));
    }

    #[test]
    fn filter_show_hidden() {
        let f = FilterOptions {
            show_hidden: true,
            ..Default::default()
        };
        assert!(f.matches(&entry(".hidden", 1, true)));
    }

    #[test]
    fn filter_name_pattern_case_insensitive() {
        let f = FilterOptions {
            name_pattern: Some("RE".into()),
            ..Default::default()
        };
        assert!(f.matches(&entry("readme.txt", 1, false)));
        assert!(!f.matches(&entry("notes.txt", 1, false)));
    }

    #[test]
    fn filter_size_bounds() {
        let f = FilterOptions {
            min_size: Some(10),
            max_size: Some(100),
            ..Default::default()
        };
        assert!(!f.matches(&entry("a", 5, false)));
        assert!(f.matches(&entry("b", 50, false)));
        assert!(!f.matches(&entry("c", 500, false)));
    }

    #[test]
    fn filter_date_range() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(500);
        let end = SystemTime::UNIX_EPOCH + Duration::from_secs(1500);
        let f = FilterOptions {
            date_range: Some((start, end)),
            ..Default::default()
        };
        assert!(f.matches(&entry("in", 1, false)));

        let mut out = entry("out", 1, false);
        out.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2000));
        assert!(!f.matches(&out));
    }
}
