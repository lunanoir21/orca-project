//! XDG Trash specification support.
//!
//! Implements the freedesktop.org Trash spec for the user's home trash at
//! `$XDG_DATA_HOME/Trash` (`files/` for the data, `info/` for `.trashinfo`
//! metadata). Deleting writes the `.trashinfo` file first (reserving the name
//! atomically with `O_EXCL`) and only then moves the data, so a crash can never
//! leave trashed data without a record of where to restore it.
//!
//! Original paths are stored percent-encoded per the spec; only path separators
//! and RFC 3986 unreserved characters are left literal.

use std::path::{Path, PathBuf};

use crate::error::{OrcaError, Result};

/// A single entry currently in the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// The entry's name inside the trash (the `files/<name>` basename).
    pub name: String,
    /// The absolute path the entry was deleted from (for restoration).
    pub original_path: PathBuf,
    /// The deletion timestamp as stored in the `.trashinfo` file (ISO-8601).
    pub deletion_date: String,
    /// The current location of the data inside the trash.
    pub trashed_path: PathBuf,
}

/// Resolve the home trash root, `files/` and `info/` directories, creating them
/// if necessary.
fn trash_dirs() -> Result<(PathBuf, PathBuf)> {
    let data = dirs::data_dir()
        .ok_or_else(|| OrcaError::Other("could not resolve XDG data directory".into()))?;
    let root = data.join("Trash");
    let files = root.join("files");
    let info = root.join("info");
    std::fs::create_dir_all(&files).map_err(|e| OrcaError::from_io(&files, e))?;
    std::fs::create_dir_all(&info).map_err(|e| OrcaError::from_io(&info, e))?;
    Ok((files, info))
}

/// Move `path` into the XDG trash, returning the resulting [`TrashEntry`].
///
/// # Errors
/// I/O errors, or if the XDG data directory cannot be resolved.
pub async fn move_to_trash(path: impl AsRef<Path>) -> Result<TrashEntry> {
    let path = path.as_ref();
    // Confirm the entry exists (without following a final symlink).
    tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))?;

    let original_path = std::path::absolute(path).map_err(|e| OrcaError::from_io(path, e))?;
    let base = original_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| OrcaError::InvalidPath(format!("cannot trash unnamed path: {path:?}")))?
        .to_string();

    let (files_dir, info_dir) = trash_dirs()?;
    let deletion_date = now_iso8601();

    // Reserve a unique name by exclusively creating the .trashinfo file. This
    // is the atomic step that prevents two deletions from colliding.
    let (name, info_path) = reserve_info(&info_dir, &base)?;
    let info_body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        crate::util::percent_encode_path(&original_path),
        deletion_date
    );
    if let Err(e) = std::fs::write(&info_path, info_body) {
        return Err(OrcaError::from_io(&info_path, e));
    }

    // Now move the data. On failure, roll back the reserved info file.
    let trashed_path = files_dir.join(&name);
    if let Err(e) = move_path(path, &trashed_path).await {
        let _ = std::fs::remove_file(&info_path);
        return Err(e);
    }

    Ok(TrashEntry {
        name,
        original_path,
        deletion_date,
        trashed_path,
    })
}

/// List all entries currently in the trash. Malformed `.trashinfo` files are
/// skipped with a logged warning rather than failing the whole listing.
///
/// # Errors
/// I/O errors reading the trash directory.
pub async fn list_trash() -> Result<Vec<TrashEntry>> {
    let (files_dir, info_dir) = trash_dirs()?;
    let mut entries = Vec::new();

    let mut read = tokio::fs::read_dir(&info_dir)
        .await
        .map_err(|e| OrcaError::from_io(&info_dir, e))?;
    while let Some(dir_entry) = read
        .next_entry()
        .await
        .map_err(|e| OrcaError::from_io(&info_dir, e))?
    {
        let info_path = dir_entry.path();
        if info_path.extension().and_then(|e| e.to_str()) != Some("trashinfo") {
            continue;
        }
        match parse_info(&info_path, &files_dir).await {
            Ok(entry) => entries.push(entry),
            Err(err) => {
                tracing::warn!(path = ?info_path, error = %err, "skipping malformed trashinfo");
            }
        }
    }
    Ok(entries)
}

/// Restore a trashed entry to its original location and remove its metadata.
///
/// # Errors
/// [`OrcaError::AlreadyExists`] if the original path is occupied; I/O errors.
pub async fn restore_from_trash(entry: &TrashEntry) -> Result<()> {
    if tokio::fs::symlink_metadata(&entry.original_path)
        .await
        .is_ok()
    {
        return Err(OrcaError::AlreadyExists(entry.original_path.clone()));
    }
    if let Some(parent) = entry.original_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OrcaError::from_io(parent, e))?;
    }
    move_path(&entry.trashed_path, &entry.original_path).await?;

    let (_, info_dir) = trash_dirs()?;
    let info_path = info_dir.join(format!("{}.trashinfo", entry.name));
    std::fs::remove_file(&info_path).map_err(|e| OrcaError::from_io(&info_path, e))?;
    Ok(())
}

/// Permanently empty the trash (both data and metadata).
///
/// # Errors
/// I/O errors removing trash contents.
pub async fn empty_trash() -> Result<()> {
    let (files_dir, info_dir) = trash_dirs()?;
    remove_dir_contents(&files_dir).await?;
    remove_dir_contents(&info_dir).await?;
    Ok(())
}

