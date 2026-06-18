//! Image thumbnails for file cells (Phase 4.5).
//!
//! Thumbnails are produced on demand from image files with GDK's built-in
//! pixbuf loaders — no extra decoding crates. Decoded textures are cached
//! (keyed by path + modification time + requested size) so scrolling and view
//! switches reuse them, and a stale cache entry is replaced when a file's mtime
//! changes. Loading happens on a `glib` idle callback so a directory full of
//! images never blocks the bind that requested them.
//!
//! GDK textures are `GObject`s and therefore main-thread only, so the cache is
//! `thread_local` and every access happens on the GTK main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use relm4::gtk;
use relm4::gtk::prelude::*;

/// Image file extensions GDK can decode.
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tiff", "tif", "avif", "jxl",
];

/// Files larger than this are not thumbnailed (decoding a very large image to a
/// tiny preview is not worth the latency); they keep their type icon.
const MAX_THUMBNAIL_BYTES: u64 = 32 * 1024 * 1024;

/// Cache key: the file, its modification time (seconds) and the requested edge
/// length, so a re-saved file or a different view size gets a fresh thumbnail.
type Key = (PathBuf, u64, i32);

thread_local! {
    static CACHE: RefCell<HashMap<Key, gtk::gdk::Texture>> = RefCell::new(HashMap::new());
}

/// Whether `ext` (lowercase, no dot) is a thumbnailable image type.
#[must_use]
pub fn is_image(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

/// Request a thumbnail for `path` into `image`, sized to `size` pixels.
///
/// Does nothing for non-images or oversized files (the caller's type icon
/// stays). A cached texture is applied immediately; otherwise the decode is
/// deferred to an idle callback, and the result is applied only if
/// `still_current` still returns `true` then (guarding against cell recycling).
pub fn apply<F>(
    image: &gtk::Image,
    path: &Path,
    mtime: u64,
    size: i32,
    bytes: u64,
    still_current: F,
) where
    F: Fn() -> bool + 'static,
{
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    if !ext.as_deref().is_some_and(is_image) || bytes > MAX_THUMBNAIL_BYTES {
        return;
    }

    let key: Key = (path.to_path_buf(), mtime, size);
    if let Some(texture) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        image.set_paintable(Some(&texture));
        image.set_pixel_size(size);
        return;
    }

    let image_weak = image.downgrade();
    gtk::glib::idle_add_local_once(move || {
        let Some(image) = image_weak.upgrade() else {
            return;
        };
        let (path, _mtime, size) = &key;
        let Some(texture) = load_texture(path, *size) else {
            return;
        };
        CACHE.with(|c| c.borrow_mut().insert(key.clone(), texture.clone()));
        if still_current() {
            image.set_paintable(Some(&texture));
            image.set_pixel_size(*size);
        }
    });
}

/// Decode `path` scaled to fit `size`×`size` (aspect preserved) into a texture.
fn load_texture(path: &Path, size: i32) -> Option<gtk::gdk::Texture> {
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true).ok()?;
    Some(gtk::gdk::Texture::for_pixbuf(&pixbuf))
}

/// The modification time of an entry as whole seconds since the Unix epoch, or
/// `0` when unavailable — a stable cache-key component either way.
#[must_use]
pub fn mtime_secs(modified: Option<SystemTime>) -> u64 {
    modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_image_extensions() {
        assert!(is_image("png"));
        assert!(is_image("jpeg"));
        assert!(!is_image("txt"));
        assert!(!is_image("rs"));
    }

    #[test]
    fn mtime_none_is_zero() {
        assert_eq!(mtime_secs(None), 0);
    }

    #[test]
    fn mtime_known_value() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        assert_eq!(mtime_secs(Some(t)), 1_000_000);
    }
}
