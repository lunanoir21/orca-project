//! Integration test: bulk rename preview detects conflicts and a clean rule
//! applies successfully.

use std::fs;
use std::path::PathBuf;

use orca_core::{apply_rename, preview_rename, BulkRenameRule, RenameConflict};

fn make(dir: &std::path::Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|n| {
            let p = dir.join(n);
            fs::write(&p, b"x").expect("write");
            p
        })
        .collect()
}

#[tokio::test]
async fn conflict_blocks_apply_then_clean_rule_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let files = make(dir.path(), &["a.txt", "b.txt"]);

    // A rule mapping both files to the same name is a duplicate-target conflict.
    let clashing = BulkRenameRule {
        replace: "same.txt".to_string(),
        ..Default::default()
    };
    let preview = preview_rename(&files, &clashing).expect("preview");
    assert!(preview
        .iter()
        .all(|p| p.conflict == Some(RenameConflict::DuplicateTarget)));
    assert!(
        apply_rename(&files, &clashing).await.is_err(),
        "apply must refuse a conflicting batch"
    );
    // Nothing renamed.
    assert!(dir.path().join("a.txt").exists());

    // A numbered rule is conflict-free and applies.
    let numbered = BulkRenameRule {
        replace: "file_{n}.{ext}".to_string(),
        counter_width: 2,
        ..Default::default()
    };
    let preview = preview_rename(&files, &numbered).expect("preview");
    assert!(preview.iter().all(|p| p.conflict.is_none()));
    let results = apply_rename(&files, &numbered).await.expect("apply");
    assert_eq!(results.len(), 2);
    assert!(dir.path().join("file_01.txt").exists());
    assert!(dir.path().join("file_02.txt").exists());
}
