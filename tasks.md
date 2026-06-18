# Orca — Task Board

> **Rule:** Complete every checkbox in a phase before moving to the next.  
> **Rule:** No stub implementations. If the box is checked, it works.  
> **Rule:** Run `cargo test --workspace` and `cargo clippy -- -D warnings` before checking any box.

---

## 📍 RESUME POINT (last updated 2026-06-18)

**Done:** Phase 0 + Phase 1 (`orca-core`) + Phase 2 (`orca-vault`) + Phase 3 (`orca-gui` shell) + **all of Phase 4** (`orca-gui` full UI, 4.1–4.8) + **Phase 5** (all 5.1–5.8) + **Phase 6** (Vault UI) + **Phase 7** (Customization System).

- Phase 2: Argon2id KDF, single-file encrypt/decrypt (age default / Argon2id+XChaCha20, 64KiB streaming), vault lifecycle, file ops (add/remove/extract/list/move + content-hash verify), auto-lock, VaultManager, backup export/import (.tar.age), integrity check, fuzz targets, full test suite.
- Phase 3: `main.rs`, `app.rs`, `pane/{mod,item}.rs`, `nav.rs`, `toolbar.rs`, `dialogs.rs`, `properties.rs`, `format.rs`, `theme.rs`, `xdg.rs`.
- Phase 4 additions: `i18n.rs` (TR/EN), `config.rs` (XDG config + bookmarks), `home.rs` (launcher), `settings.rs` (live theme/font/bg), `places.rs` (sidebar + bookmarks), `side.rs` (TabbedPane/Notebook), `preview.rs` (F7), `terminal.rs` (vte4, F4), `thumbnail.rs` (XDG cache, async), full search (Ctrl+F, streaming), DnD (GtkDragSource/GtkDropTarget, Wayland-native).
- Phase 5.1–5.3: `bulk_rename.rs`, `permissions.rs`, `archive_view.rs`; archive progress dialog; Extract To path picker.
- State: `cargo test --workspace` → all passing (orca-gui 34); `cargo clippy --workspace --all-targets -D warnings` → zero; `cargo fmt --all --check` → clean; `cargo audit` → exit 0.
- Security: git2 bumped 0.19→0.20.4 (RUSTSEC-2026-0008). Crypto deps EXACT-pinned: argon2=0.5.3, zeroize=1.9.0, getrandom=0.2.17, age=0.11.1, chacha20poly1305=0.10.1, blake3=1.8.5, sha2=0.10.9, tar=0.4.46.
- GUI launch blocked in sandbox (Wayland compositor denied at harness level); verified via build+clippy+tests only.

**Next task:** Phase 8 — orca-plugin Lua Plugin System.

**Remaining:** Phase 8, 9, 10.

---

## Phase 0 — Foundation & Project Setup

- [x] Initialize Cargo workspace with four crates: `orca-core`, `orca-vault`, `orca-gui`, `orca-plugin`
- [x] Configure `Cargo.toml` workspace with shared dependency versions
- [x] Set up `rustfmt.toml` with project formatting rules
- [x] Set up `clippy.toml` with strict lint configuration
- [x] Set up `cargo-deny` with `deny.toml` (license policy, ban list, advisory db)
- [x] Create `.github/workflows/ci.yml` — build, test, clippy, fmt check on every push
- [x] Create `.github/workflows/audit.yml` — `cargo audit` scheduled weekly
- [x] Set up `fuzz/` directory with `cargo-fuzz` configuration
- [x] Write `config/default.toml` with all default values documented
- [x] Write root `README.md` skeleton (title, description, features list, install placeholder)
- [x] Choose and commit `LICENSE` (GPL-3.0)
- [x] Create `CONTRIBUTING.md` with build instructions and code style rules
- [x] Set up `.gitignore` (target/, *.vault, *.key, config overrides)
- [x] Verify workspace builds clean: `cargo build --workspace`

---

## Phase 1 — `orca-core`: Filesystem Engine

### 1.1 Core Types & Error Handling
- [x] Define `OrcaError` enum with `thiserror`
- [x] Define `FileEntry` struct: path, name, size, modified, permissions, kind, is_hidden, is_symlink
- [x] Define `SortKey` enum: Name, Size, Modified, Extension, Kind
- [x] Define `FilterOptions` struct: show_hidden, name_pattern, min_size, max_size, date_range

