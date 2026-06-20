//! Filesystem operations: directory listing, metadata and path helpers.
//!
//! This module provides directory listing ([`list_dir`],
//! [`list_dir_cancellable`]), hidden-file detection ([`is_hidden_name`]),
//! symlink resolution with cycle detection ([`resolve_symlink`]), file mutations
//! ([`ops`]) and XDG trash support ([`trash`]).

pub mod bulk_rename;
pub mod ops;
pub mod search;
pub mod trash;
pub mod watch;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::{OrcaError, Result};
use crate::types::{FileEntry, FileKind, FilterOptions, SortKey};

/// Maximum number of symlink hops [`resolve_symlink`] will follow before giving
/// up. Matches the common kernel `MAXSYMLINKS` limit and bounds work even if
/// cycle detection somehow missed a pathological chain.
const MAX_SYMLINK_HOPS: usize = 40;

/// A cooperative cancellation flag for long-running listings.
///
/// Clone is cheap (shares the underlying flag). Call [`CancelToken::cancel`]
/// from any thread/task to ask an in-flight [`list_dir_cancellable`] to stop;
/// it will return [`OrcaError::Cancelled`] at the next entry boundary.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Create a fresh, un-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Returns `true` once [`cancel`](Self::cancel) has been called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Returns `true` if `name` denotes a hidden entry (dot-prefixed on Linux).
#[must_use]
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// List the entries of a directory, filtered and sorted.
///
/// Entries that cannot be stat'd individually (e.g. a race or a permission
/// quirk on a single child) are skipped with a logged warning rather than
/// failing the whole listing — callers get partial results. If the directory
/// itself cannot be opened, an [`OrcaError`] is returned.
///
/// # Errors
/// Returns [`OrcaError::NotADirectory`] if `path` is not a directory, or an I/O
/// error if the directory cannot be read.
pub async fn list_dir(
    path: impl AsRef<Path>,
    filter: &FilterOptions,
    sort: SortKey,
) -> Result<Vec<FileEntry>> {
    list_dir_cancellable(path, filter, sort, &CancelToken::new()).await
}

/// Like [`list_dir`], but checks `cancel` before each entry and aborts with
/// [`OrcaError::Cancelled`] if cancellation was requested.
///
/// Reads and stats every entry synchronously inside a single
/// [`tokio::task::spawn_blocking`] call, rather than awaiting `tokio::fs` for
/// each entry individually — on a 10k-entry directory the per-entry async
/// hand-off to the blocking pool dominated wall time far more than the actual
/// syscalls did.
///
/// # Errors
/// As [`list_dir`], plus [`OrcaError::Cancelled`] when cancelled mid-listing.
pub async fn list_dir_cancellable(
    path: impl AsRef<Path>,
    filter: &FilterOptions,
    sort: SortKey,
    cancel: &CancelToken,
) -> Result<Vec<FileEntry>> {
    let path = path.as_ref().to_path_buf();
    let filter = filter.clone();
    let cancel = cancel.clone();

    let mut entries = tokio::task::spawn_blocking(move || -> Result<Vec<FileEntry>> {
        let meta = std::fs::metadata(&path).map_err(|e| OrcaError::from_io(&path, e))?;
        if !meta.is_dir() {
            return Err(OrcaError::NotADirectory(path.clone()));
        }

        let read = std::fs::read_dir(&path).map_err(|e| OrcaError::from_io(&path, e))?;
        let mut entries = Vec::new();
        for dir_entry in read {
            if cancel.is_cancelled() {
                return Err(OrcaError::Cancelled);
            }
            let dir_entry = dir_entry.map_err(|e| OrcaError::from_io(&path, e))?;
            match entry_from_path(&dir_entry.path()) {
                Ok(entry) => {
                    if filter.matches(&entry) {
                        entries.push(entry);
                    }
                }
                Err(err) => {
                    // Partial-results policy: a single unreadable child must
                    // not abort the listing. Surface it in logs and continue.
                    tracing::warn!(path = ?dir_entry.path(), error = %err, "skipping unreadable entry");
                }
            }
        }
        Ok(entries)
    })
    .await
    .map_err(|e| OrcaError::Other(format!("spawn_blocking: {e}")))??;

    sort_entries(&mut entries, sort);
    Ok(entries)
}

/// Build a [`FileEntry`] from a path using a non-traversing `symlink_metadata`
/// stat (a symlink is reported as a link, not its target).
///
/// Synchronous, for use from blocking traversal contexts (e.g. [`search`]).
///
/// # Errors
/// Returns an [`OrcaError`] if the path cannot be stat'd.
pub(crate) fn entry_from_path(path: &Path) -> Result<FileEntry> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| OrcaError::from_io(path, e))?;
    let file_type = metadata.file_type();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let kind = FileKind::from_file_type(file_type);
    let is_symlink = file_type.is_symlink();
    let is_hidden = is_hidden_name(&name);
    let modified = metadata.modified().ok();
    let permissions = mode_bits(&metadata);

    Ok(FileEntry {
        path: path.to_path_buf(),
        name,
        size: metadata.len(),
        modified,
        permissions,
        kind,
        is_hidden,
        is_symlink,
    })
}

