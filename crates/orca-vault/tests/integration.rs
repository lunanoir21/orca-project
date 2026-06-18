//! End-to-end integration tests for the `orca-vault` public API (Phase 2.12).
//!
//! These complement the in-module unit tests by exercising only the crate's
//! public surface, the way the GUI will.

use std::path::{Path, PathBuf};
use std::time::Duration;

use orca_vault::{
    derive_key, encrypt_file, gen_salt, AutoLockTimer, EncryptionScheme, IntegrityHash, KdfParams,
    Vault, VaultConfig, VaultError,
};

fn tmp(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!(
        "orca-it-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cfg(name: &str, dir: &Path) -> VaultConfig {
    VaultConfig {
        name: name.into(),
        path: dir.to_path_buf(),
        auto_lock_minutes: Some(5),
        scheme: EncryptionScheme::Age,
        integrity_hash: IntegrityHash::Blake3,
    }
}

fn write(p: &Path, b: &[u8]) {
    std::fs::write(p, b).unwrap();
}

#[test]
fn full_vault_create_add_list_extract_remove_cycle() {
    let base = tmp("cycle");
    let vault_dir = base.join("vault");
    let mut v = Vault::create(cfg("Personal", &vault_dir), b"correct horse").expect("create");

    write(&base.join("alpha.txt"), b"alpha contents");
    write(&base.join("beta.bin"), &vec![7u8; 50_000]);
    v.add_file(&base.join("alpha.txt")).expect("add alpha");
    v.add_file(&base.join("beta.bin")).expect("add beta");

    let files = v.list_files().expect("list");
    assert_eq!(files.len(), 2);

    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let restored = v.extract_file("alpha.txt", &out).expect("extract");
    assert_eq!(std::fs::read(restored).unwrap(), b"alpha contents");

    v.remove_file("alpha.txt").expect("remove");
    assert_eq!(v.list_files().unwrap().len(), 1);
    assert!(v.index().unwrap().find_by_original("beta.bin").is_some());

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn index_survives_lock_unlock_cycle() {
    let base = tmp("survive");
    let vault_dir = base.join("vault");
    let mut v = Vault::create(cfg("V", &vault_dir), b"pw").expect("create");
    write(&base.join("keep.txt"), b"persisted across lock");
    v.add_file(&base.join("keep.txt")).expect("add");
    v.lock().expect("lock");
    assert!(v.is_locked());

    let v2 = Vault::open(cfg("V", &vault_dir), b"pw").expect("reopen");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let f = v2
        .extract_file("keep.txt", &out)
        .expect("extract after reopen");
    assert_eq!(std::fs::read(f).unwrap(), b"persisted across lock");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn wrong_passphrase_returns_error_not_garbage() {
    let base = tmp("wrongpass");
    let vault_dir = base.join("vault");
    {
        let mut v = Vault::create(cfg("V", &vault_dir), b"right-pass").expect("create");
        write(&base.join("s.txt"), b"secret");
        v.add_file(&base.join("s.txt")).expect("add");
    }
    // Vault open with wrong passphrase: error, never a usable vault.
    let err = Vault::open(cfg("V", &vault_dir), b"bad-pass").unwrap_err();
    assert!(matches!(err, VaultError::WrongPassphrase));

    // Single-file: wrong passphrase yields an error and leaves no plaintext.
    let plain = base.join("doc.dat");
    write(&plain, b"top secret bytes here");
    let enc = encrypt_file(&plain, b"right-pass", EncryptionScheme::Argon2idXchacha20).unwrap();
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let derr = orca_vault::decrypt_file(&enc, b"bad-pass", &out).unwrap_err();
    assert!(matches!(derr, VaultError::WrongPassphrase));
    assert!(std::fs::read_dir(&out).unwrap().next().is_none());

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn key_inaccessible_after_lock() {
    // The master key is dropped (and zeroized via DerivedKey) on lock; behaviour
    // proves it is gone: every key-dependent op now fails with `Locked`.
    let base = tmp("locked");
    let mut v = Vault::create(cfg("V", &base.join("vault")), b"pw").expect("create");
    write(&base.join("a.txt"), b"a");
    v.add_file(&base.join("a.txt")).expect("add");
    v.lock().expect("lock");

    assert!(matches!(v.list_files(), Err(VaultError::Locked)));
    assert!(matches!(v.verify_integrity(), Err(VaultError::Locked)));
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    assert!(matches!(
        v.extract_file("a.txt", &out),
        Err(VaultError::Locked)
    ));
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn backup_export_import_roundtrip() {
    let base = tmp("backup");
    let mut v = Vault::create(cfg("V", &base.join("vault")), b"pw").expect("create");
    write(&base.join("doc.txt"), b"backup payload");
    v.add_file(&base.join("doc.txt")).expect("add");

    let backup = base.join("v.tar.age");
    v.export_backup(&backup).expect("export");

    let v2 = Vault::import_backup(&backup, b"pw", &base.join("restored"), "R").expect("import");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let f = v2.extract_file("doc.txt", &out).expect("extract");
    assert_eq!(std::fs::read(f).unwrap(), b"backup payload");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn integrity_report_detects_tampered_file() {
    let base = tmp("integ");
    let vault_dir = base.join("vault");
    let mut v = Vault::create(cfg("V", &vault_dir), b"pw").expect("create");
    write(&base.join("t.txt"), b"checked content");
    v.add_file(&base.join("t.txt")).expect("add");
    let stored = v.list_files().unwrap()[0].stored_name.clone();

    let data = vault_dir.join("data").join(&stored);
    let mut bytes = std::fs::read(&data).unwrap();
    let i = bytes.len() - 2;
    bytes[i] ^= 0xff;
    std::fs::write(&data, &bytes).unwrap();

    let report = v.verify_integrity().expect("verify");
    assert!(!report.is_clean());
    assert_eq!(report.tampered, vec!["t.txt".to_string()]);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn kdf_is_deterministic_and_salt_sensitive() {
    let salt = gen_salt().unwrap();
    let a = derive_key(b"passphrase", &salt, KdfParams::default()).unwrap();
    let b = derive_key(b"passphrase", &salt, KdfParams::default()).unwrap();
    assert_eq!(a.as_bytes(), b.as_bytes());

    let other = gen_salt().unwrap();
    let c = derive_key(b"passphrase", &other, KdfParams::default()).unwrap();
    assert_ne!(a.as_bytes(), c.as_bytes());
}

#[tokio::test]
async fn auto_lock_fires_after_timeout() {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let timer = Arc::new(Mutex::new(AutoLockTimer::new(Some(Duration::from_millis(
        150,
    )))));
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let handle = tokio::spawn(orca_vault::run_auto_lock(timer, "Personal".into(), tx));

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("auto-lock did not fire")
        .expect("channel closed");
    assert_eq!(event.vault, "Personal");
    handle.await.expect("task panicked");
}
