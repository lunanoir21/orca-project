# Orca — Claude Code Project Guide

> A full-featured, security-focused Linux file manager written in Rust.  
> Inspired by KDE Dolphin. Built from scratch. No compromises.

---

## Project Identity

**Name:** Orca  
**Language:** Rust (stable)  
**GUI:** GTK4 + relm4  
**Target:** Linux, Wayland-first (Hyprland tested)  
**License:** GPL-3.0  

Orca is a full-featured Linux file manager with an integrated encrypted vault system, deep customization, and a Lua plugin API. Every feature is production-grade. There are no placeholder implementations, no half-finished modules, no deferred quality.

**No shortcuts. No `todo!()`. No excuses.**

---

## Architecture

Orca is a Cargo workspace with four crates:

```
orca/
├── Cargo.toml                  ← workspace root
├── CLAUDE.md                   ← this file
├── TASKS.md                    ← phase-by-phase task list
├── crates/
│   ├── orca-core/              ← filesystem engine
│   ├── orca-vault/             ← encryption engine
│   ├── orca-gui/               ← GTK4/relm4 UI
│   └── orca-plugin/            ← Lua plugin system
├── plugins/                    ← built-in Lua plugins
├── themes/                     ← CSS theme files
├── config/
│   └── default.toml            ← shipped default config
├── fuzz/                       ← fuzzing targets (orca-vault)
└── tests/                      ← workspace integration tests
```

---

## Crate Responsibilities

### `orca-core` — Filesystem Engine
- Directory listing, metadata, stat
- File operations: copy, move, delete, rename, hard link, symlink
- Trash support (XDG Trash spec)
- File watching (`notify` crate)
- Hidden file toggle
- Sorting & filtering (name, size, date, type, extension)
- Recent files (XDG recent documents)
- Filename search + content search (ripgrep integration)
- Mount detection & management (UDisks2 via D-Bus)
- Checksum generation: SHA256, MD5, Blake3
- Bulk rename engine with regex and preview
- Archive operations: zip, tar.gz, tar.xz, tar.bz2 — compress & extract
- Git status reader (libgit2)
- Disk usage calculator (recursive)
- SFTP/FTP client (`ssh2`)
- File permissions reader & writer

### `orca-vault` — Encryption Engine
- `age` encryption integration
- Vault directory creation and structure
- Passphrase-based key derivation (Argon2id)
- Single file encrypt / decrypt
- Vault lock / unlock lifecycle
- Auto-lock timer (configurable per vault)
- Multiple vault support
- Vault backup / encrypted export
- Vault integrity verification (HMAC)
- Encrypted vault index (filenames never stored in plaintext)

### `orca-gui` — GTK4/relm4 UI
- Application shell and window management
- Dual pane file browser
- Tab system (per pane)
- Breadcrumb navigation bar
- Places & Bookmarks sidebar
- Vault panel (sidebar section)
- Preview panel (text, image, PDF, media)
- Thumbnail engine (image, video via gstreamer)
- Right-click context menu (extensible)
- Search bar with live results
- Embedded terminal panel (`vte`)
- Mount manager dialog
- Vault unlock dialog & auto-lock indicator
- Multiple vault switcher
- Vault backup dialog
- Settings dialog (all TOML options exposed)
- Keybind handler (action map)
- CSS theme loader & switcher
- Disk usage visualizer (treemap or bar)
- Git status badges on file items
- Bulk rename dialog with regex preview
- File permissions editor dialog
- Drag & drop (xdg-portal, Wayland native)
- Status bar (item count, selection size, vault status)
- Toolbar with configurable actions

### `orca-plugin` — Lua Plugin System
- `mlua` integration (Lua 5.4)
- Plugin loader with sandboxed environment
- Plugin lifecycle: load, enable, disable, unload
- Plugin API (`orca.*` namespace — see Plugin API section)
- Plugin manager UI (list, enable/disable, reload)
- Built-in plugins: `git-status.lua`, `archive.lua`

---

## Plugin API Reference

Plugins run in a sandboxed Lua environment. They cannot access vault internals, system calls, or the network unless explicitly whitelisted.

```lua
-- Event hooks
orca.on_file_select(function(file) end)
orca.on_dir_change(function(path) end)
orca.on_vault_lock(function(vault_name) end)

-- UI extensions
orca.register_action("action_id", "Display Name", function(files) end)
orca.set_badge(path, text, color)       -- badge on file item
orca.add_context_item("Label", function(files) end)

-- Utilities
orca.exec(cmd)                          -- run shell command, returns stdout
orca.notify(title, body)               -- desktop notification
orca.open(path)                         -- open with default app
orca.log(msg)                           -- plugin log (not vault-visible)
```

---

## Tech Stack

