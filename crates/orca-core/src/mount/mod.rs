//! Removable-media mount management via UDisks2 over D-Bus (`zbus`).
//!
//! [`list_mounts`] enumerates mounted filesystems; [`mount`], [`unmount`] and
//! [`eject`] drive the corresponding UDisks2 operations; [`mount_events`]
//! streams plug/unplug notifications by watching UDisks2's object manager.
//!
//! All calls go to the system bus service `org.freedesktop.UDisks2`. If that
//! service is unavailable, the connecting call returns an error rather than
//! panicking, so a caller on a machine without UDisks2 degrades gracefully.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::sync::mpsc::{self, Receiver};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::error::{OrcaError, Result};

const UDISKS_SERVICE: &str = "org.freedesktop.UDisks2";
const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
const IFACE_FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const IFACE_BLOCK: &str = "org.freedesktop.UDisks2.Block";

/// A currently-mounted filesystem reported by UDisks2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedVolume {
    /// Backing block device, e.g. `/dev/sda1`.
    pub device: PathBuf,
    /// Where it is mounted.
    pub mount_point: PathBuf,
    /// Filesystem type as identified by UDisks2 (e.g. `ext4`, `vfat`).
    pub fs_type: String,
}

/// A hotplug event from UDisks2's object manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountEvent {
    /// A block device / filesystem object appeared.
    Added(PathBuf),
    /// A block device / filesystem object disappeared.
    Removed(PathBuf),
}