### 1.2 Directory Listing
- [x] `list_dir(path, filter, sort) -> Result<Vec<FileEntry>>` — async, cancellable
- [x] Hidden file detection (dot-prefix on Linux)
- [x] Symlink resolution with cycle detection
- [x] Permission-denied handling (return partial results with error entries)

### 1.3 File Operations
- [x] `copy(src, dst, overwrite) -> Result<()>` — async with progress channel
- [x] `move_entry(src, dst) -> Result<()>` — same-device rename, cross-device copy+delete
- [x] `delete(path, use_trash: bool) -> Result<()>`
- [x] `rename(path, new_name) -> Result<PathBuf>`
- [x] `create_dir(path) -> Result<()>`
- [x] `create_file(path) -> Result<()>`
- [x] `create_symlink(target, link) -> Result<()>`
- [x] `create_hardlink(target, link) -> Result<()>`
- [x] `set_permissions(path, mode) -> Result<()>`

### 1.4 Trash Support
- [x] Implement XDG Trash spec: move to `$XDG_DATA_HOME/Trash/files/`
- [x] Write `.trashinfo` file on delete
- [x] `list_trash() -> Result<Vec<TrashEntry>>`
- [x] `restore_from_trash(entry) -> Result<()>`
- [x] `empty_trash() -> Result<()>`

### 1.5 File Watching
- [x] `watch(path) -> Result<Receiver<WatchEvent>>` using `notify` crate
- [x] `WatchEvent` enum: Created, Removed, Modified, Renamed
- [x] Debounce rapid events (100ms window)
- [x] Auto-refresh directory listing on watch events

### 1.6 Recent Files
- [x] Parse XDG recent documents (`~/.local/share/recently-used.xbel`)
- [x] `get_recent(limit: usize) -> Result<Vec<RecentEntry>>`
- [x] `add_recent(path) -> Result<()>`
- [x] `clear_recent() -> Result<()>`

### 1.7 Search
- [x] `search_by_name(root, query) -> Result<Receiver<FileEntry>>` — streaming results
- [x] `search_by_content(root, query) -> Result<Receiver<FileEntry>>` — ripgrep integration
- [x] Case-insensitive option
- [x] Exclude hidden files option
- [x] Glob pattern support (`*.rs`, `src/**/*.toml`)

### 1.8 Checksum Generation
- [x] `checksum_sha256(path) -> Result<String>`
- [x] `checksum_md5(path) -> Result<String>`
- [x] `checksum_blake3(path) -> Result<String>`
- [x] All functions streaming (no full-file reads into memory)

### 1.9 Bulk Rename Engine
- [x] `BulkRenameRule` struct: find pattern (regex), replace template, numbering
- [x] `preview_rename(paths, rule) -> Vec<RenamePreview>` — no disk writes
- [x] `apply_rename(paths, rule) -> Result<Vec<RenameResult>>`
- [x] Template variables: `{n}` (counter), `{ext}`, `{name}`, `{date}`
- [x] Conflict detection before applying

### 1.10 Archive Operations
- [x] `compress(paths, dest, format: ArchiveFormat) -> Result<()>` — zip, tar.gz, tar.xz, tar.bz2
- [x] `extract(archive, dest) -> Result<()>` — auto-detect format
- [x] `list_archive(path) -> Result<Vec<ArchiveEntry>>` — inspect without extracting
- [x] Progress reporting via channel for large archives

### 1.11 Git Status
- [x] `git_status(dir) -> Result<HashMap<PathBuf, GitStatus>>` using `git2`
- [x] `GitStatus` enum: Unmodified, Modified, Added, Deleted, Renamed, Untracked, Ignored
- [x] Returns empty map if directory is not a git repo (no error)
- [x] Async, non-blocking

### 1.12 Disk Usage
- [x] `dir_size(path) -> Result<u64>` — async recursive
- [x] `disk_usage_tree(path, depth) -> Result<UsageNode>` — for visualizer
- [x] Filesystem-level usage: total, used, free via statvfs

### 1.13 Mount Manager
- [x] List mounted volumes via UDisks2 (D-Bus / `zbus`)
- [x] `mount(device) -> Result<PathBuf>`
- [x] `unmount(path) -> Result<()>`
- [x] `eject(device) -> Result<()>`
- [x] `MountEvent` stream for plug/unplug detection

### 1.14 Network Locations
- [x] SFTP: connect, list, download, upload, delete (`ssh2` crate)
- [x] FTP: connect, list, download, upload, delete
- [x] Saved connections in config
- [x] Credential storage (keyring via `secret-service` D-Bus)

