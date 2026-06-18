//! Fuzz target: feed arbitrary bytes as a sealed vault index blob.
//!
//! Goal: the blob opener and the index TOML parser must never panic on
//! malformed input — only return `Result::Err`.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    orca_vault::fuzz::parse_index(data);
});