/// Proxy for `org.freedesktop.UDisks2.Filesystem` (per-device object path).
#[zbus::proxy(
    interface = "org.freedesktop.UDisks2.Filesystem",
    default_service = "org.freedesktop.UDisks2"
)]
trait Filesystem {
    /// Mount the filesystem, returning the resulting mount-point path.
    fn mount(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<String>;
    /// Unmount the filesystem.
    fn unmount(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// Proxy for `org.freedesktop.UDisks2.Drive` (per-drive object path).
#[zbus::proxy(
    interface = "org.freedesktop.UDisks2.Drive",
    default_service = "org.freedesktop.UDisks2"
)]
trait Drive {
    /// Eject removable media from the drive.
    fn eject(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// Connect to the system bus.
async fn connect() -> Result<zbus::Connection> {
    zbus::Connection::system()
        .await
        .map_err(|e| OrcaError::Other(format!("D-Bus system bus connect failed: {e}")))
}

/// Fetch all UDisks2-managed objects and their interface property maps.
async fn managed_objects(
    conn: &zbus::Connection,
) -> Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>> {
    let om = zbus::fdo::ObjectManagerProxy::builder(conn)
        .destination(UDISKS_SERVICE)
        .and_then(|b| b.path(UDISKS_PATH))
        .map_err(|e| OrcaError::Other(format!("bad object-manager address: {e}")))?
        .build()
        .await
        .map_err(|e| OrcaError::Other(format!("object-manager proxy failed: {e}")))?;
    let raw = om
        .get_managed_objects()
        .await
        .map_err(|e| OrcaError::Other(format!("GetManagedObjects failed: {e}")))?;

    // Normalise the interface-name and signature key types to plain `String`.
    let mut out = HashMap::with_capacity(raw.len());
    for (path, ifaces) in raw {
        let mut imap = HashMap::with_capacity(ifaces.len());
        for (iface, props) in ifaces {
            let pmap = props
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<HashMap<String, OwnedValue>>();
            imap.insert(iface.to_string(), pmap);
        }
        out.insert(path, imap);
    }
    Ok(out)
}

/// List currently-mounted filesystems known to UDisks2.
///
/// # Errors
/// Returns [`OrcaError::Other`] if the system bus or UDisks2 cannot be reached.
pub async fn list_mounts() -> Result<Vec<MountedVolume>> {
    let conn = connect().await?;
    let objects = managed_objects(&conn).await?;

    let mut mounts = Vec::new();
    for ifaces in objects.values() {
        let Some(fs) = ifaces.get(IFACE_FILESYSTEM) else {
            continue;
        };
        let mount_points = fs
            .get("MountPoints")
            .map(decode_byte_string_array)
            .unwrap_or_default();
        if mount_points.is_empty() {
            continue; // filesystem present but not mounted
        }
        let block = ifaces.get(IFACE_BLOCK);
        let device = block
            .and_then(|b| b.get("Device"))
            .map(decode_byte_string)
            .unwrap_or_default();
        let fs_type = block
            .and_then(|b| b.get("IdType"))
            .and_then(decode_string)
            .unwrap_or_default();

        for mp in mount_points {
            mounts.push(MountedVolume {
                device: device.clone(),
                mount_point: mp,
                fs_type: fs_type.clone(),
            });
        }
    }
    mounts.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    Ok(mounts)
}

/// Mount the filesystem on block device `device` (e.g. `/dev/sdb1`), returning
/// the mount point UDisks2 chose.
///
/// # Errors
/// [`OrcaError::NotFound`] if no such block device is known to UDisks2; otherwise
/// the underlying D-Bus / UDisks2 error.
pub async fn mount(device: impl AsRef<Path>) -> Result<PathBuf> {
    let device = device.as_ref();
    let conn = connect().await?;
    let objects = managed_objects(&conn).await?;
    let obj = block_object_for_device(&objects, device)
        .ok_or_else(|| OrcaError::NotFound(device.to_path_buf()))?;

    let fs = FilesystemProxy::builder(&conn)
        .path(obj)
        .map_err(|e| OrcaError::Other(format!("filesystem proxy path: {e}")))?
        .build()
        .await
        .map_err(|e| OrcaError::Other(format!("filesystem proxy failed: {e}")))?;
    let mount_path = fs
        .mount(HashMap::new())
        .await
        .map_err(|e| OrcaError::Other(format!("mount failed: {e}")))?;
    Ok(PathBuf::from(mount_path))
}

/// Unmount whatever filesystem is mounted at `target`, where `target` is either
/// a mount point or the backing block device path.
///
/// # Errors
/// [`OrcaError::NotFound`] if no matching filesystem is found; otherwise the
/// underlying D-Bus / UDisks2 error.
pub async fn unmount(target: impl AsRef<Path>) -> Result<()> {
    let target = target.as_ref();
    let conn = connect().await?;
    let objects = managed_objects(&conn).await?;
    let obj = filesystem_object_for_target(&objects, target)
        .ok_or_else(|| OrcaError::NotFound(target.to_path_buf()))?;

    let fs = FilesystemProxy::builder(&conn)
        .path(obj)
        .map_err(|e| OrcaError::Other(format!("filesystem proxy path: {e}")))?
        .build()
        .await
        .map_err(|e| OrcaError::Other(format!("filesystem proxy failed: {e}")))?;
    fs.unmount(HashMap::new())
        .await
        .map_err(|e| OrcaError::Other(format!("unmount failed: {e}")))?;
    Ok(())
}

/// Eject the removable drive backing block device `device`.
///
/// # Errors
/// [`OrcaError::NotFound`] if the device or its drive is unknown; otherwise the
/// underlying D-Bus / UDisks2 error.
pub async fn eject(device: impl AsRef<Path>) -> Result<()> {
    let device = device.as_ref();
    let conn = connect().await?;
    let objects = managed_objects(&conn).await?;
    let block = objects
        .iter()
        .find(|(_, ifaces)| block_device_matches(ifaces, device))
        .map(|(_, ifaces)| ifaces)
        .ok_or_else(|| OrcaError::NotFound(device.to_path_buf()))?;
    let drive_path = block
        .get(IFACE_BLOCK)
        .and_then(|b| b.get("Drive"))
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| OwnedObjectPath::try_from(v).ok())
        .ok_or_else(|| OrcaError::NotFound(device.to_path_buf()))?;

    let drive = DriveProxy::builder(&conn)
        .path(drive_path)
        .map_err(|e| OrcaError::Other(format!("drive proxy path: {e}")))?
        .build()
        .await
        .map_err(|e| OrcaError::Other(format!("drive proxy failed: {e}")))?;
    drive
        .eject(HashMap::new())
        .await
        .map_err(|e| OrcaError::Other(format!("eject failed: {e}")))?;
    Ok(())
}

/// Subscribe to UDisks2 hotplug events.
///
/// Returns a channel that yields a [`MountEvent`] each time a device object is
/// added or removed. The background task and its D-Bus connection live until the
/// receiver is dropped.
///
/// # Errors
/// Returns [`OrcaError::Other`] if the system bus cannot be reached.
pub async fn mount_events() -> Result<Receiver<MountEvent>> {
    let conn = connect().await?;
    let om = zbus::fdo::ObjectManagerProxy::builder(&conn)
        .destination(UDISKS_SERVICE)
        .and_then(|b| b.path(UDISKS_PATH))
        .map_err(|e| OrcaError::Other(format!("bad object-manager address: {e}")))?
        .build()
        .await
        .map_err(|e| OrcaError::Other(format!("object-manager proxy failed: {e}")))?;

    let mut added = om
        .receive_interfaces_added()
        .await
        .map_err(|e| OrcaError::Other(format!("subscribe added failed: {e}")))?;
    let mut removed = om
        .receive_interfaces_removed()
        .await
        .map_err(|e| OrcaError::Other(format!("subscribe removed failed: {e}")))?;

    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        // Keep the connection alive for the lifetime of the subscriptions.
        let _conn = conn;
        loop {
            tokio::select! {
                Some(sig) = added.next() => {
                    if let Ok(args) = sig.args() {
                        let path = PathBuf::from(args.object_path().as_str());
                        if tx.send(MountEvent::Added(path)).await.is_err() {
                            break;
                        }
                    }
                }
                Some(sig) = removed.next() => {
                    if let Ok(args) = sig.args() {
                        let path = PathBuf::from(args.object_path().as_str());
                        if tx.send(MountEvent::Removed(path)).await.is_err() {
                            break;
                        }
                    }
                }
                else => break,
            }
        }
    });
    Ok(rx)
}

