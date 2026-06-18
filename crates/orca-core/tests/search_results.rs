//! Integration test: name and content search return the expected files.

use std::fs;

use orca_core::{search_by_content, search_by_name, CancelToken, FileEntry, SearchOptions};
use tokio::sync::mpsc::Receiver;

async fn collect(mut rx: Receiver<FileEntry>) -> Vec<String> {
    let mut names = Vec::new();
    while let Some(e) = rx.recv().await {
        names.push(e.name);
    }
    names.sort();
    names
}

#[tokio::test]
async fn name_and_content_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src")).expect("mkdir");
    fs::write(root.join("src/main.rs"), b"fn main() { println!(\"hi\"); }").expect("write");
    fs::write(root.join("src/lib.rs"), b"pub fn add() {}").expect("write");
    fs::write(root.join("README.md"), b"orca file manager").expect("write");

    // Glob name search for Rust files.
    let opts = SearchOptions {
        glob: true,
        ..Default::default()
    };
    let rx = search_by_name(root, "*.rs", &opts, CancelToken::new()).expect("name search");
    assert_eq!(collect(rx).await, vec!["lib.rs", "main.rs"]);

    // Content search for a token only in main.rs.
    let rx = search_by_content(
        root,
        "println",
        &SearchOptions::default(),
        CancelToken::new(),
    )
    .expect("content search");
    assert_eq!(collect(rx).await, vec!["main.rs"]);
}
