//! Filesystem watching with event debouncing and directory auto-refresh.
//!
//! Built on the `notify` crate (inotify on Linux). Raw backend events are
//! mapped to the simpler [`WatchEvent`] enum and debounced in a 100 ms window so
//! a burst of inotify events (common when an editor saves a file) collapses into
//! a small, de-duplicated set. [`watch_dir`] layers on top to re-list a
//! directory once per debounce window, giving the UI a ready-to-render listing.
//!
//! Both entry points require a running `tokio` runtime (they spawn a background
//! task). Dropping the returned handle stops watching and ends the task.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::error::{OrcaError, Result};
use crate::types::{FileEntry, FilterOptions, SortKey};

/// The debounce window: events arriving within this window of the first event
/// in a burst are coalesced together.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

/// A normalized filesystem change event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchEvent {
    /// A path was created.
    Created(PathBuf),
    /// A path was removed.
    Removed(PathBuf),
    /// A path's contents or metadata changed.
    Modified(PathBuf),
    /// A path was renamed. Either side may be `None` if the backend only
    /// reported one half of the rename.
    Renamed {
        /// The old path, if known.
        from: Option<PathBuf>,
        /// The new path, if known.
        to: Option<PathBuf>,
    },
}

/// A handle to an active watch. Holds the backend watcher alive; drop it to stop
/// watching. Receive debounced [`WatchEvent`]s with [`Watch::recv`].
pub struct Watch {
    _backend: RecommendedWatcher,
    rx: UnboundedReceiver<WatchEvent>,
}

impl Watch {
    /// Await the next debounced event, or `None` once watching has stopped.
    pub async fn recv(&mut self) -> Option<WatchEvent> {
        self.rx.recv().await
    }
}

/// Watch `path` (non-recursively) and receive debounced [`WatchEvent`]s.
///
/// # Errors
/// Returns an error if the backend watcher cannot be created or cannot begin
/// watching `path`.
pub fn watch(path: impl AsRef<Path>) -> Result<Watch> {
    let (backend, mut raw_rx) = make_watcher(path.as_ref())?;
    let (out_tx, out_rx) = unbounded_channel();

    tokio::spawn(async move {
        while let Some(first) = raw_rx.recv().await {
            let batch = drain_window(&mut raw_rx, first).await;
            // De-duplicate within the window while preserving first-seen order.
            let mut seen: Vec<WatchEvent> = Vec::new();
            for ev in batch {
                if !seen.contains(&ev) {
                    seen.push(ev.clone());
                    if out_tx.send(ev).is_err() {
                        return; // receiver dropped
                    }
                }
            }
        }
    });

    Ok(Watch {
        _backend: backend,
        rx: out_rx,
    })
}

/// A handle that re-lists a directory whenever its contents change. Holds the
/// backend watcher alive; drop it to stop. Receive fresh listings (or listing
/// errors) with [`DirWatch::recv`].
pub struct DirWatch {
    _backend: RecommendedWatcher,
    rx: UnboundedReceiver<Result<Vec<FileEntry>>>,
}

impl DirWatch {
    /// Await the next directory listing. The first item is the initial listing;
    /// subsequent items arrive after each debounce window in which the directory
    /// changed. Returns `None` once watching has stopped.
    pub async fn recv(&mut self) -> Option<Result<Vec<FileEntry>>> {
        self.rx.recv().await
    }
}

/// Watch `path` and emit a fresh [`list_dir`](crate::fs::list_dir) result on
/// start and after every debounce window in which the directory changed.
///
/// # Errors
/// Returns an error if the backend watcher cannot be created or cannot begin
/// watching `path`.
pub fn watch_dir(path: impl AsRef<Path>, filter: FilterOptions, sort: SortKey) -> Result<DirWatch> {
    let path = path.as_ref().to_path_buf();
    let (backend, mut raw_rx) = make_watcher(&path)?;
    let (out_tx, out_rx) = unbounded_channel();

    tokio::spawn(async move {
        // Emit the initial listing immediately.
        if out_tx
            .send(crate::fs::list_dir(&path, &filter, sort).await)
            .is_err()
        {
            return;
        }
        while let Some(first) = raw_rx.recv().await {
            // Coalesce a burst into a single re-list.
            let _ = drain_window(&mut raw_rx, first).await;
            if out_tx
                .send(crate::fs::list_dir(&path, &filter, sort).await)
                .is_err()
            {
                return;
            }
        }
    });

    Ok(DirWatch {
        _backend: backend,
        rx: out_rx,
    })
}

/// Create a backend watcher for `path` plus a channel of mapped raw events.
fn make_watcher(path: &Path) -> Result<(RecommendedWatcher, UnboundedReceiver<WatchEvent>)> {
    let (raw_tx, raw_rx) = unbounded_channel();
    let handler_tx: UnboundedSender<WatchEvent> = raw_tx;
    let mut backend = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for mapped in map_event(event) {
                // If the receiver is gone the send fails; nothing to do.
                let _ = handler_tx.send(mapped);
            }
        }
    })
    .map_err(|e| OrcaError::Other(format!("failed to create watcher: {e}")))?;

    backend
        .watch(path, RecursiveMode::NonRecursive)
        .map_err(|e| OrcaError::Other(format!("failed to watch {path:?}: {e}")))?;

    Ok((backend, raw_rx))
}

