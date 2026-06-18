//! File mutation operations: copy, move, delete, rename and node creation.
//!
//! All operations are async (`tokio::fs`). Copy reports progress through an
//! optional [`tokio::sync::mpsc`] channel so the GUI can render a progress bar
//! without blocking. Symlinks are copied as links (never followed), so a copy
//! can never silently duplicate a link's target tree.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::Sender;

use crate::error::{OrcaError, Result};
use crate::fs::trash;

/// I/O buffer size for streaming file copies (64 KiB).
const COPY_BUF_SIZE: usize = 64 * 1024;

/// `EXDEV` errno on Linux: a rename crossed a filesystem boundary. Used to fall
/// back from `rename(2)` to copy-and-delete in [`move_entry`].
const EXDEV: i32 = 18;

/// Progress update emitted by [`copy`] as bytes are transferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyProgress {
    /// The file currently being copied.
    pub current: PathBuf,
    /// Total bytes copied so far across the whole operation.
    pub bytes_copied: u64,
    /// Total bytes the operation will copy (computed before copying starts).
    pub total_bytes: u64,
}

/// Recursively copy `src` to `dst`.
///
/// If `src` is a directory it is copied recursively; symlinks are recreated as
/// links. When `overwrite` is `false` and `dst` already exists, returns
/// [`OrcaError::AlreadyExists`]. Progress is reported on `progress` if provided.
///
/// # Errors
/// I/O errors, or [`OrcaError::AlreadyExists`] when the destination exists and
/// `overwrite` is `false`.
pub async fn copy(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    overwrite: bool,
    progress: Option<Sender<CopyProgress>>,
) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    if !overwrite && tokio::fs::symlink_metadata(dst).await.is_ok() {
        return Err(OrcaError::AlreadyExists(dst.to_path_buf()));
    }

    let total_bytes = tree_size(src).await?;
    let mut copied: u64 = 0;

    // Explicit work stack avoids async recursion. Each item is a (source,
    // destination) pair; directories expand their children onto the stack.
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((s, d)) = stack.pop() {
        let meta = tokio::fs::symlink_metadata(&s)
            .await
            .map_err(|e| OrcaError::from_io(&s, e))?;
        let ft = meta.file_type();

        if ft.is_symlink() {
            copy_symlink(&s, &d).await?;
        } else if ft.is_dir() {
            ensure_dir(&d).await?;
            let mut read = tokio::fs::read_dir(&s)
                .await
                .map_err(|e| OrcaError::from_io(&s, e))?;
            while let Some(child) = read
                .next_entry()
                .await
                .map_err(|e| OrcaError::from_io(&s, e))?
            {
                stack.push((child.path(), d.join(child.file_name())));
            }
        } else {
            copied = copy_file(&s, &d, copied, total_bytes, progress.as_ref()).await?;
        }
    }

    Ok(())
}

/// Copy a single regular file, streaming in [`COPY_BUF_SIZE`] chunks, preserving
/// unix permission bits. Returns the running `copied` byte total.
async fn copy_file(
    src: &Path,
    dst: &Path,
    mut copied: u64,
    total_bytes: u64,
    progress: Option<&Sender<CopyProgress>>,
) -> Result<u64> {
    let mut reader = tokio::fs::File::open(src)
        .await
        .map_err(|e| OrcaError::from_io(src, e))?;
    let mut writer = tokio::fs::File::create(dst)
        .await
        .map_err(|e| OrcaError::from_io(dst, e))?;

    let mut buf = vec![0u8; COPY_BUF_SIZE];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .map_err(|e| OrcaError::from_io(src, e))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| OrcaError::from_io(dst, e))?;
        copied += n as u64;
        if let Some(tx) = progress {
            // Drop progress if the receiver is gone; copying continues.
            let _ = tx
                .send(CopyProgress {
                    current: src.to_path_buf(),
                    bytes_copied: copied,
                    total_bytes,
                })
                .await;
        }
    }
    writer
        .flush()
        .await
        .map_err(|e| OrcaError::from_io(dst, e))?;
    copy_permissions(src, dst).await?;
    Ok(copied)
}