/// Extract the low 12 permission bits from metadata (unix); `0` elsewhere.
fn mode_bits(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

/// Sort entries in place: directories first, then by `sort` within each group.
fn sort_entries(entries: &mut [FileEntry], sort: SortKey) {
    entries.sort_by(|a, b| {
        // Group directories ahead of everything else.
        let dir_rank = b.kind.is_dir().cmp(&a.kind.is_dir());
        if dir_rank != std::cmp::Ordering::Equal {
            return dir_rank;
        }
        match sort {
            SortKey::Name => name_cmp(a, b),
            SortKey::Size => a.size.cmp(&b.size).then_with(|| name_cmp(a, b)),
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(|| name_cmp(a, b)),
            SortKey::Extension => a
                .extension()
                .cmp(&b.extension())
                .then_with(|| name_cmp(a, b)),
            SortKey::Kind => kind_ord(a.kind)
                .cmp(&kind_ord(b.kind))
                .then_with(|| name_cmp(a, b)),
        }
    });
}

/// Case-insensitive name comparison, falling back to a case-sensitive tie-break
/// so the order is deterministic for names differing only in case.
fn name_cmp(a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
}

/// A stable numeric rank for [`FileKind`] used by [`SortKey::Kind`].
fn kind_ord(kind: FileKind) -> u8 {
    match kind {
        FileKind::Directory => 0,
        FileKind::File => 1,
        FileKind::Symlink => 2,
        FileKind::BlockDevice => 3,
        FileKind::CharDevice => 4,
        FileKind::Fifo => 5,
        FileKind::Socket => 6,
        FileKind::Unknown => 7,
    }
}

/// Resolve a symlink chain to its ultimate target, detecting cycles.
///
/// Follows up to [`MAX_SYMLINK_HOPS`] links. Relative link targets are resolved
/// against the directory of the link being followed. Returns the final
/// (non-symlink) path.
///
/// # Errors
/// - [`OrcaError::SymlinkCycle`] if a cycle is detected or the hop limit is hit.
/// - [`OrcaError`] if a link in the chain cannot be read.
pub fn resolve_symlink(path: impl AsRef<Path>) -> Result<PathBuf> {
    let mut current = path.as_ref().to_path_buf();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    for _ in 0..=MAX_SYMLINK_HOPS {
        let meta =
            std::fs::symlink_metadata(&current).map_err(|e| OrcaError::from_io(&current, e))?;
        if !meta.file_type().is_symlink() {
            return Ok(current);
        }
        if !visited.insert(current.clone()) {
            return Err(OrcaError::SymlinkCycle(current));
        }
        let target = std::fs::read_link(&current).map_err(|e| OrcaError::from_io(&current, e))?;
        current = if target.is_absolute() {
            target
        } else {
            match current.parent() {
                Some(parent) => parent.join(target),
                None => target,
            }
        };
    }

    Err(OrcaError::SymlinkCycle(current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn hidden_name_detection() {
        assert!(is_hidden_name(".bashrc"));
        assert!(!is_hidden_name("notes.txt"));
    }

    #[test]
    fn cancel_token_reports_state() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[tokio::test]
    async fn lists_and_sorts_directories_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("zsub")).expect("mkdir");
        fs::File::create(dir.path().join("afile.txt")).expect("file");
        fs::File::create(dir.path().join("bfile.txt")).expect("file");

        let entries = list_dir(dir.path(), &FilterOptions::default(), SortKey::Name)
            .await
            .expect("list");
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["zsub", "afile.txt", "bfile.txt"]);
    }

    #[tokio::test]
    async fn hidden_filtered_by_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::File::create(dir.path().join(".secret")).expect("file");
        fs::File::create(dir.path().join("public")).expect("file");

        let visible = list_dir(dir.path(), &FilterOptions::default(), SortKey::Name)
            .await
            .expect("list");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "public");

        let all = list_dir(
            dir.path(),
            &FilterOptions {
                show_hidden: true,
                ..Default::default()
            },
            SortKey::Name,
        )
        .await
        .expect("list");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn size_sort_orders_by_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut big = fs::File::create(dir.path().join("big")).expect("file");
        big.write_all(&[0u8; 100]).expect("write");
        fs::File::create(dir.path().join("small")).expect("file");

        let entries = list_dir(dir.path(), &FilterOptions::default(), SortKey::Size)
            .await
            .expect("list");
        assert_eq!(entries[0].name, "small");
        assert_eq!(entries[1].name, "big");
    }

    #[tokio::test]
    async fn errors_on_non_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("f");
        fs::File::create(&file).expect("file");
        let err = list_dir(&file, &FilterOptions::default(), SortKey::Name)
            .await
            .expect_err("should fail");
        assert!(matches!(err, OrcaError::NotADirectory(_)));
    }

    #[tokio::test]
    async fn cancelled_listing_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::File::create(dir.path().join("a")).expect("file");
        let token = CancelToken::new();
        token.cancel();
        let err =
            list_dir_cancellable(dir.path(), &FilterOptions::default(), SortKey::Name, &token)
                .await
                .expect_err("cancelled");
        assert!(matches!(err, OrcaError::Cancelled));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_is_detected_not_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.txt");
        fs::File::create(&target).expect("file");
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let entries = list_dir(dir.path(), &FilterOptions::default(), SortKey::Name)
            .await
            .expect("list");
        let link_entry = entries.iter().find(|e| e.name == "link.txt").expect("link");
        assert!(link_entry.is_symlink);
        assert_eq!(link_entry.kind, FileKind::Symlink);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_symlink_follows_to_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.txt");
        fs::File::create(&target).expect("file");
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let resolved = resolve_symlink(&link).expect("resolve");
        assert_eq!(resolved, target);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_symlink_detects_cycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::os::unix::fs::symlink(&b, &a).expect("symlink a->b");
        std::os::unix::fs::symlink(&a, &b).expect("symlink b->a");

        let err = resolve_symlink(&a).expect_err("cycle");
        assert!(matches!(err, OrcaError::SymlinkCycle(_)));
    }
}
