# Contributing to Orca

Thanks for your interest in Orca. This project holds a strict quality bar: if
code is committed, it works. Please read this before opening a pull request.

## Build & Test

```bash
# Build the whole workspace
cargo build --workspace

# Run the application
cargo run -p orca-gui

# Run all tests (must pass)
cargo test --workspace

# Lint with zero-warnings policy (must pass)
cargo clippy --workspace -- -D warnings

# Format (must produce no changes)
cargo fmt --all

# Security checks
cargo audit
cargo deny check
```

A pull request is only mergeable when `cargo test --workspace`,
`cargo clippy --workspace -- -D warnings`, and `cargo fmt --all --check` all
pass. These are enforced in CI.

## Code Style

- **No `unwrap()` / `expect()` in library crates** (`orca-core`, `orca-vault`,
  `orca-plugin`). Use `?` with proper error types. In `orca-gui` they are
  allowed only where a panic is genuinely unrecoverable.
- **No `todo!()`, `unimplemented!()`, or stub functions** in committed code.
- **No `unsafe`** without a `// SAFETY:` comment fully justifying soundness.
- Every public item has a `///` doc comment; every module has a `//!` comment.
- Every public function has at least one unit test; integration tests live in
  each crate's `tests/` directory.
- Logging uses `tracing` — never `println!` in library code.
- Errors use `thiserror` in libraries; `anyhow` for propagation in the binary.
- `#[allow(dead_code)]` is forbidden without an accompanying justification.

## Vault Code

`orca-vault` is security-critical. Every change must include a reasoning comment
explaining its security implications. Encryption keys and passphrases must never
be logged, printed, or written to disk in plaintext.

## Plugins

The Lua plugin sandbox is a hard boundary. Plugins interact with Orca only
through the `orca.*` API and can never access `orca-vault` internals.

## Workflow

Work follows `TASKS.md` phase by phase. Do not start a new phase until every
checkbox in the current phase is complete and verified.
