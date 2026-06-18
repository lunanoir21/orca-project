//! Multiple-vault management.
//!
//! [`VaultManager`] is the registry the application talks to: it tracks every
//! configured vault, holds the currently-opened [`Vault`] instances, remembers
//! which vault is active, and gives each opened vault its own independent
//! [`AutoLockTimer`]. Lock state is per-vault — locking one never affects
//! another.
//!
//! # Security
//! Removing a vault only unregisters it; it never deletes the on-disk vault
//! files (which stay encrypted). Each opened vault owns its own master key via
//! its [`Vault`], zeroized independently on lock.

use std::collections::HashMap;

use crate::autolock::AutoLockTimer;
use crate::{Result, Vault, VaultConfig, VaultError};

/// An opened vault together with its own inactivity timer.
struct ManagedVault {
    vault: Vault,
    timer: AutoLockTimer,
}

/// Registry of all configured vaults and the subset currently opened.
#[derive(Default)]
pub struct VaultManager {
    configs: Vec<VaultConfig>,
    open: HashMap<String, ManagedVault>,
    active: Option<String>,
}

impl VaultManager {
    /// Build a manager from the vault configurations loaded from `config.toml`.
    /// No vaults are opened yet.
    ///
    /// # Errors
    /// Returns [`VaultError::Config`] if two configs share a name.
    pub fn new(configs: Vec<VaultConfig>) -> Result<Self> {
        let mut seen = std::collections::HashSet::new();
        for c in &configs {
            if !seen.insert(c.name.clone()) {
                return Err(VaultError::Config(format!(
                    "duplicate vault name: {}",
                    c.name
                )));
            }
        }
        Ok(Self {
            configs,
            open: HashMap::new(),
            active: None,
        })
    }

    /// All configured vaults (opened or not).
    #[must_use]
    pub fn configs(&self) -> &[VaultConfig] {
        &self.configs
    }

    /// Number of configured vaults.
    #[must_use]
    pub fn len(&self) -> usize {
        self.configs.len()
    }

