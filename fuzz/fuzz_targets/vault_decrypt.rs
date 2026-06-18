//! Fuzz target: feed arbitrary bytes as an encrypted single-file container.
//!
//! Goal: the container parser + AEAD stream decryptor must never panic on
//! malformed input — only return `Result::Err`.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    orca_vault::fuzz::decrypt_container(data);
});
