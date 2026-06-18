//! Orca filesystem engine.
//!
//! `orca-core` provides the filesystem primitives Orca is built on: directory
//! listing and metadata, file operations (copy/move/delete/rename/links),
//! XDG trash, filesystem watching, search, checksums, bulk rename, archive
//! operations, git status, disk usage, mount management and network locations.
//!
//! This crate contains no `unwrap()`/`expect()` in library paths; all fallible
//! operations return [`Result`] via the crate error type [`OrcaError`].
//!
//! Functionality is implemented phase by phase per `TASKS.md`.
#![forbid(unsafe_code)]

pub mod archive;
pub mod checksum;
pub mod disk_usage;
pub mod error;
pub mod fs;
pub mod git;
pub mod mount;
pub mod net;
pub mod recent;
pub mod types;

mod util;

pub use archive::{compress, extract, list_archive, ArchiveEntry, ArchiveFormat, ArchiveProgress};
pub use checksum::{checksum_blake3, checksum_md5, checksum_sha256};
pub use disk_usage::{dir_size, disk_usage_tree, filesystem_usage, FilesystemUsage, UsageNode};
pub use error::{OrcaError, Result};
pub use fs::bulk_rename::{
    apply_rename, preview_rename, BulkRenameRule, RenameConflict, RenamePreview, RenameResult,
};
pub use fs::ops::{
    copy, create_dir, create_file, create_hardlink, create_symlink, delete, move_entry, rename,
    set_owner, set_permissions, CopyProgress,
};
pub use fs::search::{search_by_content, search_by_name, SearchOptions};
pub use fs::trash::{empty_trash, list_trash, move_to_trash, restore_from_trash, TrashEntry};
pub use fs::watch::{watch, watch_dir, DirWatch, Watch, WatchEvent};
pub use fs::{is_hidden_name, list_dir, list_dir_cancellable, resolve_symlink, CancelToken};
pub use git::{git_status, GitStatus};
pub use mount::{eject, list_mounts, mount, mount_events, unmount, MountEvent, MountedVolume};
pub use net::connection::{
    load_connections, remove_connection, save_connections, upsert_connection, Protocol,
    SavedConnection,
};
pub use net::ftp::FtpClient;
pub use net::sftp::{SftpAuth, SftpClient};
pub use net::RemoteEntry;
pub use recent::{add_recent, clear_recent, get_recent, RecentEntry};
pub use types::{FileEntry, FileKind, FilterOptions, SortKey};
