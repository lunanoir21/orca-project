//! Archive operations: create, extract and inspect zip and tar archives.
//!
//! Supported formats: zip, `tar.gz`, `tar.xz`, `tar.bz2` (and plain `tar` on
//! extraction). The underlying archive crates are synchronous and CPU-bound, so
//! the public async entry points run the real work on a blocking task and report
//! progress through an optional channel.
//!
//! # Security
//! Extraction is hardened against "zip-slip" path traversal: every archived
//! entry path is validated to contain only normal components (no `..`, no root
//! or drive prefix) before it is joined onto the destination, so a malicious
//! archive cannot write outside the target directory.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use tokio::sync::mpsc::Sender;

use crate::error::{OrcaError, Result};

/// A supported archive container + compression codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    /// PKZIP container with per-entry deflate compression.
    Zip,
    /// tar container, gzip-compressed.
    TarGz,
    /// tar container, xz-compressed.
    TarXz,
    /// tar container, bzip2-compressed.
    TarBz2,
    /// Uncompressed tar container (extraction/inspection only path uses this on
    /// auto-detect; [`compress`] accepts it too).
    Tar,
}

/// One entry listed by [`list_archive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Entry path as stored in the archive.
    pub path: String,
    /// Uncompressed size in bytes (0 for directories).
    pub size: u64,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// Progress update emitted while compressing or extracting.
#[derive(Debug, Clone)]
pub struct ArchiveProgress {
    /// Bytes processed so far.
    pub processed: u64,
    /// Total bytes expected (best-effort; may be 0 if unknown).
    pub total: u64,
    /// The entry currently being processed.
    pub current: PathBuf,
}

/// A file gathered for compression: its location on disk and its name inside the
/// archive.
struct PendingEntry {
    fs_path: PathBuf,
    arc_name: String,
    is_dir: bool,
    size: u64,
}

/// Compress `paths` into `dest` using `format`.
///
/// Directory inputs are added recursively. Entry names inside the archive are
/// rooted at each input's final component (e.g. compressing `/a/b/proj` stores
/// `proj/...`). If `progress` is supplied, an [`ArchiveProgress`] is sent per
/// file as it is written.
///
/// # Errors
/// I/O errors creating `dest` or reading inputs; [`OrcaError::Other`] if the
/// blocking task panics.
pub async fn compress(
    paths: Vec<PathBuf>,
    dest: PathBuf,
    format: ArchiveFormat,
    progress: Option<Sender<ArchiveProgress>>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || compress_blocking(&paths, &dest, format, progress.as_ref()))
        .await
        .map_err(|e| OrcaError::Other(format!("compress task failed: {e}")))?
}

/// Extract `archive` into directory `dest`, auto-detecting the format from the
/// file's magic bytes.
///
/// `dest` is created if missing. Entry paths are sanitised against traversal.
///
/// # Errors
/// I/O errors; [`OrcaError::InvalidPath`] on an unsafe entry path or unknown
/// format; [`OrcaError::Other`] if the blocking task panics.
pub async fn extract(
    archive: PathBuf,
    dest: PathBuf,
    progress: Option<Sender<ArchiveProgress>>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || extract_blocking(&archive, &dest, progress.as_ref()))
        .await
        .map_err(|e| OrcaError::Other(format!("extract task failed: {e}")))?
}

/// List the entries of `archive` without extracting it. Format is auto-detected.
///
/// # Errors
/// I/O errors; [`OrcaError::InvalidPath`] on unknown format; [`OrcaError::Other`]
/// if the blocking task panics.
pub async fn list_archive(archive: PathBuf) -> Result<Vec<ArchiveEntry>> {
    tokio::task::spawn_blocking(move || list_blocking(&archive))
        .await
        .map_err(|e| OrcaError::Other(format!("list task failed: {e}")))?
}

// ---------------------------------------------------------------------------
// Blocking implementations
// ---------------------------------------------------------------------------

