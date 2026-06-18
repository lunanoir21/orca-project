//! Integration test: a full copy → move → delete lifecycle through the public
//! `orca-core` API.

use std::fs;

use orca_core::{copy, delete, list_dir, move_entry, FilterOptions, SortKey};

#[tokio::test]
async fn copy_move_delete_cycle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // Seed a small tree: src/{a.txt, nested/b.txt}.
    fs::create_dir_all(root.join("src/nested")).expect("mkdir");
    fs::write(root.join("src/a.txt"), b"alpha").expect("write");
    fs::write(root.join("src/nested/b.txt"), b"beta").expect("write");

    // Copy the whole tree to dst.
    let dst = root.join("dst");
    copy(root.join("src"), &dst, false, None)
        .await
        .expect("copy");
    assert_eq!(fs::read(dst.join("a.txt")).expect("read"), b"alpha");
    assert_eq!(fs::read(dst.join("nested/b.txt")).expect("read"), b"beta");
    // Original still present after a copy.
    assert!(root.join("src/a.txt").exists());

    // Move dst to moved.
    let moved = root.join("moved");
    move_entry(&dst, &moved).await.expect("move");
    assert!(!dst.exists(), "source gone after move");
    assert_eq!(fs::read(moved.join("a.txt")).expect("read"), b"alpha");

    // Permanently delete the moved tree (no trash).
    delete(&moved, false).await.expect("delete");
    assert!(!moved.exists(), "deleted tree is gone");

    // src remains; listing it shows the two top-level entries.
    let entries = list_dir(root.join("src"), &FilterOptions::default(), SortKey::Name)
        .await
        .expect("list");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["nested", "a.txt"]);
}
