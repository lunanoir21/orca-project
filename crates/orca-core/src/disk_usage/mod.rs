//! Disk usage: recursive directory size, a depth-limited size tree for the
//! visualizer, and filesystem-level capacity via `statvfs`.
//!
//! Recursive sizing never follows symlinks (a link counts as its own small
//! entry, its target is not descended into), which keeps the walk bounded and
//! cycle-free. The size reported is the sum of file lengths (apparent size).
//!
//! The recursive walks are CPU/IO-bound and run on a blocking task.

use std::path::{Path, PathBuf};

use crate::error::{OrcaError, Result};

/// One node in a [`disk_usage_tree`] result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageNode {
    /// Absolute (or as-passed) path of this node.
    pub path: PathBuf,
    /// Final path component, for display.
    pub name: String,
    /// Total bytes contained (the whole subtree, even below the expansion depth).
    pub size: u64,
    /// Whether this node is a directory.
    pub is_dir: bool,
    /// Expanded children, largest first. Empty at the depth limit or for files.
    pub children: Vec<UsageNode>,
}

/// Filesystem capacity for the mount containing a path, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemUsage {
    /// Total size of the filesystem.
    pub total: u64,
    /// Bytes in use (total minus free).
    pub used: u64,
    /// Bytes available to an unprivileged user.
    pub free: u64,
}

/// Recursively sum the apparent size of every file under `path`.
///
/// A single file path returns its own length. Symlinks are not followed.
///
/// # Errors
/// I/O error if `path` cannot be stat'd; [`OrcaError::Other`] on task panic.
pub async fn dir_size(path: impl AsRef<Path>) -> Result<u64> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || size_recursive(&path))
        .await
        .map_err(|e| OrcaError::Other(format!("dir_size task failed: {e}")))?
}

/// Build a size tree rooted at `path`, expanding children down to `depth`
/// levels (the root is level 0). Nodes at the depth limit still report their
/// full recursive size; they just don't list their own children.
///
/// # Errors
/// I/O error if `path` cannot be read; [`OrcaError::Other`] on task panic.
pub async fn disk_usage_tree(path: impl AsRef<Path>, depth: usize) -> Result<UsageNode> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || build_node(&path, depth))
        .await
        .map_err(|e| OrcaError::Other(format!("disk_usage_tree task failed: {e}")))?
}

/// Query filesystem-level capacity for the mount containing `path`.
///
/// # Errors
/// I/O error if the `statvfs` call fails (e.g. the path does not exist).
pub fn filesystem_usage(path: impl AsRef<Path>) -> Result<FilesystemUsage> {
    let path = path.as_ref();
    let stat = rustix::fs::statvfs(path).map_err(|e| OrcaError::from_io(path, e.into()))?;
    // `f_frsize` is the fundamental block size; counts are in those units.
    let frsize = stat.f_frsize;
    let total = stat.f_blocks.saturating_mul(frsize);
    // `f_bfree` counts all free blocks; `f_bavail` excludes those reserved for
    // root, so it is the right figure for "free to the user".
    let free = stat.f_bavail.saturating_mul(frsize);
    let used = stat
        .f_blocks
        .saturating_sub(stat.f_bfree)
        .saturating_mul(frsize);
    Ok(FilesystemUsage { total, used, free })
}

/// Sum apparent file sizes under `path` without following symlinks.
fn size_recursive(path: &Path) -> Result<u64> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| OrcaError::from_io(path, e))?;
    if meta.file_type().is_symlink() {
        // Count the link itself, do not traverse its target.
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let mut total = 0u64;
    for child in read_children(path)? {
        // A child that races away mid-walk is skipped rather than aborting.
        match size_recursive(&child) {
            Ok(n) => total = total.saturating_add(n),
            Err(err) => tracing::warn!(path = ?child, error = %err, "skipping during size walk"),
        }
    }
    Ok(total)
}