fn compress_blocking(
    paths: &[PathBuf],
    dest: &Path,
    format: ArchiveFormat,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    let entries = gather_entries(paths)?;
    let total: u64 = entries.iter().map(|e| e.size).sum();

    let file = std::fs::File::create(dest).map_err(|e| OrcaError::from_io(dest, e))?;
    let writer = std::io::BufWriter::new(file);

    match format {
        ArchiveFormat::Zip => write_zip(writer, &entries, total, progress),
        ArchiveFormat::Tar => {
            let mut builder = tar::Builder::new(writer);
            write_tar(&mut builder, &entries, total, progress)?;
            builder
                .into_inner()
                .map_err(|e| OrcaError::from_io(dest, e))?;
            Ok(())
        }
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            write_tar(&mut builder, &entries, total, progress)?;
            let enc = builder
                .into_inner()
                .map_err(|e| OrcaError::from_io(dest, e))?;
            enc.finish().map_err(|e| OrcaError::from_io(dest, e))?;
            Ok(())
        }
        ArchiveFormat::TarXz => {
            let enc = xz2::write::XzEncoder::new(writer, 6);
            let mut builder = tar::Builder::new(enc);
            write_tar(&mut builder, &entries, total, progress)?;
            let enc = builder
                .into_inner()
                .map_err(|e| OrcaError::from_io(dest, e))?;
            enc.finish().map_err(|e| OrcaError::from_io(dest, e))?;
            Ok(())
        }
        ArchiveFormat::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(writer, bzip2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            write_tar(&mut builder, &entries, total, progress)?;
            let enc = builder
                .into_inner()
                .map_err(|e| OrcaError::from_io(dest, e))?;
            enc.finish().map_err(|e| OrcaError::from_io(dest, e))?;
            Ok(())
        }
    }
}

/// Walk the input paths, producing a flat, ordered list of directory and file
/// entries with archive-relative names.
fn gather_entries(paths: &[PathBuf]) -> Result<Vec<PendingEntry>> {
    let mut out = Vec::new();
    for input in paths {
        let meta = std::fs::symlink_metadata(input).map_err(|e| OrcaError::from_io(input, e))?;
        let base = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| OrcaError::InvalidPath(format!("input has no file name: {input:?}")))?;

        if meta.is_dir() {
            for entry in walkdir::WalkDir::new(input).sort_by_file_name() {
                let entry = entry
                    .map_err(|e| OrcaError::Other(format!("walk failed under {input:?}: {e}")))?;
                let rel = entry
                    .path()
                    .strip_prefix(input)
                    .unwrap_or_else(|_| entry.path());
                let arc_name = join_arcname(&base, rel);
                let is_dir = entry.file_type().is_dir();
                let size = if is_dir {
                    0
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                };
                out.push(PendingEntry {
                    fs_path: entry.path().to_path_buf(),
                    arc_name,
                    is_dir,
                    size,
                });
            }
        } else {
            out.push(PendingEntry {
                fs_path: input.clone(),
                arc_name: base,
                is_dir: false,
                size: meta.len(),
            });
        }
    }
    Ok(out)
}

/// Build a forward-slash archive name from a base component and a relative path.
fn join_arcname(base: &str, rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        return base.to_string();
    }
    let mut name = base.to_string();
    for comp in rel.components() {
        if let Component::Normal(c) = comp {
            name.push('/');
            name.push_str(&c.to_string_lossy());
        }
    }
    name
}

fn write_tar<W: Write>(
    builder: &mut tar::Builder<W>,
    entries: &[PendingEntry],
    total: u64,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    let mut processed = 0u64;
    for e in entries {
        if e.is_dir {
            builder
                .append_dir(&e.arc_name, &e.fs_path)
                .map_err(|err| OrcaError::from_io(&e.fs_path, err))?;
        } else {
            let mut f = std::fs::File::open(&e.fs_path)
                .map_err(|err| OrcaError::from_io(&e.fs_path, err))?;
            builder
                .append_file(&e.arc_name, &mut f)
                .map_err(|err| OrcaError::from_io(&e.fs_path, err))?;
            processed += e.size;
            send_progress(progress, processed, total, &e.fs_path);
        }
    }
    Ok(())
}

fn write_zip<W: Write + std::io::Seek>(
    writer: W,
    entries: &[PendingEntry],
    total: u64,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(writer);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut processed = 0u64;
    for e in entries {
        if e.is_dir {
            // Zip directory entries are conventionally suffixed with '/'.
            let name = format!("{}/", e.arc_name.trim_end_matches('/'));
            zip.add_directory(name, opts)
                .map_err(|err| OrcaError::Other(format!("zip dir {:?}: {err}", e.arc_name)))?;
        } else {
            zip.start_file(&e.arc_name, opts)
                .map_err(|err| OrcaError::Other(format!("zip file {:?}: {err}", e.arc_name)))?;
            let mut f = std::fs::File::open(&e.fs_path)
                .map_err(|err| OrcaError::from_io(&e.fs_path, err))?;
            std::io::copy(&mut f, &mut zip).map_err(|err| OrcaError::from_io(&e.fs_path, err))?;
            processed += e.size;
            send_progress(progress, processed, total, &e.fs_path);
        }
    }
    zip.finish()
        .map_err(|err| OrcaError::Other(format!("zip finalize: {err}")))?;
    Ok(())
}

fn extract_blocking(
    archive: &Path,
    dest: &Path,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| OrcaError::from_io(dest, e))?;
    match detect_format(archive)? {
        ArchiveFormat::Zip => extract_zip(archive, dest, progress),
        other => extract_tar(archive, dest, other, progress),
    }
}

