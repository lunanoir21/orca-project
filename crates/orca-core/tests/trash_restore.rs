//! Integration test: trashing a file and restoring it via the XDG trash API.

use std::fs;

use orca_core::{list_trash, move_to_trash, restore_from_trash};

#[tokio::test]
async fn trash_then_restore() {
    let home = tempfile::tempdir().expect("tempdir");
    // Point XDG data (and thus the trash root) at an isolated directory. This is
    // the only test in this binary that touches the environment, so there is no
    // in-process race.
    std::env::set_var("XDG_DATA_HOME", home.path());

    let work = tempfile::tempdir().expect("workdir");
    let file = work.path().join("notes.txt");
    fs::write(&file, b"important").expect("write");

    // Trash it: the original disappears, and it shows up in the trash listing.
    move_to_trash(&file).await.expect("trash");
    assert!(!file.exists(), "original removed after trashing");

    let listed = list_trash().await.expect("list trash");
    let entry = listed
        .iter()
        .find(|e| e.original_path == file)
        .expect("trashed entry present");
    assert_eq!(entry.name, "notes.txt");

    // Restore it back to its original path with original contents.
    restore_from_trash(entry).await.expect("restore");
    assert_eq!(fs::read(&file).expect("read restored"), b"important");
    assert!(
        list_trash().await.expect("list").is_empty(),
        "trash empty after restore"
    );

    std::env::remove_var("XDG_DATA_HOME");
}