| Purpose           | Crate(s)                        |
|-------------------|---------------------------------|
| GUI framework     | `relm4`, `gtk4`                 |
| Encryption        | `age`                           |
| Key derivation    | `argon2`                        |
| Plugin system     | `mlua` (Lua 5.4)                |
| Async runtime     | `tokio`                         |
| Filesystem watch  | `notify`                        |
| Config            | `toml`, `serde`, `serde_derive` |
| Checksums         | `sha2`, `md-5`, `blake3`        |
| Regex             | `regex`                         |
| Archive           | `zip`, `tar`, `flate2`, `bzip2` |
| Terminal embed    | `vte` (gtk4 binding)            |
| SFTP              | `ssh2`                          |
| Git status        | `git2`                          |
| Error handling    | `thiserror`, `anyhow`           |
| Logging           | `tracing`, `tracing-subscriber` |
| Security audit    | `cargo-audit`, `cargo-deny`     |
| Fuzzing           | `cargo-fuzz`                    |
| D-Bus (mounts)    | `zbus`                          |

---

## Build & Test Commands

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run application
cargo run -p orca-gui

# Run all tests
cargo test --workspace

# Lint (zero warnings policy)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Security vulnerability audit
cargo audit

# Dependency policy check
cargo deny check

# Fuzz vault decryption
cargo +nightly fuzz run vault_decrypt

# Check for unused dependencies
cargo machete

# Generate docs
cargo doc --workspace --no-deps --open
```

---

## Directory & File Conventions

```
crates/orca-core/
├── src/
│   ├── lib.rs
│   ├── fs/           ← filesystem ops
│   ├── search/       ← search engine
│   ├── archive/      ← archive ops
│   ├── mount/        ← mount manager
│   ├── checksum/     ← hash generation
│   ├── git/          ← git status
│   └── error.rs      ← crate error type
└── tests/
    └── integration/

crates/orca-vault/
├── src/
│   ├── lib.rs
│   ├── vault.rs      ← vault lifecycle
│   ├── crypto.rs     ← encryption primitives
│   ├── index.rs      ← encrypted file index
│   ├── autolock.rs   ← auto-lock timer
│   └── error.rs
├── fuzz/
│   └── targets/
└── tests/

crates/orca-gui/
├── src/
│   ├── main.rs
│   ├── app.rs        ← app model
│   ├── pane/         ← file browser pane
│   ├── sidebar/      ← places, bookmarks, vault
│   ├── preview/      ← preview panel
│   ├── dialogs/      ← all dialogs
│   ├── vault_ui/     ← vault-specific UI
│   ├── theme/        ← CSS loader
│   └── keybind/      ← keybind handler
└── resources/
    └── style.css

crates/orca-plugin/
├── src/
│   ├── lib.rs
│   ├── loader.rs     ← plugin loader
│   ├── sandbox.rs    ← Lua sandbox
│   ├── api.rs        ← orca.* API impl
│   └── error.rs
└── tests/
```

---

## Code Conventions

- **Zero `unwrap()` / `expect()` in library code** — always `?` with proper error types
- **No `unsafe` blocks** without a `// SAFETY:` comment that fully justifies soundness
- All public API items must have `///` doc comments
- Error types defined with `thiserror`; propagation with `anyhow` in binaries
- All async code uses `tokio`
- `tracing` for all logging — never `println!` in library code
- Module structure mirrors directory structure exactly
- Every public function has at least one unit test
- Integration tests in `tests/` directory, not inline
- `#[must_use]` on all fallible return types
- No dead code in committed branches — `#[allow(dead_code)]` is forbidden without comment

---

## Configuration Schema

All user config lives at `$XDG_CONFIG_HOME/orca/config.toml`.

```toml
[general]
show_hidden = false
single_click_open = false
confirm_delete = true

[appearance]
theme = "catppuccin-mocha"
font = "JetBrains Mono 11"
icon_size = 32

[keybinds]
new_tab       = "Ctrl+T"
close_tab     = "Ctrl+W"
toggle_hidden = "Ctrl+H"
vault_add     = "Ctrl+Shift+V"
open_terminal = "F4"

[[vaults]]
name              = "Personal"
path              = "~/.local/share/orca/vaults/personal"
auto_lock_minutes = 5

[plugins]
enabled = ["git-status", "archive"]
```

---

## Strict Rules

1. **TASKS.md is the source of truth.** Nothing gets built that isn't in TASKS.md. Nothing in TASKS.md gets skipped.
2. **One phase at a time.** Do not start Phase N+1 until every checkbox in Phase N is checked.
3. **No `todo!()`, `unimplemented!()`, or stub functions in committed code.** If it's committed, it works.
4. **Vault code is security-critical.** Every change to `orca-vault` must include a reasoning comment explaining the security implications.
5. **Encryption keys must never be logged, printed, or stored in plaintext** under any circumstance.
6. **Plugin sandbox is a hard boundary.** Plugins cannot call into `orca-vault` internals under any circumstances.
7. **Do not add a dependency without checking `cargo audit` output for it.**
8. **All new dependencies must be justified** in a comment in `Cargo.toml`.
9. **Zero clippy warnings.** `cargo clippy -- -D warnings` must pass before any commit.
10. **Wayland-first.** All GUI features must be tested on a Wayland compositor. X11 fallback is acceptable, X11-only is not.
11. **XDG compliance.** Config in `$XDG_CONFIG_HOME`, data in `$XDG_DATA_HOME`, cache in `$XDG_CACHE_HOME`.
12. **Failing tests are a hard blocker.** Never commit with a failing test.
13. **Security audit phase is mandatory** and must pass clean before any release tag.