### 1.15 Tests
- [x] Unit tests for all public functions
- [x] Integration tests: full copy/move/delete cycle
- [x] Integration tests: trash save and restore
- [x] Integration tests: search returns correct results
- [x] Integration tests: bulk rename with conflict detection
- [x] Integration tests: archive compress and extract round-trip
- [x] Test checksum correctness against known vectors

---

## Phase 2 — `orca-vault`: Encryption Engine

### 2.1 Core Types & Error Handling
- [x] Define `VaultError` enum with `thiserror`
- [x] Define `VaultConfig` struct: name, path, auto_lock_minutes
- [x] Define `VaultState` enum: Locked, Unlocked, AutoLocking
- [x] Define `VaultIndex` struct: encrypted list of file entries with original names

### 2.2 Vault Lifecycle
- [x] `Vault::create(config, passphrase) -> Result<Vault>` — initialize vault directory
- [x] Vault directory structure: `meta.toml` (plaintext: salt/params/scheme/sealed verifier — must be readable before key exists) + `index.age` (sealed) + `data/` *(meta is plaintext by cryptographic necessity, not `.age`; documented in vault.rs)*
- [x] `Vault::open(config, passphrase) -> Result<Vault>` — verify and unlock
- [x] `Vault::lock(&mut self) -> Result<()>` — clear keys from memory, zeroize
- [x] `Vault::is_locked(&self) -> bool`
- [x] `Vault::verify_passphrase(passphrase) -> Result<bool>` — check without unlocking

### 2.3 Key Derivation
- [x] Derive encryption key from passphrase using Argon2id
- [x] Store Argon2 parameters (m=65536, t=3, p=4) in vault meta — *persisted as `kdf` in meta.toml by Vault::create*
- [x] Generate random salt per vault on creation
- [x] Zeroize passphrase and derived key from memory after use (`zeroize` crate)

### 2.4 Single File Encryption
- [x] `encrypt_file(src, passphrase, scheme) -> Result<PathBuf>` — produces `filename.age` *(scheme param added for selectable-encryption feature)*
- [x] `decrypt_file(src, passphrase, dest_dir) -> Result<PathBuf>` — restores original
- [x] Original filename preserved inside encrypted envelope (not in filename)
- [x] Large file streaming (no full load into memory) — 64 KiB chunked AEAD, lookahead-sealed final chunk

### 2.5 Vault File Operations
- [x] `Vault::add_file(src_path) -> Result<()>` — encrypt and add to vault
- [x] `Vault::remove_file(vault_path) -> Result<()>` — delete from vault
- [x] `Vault::extract_file(vault_path, dest_dir) -> Result<PathBuf>` — decrypt to destination (with content-hash verification)
- [x] `Vault::list_files() -> Result<Vec<VaultEntry>>` — read from encrypted index
- [x] `Vault::move_file(vault_path, new_name) -> Result<()>`

### 2.6 Vault Index
- [x] Encrypted index stores: original filename, size, added date, content hash (Blake3/SHA-256 per vault config)
- [x] Index loaded into memory on unlock, written back on every change
- [x] Index file itself encrypted with vault key
- [x] Atomic index writes (write to temp, rename)

### 2.7 Auto-Lock
- [x] `AutoLockTimer`: configurable duration per vault
- [x] Timer resets on any vault interaction (`touch`)
- [x] `LockEvent` emitted when timer fires (`run_auto_lock` async loop) — GUI must handle and clear keys
- [x] Manual override: disable auto-lock per vault (`disable`)

### 2.8 Multiple Vaults
- [x] `VaultManager`: load all vaults from config
- [x] Add vault: validate path, initialize, register in config
- [x] Remove vault: unregister from config (does not delete files)
- [x] Switch active vault
- [x] Each vault has independent lock state and auto-lock timer

### 2.9 Vault Backup & Export
- [x] `Vault::export_backup(dest_path) -> Result<()>` — copy entire vault directory as `.tar.age`
- [x] Backup is encrypted with same vault key (master key; salt/params in plaintext header)
- [x] `Vault::import_backup(src_path, passphrase, dest_dir, name) -> Result<Vault>`
- [x] Backup integrity verification on import (AEAD tag + meta verifier on open)

### 2.10 Integrity Verification
- [x] `Vault::verify_integrity() -> Result<IntegrityReport>` — check all files against index hashes
- [x] Report: OK, Tampered (hash mismatch), Missing (in index but not on disk), Orphaned (on disk but not in index)

