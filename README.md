# Orca

> A full-featured, security-focused Linux file manager written in Rust.
> BU PROJE TAMAMEN DEMO VE KESİNLİKTE TAM ENTEGRE BİR SİSTEM UYGULAMASI DEĞİLİDİR

> Inspired by KDE Dolphin. Built from scratch. Wayland-first.

Orca is a production-grade Linux file manager with an integrated encrypted
vault system, deep customization, and a sandboxed Lua plugin API.

---

## Features

- **Dual-pane browser** with tabs, breadcrumb navigation, and configurable views
  (list / icon / detail).
- **Complete file operations**: copy, move, delete, rename, hard/symlinks, XDG
  trash with restore.
- **Encrypted vaults**: `age` encryption, Argon2id key derivation, auto-lock,
  multiple vaults, encrypted file index (filenames never stored in plaintext).
- **Search**: live filename search and ripgrep-backed content search with glob
  support.
- **Archives**: zip, tar.gz, tar.xz, tar.bz2 — compress, extract, and browse.
- **Checksums**: SHA-256, MD5, Blake3.
- **Bulk rename** with regex and live preview.
- **Git status** badges, **disk usage** visualizer, **mount manager** (UDisks2).
- **Network locations**: SFTP / FTP with keyring-backed credentials.
- **Embedded terminal** (`vte`) synced to the active directory.
- **Customization**: TOML config, rebindable keys, CSS themes (live switch).
- **Lua plugins**: sandboxed `orca.*` API, hot-reload, built-in plugins.

## Security

Orca treats the vault as security-critical. All cryptography comes from audited
crates (no custom crypto); key material is zeroized after use and never logged.
The plugin sandbox is a hard boundary — plugins cannot reach vault internals.
See `SECURITY.md` (added in the security-audit phase) for the threat model.

## Install

_Packaging (AUR, Flatpak) lands in the release phase. Build from source below._

## Build

```bash
cargo build --release
cargo run -p orca-gui
```

Requires a stable Rust toolchain (1.85+) and GTK4 development libraries.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions and code style.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
# orca-project
