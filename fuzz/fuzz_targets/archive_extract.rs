//! Fuzz target: feed arbitrary bytes as an archive and attempt extraction.
//!
//! Goal: archive format detection + extraction (zip/tar/gz/xz/bz2) must never
//! panic on malformed input — only return `Result::Err`. A malformed archive
//! must not write outside the destination directory either.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Stage the fuzz input as a candidate archive in a throwaway temp dir.
    let dir = std::env::temp_dir().join(format!("orca-fuzz-arc-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let archive = dir.join("input.bin");
    if std::fs::write(&archive, data).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let dest = dir.join("out");
    let _ = std::fs::create_dir_all(&dest);

    // `extract` is async; drive it on a tiny current-thread runtime.
    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        let _ = rt.block_on(orca_core::extract(archive, dest, None));
    }

    let _ = std::fs::remove_dir_all(&dir);
});