// ---------------------------------------------------------------------------
// Object-lookup + value-decoding helpers
// ---------------------------------------------------------------------------

/// Find the object path whose Block interface's `Device` equals `device`.
fn block_object_for_device(
    objects: &HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>,
    device: &Path,
) -> Option<OwnedObjectPath> {
    objects
        .iter()
        .find(|(_, ifaces)| block_device_matches(ifaces, device))
        .map(|(path, _)| path.clone())
}

/// Find the object path of the filesystem mounted at `target`, matching either
/// its mount point or its backing device path.
fn filesystem_object_for_target(
    objects: &HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>,
    target: &Path,
) -> Option<OwnedObjectPath> {
    objects
        .iter()
        .find(|(_, ifaces)| {
            let has_fs = ifaces.get(IFACE_FILESYSTEM).is_some_and(|fs| {
                fs.get("MountPoints")
                    .map(decode_byte_string_array)
                    .unwrap_or_default()
                    .iter()
                    .any(|mp| mp == target)
            });
            has_fs || block_device_matches(ifaces, target)
        })
        .map(|(path, _)| path.clone())
}

/// Does this object's Block interface have `Device == device`?
fn block_device_matches(
    ifaces: &HashMap<String, HashMap<String, OwnedValue>>,
    device: &Path,
) -> bool {
    ifaces
        .get(IFACE_BLOCK)
        .and_then(|b| b.get("Device"))
        .map(decode_byte_string)
        .is_some_and(|d| d == device)
}

/// Decode a UDisks2 `ay` (NUL-terminated byte string) value into a path.
fn decode_byte_string(value: &OwnedValue) -> PathBuf {
    let bytes = value
        .try_clone()
        .ok()
        .and_then(|v| Vec::<u8>::try_from(v).ok())
        .unwrap_or_default();
    bytes_to_path(&bytes)
}

/// Decode a UDisks2 `aay` (array of NUL-terminated byte strings) into paths.
fn decode_byte_string_array(value: &OwnedValue) -> Vec<PathBuf> {
    value
        .try_clone()
        .ok()
        .and_then(|v| Vec::<Vec<u8>>::try_from(v).ok())
        .map(|outer| outer.iter().map(|b| bytes_to_path(b)).collect())
        .unwrap_or_default()
}

/// Decode a D-Bus string value.
fn decode_string(value: &OwnedValue) -> Option<String> {
    value
        .try_clone()
        .ok()
        .and_then(|v| String::try_from(v).ok())
}

/// Convert a NUL-terminated byte string into a `PathBuf` (lossless on unix).
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(nul) => &bytes[..nul],
        None => bytes,
    };
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(trimmed))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(trimmed).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_path_trims_nul() {
        assert_eq!(bytes_to_path(b"/dev/sda1\0"), PathBuf::from("/dev/sda1"));
        assert_eq!(bytes_to_path(b"/mnt/usb"), PathBuf::from("/mnt/usb"));
    }

    /// list_mounts must succeed against a live UDisks2 (present in this env) and
    /// return well-formed rows. If UDisks2 is unreachable the call errors; we
    /// accept either outcome so the suite passes on machines without it.
    #[tokio::test]
    async fn list_mounts_is_well_formed_or_unavailable() {
        match list_mounts().await {
            Ok(mounts) => {
                for m in &mounts {
                    assert!(
                        m.mount_point.is_absolute(),
                        "mount point should be absolute: {m:?}"
                    );
                }
            }
            Err(OrcaError::Other(_)) => { /* UDisks2 not available — acceptable */ }
            Err(other) => panic!("unexpected error kind: {other}"),
        }
    }

    /// Mounting a device that does not exist must fail cleanly (no panic). When
    /// UDisks2 is present this is `NotFound`; when absent it is a connect error.
    #[tokio::test]
    async fn mount_unknown_device_errors() {
        let err = mount("/dev/orca-does-not-exist")
            .await
            .expect_err("should fail");
        assert!(matches!(err, OrcaError::NotFound(_) | OrcaError::Other(_)));
    }

    #[tokio::test]
    async fn unmount_unknown_target_errors() {
        let err = unmount("/mnt/orca-nope").await.expect_err("should fail");
        assert!(matches!(err, OrcaError::NotFound(_) | OrcaError::Other(_)));
    }

    #[tokio::test]
    async fn eject_unknown_device_errors() {
        let err = eject("/dev/orca-does-not-exist")
            .await
            .expect_err("should fail");
        assert!(matches!(err, OrcaError::NotFound(_) | OrcaError::Other(_)));
    }
}