fn extract_zip(
    archive: &Path,
    dest: &Path,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(|e| OrcaError::from_io(archive, e))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| OrcaError::Other(format!("open zip: {e}")))?;
    let total = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.size()))
        .sum();
    let mut processed = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| OrcaError::Other(format!("zip entry {i}: {e}")))?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| OrcaError::InvalidPath(format!("unsafe zip entry: {}", entry.name())))?;
        let out_path = safe_join(dest, &rel)?;
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| OrcaError::from_io(&out_path, e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| OrcaError::from_io(parent, e))?;
            }
            let mut out =
                std::fs::File::create(&out_path).map_err(|e| OrcaError::from_io(&out_path, e))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| OrcaError::from_io(&out_path, e))?;
            processed += entry.size();
            send_progress(progress, processed, total, &out_path);
        }
    }
    Ok(())
}

fn extract_tar(
    archive: &Path,
    dest: &Path,
    format: ArchiveFormat,
    progress: Option<&Sender<ArchiveProgress>>,
) -> Result<()> {
    let reader = open_tar_reader(archive, format)?;
    let mut tar = tar::Archive::new(reader);
    let mut processed = 0u64;
    let entries = tar.entries().map_err(|e| OrcaError::from_io(archive, e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| OrcaError::from_io(archive, e))?;
        let rel = entry
            .path()
            .map_err(|e| OrcaError::from_io(archive, e))?
            .into_owned();
        let out_path = safe_join(dest, &rel)?;
        entry
            .unpack(&out_path)
            .map_err(|e| OrcaError::from_io(&out_path, e))?;
        processed += entry.size();
        send_progress(progress, processed, 0, &out_path);
    }
    Ok(())
}

fn list_blocking(archive: &Path) -> Result<Vec<ArchiveEntry>> {
    match detect_format(archive)? {
        ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| OrcaError::from_io(archive, e))?;
            let mut zip = zip::ZipArchive::new(file)
                .map_err(|e| OrcaError::Other(format!("open zip: {e}")))?;
            let mut out = Vec::with_capacity(zip.len());
            for i in 0..zip.len() {
                let entry = zip
                    .by_index(i)
                    .map_err(|e| OrcaError::Other(format!("zip entry {i}: {e}")))?;
                out.push(ArchiveEntry {
                    path: entry.name().to_string(),
                    size: entry.size(),
                    is_dir: entry.is_dir(),
                });
            }
            Ok(out)
        }
        other => {
            let reader = open_tar_reader(archive, other)?;
            let mut tar = tar::Archive::new(reader);
            let mut out = Vec::new();
            for entry in tar.entries().map_err(|e| OrcaError::from_io(archive, e))? {
                let entry = entry.map_err(|e| OrcaError::from_io(archive, e))?;
                let header = entry.header();
                let is_dir = header.entry_type().is_dir();
                let path = entry
                    .path()
                    .map_err(|e| OrcaError::from_io(archive, e))?
                    .to_string_lossy()
                    .into_owned();
                out.push(ArchiveEntry {
                    path,
                    size: header.size().unwrap_or(0),
                    is_dir,
                });
            }
            Ok(out)
        }
    }
}

/// Open a tar archive's byte stream, wrapping it in the right decompressor.
fn open_tar_reader(archive: &Path, format: ArchiveFormat) -> Result<Box<dyn Read>> {
    let file = std::fs::File::open(archive).map_err(|e| OrcaError::from_io(archive, e))?;
    let buf = std::io::BufReader::new(file);
    let reader: Box<dyn Read> = match format {
        ArchiveFormat::TarGz => Box::new(flate2::read::GzDecoder::new(buf)),
        ArchiveFormat::TarXz => Box::new(xz2::read::XzDecoder::new(buf)),
        ArchiveFormat::TarBz2 => Box::new(bzip2::read::BzDecoder::new(buf)),
        ArchiveFormat::Tar => Box::new(buf),
        ArchiveFormat::Zip => {
            return Err(OrcaError::InvalidPath(
                "zip is not a tar stream".to_string(),
            ))
        }
    };
    Ok(reader)
}

/// Detect the archive format from leading magic bytes, falling back to plain tar.
fn detect_format(archive: &Path) -> Result<ArchiveFormat> {
    let mut file = std::fs::File::open(archive).map_err(|e| OrcaError::from_io(archive, e))?;
    let mut magic = [0u8; 6];
    let n = file
        .read(&mut magic)
        .map_err(|e| OrcaError::from_io(archive, e))?;
    let magic = &magic[..n];
    if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
        Ok(ArchiveFormat::Zip)
    } else if magic.starts_with(&[0x1f, 0x8b]) {
        Ok(ArchiveFormat::TarGz)
    } else if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        Ok(ArchiveFormat::TarXz)
    } else if magic.starts_with(b"BZh") {
        Ok(ArchiveFormat::TarBz2)
    } else {
        // No recognised compression magic: treat as an uncompressed tar.
        Ok(ArchiveFormat::Tar)
    }
}