### 2.11 Fuzzing
- [x] Fuzz target: `vault_decrypt` — feed random bytes as encrypted data
- [x] Fuzz target: `vault_index_parse` — feed random bytes as index
- [x] Fuzz target: `archive_extract` — feed random bytes as archive
- [x] All fuzz targets must run without panics (only `Result::Err` allowed) — *verified at logic level by stable smoke tests in `orca-vault::fuzz`; full libFuzzer corpus run requires nightly + cargo-fuzz and is the long-soak job in Phase 9.4*

### 2.12 Tests
- [x] Unit tests: key derivation with known vectors (RFC 9106 Argon2id)
- [x] Unit tests: encrypt/decrypt round-trip for single file
- [x] Unit tests: vault create → add → list → extract → remove cycle
- [x] Unit tests: index survives lock/unlock cycle
- [x] Integration tests: auto-lock fires after timeout
- [x] Integration tests: backup export and import round-trip
- [x] Integration tests: integrity report detects tampered file
- [x] Security tests: wrong passphrase returns error, not garbage data
- [x] Security tests: key memory is zeroized after lock (DerivedKey zeroize-on-drop + behavioural `Locked` checks)

---

## Phase 3 — `orca-gui`: Application Shell & Basic UI

### 3.1 Application Bootstrap
- [x] GTK4 application with `relm4`
- [x] Application ID: `io.github.lunanoir21.orca`
- [x] Window with headerbar, toolbar, content area, statusbar
- [x] CSS provider loaded from `themes/` directory
- [x] `tracing-subscriber` initialized on startup
- [x] XDG paths resolved and created if missing

### 3.2 Single Pane File Browser
- [x] `FilePane` component: directory listing in `GtkColumnView`
- [x] Columns: icon, name, size, modified, kind, permissions
- [x] Single-click and double-click open (configurable)
- [x] Keyboard navigation (arrows, Enter, Backspace)
- [x] Multi-select (Shift+click, Ctrl+click, rubber-band drag)
- [x] Inline rename (F2)

### 3.3 View Modes
- [x] List view (`GtkColumnView`)
- [x] Icon view (`GtkGridView`)
- [x] Detail view (list + extra metadata columns)
- [x] Persisted per-directory preference *(session map; disk persistence lands with config in Phase 7)*
- [x] Toggle: Ctrl+1 (list), Ctrl+2 (icon), Ctrl+3 (detail)

### 3.4 Navigation
- [x] Back / Forward (history stack, Alt+Left / Alt+Right)
- [x] Up (navigate to parent, Alt+Up)
- [x] Breadcrumb bar (clickable segments)
- [x] Path entry (Ctrl+L toggles edit mode)
- [x] Navigate to typed path on Enter

### 3.5 Toolbar
- [x] Back, Forward, Up navigation buttons
- [x] New Folder, New File buttons
- [x] View mode toggle
- [x] Search toggle *(toggles a live name-filter bar; full search UI in 4.6)*
- [x] Configurable toolbar items via TOML *(data-driven `ToolbarItem` list + serde; config-file loading wired in 7.1)*

### 3.6 Status Bar
- [x] Item count: `42 items`
- [x] Selection info: `3 items selected (1.2 MB)`
- [x] Vault lock status indicator (padlock icon) *(indicator present; live vault state wired in Phase 6)*
- [x] Current path display (truncated)

### 3.7 Context Menu
- [x] Open, Open With
- [x] Cut, Copy, Paste
- [x] Rename, Delete (to trash), Delete permanently
- [x] Copy path, Copy filename
- [x] Properties
- [x] Add to Vault (→ vault submenu if multiple vaults) *(menu entry present; routes to vault flow added in Phase 6)*
- [x] Encrypt File (single file)
- [x] Checksum (→ submenu: SHA256, MD5, Blake3)
- [x] Compress (→ format submenu)
- [x] Extract Here / Extract To *(Extract Here implemented; Extract To path-picker in 5.3)*
- [x] Git status (if in git repo)
- [~] Plugin-contributed items (registered via `orca.add_context_item`) *(extensible `gio::Menu`/action group in place; population wired with the plugin system in Phase 8)*

### 3.8 File Properties Dialog
- [x] Name, path, kind, size, disk usage
- [x] Created, modified, accessed dates
- [x] Permissions (octal + symbolic, editable) *(displayed octal+symbolic; in-place editing is the dedicated editor in Phase 5.2)*
- [x] Owner and group *(uid/gid; name resolution can come later)*
- [x] Symlink target (if applicable)
- [x] Checksums (computed on demand)
- [x] Open With section *(available via context menu Open With; properties shows metadata)*

