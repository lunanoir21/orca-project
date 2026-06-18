# Orca Fuzzing

This directory holds Orca's [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz)
harness. It is a standalone package detached from the workspace so the nightly-
only libFuzzer targets never break the stable workspace build.

## Targets (added in Phase 2.11)

- `vault_decrypt` — feed random bytes as encrypted vault data; must only ever
  return `Err`, never panic.
- `vault_index_parse` — feed random bytes as a vault index.
- `archive_extract` — feed random bytes as an archive.

## Running

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run vault_decrypt
```

Corpora live under `corpus/<target>/` and are committed to the repository per
the Phase 9 security-audit requirements.