/// Collect events arriving within [`DEBOUNCE_WINDOW`] of `first`.
async fn drain_window(
    rx: &mut UnboundedReceiver<WatchEvent>,
    first: WatchEvent,
) -> Vec<WatchEvent> {
    let mut batch = vec![first];
    let deadline = tokio::time::sleep(DEBOUNCE_WINDOW);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            maybe = rx.recv() => match maybe {
                Some(ev) => batch.push(ev),
                None => break,
            },
        }
    }
    batch
}

/// Map a `notify` event into zero or more [`WatchEvent`]s. Access-only events
/// (reads) produce nothing.
fn map_event(event: notify::Event) -> Vec<WatchEvent> {
    use notify::event::ModifyKind;
    use notify::EventKind;

    match event.kind {
        EventKind::Create(_) => event.paths.into_iter().map(WatchEvent::Created).collect(),
        EventKind::Remove(_) => event.paths.into_iter().map(WatchEvent::Removed).collect(),
        EventKind::Modify(ModifyKind::Name(mode)) => map_rename(mode, event.paths),
        EventKind::Modify(_) => event.paths.into_iter().map(WatchEvent::Modified).collect(),
        // Access and Any/Other events are not actionable for a file manager.
        _ => Vec::new(),
    }
}

/// Translate a rename event (which may report one or both sides) into a
/// [`WatchEvent::Renamed`].
fn map_rename(mode: notify::event::RenameMode, mut paths: Vec<PathBuf>) -> Vec<WatchEvent> {
    use notify::event::RenameMode;
    match mode {
        RenameMode::Both if paths.len() >= 2 => {
            let to = paths.pop();
            let from = paths.pop();
            vec![WatchEvent::Renamed { from, to }]
        }
        RenameMode::From => paths
            .into_iter()
            .map(|p| WatchEvent::Renamed {
                from: Some(p),
                to: None,
            })
            .collect(),
        // `To`, `Any` and any other single-sided report: treat as a new path.
        _ => paths
            .into_iter()
            .map(|p| WatchEvent::Renamed {
                from: None,
                to: Some(p),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RenameMode};
    use notify::{Event, EventKind};
    use std::time::Duration;

    fn ev(kind: EventKind, paths: &[&str]) -> Event {
        Event {
            kind,
            paths: paths.iter().map(PathBuf::from).collect(),
            attrs: Default::default(),
        }
    }

    #[test]
    fn maps_create_and_remove() {
        let c = map_event(ev(EventKind::Create(CreateKind::File), &["/a"]));
        assert_eq!(c, vec![WatchEvent::Created(PathBuf::from("/a"))]);

        let r = map_event(ev(
            EventKind::Remove(notify::event::RemoveKind::File),
            &["/b"],
        ));
        assert_eq!(r, vec![WatchEvent::Removed(PathBuf::from("/b"))]);
    }

    #[test]
    fn maps_modify_and_rename_both() {
        let m = map_event(ev(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            &["/c"],
        ));
        assert_eq!(m, vec![WatchEvent::Modified(PathBuf::from("/c"))]);

        let rn = map_event(ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/old", "/new"],
        ));
        assert_eq!(
            rn,
            vec![WatchEvent::Renamed {
                from: Some(PathBuf::from("/old")),
                to: Some(PathBuf::from("/new")),
            }]
        );
    }

    #[test]
    fn access_events_are_ignored() {
        let a = map_event(ev(
            EventKind::Access(notify::event::AccessKind::Read),
            &["/x"],
        ));
        assert!(a.is_empty());
    }

    #[tokio::test]
    async fn watch_reports_file_creation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut w = watch(dir.path()).expect("watch");

        // Give the watcher a moment to register before mutating.
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::fs::write(dir.path().join("new.txt"), b"x").expect("write");

        let got = tokio::time::timeout(Duration::from_secs(3), w.recv())
            .await
            .expect("timeout waiting for event")
            .expect("channel closed");
        // Any event referencing the new file is acceptable (backends differ on
        // exact kind/order for a create+write).
        let mentions = match &got {
            WatchEvent::Created(p) | WatchEvent::Modified(p) | WatchEvent::Removed(p) => {
                p.ends_with("new.txt")
            }
            WatchEvent::Renamed { from, to } => {
                let m = |o: &Option<PathBuf>| o.as_ref().is_some_and(|p| p.ends_with("new.txt"));
                m(from) || m(to)
            }
        };
        assert!(mentions, "unexpected event: {got:?}");
    }

    #[tokio::test]
    async fn watch_dir_emits_initial_then_refresh() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").expect("write");

        let mut dw =
            watch_dir(dir.path(), FilterOptions::default(), SortKey::Name).expect("watch_dir");

        // Initial listing has the one existing file.
        let initial = tokio::time::timeout(Duration::from_secs(3), dw.recv())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("listing");
        assert_eq!(initial.len(), 1);

        // Create another file; expect a refreshed listing with two entries.
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::fs::write(dir.path().join("b.txt"), b"b").expect("write");

        let refreshed = tokio::time::timeout(Duration::from_secs(3), dw.recv())
            .await
            .expect("timeout")
            .expect("closed")
            .expect("listing");
        assert_eq!(refreshed.len(), 2);
    }
}