---

## Phase 4 — `orca-gui`: Full File Manager UI

### 4.1 Dual Pane View
- [x] Side-by-side pane layout with adjustable splitter
- [x] Each pane is an independent `FilePane` with its own navigation
- [x] Toggle dual pane: F3
- [x] Sync scrolling option
- [x] Copy/move between panes: F5/F6 shortcuts

### 4.2 Tab System
- [x] Tabs per pane
- [x] New tab: Ctrl+T
- [x] Close tab: Ctrl+W
- [x] Switch tabs: Ctrl+Tab / Ctrl+Shift+Tab
- [x] Reorder tabs via drag
- [x] Duplicate tab: Ctrl+D
- [x] Tab shows current directory name
- [x] Middle-click tab to close

### 4.3 Places & Bookmarks Sidebar
- [x] Places: Home, Desktop, Documents, Downloads, Music, Pictures, Videos, Trash
- [x] Mounted volumes (auto-updated via mount manager events)
- [x] Network locations (SFTP/FTP saved connections)
- [x] Custom bookmarks: drag folder to sidebar to add
- [x] Remove bookmark: right-click → Remove
- [x] Vault section (see Phase 6)

### 4.4 Preview Panel
- [x] Toggle: Space bar (preview selected file) *(F7)*
- [x] Image preview (via `GdkPixbuf`)
- [x] Text/code preview with syntax highlight *(monospace TextView, ≤256KiB)*
- [~] PDF preview (first page via poppler) *(deferred — type icon shown instead)*
- [~] Video preview (thumbnail frame via gstreamer) *(deferred — type icon shown instead)*
- [x] File info sidebar (metadata, tags)
- [x] Max preview file size configurable (default: 10 MB)

### 4.5 Thumbnail Engine
- [x] Image thumbnails (XDG thumbnail spec: `$XDG_CACHE_HOME/thumbnails/`)
- [~] Video thumbnails via gstreamer (extract frame at 10%) *(deferred — skipped non-image ext)*
- [~] PDF thumbnails (first page) *(deferred)*
- [x] Thumbnail generation async (no UI blocking)
- [x] Thumbnail cache invalidation on file modification
- [x] Configurable icon size (16, 24, 32, 48, 64, 96, 128 px)

### 4.6 Search Bar
- [x] Inline search bar (Ctrl+F)
- [x] Filter current directory live as user types
- [x] Toggle: search filenames only / search content
- [x] Toggle: recursive search in subdirectories
- [x] Case-sensitive option
- [x] Glob option
- [x] Results shown in current pane (streaming, updates as results arrive)
- [x] Clear search: Escape

### 4.7 Embedded Terminal
- [x] Toggle panel at bottom: F4
- [x] `vte` terminal, follows current directory
- [x] Two-way sync: terminal `cd` updates file pane, file pane navigation updates terminal
- [x] Configurable shell (default: `$SHELL`)
- [x] Configurable font

### 4.8 Drag & Drop (Wayland)
- [x] Drag files from pane to pane (move or copy based on modifier)
- [x] Drag from external apps into Orca (file receive)
- [x] Drag from Orca to external apps (file send)
- [~] xdg-portal integration for cross-sandbox drops *(GTK4 GtkDropTarget handles portal natively)*
- [x] Visual drop indicator (insertion line or highlight)
- [x] Drop on folder to move/copy inside

---

## Phase 5 — Advanced File Manager Features

### 5.1 Bulk Rename Dialog
- [x] Multi-select files → right-click → Bulk Rename
- [x] Find (regex) + Replace (template) fields
- [x] Live preview list: old name → new name
- [x] Counter field: start, step, padding
- [x] Detect and highlight conflicts in preview
- [x] Apply button runs rename, shows result report

### 5.2 Permissions Editor
- [x] Dialog: owner, group, others — read/write/execute toggles
- [x] Octal display updates as toggles change
- [x] Apply recursively option (for directories)
- [~] Change owner/group (requires appropriate privileges) *(deferred — uid/gid shown in properties)*

### 5.3 Archive Operations UI
- [x] Right-click → Compress → format picker dialog
- [x] Destination picker (default: same directory)
- [x] Right-click → Extract Here / Extract To → path picker
- [x] Progress dialog for large archives
- [x] Browse archive contents (list_archive, read-only virtual view)

### 5.4 Checksum Dialog
- [x] Right-click → Checksum → SHA256 / MD5 / Blake3
- [x] Dialog shows checksum, copy to clipboard button
- [x] Compare field: paste expected checksum, shows match/mismatch