/// Recreate a symlink at `dst` pointing at the same target as the one at `src`.
async fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    let target = tokio::fs::read_link(src)
        .await
        .map_err(|e| OrcaError::from_io(src, e))?;
    if tokio::fs::symlink_metadata(dst).await.is_ok() {
        tokio::fs::remove_file(dst)
            .await
            .map_err(|e| OrcaError::from_io(dst, e))?;
    }
    #[cfg(unix)]
    {
        tokio::fs::symlink(&target, dst)
            .await
            .map_err(|e| OrcaError::from_io(dst, e))
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        Err(OrcaError::Other(
            "symlink copy unsupported on this platform".into(),
        ))
    }
}

/// Copy unix permission bits from `src` to `dst`. No-op on non-unix.
async fn copy_permissions(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = tokio::fs::metadata(src)
            .await
            .map_err(|e| OrcaError::from_io(src, e))?;
        let perms = std::fs::Permissions::from_mode(meta.permissions().mode());
        tokio::fs::set_permissions(dst, perms)
            .await
            .map_err(|e| OrcaError::from_io(dst, e))
    }
    #[cfg(not(unix))]
    {
        let _ = (src, dst);
        Ok(())
    }
}

/// Create `dir` if it does not already exist (no error if it does).
async fn ensure_dir(dir: &Path) -> Result<()> {
    match tokio::fs::create_dir(dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(OrcaError::from_io(dir, e)),
    }
}

/// Compute the total byte size of a file or directory tree (symlinks counted as
/// their own small size, not followed).
async fn tree_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let meta = tokio::fs::symlink_metadata(&p)
            .await
            .map_err(|e| OrcaError::from_io(&p, e))?;
        let ft = meta.file_type();
        if ft.is_dir() {
            let mut read = tokio::fs::read_dir(&p)
                .await
                .map_err(|e| OrcaError::from_io(&p, e))?;
            while let Some(child) = read
                .next_entry()
                .await
                .map_err(|e| OrcaError::from_io(&p, e))?
            {
                stack.push(child.path());
            }
        } else if !ft.is_symlink() {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Move `src` to `dst`, using `rename(2)` on the same filesystem and falling
/// back to copy-and-delete across filesystem boundaries (`EXDEV`).
///
/// # Errors
/// [`OrcaError::AlreadyExists`] if `dst` exists; I/O errors otherwise.
pub async fn move_entry(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    if tokio::fs::symlink_metadata(dst).await.is_ok() {
        return Err(OrcaError::AlreadyExists(dst.to_path_buf()));
    }
    match tokio::fs::rename(src, dst).await {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy(src, dst, false, None).await?;
            delete(src, false).await
        }
        Err(e) => Err(OrcaError::from_io(src, e)),
    }
}

/// Delete `path`. When `use_trash` is `true` the entry is moved to the XDG
/// trash (recoverable); otherwise it is removed permanently.
///
/// # Errors
/// I/O errors, or trash errors when `use_trash` is `true`.
pub async fn delete(path: impl AsRef<Path>, use_trash: bool) -> Result<()> {
    let path = path.as_ref();
    if use_trash {
        // `Box::pin` breaks the otherwise-cyclic async dependency between
        // `delete` and `trash::move_to_trash` (which moves across filesystems by
        // calling back into `delete`); the boxed future is heap-allocated so its
        // size is finite.
        Box::pin(trash::move_to_trash(path)).await.map(|_| ())
    } else {
        let meta = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| OrcaError::from_io(path, e))?;
        if meta.is_dir() {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| OrcaError::from_io(path, e))
        } else {
            tokio::fs::remove_file(path)
                .await
                .map_err(|e| OrcaError::from_io(path, e))
        }
    }
}

