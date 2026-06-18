//! Orca Lua plugin system.
//!
//! `orca-plugin` loads `.lua` plugins into a sandboxed Lua 5.4 environment
//! (`mlua`) and exposes the `orca.*` API as the only interface to Orca
//! internals. The sandbox blocks `io`, `os`, `package` and `debug`, enforces
//! per-hook execution timeouts, and catches panicking plugins so a misbehaving
//! plugin can never crash Orca or reach `orca-vault` internals.
#![forbid(unsafe_code)]

pub mod api;
pub mod error;
pub mod loader;
pub mod manager;
pub mod sandbox;
pub mod types;

pub use error::PluginError;
pub use manager::{PluginInfo, PluginManager};
pub use types::{Badge, BadgeMap, PluginMeta, PluginState};