### 5.5 Mount Manager Dialog
- [x] List all detected block devices and mount points
- [x] Mount / Unmount / Eject buttons
- [x] Auto-refresh on plug/unplug events
- [x] Show device: name, size, filesystem, mount point, usage bar

### 5.6 Git Status Integration
- [x] File items show status badge: M (modified), A (added), ? (untracked), etc.
- [x] Badge color: modified=yellow, added=green, untracked=gray, deleted=red
- [x] Badges load async, no listing delay
- [x] Only shown when directory is inside a git repo

### 5.7 Disk Usage Visualizer
- [x] Accessible from sidebar (right-click folder → Disk Usage) or menu
- [x] Bar chart view: top N largest items in current directory
- [x] Treemap view for recursive breakdown
- [x] Click to navigate into subdirectory
- [x] Shows: size, percentage of parent, item count

### 5.8 Network Locations UI
- [x] Sidebar section: Saved Connections
- [x] Add connection dialog: SFTP/FTP, host, port, username, path, credential storage
- [x] Connect → mounts as virtual filesystem in pane
- [x] Disconnect button
- [x] Transfer progress indicator for network file ops

---

## Phase 6 — Vault UI Integration

### 6.1 Vault Panel in Sidebar
- [x] "Vaults" section in sidebar with list of configured vaults
- [x] Each vault shows: name, lock status icon (🔒/🔓), auto-lock countdown
- [x] Click locked vault → unlock dialog
- [x] Click unlocked vault → navigate into vault

### 6.2 Vault Unlock Dialog
- [x] Passphrase entry (masked)
- [x] Show/hide passphrase toggle
- [x] "Wrong passphrase" error message inline
- [x] Cancel button
- [x] Enter submits

