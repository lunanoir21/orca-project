# Security Policy

## Threat Model

Orca is a desktop file manager with an integrated encrypted vault. The primary threats it protects against:

1. **Vault data confidentiality** — files stored in an Orca vault must be unreadable without the correct passphrase, even if an attacker has read access to the vault directory.
2. **Vault integrity** — tampered or partially overwritten vault files must be detected.
3. **Credential safety** — passphrases and derived keys must never appear in logs, on disk in plaintext, or in error messages.
4. **Plugin isolation** — Lua plugins must not be able to read vault keys or access the filesystem outside the `orca.*` API.
5. **Network MitM** — SFTP connections must verify host keys before transmitting credentials.

Out of scope: protecting against a root-level attacker on the same machine who can read process memory.

---

## Vault Encryption Design

### Key Derivation

Passphrases are never stored. A 256-bit encryption key is derived on every unlock using **Argon2id** (RFC 9106) with the following parameters, which meet or exceed OWASP recommendations:

| Parameter | Value | OWASP minimum |
|-----------|-------|---------------|
| Memory (m) | 65 536 KiB (64 MiB) | 65 536 KiB |
| Iterations (t) | 3 | 3 |
| Parallelism (p) | 4 | — |

A unique 16-byte random salt is generated per vault on creation and stored in `meta.toml` alongside the KDF parameters. The salt is never reused.

### Encryption Scheme

Individual files and the vault index are encrypted using the **`age`** file encryption format (`age` v0.11, RFC draft). Internally `age` uses X25519 (key agreement) and ChaCha20-Poly1305 (AEAD). The derived Argon2id key is used as a passphrase recipient.

Streaming 64 KiB chunks are used for all file operations — files are never fully loaded into memory before encryption or decryption begins.

### Index Confidentiality

Original filenames are **never stored in plaintext**. The vault index (`index.age`) is itself encrypted with the vault key. On disk, vault entries are identified only by UUIDs.

### Integrity

Every vault entry is covered by its ChaCha20-Poly1305 AEAD tag. The vault `verify_integrity` operation additionally re-hashes each file's decrypted content against the Blake3/SHA-256 hash stored in the encrypted index.

### Key Zeroization

All derived keys are wrapped in a `DerivedKey` struct that implements `Drop` with `zeroize::Zeroize`. Keys are zeroized immediately after use in `Vault::lock()` and on drop.

### What `meta.toml` Contains (Plaintext by Design)

`meta.toml` stores: the Argon2id salt, KDF parameters, and a sealed passphrase verifier. It must be readable before any key is available so the unlock dialog can run KDF before attempting decryption. It does **not** contain any key material.

---

## Plugin Sandbox

Lua plugins run in a `mlua` Lua 5.4 environment with:

- Only `string`, `table`, and `math` from the standard library available.
- `io`, `os`, `package`, `debug` explicitly removed from globals.
- `require` replaced with a function that always raises an error.
- Per-hook execution timeout of 500 ms (enforced via Lua instruction-count hooks).
- A 64 MiB Lua heap limit (`Lua::set_memory_limit`), so a runaway allocation loop is killed rather than exhausting host memory.
- Plugins cannot import `orca-vault` internals — the only interface is the `orca.*` table.

A plugin that panics, errors, or exceeds its timeout is caught, its error is logged, and it is moved to `PluginState::Error`. It cannot crash Orca.

### `orca.exec` vs. `orca.exec_argv` / `orca.spawn_argv`

`orca.exec(cmd)` runs `cmd` through `sh -c`. It exists for plugins that only
ever pass fixed string literals (e.g. `orca.exec("git status --porcelain")`).
Any plugin that interpolates a file path or other variable data into the
command **must** use `orca.exec_argv(program, args)` or `orca.spawn_argv(program,
args)` instead — both pass `args` straight to `execve` with no shell in
between, so a path containing `;`, backticks, or spaces cannot inject a
second command. The built-in `git-status.lua` and `archive.lua` plugins use
`exec_argv` for exactly this reason.

### `secure-open.lua` and bubblewrap

The built-in `secure-open.lua` plugin adds a "Secure Open" context-menu item
that runs the selected file's default opener inside a [bubblewrap](https://github.com/containers/bubblewrap)
sandbox: read-only root filesystem, an empty `/home`, the target file
re-exposed read-only, and `--unshare-all` (no network, separate PID/IPC/UTS
namespaces). This is **namespace-based isolation, not a hardened security
boundary**: it stops the opened app from reading or writing other files and
from establishing a routed network connection, but it does not add seccomp
filtering and cannot stop every local side channel (for example, a host
service reachable over a Unix socket under `/run` is still reachable, since
isolating that would break normal desktop integration like the Wayland
socket the opened app needs to draw a window). Treat it as a strong
reduction in blast radius for an untrusted file, not a guarantee. It requires
`bubblewrap` (`bwrap`) to be installed; if it isn't, the plugin notifies once
and does nothing rather than silently opening the file unsandboxed.

---

## SFTP Security

SFTP connections verify the server's host key against `~/.ssh/known_hosts` before transmitting credentials. An unknown or mismatched host key causes the connection to fail immediately with a descriptive error message.

Plain FTP connections log a `WARN`-level message before connecting, noting that credentials travel in cleartext.

---

## Reporting a Vulnerability

Please report security vulnerabilities by emailing **anilbektas541@gmail.com** with the subject line `[ORCA SECURITY]`. Include:

- A description of the vulnerability and its impact.
- Steps to reproduce or a proof-of-concept.
- Any suggested mitigations.

Do **not** open a public GitHub issue for security vulnerabilities until a fix has been prepared and coordinated.

Expected response time: within 5 business days.

---

## Dependency Trust

All cryptographic dependencies in `orca-vault` have their versions pinned exactly in `Cargo.toml`:

```
age = "0.11.1"
argon2 = "0.5.3"
chacha20poly1305 = "0.10.1"
blake3 = "1.8.5"
sha2 = "0.10.9"
zeroize = "1.9.0"
```

`cargo audit` and `cargo deny check` run on every CI push and on a weekly schedule. The `Cargo.lock` is committed to ensure reproducible builds.