/// Build a [`UsageNode`] for `path`, expanding children while `depth > 0`.
fn build_node(path: &Path, depth: usize) -> Result<UsageNode> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| OrcaError::from_io(path, e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let is_dir = meta.is_dir() && !meta.file_type().is_symlink();

    if !is_dir {
        return Ok(UsageNode {
            path: path.to_path_buf(),
            name,
            size: meta.len(),
            is_dir: false,
            children: Vec::new(),
        });
    }

    let mut children = Vec::new();
    let mut total = 0u64;
    for child in read_children(path)? {
        if depth > 0 {
            match build_node(&child, depth - 1) {
                Ok(node) => {
                    total = total.saturating_add(node.size);
                    children.push(node);
                }
                Err(err) => tracing::warn!(path = ?child, error = %err, "skipping child node"),
            }
        } else {
            // At the depth limit: still account for the size, but don't expand.
            match size_recursive(&child) {
                Ok(n) => total = total.saturating_add(n),
                Err(err) => tracing::warn!(path = ?child, error = %err, "skipping child size"),
            }
        }
    }
    // Largest children first for a treemap/bar visualizer.
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));

    Ok(UsageNode {
        path: path.to_path_buf(),
        name,
        size: total,
        is_dir: true,
        children,
    })
}

/// List the immediate children of a directory as full paths.
fn read_children(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let read = std::fs::read_dir(dir).map_err(|e| OrcaError::from_io(dir, e))?;
    for entry in read {
        let entry = entry.map_err(|e| OrcaError::from_io(dir, e))?;
        out.push(entry.path());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a tree: root/{a.txt(10), sub/{b.txt(20), c.txt(5)}} → 35 bytes.
    fn make_tree(root: &Path) {
        fs::create_dir_all(root.join("sub")).expect("mkdir");
        fs::write(root.join("a.txt"), vec![0u8; 10]).expect("write");
        fs::write(root.join("sub/b.txt"), vec![0u8; 20]).expect("write");
        fs::write(root.join("sub/c.txt"), vec![0u8; 5]).expect("write");
    }

    #[tokio::test]
    async fn dir_size_sums_recursively() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_tree(dir.path());
        assert_eq!(dir_size(dir.path()).await.expect("size"), 35);
    }

    #[tokio::test]
    async fn dir_size_of_single_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("x");
        fs::write(&f, vec![0u8; 42]).expect("write");
        assert_eq!(dir_size(&f).await.expect("size"), 42);
    }

    #[tokio::test]
    async fn tree_expands_to_depth_and_aggregates_beyond() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_tree(dir.path());

        // Depth 1: root expands to a.txt and sub, but sub's children are summed,
        // not listed.
        let node = disk_usage_tree(dir.path(), 1).await.expect("tree");
        assert_eq!(node.size, 35);
        assert!(node.is_dir);
        let sub = node
            .children
            .iter()
            .find(|c| c.name == "sub")
            .expect("sub present");
        assert_eq!(sub.size, 25); // 20 + 5, aggregated
        assert!(sub.children.is_empty(), "depth limit: sub not expanded");

        // Largest-first ordering: sub (25) before a.txt (10).
        assert_eq!(node.children[0].name, "sub");
    }

    #[tokio::test]
    async fn tree_full_depth_lists_leaves() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_tree(dir.path());
        let node = disk_usage_tree(dir.path(), 5).await.expect("tree");
        let sub = node.children.iter().find(|c| c.name == "sub").expect("sub");
        assert_eq!(sub.children.len(), 2);
        assert!(sub
            .children
            .iter()
            .any(|c| c.name == "b.txt" && c.size == 20));
    }

    #[test]
    fn filesystem_usage_is_sane() {
        let dir = tempfile::tempdir().expect("tempdir");
        let u = filesystem_usage(dir.path()).expect("statvfs");
        assert!(u.total > 0, "total should be positive");
        assert!(u.used <= u.total, "used must not exceed total");
        assert!(u.free <= u.total, "free must not exceed total");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_are_not_followed_in_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("big.bin");
        fs::write(&target, vec![0u8; 1000]).expect("write");
        let linkdir = dir.path().join("links");
        fs::create_dir(&linkdir).expect("mkdir");
        std::os::unix::fs::symlink(&target, linkdir.join("ln")).expect("symlink");

        // The link directory should not count the 1000-byte target.
        let size = dir_size(&linkdir).await.expect("size");
        assert!(
            size < 1000,
            "symlink target must not be traversed, got {size}"
        );
    }
}
