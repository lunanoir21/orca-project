//! Integration test: compress to each supported format and extract back,
//! verifying the round-trip preserves file contents.

use std::fs;
use std::path::Path;

use orca_core::{compress, extract, list_archive, ArchiveFormat};

fn make_tree(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("proj");
    fs::create_dir_all(root.join("sub")).expect("mkdir");
    fs::write(root.join("a.txt"), b"alpha contents").expect("write");
    fs::write(root.join("sub/b.txt"), b"beta contents").expect("write");
    root
}

async fn roundtrip(format: ArchiveFormat, ext: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = make_tree(dir.path());
    let archive = dir.path().join(format!("bundle.{ext}"));

    compress(vec![root], archive.clone(), format, None)
        .await
        .expect("compress");

    let listed = list_archive(archive.clone()).await.expect("list");
    assert!(listed.iter().any(|e| e.path.contains("a.txt")));

    let out = dir.path().join("out");
    extract(archive, out.clone(), None).await.expect("extract");
    assert_eq!(
        fs::read(out.join("proj/a.txt")).expect("read"),
        b"alpha contents"
    );
    assert_eq!(
        fs::read(out.join("proj/sub/b.txt")).expect("read"),
        b"beta contents"
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