/// Remove every entry inside `dir` without removing `dir` itself.
async fn remove_dir_contents(dir: &Path) -> Result<()> {
    let mut read = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| OrcaError::from_io(dir, e))?;
    while let Some(entry) = read
        .next_entry()
        .await
        .map_err(|e| OrcaError::from_io(dir, e))?
    {
        let p = entry.path();
        let meta = tokio::fs::symlink_metadata(&p)
            .await
            .map_err(|e| OrcaError::from_io(&p, e))?;
        if meta.is_dir() {
            tokio::fs::remove_dir_all(&p)
                .await
                .map_err(|e| OrcaError::from_io(&p, e))?;
        } else {
            tokio::fs::remove_file(&p)
                .await
                .map_err(|e| OrcaError::from_io(&p, e))?;
        }
    }
    Ok(())
}

/// Move a path, falling back to copy-and-delete across filesystem boundaries.
async fn move_path(src: &Path, dst: &Path) -> Result<()> {
    match tokio::fs::rename(src, dst).await {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV: trash is on a different filesystem than the source.
            super::ops::copy(src, dst, false, None).await?;
            super::ops::delete(src, false).await
        }
        Err(e) => Err(OrcaError::from_io(src, e)),
    }
}

/// Exclusively create a `.trashinfo` file for `base`, disambiguating with a
/// numeric suffix on collision. Returns the chosen `(name, info_path)`.
fn reserve_info(info_dir: &Path, base: &str) -> Result<(String, PathBuf)> {
    for n in 0..10_000u32 {
        let name = if n == 0 {
            base.to_string()
        } else {
            suffix_name(base, n)
        };
        let info_path = info_dir.join(format!("{name}.trashinfo"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_path)
        {
            Ok(_) => return Ok((name, info_path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(OrcaError::from_io(&info_path, e)),
        }
    }
    Err(OrcaError::Other(format!(
        "too many trashed entries named like {base:?}"
    )))
}

/// Insert `_n` before the extension of `base` (e.g. `a.txt` -> `a_1.txt`).
fn suffix_name(base: &str, n: u32) -> String {
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}_{n}.{ext}"),
        _ => format!("{base}_{n}"),
    }
}

/// Parse a `.trashinfo` file into a [`TrashEntry`].
async fn parse_info(info_path: &Path, files_dir: &Path) -> Result<TrashEntry> {
    let text = tokio::fs::read_to_string(info_path)
        .await
        .map_err(|e| OrcaError::from_io(info_path, e))?;
    let mut original = None;
    let mut date = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Path=") {
            original = Some(PathBuf::from(crate::util::percent_decode(v.trim())));
        } else if let Some(v) = line.strip_prefix("DeletionDate=") {
            date = Some(v.trim().to_string());
        }
    }
    let original_path =
        original.ok_or_else(|| OrcaError::Other("trashinfo missing Path".into()))?;
    let deletion_date = date.unwrap_or_default();
    let name = info_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".trashinfo"))
        .ok_or_else(|| OrcaError::Other("invalid trashinfo file name".into()))?
        .to_string();

    Ok(TrashEntry {
        trashed_path: files_dir.join(&name),
        name,
        original_path,
        deletion_date,
    })
}

/// Current local time formatted as `YYYY-MM-DDThh:mm:ss`, falling back to UTC if
/// the local offset cannot be determined. Built manually to avoid pulling in the
/// `time` formatting feature.
fn now_iso8601() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::ENV_LOCK;

    #[test]
    fn suffix_name_inserts_before_extension() {
        assert_eq!(suffix_name("a.txt", 1), "a_1.txt");
        assert_eq!(suffix_name("noext", 2), "noext_2");
        assert_eq!(suffix_name(".hidden", 3), ".hidden_3");
    }

    #[test]
    fn iso8601_has_expected_shape() {
        let s = now_iso8601();
        // YYYY-MM-DDThh:mm:ss
        assert_eq!(s.len(), 19, "got {s:?}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }

    // Trash filesystem round-trips run against an isolated XDG_DATA_HOME so they
    // never touch the developer's real trash. They are serialized because they
    // mutate a process-wide environment variable.
    #[tokio::test]
    async fn trash_move_list_restore_empty_cycle() {
        let _guard = ENV_LOCK.lock().await;
        let home = tempfile::tempdir().expect("tempdir");
        // The ENV_LOCK guard ensures no other test reads/writes XDG_DATA_HOME
        // while this test owns it.
        std::env::set_var("XDG_DATA_HOME", home.path());

        let work = tempfile::tempdir().expect("work");
        let file = work.path().join("doc.txt");
        std::fs::write(&file, b"content").expect("write");

        let entry = move_to_trash(&file).await.expect("trash");
        assert!(!file.exists());
        assert!(entry.trashed_path.exists());

        let listed = list_trash().await.expect("list");
        assert!(listed.iter().any(|e| e.name == entry.name));

        restore_from_trash(&entry).await.expect("restore");
        assert!(file.exists());
        assert_eq!(std::fs::read(&file).expect("read"), b"content");

        // Trash again, then empty.
        let entry2 = move_to_trash(&file).await.expect("trash2");
        empty_trash().await.expect("empty");
        assert!(!entry2.trashed_path.exists());
        assert!(list_trash().await.expect("list").is_empty());

        std::env::remove_var("XDG_DATA_HOME");
    }

    #[tokio::test]
    async fn restore_refuses_when_original_exists() {
        let _guard = ENV_LOCK.lock().await;
        let home = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_DATA_HOME", home.path());

        let work = tempfile::tempdir().expect("work");
        let file = work.path().join("keep.txt");
        std::fs::write(&file, b"a").expect("write");
        let entry = move_to_trash(&file).await.expect("trash");

        // Recreate the original so restore must refuse.
        std::fs::write(&file, b"b").expect("write");
        let err = restore_from_trash(&entry).await.expect_err("conflict");
        assert!(matches!(err, OrcaError::AlreadyExists(_)));

        std::env::remove_var("XDG_DATA_HOME");
    }
}