    /// Returns `true` if no vaults are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Returns `true` if a vault with `name` is configured.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.configs.iter().any(|c| c.name == name)
    }

    fn config_of(&self, name: &str) -> Option<&VaultConfig> {
        self.configs.iter().find(|c| c.name == name)
    }

    /// Create a brand-new vault on disk, register it, open it, and make it
    /// active.
    ///
    /// # Errors
    /// [`VaultError::AlreadyExists`] if the name is taken, plus any error from
    /// [`Vault::create`].
    pub fn add_vault(&mut self, config: VaultConfig, passphrase: &[u8]) -> Result<()> {
        config.validate()?;
        if self.contains(&config.name) {
            return Err(VaultError::AlreadyExists(config.path.clone()));
        }
        let vault = Vault::create(config.clone(), passphrase)?;
        let timer = AutoLockTimer::from_minutes(config.auto_lock_minutes);
        let name = config.name.clone();
        self.configs.push(config);
        self.open
            .insert(name.clone(), ManagedVault { vault, timer });
        self.active = Some(name);
        Ok(())
    }

    /// Register and open an existing on-disk vault from a config, without
    /// creating it. Makes it active.
    ///
    /// # Errors
    /// [`VaultError::AlreadyExists`] if the name is already configured, plus any
    /// error from [`Vault::open`].
    pub fn register_existing(&mut self, config: VaultConfig, passphrase: &[u8]) -> Result<()> {
        config.validate()?;
        if self.contains(&config.name) {
            return Err(VaultError::AlreadyExists(config.path.clone()));
        }
        let vault = Vault::open(config.clone(), passphrase)?;
        let timer = AutoLockTimer::from_minutes(config.auto_lock_minutes);
        let name = config.name.clone();
        self.configs.push(config);
        self.open
            .insert(name.clone(), ManagedVault { vault, timer });
        self.active = Some(name);
        Ok(())
    }

    /// Open an already-registered vault (unlock it into memory) and make it
    /// active.
    ///
    /// # Errors
    /// [`VaultError::NotFound`] if `name` is not configured, plus any error from
    /// [`Vault::open`].
    pub fn open_vault(&mut self, name: &str, passphrase: &[u8]) -> Result<()> {
        let config = self
            .config_of(name)
            .ok_or_else(|| VaultError::NotFound(name.into()))?
            .clone();
        let vault = Vault::open(config.clone(), passphrase)?;
        let timer = AutoLockTimer::from_minutes(config.auto_lock_minutes);
        self.open
            .insert(name.to_owned(), ManagedVault { vault, timer });
        self.active = Some(name.to_owned());
        Ok(())
    }

    /// Lock an opened vault (zeroizes its key). No effect on other vaults.
    ///
    /// # Errors
    /// [`VaultError::NotFound`] if the vault is not currently open.
    pub fn lock_vault(&mut self, name: &str) -> Result<()> {
        let mv = self
            .open
            .get_mut(name)
            .ok_or_else(|| VaultError::NotFound(name.into()))?;
        mv.vault.lock()
    }

    /// Lock every opened vault.
    ///
    /// # Errors
    /// Propagates the first lock error encountered.
    pub fn lock_all(&mut self) -> Result<()> {
        for mv in self.open.values_mut() {
            mv.vault.lock()?;
        }
        Ok(())
    }

    /// Unregister a vault. Does **not** delete its on-disk files. Returns the
    /// removed configuration.
    ///
    /// # Errors
    /// [`VaultError::NotFound`] if `name` is not configured.
    pub fn remove_vault(&mut self, name: &str) -> Result<VaultConfig> {
        let pos = self
            .configs
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| VaultError::NotFound(name.into()))?;
        // Dropping the ManagedVault zeroizes the key if it was unlocked.
        self.open.remove(name);
        if self.active.as_deref() == Some(name) {
            self.active = None;
        }
        Ok(self.configs.remove(pos))
    }

    /// The active vault's name, if any.
    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Set the active vault. It must be configured.
    ///
    /// # Errors
    /// [`VaultError::NotFound`] if `name` is not configured.
    pub fn set_active(&mut self, name: &str) -> Result<()> {
        if !self.contains(name) {
            return Err(VaultError::NotFound(name.into()));
        }
        self.active = Some(name.to_owned());
        Ok(())
    }

    /// Borrow an opened vault by name.
    #[must_use]
    pub fn vault(&self, name: &str) -> Option<&Vault> {
        self.open.get(name).map(|mv| &mv.vault)
    }

    /// Mutably borrow an opened vault by name.
    #[must_use]
    pub fn vault_mut(&mut self, name: &str) -> Option<&mut Vault> {
        self.open.get_mut(name).map(|mv| &mut mv.vault)
    }

    /// Borrow an opened vault's auto-lock timer.
    #[must_use]
    pub fn timer(&self, name: &str) -> Option<&AutoLockTimer> {
        self.open.get(name).map(|mv| &mv.timer)
    }

    /// Mutably borrow an opened vault's auto-lock timer (e.g. to `touch` it).
    #[must_use]
    pub fn timer_mut(&mut self, name: &str) -> Option<&mut AutoLockTimer> {
        self.open.get_mut(name).map(|mv| &mut mv.timer)
    }

    /// Returns `true` if the named vault is currently open and unlocked.
    #[must_use]
    pub fn is_unlocked(&self, name: &str) -> bool {
        self.open.get(name).is_some_and(|mv| !mv.vault.is_locked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncryptionScheme, IntegrityHash};
    use std::path::{Path, PathBuf};

    fn tmp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "orca-mgr-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

    #[test]
    fn new_rejects_duplicate_names() {
        let d = tmp_dir("dup");
        let configs = vec![cfg("A", &d.join("a")), cfg("A", &d.join("b"))];
        assert!(matches!(
            VaultManager::new(configs),
            Err(VaultError::Config(_))
        ));
    }

    #[test]
    fn add_open_and_independent_lock_state() {
        let base = tmp_dir("multi");
        let mut m = VaultManager::new(vec![]).unwrap();
        m.add_vault(cfg("One", &base.join("one")), b"pw1").unwrap();
        m.add_vault(cfg("Two", &base.join("two")), b"pw2").unwrap();

        assert_eq!(m.len(), 2);
        assert_eq!(m.active(), Some("Two"));
        assert!(m.is_unlocked("One"));
        assert!(m.is_unlocked("Two"));

        // Lock only One; Two stays unlocked → independent state.
        m.lock_vault("One").unwrap();
        assert!(!m.is_unlocked("One"));
        assert!(m.is_unlocked("Two"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn switch_active() {
        let base = tmp_dir("active");
        let mut m = VaultManager::new(vec![]).unwrap();
        m.add_vault(cfg("One", &base.join("one")), b"pw").unwrap();
        m.add_vault(cfg("Two", &base.join("two")), b"pw").unwrap();
        m.set_active("One").unwrap();
        assert_eq!(m.active(), Some("One"));
        assert!(matches!(
            m.set_active("Ghost"),
            Err(VaultError::NotFound(_))
        ));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn remove_keeps_files_and_can_reopen() {
        let base = tmp_dir("remove");
        let dir = base.join("v");
        let mut m = VaultManager::new(vec![]).unwrap();
        m.add_vault(cfg("V", &dir), b"pw").unwrap();

        let removed = m.remove_vault("V").unwrap();
        assert_eq!(removed.name, "V");
        assert!(!m.contains("V"));
        assert_eq!(m.active(), None);
        // Files still on disk.
        assert!(dir.join("meta.toml").exists());

        // Re-register the same on-disk vault.
        m.register_existing(cfg("V", &dir), b"pw").unwrap();
        assert!(m.is_unlocked("V"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn add_duplicate_name_rejected() {
        let base = tmp_dir("dupadd");
        let mut m = VaultManager::new(vec![]).unwrap();
        m.add_vault(cfg("X", &base.join("x1")), b"pw").unwrap();
        let err = m.add_vault(cfg("X", &base.join("x2")), b"pw").unwrap_err();
        assert!(matches!(err, VaultError::AlreadyExists(_)));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn open_unregistered_is_not_found() {
        let mut m = VaultManager::new(vec![]).unwrap();
        assert!(matches!(
            m.open_vault("nope", b"pw"),
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn each_open_vault_has_own_timer() {
        let base = tmp_dir("timers");
        let mut m = VaultManager::new(vec![]).unwrap();
        m.add_vault(cfg("One", &base.join("one")), b"pw").unwrap();
        m.add_vault(cfg("Two", &base.join("two")), b"pw").unwrap();
        assert!(m.timer("One").unwrap().is_enabled());
        // Touch one timer; the other is a distinct instance.
        m.timer_mut("Two").unwrap().disable();
        assert!(m.timer("One").unwrap().is_enabled());
        assert!(!m.timer("Two").unwrap().is_enabled());
        std::fs::remove_dir_all(&base).ok();
    }
}