/// Join an archive-relative path onto `dest`, rejecting any component that could
/// escape the destination directory (`..`, absolute roots, drive prefixes).
fn safe_join(dest: &Path, rel: &Path) -> Result<PathBuf> {
    let mut out = dest.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(OrcaError::InvalidPath(format!(
                    "archive entry escapes destination: {rel:?}"
                )));
            }
        }
    }
    Ok(out)
}

/// Best-effort progress notification; a dropped receiver is ignored.
fn send_progress(
    progress: Option<&Sender<ArchiveProgress>>,
    processed: u64,
    total: u64,
    current: &Path,
) {
    if let Some(tx) = progress {
        let _ = tx.blocking_send(ArchiveProgress {
            processed,
            total,
            current: current.to_path_buf(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_tree(dir: &Path) -> PathBuf {
        let root = dir.join("proj");
        fs::create_dir_all(root.join("sub")).expect("mkdir");
        fs::write(root.join("a.txt"), b"hello alpha").expect("write");
        fs::write(root.join("sub/b.txt"), b"hello beta").expect("write");
        root
    }

    async fn roundtrip(format: ArchiveFormat, ext: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = make_tree(dir.path());
        let archive = dir.path().join(format!("out.{ext}"));

        compress(vec![root.clone()], archive.clone(), format, None)
            .await
            .expect("compress");
        assert!(archive.exists());

        // List shows the files.
        let listed = list_archive(archive.clone()).await.expect("list");
        let names: Vec<&str> = listed.iter().map(|e| e.path.as_str()).collect();
        assert!(
            names.iter().any(|n| n.contains("a.txt")),
            "names: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("b.txt")),
            "names: {names:?}"
        );

        // Extract into a fresh dir and compare contents.
        let out = dir.path().join("extracted");
        extract(archive, out.clone(), None).await.expect("extract");
        assert_eq!(
            fs::read(out.join("proj/a.txt")).expect("read"),
            b"hello alpha"
        );
        assert_eq!(
            fs::read(out.join("proj/sub/b.txt")).expect("read"),
            b"hello beta"
        );
    }

    #[tokio::test]
    async fn zip_roundtrip() {
        roundtrip(ArchiveFormat::Zip, "zip").await;
    }

    #[tokio::test]
    async fn tar_gz_roundtrip() {
        roundtrip(ArchiveFormat::TarGz, "tar.gz").await;
    }

    #[tokio::test]
    async fn tar_xz_roundtrip() {
        roundtrip(ArchiveFormat::TarXz, "tar.xz").await;
    }

    #[tokio::test]
    async fn tar_bz2_roundtrip() {
        roundtrip(ArchiveFormat::TarBz2, "tar.bz2").await;
    }

    #[tokio::test]
    async fn extract_auto_detects_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = make_tree(dir.path());
        // Deliberately mislabel the extension; detection must use magic bytes.
        let archive = dir.path().join("mislabeled.bin");
        compress(vec![root], archive.clone(), ArchiveFormat::TarGz, None)
            .await
            .expect("compress");
        let out = dir.path().join("ex");
        extract(archive, out.clone(), None).await.expect("extract");
        assert!(out.join("proj/a.txt").exists());
    }

    #[tokio::test]
    async fn progress_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = make_tree(dir.path());
        let archive = dir.path().join("p.zip");
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let handle = tokio::spawn(async move {
            compress(vec![root], archive, ArchiveFormat::Zip, Some(tx)).await
        });
        let mut updates = 0;
        while let Some(p) = rx.recv().await {
            assert!(p.processed <= p.total || p.total == 0);
            updates += 1;
        }
        handle.await.expect("join").expect("compress");
        assert!(updates >= 2, "expected per-file progress, got {updates}");
    }

    #[tokio::test]
    async fn rejects_zip_slip_path() {
        // safe_join must refuse a traversal component.
        let err = safe_join(Path::new("/tmp/dest"), Path::new("../escape")).expect_err("slip");
        assert!(matches!(err, OrcaError::InvalidPath(_)));
    }

    #[tokio::test]
    async fn single_file_compress() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("solo.txt");
        fs::write(&f, b"solo content").expect("write");
        let archive = dir.path().join("solo.tar.gz");
        compress(vec![f], archive.clone(), ArchiveFormat::TarGz, None)
            .await
            .expect("compress");
        let out = dir.path().join("ex");
        extract(archive, out.clone(), None).await.expect("extract");
        assert_eq!(
            fs::read(out.join("solo.txt")).expect("read"),
            b"solo content"
        );
    }
}