/// Rename `path` to `new_name` within the same parent directory.
///
/// `new_name` must be a bare file name with no path separators. Returns the new
/// path.
///
/// # Errors
/// [`OrcaError::InvalidPath`] if `new_name` is empty or contains a separator;
/// [`OrcaError::AlreadyExists`] if the target name is taken; I/O errors.
pub async fn rename(path: impl AsRef<Path>, new_name: &str) -> Result<PathBuf> {
    let path = path.as_ref();
    if new_name.is_empty() || new_name.contains('/') || new_name.contains('\\') {
        return Err(OrcaError::InvalidPath(format!(
            "rename target must be a bare file name, got {new_name:?}"
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| OrcaError::InvalidPath(format!("path has no parent: {path:?}")))?;
    let new_path = parent.join(new_name);
    if tokio::fs::symlink_metadata(&new_path).await.is_ok() {
        return Err(OrcaError::AlreadyExists(new_path));
    }
    tokio::fs::rename(path, &new_path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))?;
    Ok(new_path)
}

/// Create a single directory (its parent must already exist).
///
/// # Errors
/// [`OrcaError::AlreadyExists`] if it exists; I/O errors otherwise.
pub async fn create_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    tokio::fs::create_dir(path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))
}

/// Create an empty file. Fails if the file already exists.
///
/// # Errors
/// [`OrcaError::AlreadyExists`] if it exists; I/O errors otherwise.
pub async fn create_file(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    opts.open(path)
        .await
        .map(|_| ())
        .map_err(|e| OrcaError::from_io(path, e))
}

/// Create a symbolic link at `link` pointing to `target`.
///
/// # Errors
/// I/O errors; on non-unix platforms returns [`OrcaError::Other`].
pub async fn create_symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> Result<()> {
    let link = link.as_ref();
    #[cfg(unix)]
    {
        tokio::fs::symlink(target.as_ref(), link)
            .await
            .map_err(|e| OrcaError::from_io(link, e))
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        Err(OrcaError::Other(
            "symlinks unsupported on this platform".into(),
        ))
    }
}

/// Create a hard link at `link` referring to the same inode as `target`.
///
/// # Errors
/// I/O errors (e.g. cross-device, or `target` is a directory).
pub async fn create_hardlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> Result<()> {
    let link = link.as_ref();
    tokio::fs::hard_link(target.as_ref(), link)
        .await
        .map_err(|e| OrcaError::from_io(link, e))
}

/// Change the owning user and group of `path`.
///
/// Both `uid` and `gid` must be valid user/group IDs on the system. The
/// calling process typically needs `CAP_CHOWN` (or must own the file and
/// set gid to one of its supplementary groups).
///
/// # Errors
/// I/O errors or permission denied; non-unix returns [`OrcaError::Other`].
pub async fn set_owner(path: impl AsRef<Path>, uid: u32, gid: u32) -> Result<()> {
    let path = path.as_ref().to_path_buf();
    #[cfg(unix)]
    {
        tokio::task::spawn_blocking(move || {
            let status = std::process::Command::new("chown")
                .arg(format!("{uid}:{gid}"))
                .arg(&path)
                .status()
                .map_err(|e| OrcaError::Other(format!("chown: {e}")))?;
            if status.success() {
                Ok(())
            } else {
                Err(OrcaError::Other(format!(
                    "chown failed (check privileges): exit {:?}",
                    status.code()
                )))
            }
        })
        .await
        .map_err(|e| OrcaError::Other(format!("spawn_blocking: {e}")))?
    }
    #[cfg(not(unix))]
    {
        let _ = (path, uid, gid);
        Err(OrcaError::Other(
            "chown unsupported on this platform".into(),
        ))
    }
}

