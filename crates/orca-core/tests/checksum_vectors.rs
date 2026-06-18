//! Integration test: checksum functions match published known-answer vectors.

use std::fs;

use orca_core::{checksum_blake3, checksum_md5, checksum_sha256};

#[tokio::test]
async fn known_vectors_for_abc() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("abc.bin");
    fs::write(&file, b"abc").expect("write");

    // Published "abc" digests for each algorithm.
    assert_eq!(
        checksum_sha256(&file).await.expect("sha256"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        checksum_md5(&file).await.expect("md5"),
        "900150983cd24fb0d6963f7d28e17f72"
    );
    assert_eq!(
        checksum_blake3(&file).await.expect("blake3"),
        "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
    );
}

#[tokio::test]
async fn empty_file_vectors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("empty.bin");
    fs::write(&file, b"").expect("write");

    assert_eq!(
        checksum_sha256(&file).await.expect("sha256"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        checksum_md5(&file).await.expect("md5"),
        "d41d8cd98f00b204e9800998ecf8427e"
    );
}
