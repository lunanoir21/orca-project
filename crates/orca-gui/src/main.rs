//! Orca GTK4/relm4 desktop file manager — application entry point.
//!
//! Phase 3.1 bootstrap: initialise structured logging, resolve and create the
//! XDG directory layout, then hand control to the relm4 [`app::AppModel`] shell.
//! The file browser (Phase 3.2), navigation (3.4) and the rest hang off that
//! shell in later phases.

mod app;
mod archive_view;
mod bulk_rename;
mod keybind;
mod disk_usage;
mod mount_manager;
mod network;
mod config;
mod dialogs;
mod format;
mod home;
mod i18n;
mod nav;
mod pane;
mod permissions;
mod places;
mod preview;
mod properties;
mod settings;
mod side;
mod terminal;
mod theme;
mod thumbnail;
mod toolbar;
mod vault_ui;
mod xdg;

use relm4::RelmApp;

use crate::app::{AppInit, AppModel};
use crate::config::Config;
use crate::xdg::XdgPaths;

/// Reverse-DNS application identifier (D-Bus name, desktop-file basename).
const APP_ID: &str = "io.github.lunanoir21.orca";

fn main() {
    init_tracing();

    let paths = match XdgPaths::resolve() {
        Ok(paths) => paths,
        Err(e) => {
            tracing::error!(error = %e, "failed to resolve XDG directories");
            std::process::exit(1);
        }
    };
    if let Err(e) = paths.ensure() {
        tracing::error!(error = %e, "failed to create XDG directories");
        std::process::exit(1);
    }
    tracing::info!(
        config = %paths.config.display(),
        data = %paths.data.display(),
        cache = %paths.cache.display(),
        "orca {} starting",
        env!("CARGO_PKG_VERSION"),
    );

    // Load config and bind the interface language before any widget is built.
    let config = Config::load(&paths);
    i18n::init(config.language());

    // Resolve the built-in plugins directory: next to the executable in a release
    // build, or two levels up from the binary in a cargo dev build.
    let builtin_plugins = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::PathBuf::from))
        .map(|bin_dir| {
            // dev: target/debug/orca → workspace root / plugins
            let candidate = bin_dir.join("../../plugins");
            if candidate.is_dir() { candidate } else { bin_dir.join("plugins") }
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/usr/share/orca/plugins"));

    let app = RelmApp::new(APP_ID);
    app.run::<AppModel>(AppInit { paths, config, builtin_plugins });
}

/// Initialise `tracing-subscriber` from the `RUST_LOG` environment variable,
/// defaulting to `info` when it is unset or unparseable.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
