//! XDG base-directory resolution for Orca.
//!
//! Resolves the per-user config, data and cache directories (plus the themes
//! and plugins subdirectories) and creates them on first run. Honours
//! `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` / `$XDG_CACHE_HOME` via the `dirs`
//! crate, falling back to the spec defaults.

use std::io;
use std::path::PathBuf;

/// Resolved Orca directory layout.
#[derive(Debug, Clone)]
pub struct XdgPaths {
    /// `$XDG_CONFIG_HOME/orca` — `config.toml` lives here.
    pub config: PathBuf,
    /// `$XDG_DATA_HOME/orca` — vaults, themes, plugins.
    pub data: PathBuf,
    /// `$XDG_CACHE_HOME/orca` — thumbnails and other regenerable data.
    pub cache: PathBuf,
    /// `$XDG_DATA_HOME/orca/themes` — user CSS themes.
    pub themes: PathBuf,
    /// `$XDG_DATA_HOME/orca/plugins` — user Lua plugins.
    pub plugins: PathBuf,
}

impl XdgPaths {
    /// Resolve the Orca directory layout from the environment.
    ///
    /// # Errors
    /// Returns an error if a base directory cannot be determined (e.g. `$HOME`
    /// is unset on a platform with no other fallback).
    pub fn resolve() -> io::Result<Self> {
        let missing = |what: &str| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("cannot determine XDG {what} directory"),
            )
        };
        let config = dirs::config_dir()
            .ok_or_else(|| missing("config"))?
            .join("orca");
        let data = dirs::data_dir()
            .ok_or_else(|| missing("data"))?
            .join("orca");
        let cache = dirs::cache_dir()
            .ok_or_else(|| missing("cache"))?
            .join("orca");
        let themes = data.join("themes");
        let plugins = data.join("plugins");
        Ok(Self {
            config,
            data,
            cache,
            themes,
            plugins,
        })
    }

    /// Create every directory in the layout if it does not already exist.
    ///
    /// # Errors
    /// Returns the first I/O error encountered while creating a directory.
    pub fn ensure(&self) -> io::Result<()> {
        for dir in [
            &self.config,
            &self.data,
            &self.cache,
            &self.themes,
            &self.plugins,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Path to the user's `config.toml`.
    // Consumed by the config loader in Phase 7.1; only the unit test references
    // it in the current bootstrap build.
    #[allow(dead_code)]
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_appends_orca_and_subdirs() {
        let p = XdgPaths::resolve().expect("resolve");
        assert!(p.config.ends_with("orca"));
        assert!(p.data.ends_with("orca"));
        assert!(p.themes.ends_with("themes"));
        assert!(p.plugins.ends_with("plugins"));
        assert!(p.config_file().ends_with("config.toml"));
    }

    #[test]
    fn ensure_creates_directories() {
        // Point XDG at a throwaway root so the test never touches the real home.
        let root = std::env::temp_dir().join(format!(
            "orca-xdg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = XdgPaths {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            themes: root.join("data/themes"),
            plugins: root.join("data/plugins"),
        };
        paths.ensure().expect("ensure");
        assert!(paths.config.is_dir());
        assert!(paths.themes.is_dir());
        assert!(paths.plugins.is_dir());
        std::fs::remove_dir_all(&root).ok();
    }
}
