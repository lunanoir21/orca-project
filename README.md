# Orca

> A full-featured, security-focused Linux file manager written in Rust.
> Inspired by KDE Dolphin. Built from scratch. Wayland-first.

Orca is a production-grade Linux file manager with an integrated encrypted
vault system, deep customization, and a sandboxed Lua plugin API.

---

## Features

- **Dual-pane browser** with tabs, breadcrumb navigation, and configurable views
  (list / icon / detail).
- **Complete file operations**: copy, move, delete, rename, hard/symlinks, XDG
  trash with restore.
- **Encrypted vaults**: `age` encryption, Argon2id key derivation (64 MiB, 3 iter),
  auto-lock, multiple vaults, encrypted file index (filenames never stored in plaintext).
- **Search**: live filename search and ripgrep-backed content search with glob support.
- **Archives**: zip, tar.gz, tar.xz, tar.bz2 — compress, extract, and browse.
- **Checksums**: SHA-256, MD5, Blake3.
- **Bulk rename** with regex and live preview.
- **Git status** badges, **disk usage** visualizer, **mount manager** (UDisks2).
- **Network locations**: SFTP / FTP with keyring-backed credentials and host key verification.
- **Embedded terminal** (`vte`) synced to the active directory.
- **Customization**: TOML config, rebindable keys, CSS themes (live switch).
- **Lua plugins**: sandboxed `orca.*` API, hot-reload, built-in git-status and archive plugins.

## Security

Orca treats the vault as security-critical. All cryptography comes from audited
crates (no custom crypto); key material is zeroized after use and never logged.
The plugin sandbox is a hard boundary — plugins cannot reach vault internals.
SFTP host keys are verified against `~/.ssh/known_hosts` before credentials are sent.
See [SECURITY.md](SECURITY.md) for the full threat model and encryption design.

## Install

### From Source

**Requirements:** Rust 1.85+, GTK4 development libraries, libssh2.

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

```bash
yay -S orca-files
```

### Flatpak

```bash
flatpak install io.github.lunanoir21.orca
```

## Configuration

Configuration lives at `$XDG_CONFIG_HOME/orca/config.toml` (typically
`~/.config/orca/config.toml`). Orca writes the file with defaults on first run.

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
open_terminal = "F4"
```

## Plugins

Place `.lua` files in `$XDG_DATA_HOME/orca/plugins/` (typically
`~/.local/share/orca/plugins/`). Plugins are loaded on startup. Built-in
plugins ship in the `plugins/` directory:

- **git-status.lua** — badges git file states in tracked repositories.
- **archive.lua** — adds "Extract Here" and compress context menu items.

See [docs/plugins.md](docs/plugins.md) for the full plugin API reference.

## Themes

CSS themes live in `$XDG_DATA_HOME/orca/themes/` or the built-in `themes/`
directory. Drop any `.css` file there and switch live from Settings. Built-in
themes: `catppuccin-mocha`, `catppuccin-latte`, `gruvbox-dark`, `nord`, `dracula`.

See [docs/themes.md](docs/themes.md) for the CSS variable reference.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions and code style.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