/// Set the unix permission bits of `path` to `mode` (the low 12 bits are used).
///
/// # Errors
/// I/O errors; on non-unix platforms returns [`OrcaError::Other`].
pub async fn set_permissions(path: impl AsRef<Path>, mode: u32) -> Result<()> {
    let path = path.as_ref();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        tokio::fs::set_permissions(path, perms)
            .await
            .map_err(|e| OrcaError::from_io(path, e))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err(OrcaError::Other(
            "permission bits unsupported on this platform".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[tokio::test]
    async fn copy_file_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, b"hello orca").expect("write");

        copy(&src, &dst, false, None).await.expect("copy");
        assert_eq!(fs::read(&dst).expect("read"), b"hello orca");
    }

    #[tokio::test]
    async fn copy_refuses_existing_without_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("a");
        let dst = dir.path().join("b");
        fs::write(&src, b"x").expect("write");
        fs::write(&dst, b"y").expect("write");

        let err = copy(&src, &dst, false, None).await.expect_err("exists");
        assert!(matches!(err, OrcaError::AlreadyExists(_)));
        // Original destination untouched.
        assert_eq!(fs::read(&dst).expect("read"), b"y");
    }

    #[tokio::test]
    async fn copy_directory_recursive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("tree");
        fs::create_dir(&src).expect("mkdir");
        fs::create_dir(src.join("sub")).expect("mkdir");
        fs::write(src.join("root.txt"), b"r").expect("write");
        fs::write(src.join("sub/leaf.txt"), b"l").expect("write");

        let dst = dir.path().join("copy");
        copy(&src, &dst, false, None).await.expect("copy");
        assert_eq!(fs::read(dst.join("root.txt")).expect("read"), b"r");
        assert_eq!(fs::read(dst.join("sub/leaf.txt")).expect("read"), b"l");
    }

    #[tokio::test]
    async fn copy_reports_progress() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("big");
        let mut f = fs::File::create(&src).expect("create");
        f.write_all(&vec![7u8; COPY_BUF_SIZE * 2 + 10])
            .expect("write");
        let dst = dir.path().join("big.copy");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let handle = tokio::spawn(async move { copy(&src, &dst, false, Some(tx)).await });

        let mut last = 0;
        while let Some(p) = rx.recv().await {
            assert!(p.bytes_copied >= last);
            last = p.bytes_copied;
            assert_eq!(p.total_bytes, (COPY_BUF_SIZE * 2 + 10) as u64);
        }
        handle.await.expect("join").expect("copy");
        assert_eq!(last, (COPY_BUF_SIZE * 2 + 10) as u64);
    }

    #[tokio::test]
    async fn move_entry_same_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("a");
        let dst = dir.path().join("b");
        fs::write(&src, b"data").expect("write");

        move_entry(&src, &dst).await.expect("move");
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).expect("read"), b"data");
    }

    #[tokio::test]
    async fn delete_permanent_file_and_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("f");
        fs::write(&file, b"x").expect("write");
        delete(&file, false).await.expect("delete file");
        assert!(!file.exists());

        let sub = dir.path().join("d");
        fs::create_dir(&sub).expect("mkdir");
        fs::write(sub.join("inner"), b"y").expect("write");
        delete(&sub, false).await.expect("delete dir");
        assert!(!sub.exists());
    }

    #[tokio::test]
    async fn rename_changes_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("old.txt");
        fs::write(&path, b"x").expect("write");

        let new = rename(&path, "new.txt").await.expect("rename");
        assert_eq!(new, dir.path().join("new.txt"));
        assert!(new.exists());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn rename_rejects_separators() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f");
        fs::write(&path, b"x").expect("write");
        let err = rename(&path, "a/b").await.expect_err("invalid");
        assert!(matches!(err, OrcaError::InvalidPath(_)));
    }

    #[tokio::test]
    async fn create_helpers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let d = dir.path().join("newdir");
        create_dir(&d).await.expect("mkdir");
        assert!(d.is_dir());

        let f = dir.path().join("newfile");
        create_file(&f).await.expect("file");
        assert!(f.is_file());

        let dup = create_file(&f).await.expect_err("exists");
        assert!(matches!(dup, OrcaError::AlreadyExists(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_and_hardlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        fs::write(&target, b"t").expect("write");

        let link = dir.path().join("link");
        create_symlink(&target, &link).await.expect("symlink");
        assert!(fs::symlink_metadata(&link)
            .expect("meta")
            .file_type()
            .is_symlink());

        let hard = dir.path().join("hard");
        create_hardlink(&target, &hard).await.expect("hardlink");
        assert_eq!(fs::read(&hard).expect("read"), b"t");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn set_permissions_applies_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("f");
        fs::write(&f, b"x").expect("write");

        set_permissions(&f, 0o600).await.expect("chmod");
        let mode = fs::metadata(&f).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn copy_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("s");
        fs::write(&src, b"x").expect("write");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o640)).expect("chmod");

        let dst = dir.path().join("d");
        copy(&src, &dst, false, None).await.expect("copy");
        let mode = fs::metadata(&dst).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
}