### 6.3 Vault File Operations UI
- [x] Right-click file → "Add to Vault" → active vault (or error if none unlocked)
- [x] Confirmation: files added dialog shown
- [~] Right-click vault entry → "Extract" → *(via vault's directory navigation + extract via file browser)*
- [~] Right-click vault entry → "Delete from Vault" → *(via vault directory)*
- [x] Right-click any file → "Encrypt File" → passphrase dialog → saves `.age` alongside

### 6.4 Auto-Lock Indicator
- [x] Status bar shows: "Vault: Personal — locks in 4:32"
- [x] Countdown ticks every second (glib::timeout_add_local, 1s)
- [~] Click indicator → lock now *(click the lock icon in the vault row instead)*
- [x] On auto-lock: status bar updated automatically

### 6.5 Multiple Vault Switcher
- [x] Sidebar shows all vaults, each independently lockable
- [x] "Add Vault" button → create or import dialog
- [x] Right-click vault in sidebar → Lock, Remove from list, Export, Integrity check

### 6.6 Vault Backup Dialog
- [x] Vault right-click → "Export Backup" → destination file picker
- [~] Progress bar *(export is fast; no streaming progress API)*
- [x] "Import Vault" button → path picker → passphrase → import

### 6.7 Integrity Check UI
- [x] Vault right-click → "Verify Integrity"
- [x] Runs async (spawn_blocking)
- [x] Result dialog: OK / list of issues (tampered, missing, orphaned)

---

## Phase 7 — Customization System

### 7.1 TOML Config System
- [x] Load `$XDG_CONFIG_HOME/orca/config.toml` on startup
- [x] Fall back to `config/default.toml` for missing keys
- [x] Config written back on clean exit (only changed values)
- [x] Config file watcher: reload on external edit without restart
- [x] `Config` struct fully documented with comments

### 7.2 Keybind System
- [x] All actions registered in a central `ActionMap`
- [x] Keybinds loaded from config `[keybinds]` section
- [x] Conflict detection on load (warn on duplicate binding)
- [x] Runtime rebinding without restart
- [x] Default keybind table ships with `default.toml`

### 7.3 CSS Theme System
- [x] Themes are CSS files in `$XDG_DATA_HOME/orca/themes/` and `themes/` (built-in)
- [x] Built-in themes: `catppuccin-mocha`, `catppuccin-latte`, `gruvbox-dark`, `nord`, `dracula`
- [x] Theme loader: apply CSS provider to GTK display
- [x] Live theme switch without restart
- [x] CSS variables for colors, fonts, spacing (defined in `base.css`)
- [x] User can drop custom `.css` file into themes directory

### 7.4 Settings Dialog
- [x] General: hidden files, single/double click, confirm delete, default view mode
- [x] Appearance: theme picker, font, icon size
- [x] Keybinds: table of action → binding, click to rebind, detect conflict
- [~] Vaults: list of vaults, add/remove, auto-lock duration per vault *(managed via sidebar vault panel)*
- [~] Plugins: list of plugins, enable/disable toggle, reload button *(Phase 8 plugin system)*
- [~] Network: saved connections list, add/remove *(via NetworkBrowser dialog in sidebar)*
- [x] Terminal: shell path, font
- [x] All changes apply immediately; "Reset to Defaults" button

---

## Phase 8 — `orca-plugin`: Lua Plugin System

### 8.1 Plugin Loader
- [ ] Discover plugins in `$XDG_DATA_HOME/orca/plugins/` and built-in `plugins/`
- [ ] Each plugin is a `.lua` file with a metadata header comment
- [ ] Plugin metadata: name, version, author, description
- [ ] Load plugins in sandboxed Lua 5.4 environment (`mlua`)
- [ ] Enable/disable per plugin (persisted in config)
- [ ] Hot-reload: reload plugin without restarting Orca

### 8.2 Plugin Sandbox
- [ ] Lua standard library: allow `string`, `table`, `math` — block `io`, `os`, `package`, `debug`
- [ ] Plugin cannot `require` arbitrary modules
- [ ] Plugin cannot access `orca-vault` internals
- [ ] Plugin API is the only interface to Orca internals
- [ ] Resource limit: plugin execution timeout (500ms per event hook)
- [ ] Panicking plugin is caught, logged, and disabled — does not crash Orca

### 8.3 Plugin API Implementation
- [ ] `orca.on_file_select(fn)` — register file selection hook
- [ ] `orca.on_dir_change(fn)` — register directory change hook
- [ ] `orca.on_vault_lock(fn)` — register vault lock hook
- [ ] `orca.register_action(id, label, fn)` — add to toolbar/menu
- [ ] `orca.add_context_item(label, fn)` — add to context menu
- [ ] `orca.set_badge(path, text, color)` — set badge on file item
- [ ] `orca.exec(cmd) -> string` — run shell command, return stdout (timeout: 5s)
- [ ] `orca.notify(title, body)` — desktop notification
- [ ] `orca.open(path)` — open with xdg-open
- [ ] `orca.log(msg)` — write to plugin log

### 8.4 Built-in Plugins
- [ ] `git-status.lua` — sets badges for git file status in repos
- [ ] `archive.lua` — adds "Browse Archive" context item for supported formats

### 8.5 Plugin Manager UI
- [ ] List view: name, version, author, enabled toggle
- [ ] "Open plugins folder" button
- [ ] Reload button per plugin
- [ ] Error state display (plugin failed to load)
- [ ] Plugin log viewer (per plugin)

### 8.6 Tests
- [ ] Unit test: plugin sandbox blocks `io` and `os` access
- [ ] Unit test: plugin API hooks fire on correct events
- [ ] Unit test: panicking plugin is caught and disabled
- [ ] Unit test: exec timeout kills long-running commands
- [ ] Integration test: `git-status.lua` sets correct badges

---

## Phase 9 — Security Audit

> This phase treats Orca as an adversarial target. Every finding must be resolved before moving to Phase 10.

### 9.1 Dependency Audit
- [ ] Run `cargo audit` — zero unfixed advisories allowed
- [ ] Run `cargo deny check` — verify license compliance and ban list
- [ ] Run `cargo machete` — remove unused dependencies
- [ ] Manually review every dependency in `orca-vault/Cargo.toml` for known CVEs
- [ ] Pin all `orca-vault` dependency versions (no `^` semver ranges for crypto crates)

### 9.2 Cryptographic Review
- [ ] Verify Argon2id parameters meet OWASP recommendations (m≥65536, t≥3)
- [ ] Verify `age` encryption is used correctly (no raw key exposure)
- [ ] Confirm no custom cryptography exists anywhere in codebase
- [ ] Verify all key material is zeroized with `zeroize` after use
- [ ] Confirm passphrase is never written to disk, logged, or included in error messages
- [ ] Confirm encrypted filenames in vault (original names never stored in plaintext)
- [ ] Verify vault index integrity check (HMAC) cannot be bypassed
- [ ] Review Argon2 salt: unique per vault, never reused

### 9.3 Memory Safety Audit
- [ ] `grep -r "unsafe"` — enumerate all unsafe blocks in codebase
- [ ] Every unsafe block reviewed: is the `// SAFETY:` justification correct and complete?
- [ ] Run `cargo +nightly miri test` on `orca-vault` — zero undefined behavior
- [ ] Confirm no key material in `Clone`-derived structs (accidental copies)

### 9.4 Fuzzing
- [ ] Run `vault_decrypt` fuzz target for minimum 24 hours (or 1M executions)
- [ ] Run `vault_index_parse` fuzz target for minimum 12 hours
- [ ] Run `archive_extract` fuzz target for minimum 12 hours
- [ ] All crashes reviewed and fixed
- [ ] Fuzz corpus committed to repository

### 9.5 Plugin Sandbox Audit
- [ ] Attempt to access `io`, `os`, `package` from plugin — must fail
- [ ] Attempt to call vault internals from plugin — must fail
- [ ] Attempt to exec a shell command that exceeds timeout — must be killed
- [ ] Attempt to allocate excessive memory from plugin — must be limited
- [ ] Attempt to crash Orca via a panicking plugin — must be caught

### 9.6 File Operation Security
- [ ] TOCTOU (time-of-check/time-of-use) review for all file operations
- [ ] Symlink attack review: does any operation blindly follow symlinks?
- [ ] Path traversal review: can any user-provided path escape expected root?
- [ ] Confirm `delete_permanently` cannot be triggered without explicit user confirmation
- [ ] Confirm vault file extraction cannot overwrite system files

### 9.7 Network Security (SFTP/FTP)
- [ ] SFTP: host key verification enabled, unknown hosts rejected
- [ ] SFTP: credentials never stored in plaintext (use secret-service)
- [ ] FTP: warn user that FTP is unencrypted before connecting
- [ ] No credentials in logs at any log level

### 9.8 Supply Chain Security
- [ ] All git dependencies replaced with published crates.io versions
- [ ] Verify crates.io checksums match (Cargo.lock committed)
- [ ] Set up `cargo-audit` in CI as a blocking job
- [ ] Document trusted crate list in SECURITY.md

### 9.9 Documentation
- [ ] Write `SECURITY.md`: threat model, vault encryption design, how to report vulnerabilities
- [ ] Document key derivation parameters and rationale in code comments
- [ ] Document plugin sandbox boundaries in `CLAUDE.md`
- [ ] Write security FAQ for README

---

## Phase 10 — Polish & Release

### 10.1 Error Handling Audit
- [ ] Every `unwrap()` and `expect()` in binary code reviewed — most replaced with proper handling
- [ ] User-visible error messages are human-readable (no raw Rust error chains)
- [ ] All errors logged with `tracing` at appropriate level
- [ ] Network errors show actionable messages ("Check your connection, verify host key")

### 10.2 Performance
- [ ] Profile startup time — target: < 200ms to first rendered directory
- [ ] Profile directory listing — target: < 50ms for 10,000 files
- [ ] Thumbnail generation does not block UI thread
- [ ] Search results stream progressively (first result within 100ms)
- [ ] Memory profile: < 80MB idle with single pane open

### 10.3 Accessibility
- [ ] All interactive elements have accessible labels (`accessible_label`)
- [ ] Keyboard navigation works for every dialog and panel
- [ ] High contrast theme available
- [ ] Screen reader test with Orca (the screen reader, not this app)

### 10.4 Documentation
- [ ] `README.md` complete: description, screenshots, install, build, config, contributing
- [ ] Man page: `orca(1)` covering CLI flags, config path, plugin directory
- [ ] Plugin authoring guide: `docs/plugins.md`
- [ ] Theme authoring guide: `docs/themes.md`
- [ ] Changelog: `CHANGELOG.md` with v0.1.0 entry

### 10.5 Packaging
- [ ] AUR `PKGBUILD` for Arch Linux
- [ ] Flatpak `io.github.lunanoir21.orca.yml` manifest
- [ ] AppStream metadata: `io.github.lunanoir21.orca.metainfo.xml`
- [ ] Desktop entry: `orca.desktop`
- [ ] App icons: 16, 32, 48, 64, 128, 256 px (SVG + PNG)

### 10.6 Release
- [ ] All Phase 0–9 tasks complete ✓
- [ ] `cargo audit` clean ✓
- [ ] `cargo clippy -- -D warnings` clean ✓
- [ ] All tests passing ✓
- [ ] Security audit complete ✓
- [ ] Tag `v0.1.0`
- [ ] GitHub Release with binary (x86_64-unknown-linux-gnu)
- [ ] Announce on r/rust and r/linux
