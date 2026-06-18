# Changelog

All notable changes to Orca are documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [0.1.0] — 2026-06-18

### Added

**Core (`orca-core`)**
- Directory listing with sorting (name, size, date, type, extension) and filtering (hidden, glob, size, date range).
- File operations: copy (with progress), move, delete, rename, hard link, symlink.
- XDG Trash support: move to trash, restore, empty trash (`.trashinfo` files written).
- File watching with `notify` (debounced, 100 ms window).
- XDG recent documents: parse, add, clear.
- Filename search (streaming) and content search via ripgrep integration (glob + case options).
- Checksum generation: SHA-256, MD5, Blake3 (streaming).
- Bulk rename engine: regex find/replace, counter templates, conflict detection, preview.
- Archive operations: zip, tar.gz, tar.xz, tar.bz2 — compress, extract, list without extracting.
- Git status reader via `git2` (async, returns empty map for non-repos).
- Disk usage: recursive size, usage tree for visualizer, statvfs for filesystem stats.
- Mount manager via UDisks2/D-Bus: list, mount, unmount, eject, plug/unplug events.
- SFTP client with host key verification against `~/.ssh/known_hosts`.
- FTP client (warns about cleartext before connecting).
- File permissions writer (`chmod`, `chown` via subprocess).

**Vault (`orca-vault`)**
- Vault lifecycle: create, open, lock, unlock, verify passphrase.
- Argon2id key derivation (m=65 536 KiB, t=3, p=4) with per-vault random salt.
- `age` file encryption (ChaCha20-Poly1305, 64 KiB streaming chunks).
- Vault file operations: add, remove, extract (with content-hash verification), list, move.
- Encrypted vault index: filenames never stored in plaintext.
- Auto-lock timer: configurable per vault, resets on interaction.
- Multiple vault support via `VaultManager`.
- Vault backup export/import (`.tar.age`, encrypted with vault key).
- Integrity verification: OK / Tampered / Missing / Orphaned reports.
- Fuzz targets: `vault_decrypt`, `vault_index_parse`, `archive_extract`.

**GUI (`orca-gui`)**
- Dual-pane file browser with tabs (Ctrl+T/W/Tab), breadcrumb nav, and path entry (Ctrl+L).
- Three view modes: list (Ctrl+1), icon (Ctrl+2), detail (Ctrl+3).
- Navigation: back/forward history, up, Alt+arrow keys.
- Full context menu: open, open with, cut/copy/paste, rename, trash, delete, checksum, compress, extract, git status, permissions, properties, bulk rename, disk usage, add to vault, encrypt file.
- Preview panel (F7): image, text/code, file metadata. Max 10 MB configurable.
- Thumbnail engine: XDG cache, async generation, cache invalidation.
- Inline search bar (Ctrl+F): live filter, recursive, content search, glob, case toggle.
- Embedded `vte` terminal (F4): bidirectional directory sync.
- Drag & drop: pane-to-pane, Orca-to-external, external-to-Orca (Wayland native).
- Places sidebar: XDG dirs, drives, bookmarks (drag to add, right-click to remove), network, vault panel.
- Vault panel: list vaults, lock/unlock, auto-lock countdown, add/import vault.
- Vault unlock dialog, vault backup dialog, integrity report dialog.
- Bulk rename dialog with live preview and conflict highlighting.
- Permissions editor: 9-bit checkboxes, octal display, recursive option.
- Checksum dialog with copy-to-clipboard and compare field.
- Mount manager dialog: list, mount, unmount, eject, auto-refresh.
- Disk usage visualizer: bar chart and treemap views.
- Network browser dialog: SFTP/FTP saved connections, connect, browse, transfer progress.
- Plugin manager dialog: list plugins, view state/log.
- Settings dialog: general, appearance, keybinds, terminal (all live-apply).
- CSS theme system: live switch, built-in Catppuccin Mocha/Latte, Gruvbox Dark, Nord, Dracula.
- TOML config with XDG paths, file watcher for external edits.
- Rebindable keybindings with conflict detection.
- i18n: Turkish and English.

**Plugin system (`orca-plugin`)**
- Lua 5.4 sandbox: blocks `io`, `os`, `package`, `debug`; overrides `require`.
- Plugin metadata header (`@name`, `@version`, `@author`, `@description`).
- `orca.*` API: event hooks (`on_file_select`, `on_dir_change`, `on_vault_lock`), `register_action`, `add_context_item`, `set_badge`, `exec` (5 s timeout), `notify`, `open`, `log`.
- Per-hook 500 ms timeout with Lua instruction-count enforcement.
- Panicking/erroring plugins caught and moved to error state — cannot crash Orca.
- Hot-reload without restart.
- Built-in plugins: `git-status.lua`, `archive.lua`.
- Plugin manager UI.

### Security

- No custom cryptography — all crypto from audited `age`, `argon2`, `chacha20poly1305`, `zeroize`.
- Encryption keys zeroized on drop and after lock.
- Passphrase never stored, logged, or included in error messages.
- Vault filenames never in plaintext.
- Plugin sandbox hard boundary: plugins cannot access vault internals.
- SFTP host key verification against `~/.ssh/known_hosts` (MITM guard).
- `cargo audit` passes (zero fixed advisories).

[0.1.0]: https://github.com/lunanoir21/orca/releases/tag/v0.1.0
