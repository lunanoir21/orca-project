# Orca

[![CI](https://github.com/lunanoir21/orca/actions/workflows/ci.yml/badge.svg)](https://github.com/lunanoir21/orca/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](Cargo.toml)

> A full-featured, security-focused Linux file manager written in Rust.
> Inspired by KDE Dolphin. Built from scratch. Wayland-first.

Orca is a Linux file manager with an integrated encrypted vault system, deep
visual customization, and a sandboxed Lua plugin API.

<!-- TODO: screenshot — drop a real capture at docs/screenshots/home.png and
     docs/screenshots/files.png and reference them here. Not included yet so
     this README doesn't show a stale or staged shot. -->

---

## Features

- **Dual-pane browser** with tabs, breadcrumb navigation, and configurable views
  (list / icon / detail).
- **Home page**: personalized greeting, quick actions (new folder/file, connect
  to server), user-directory cards, and a recently-used list.
- **Quick Look**: press Space on a selected file for a fullscreen preview
  overlay, without leaving the file list.
- **Complete file operations**: copy, move, delete, rename, hard/symlinks, XDG
  trash with restore, owner/group (`chown`) and permission editing.
- **Encrypted vaults**: `age` encryption, Argon2id key derivation (64 MiB, 3 iter),
  auto-lock, multiple vaults, encrypted file index (filenames never stored in
  plaintext), live passphrase strength feedback and inline retry on a wrong
  passphrase (no extra popup).
- **Search**: live filename search and ripgrep-backed content search with glob support.
- **Archives**: zip, tar.gz, tar.xz, tar.bz2 — compress, extract, and browse.
- **Checksums**: SHA-256, MD5, Blake3.
- **Bulk rename** with regex and live preview.
- **Git status** badges, **disk usage** visualizer, **mount manager** (UDisks2),
  sidebar drive capacity bars.
- **Network locations**: SFTP / FTP with keyring-backed credentials and host key verification.
- **Embedded terminal** (`vte`) synced to the active directory.
- **Customization**: TOML config, rebindable keys, 10 built-in color schemes
  plus a high-contrast option, adjustable panel transparency, background image
  with scrim control, live CSS override.
- **Lua plugins**: sandboxed `orca.*` API (64 MiB heap limit, per-hook
  timeout), enable/disable/reload from the Plugin Manager UI, built-in
  git-status, archive, and **Secure Open** (opens any file in a bubblewrap
  sandbox — no network, no view of the rest of your files) plugins.

## Security

Orca treats the vault as security-critical. All cryptography comes from audited
crates (no custom crypto); key material is zeroized after use and never logged.
The plugin sandbox is a hard boundary — plugins cannot reach vault internals.
SFTP host keys are verified against `~/.ssh/known_hosts` before credentials are sent.
See [SECURITY.md](SECURITY.md) for the full threat model and encryption design.

## Install

### From Source

**Requirements:** Rust 1.85+, and (Debian/Ubuntu package names) `libgtk-4-dev
libadwaita-1-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev libssh2-1-dev
libgit2-dev` — see [.github/workflows/ci.yml](.github/workflows/ci.yml) for
the exact list this is built and tested against.

```bash
# Clone
git clone https://github.com/lunanoir21/orca
cd orca

# Build
cargo build --release

# Run
./target/release/orca-gui
```

### Arch Linux (AUR)

A [PKGBUILD](packaging/PKGBUILD) ships in this repo, but the package is not
yet published to the AUR — build it locally for now:

```bash
cd packaging && makepkg -si
```

### Flatpak

An [AppStream manifest](packaging/io.github.lunanoir21.orca.yml) ships in
this repo, but the app is not yet published on Flathub — build it locally
with `flatpak-builder` for now.

## Configuration

Configuration lives at `$XDG_CONFIG_HOME/orca/config.toml` (typically
`~/.config/orca/config.toml`). Orca writes the file with defaults on first run;
[config/default.toml](config/default.toml) documents every option (it's a
reference, not a file Orca reads).

```toml
[general]
show_hidden = false
single_click_open = false
confirm_delete = true

[appearance]
scheme = "graphite"      # see Settings → Appearance → Theme for the full list
font = "JetBrains Mono 11"
icon_size = 32
panel_opacity = 1.0       # 0.3 (very see-through) – 1.0 (as designed)

[keybinds]
new_tab       = "Ctrl+T"
close_tab     = "Ctrl+W"
toggle_hidden = "Ctrl+H"
open_terminal = "F4"
quick_look    = "Space"
```

## Plugins

Place `.lua` files in `$XDG_DATA_HOME/orca/plugins/` (typically
`~/.local/share/orca/plugins/`). Plugins are loaded on startup. Built-in
plugins ship in the `plugins/` directory:

- **git-status.lua** — badges git file states in tracked repositories.
- **archive.lua** — adds "Extract Here" and compress context menu items.
- **secure-open.lua** — adds a "Secure Open" context item that opens the
  selected file through `bubblewrap` (read-only filesystem, no network).
  Requires `bwrap` to be installed; notifies once and does nothing if it isn't.

Manage enabled/disabled state and hot-reload from the Plugin Manager
(sidebar → Eklentiler / Plugins). See [docs/plugins.md](docs/plugins.md) for
the full plugin API reference, including `orca.exec` vs. `orca.exec_argv` —
the latter avoids shell injection for any argument that isn't a fixed literal.

## Themes

Settings → Appearance → Theme picks from 10 built-in color schemes (incl. a
high-contrast one) defined in `crate::theme::SCHEMES` — these aren't files on
disk, they're generated at runtime, with live accent/background/opacity
controls alongside. The `themes/` directory at the repo root holds reference
`.css` starting points; copy one to `$XDG_DATA_HOME/orca/themes/active.css`
to layer a hand-written override on top of the generated colors. See
[docs/themes.md](docs/themes.md) for the full picture.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions and code style.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
