//! The preview panel (Phase 4.4).
//!
//! A collapsible side panel that previews the single selected entry: images are
//! shown with [`gtk::Picture`], text-like files in a read-only monospace view,
//! and everything else falls back to a type icon. A metadata block (name, type,
//! size, modified, permissions) is always shown.
//!
//! The panel is dependency-light by design: image decoding rides on GDK's
//! built-in pixbuf loaders and text is read directly, so no PDF/video/codec
//! crates are pulled in here (richer media preview belongs to the dedicated
//! thumbnail phase).

use std::path::Path;

use orca_core::{FileEntry, FileKind};
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

use crate::format;
use crate::i18n;

/// File extensions GDK can decode into a picture.
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "tiff", "tif", "avif", "jxl",
];

/// Maximum number of bytes read for a text preview, to keep large files cheap.
const TEXT_PREVIEW_LIMIT: usize = 256 * 1024;

/// Messages the preview panel handles.
#[derive(Debug)]
pub enum PreviewInput {
    /// Show the given entry, or clear the panel when `None`.
    Show(Option<FileEntry>),
}

/// The preview panel component.
pub struct PreviewPanel {
    /// The body container, rebuilt on each [`PreviewInput::Show`].
    body: gtk::Box,
}

impl SimpleComponent for PreviewPanel {
    type Init = ();
    type Input = PreviewInput;
    type Output = ();
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .css_classes(["orca-preview"])
            .build()
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.append(
            &gtk::Label::builder()
                .label(i18n::t("preview.title"))
                .halign(gtk::Align::Start)
                .css_classes(["orca-section-label"])
                .build(),
        );

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .vexpand(true)
            .build();
        root.append(&body);

        let model = PreviewPanel { body };
        model.show_placeholder();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            PreviewInput::Show(entry) => match entry {
                Some(entry) if !entry.kind.is_dir() => self.show_entry(&entry),
                _ => self.show_placeholder(),
            },
        }
    }
}

impl PreviewPanel {
    /// Remove every child currently in the body container.
    fn clear(&self) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
    }

    /// Show the empty-state placeholder (no single file selected).
    fn show_placeholder(&self) {
        self.clear();
        let placeholder = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .build();
        placeholder.append(
            &gtk::Image::builder()
                .icon_name("image-x-generic-symbolic")
                .pixel_size(48)
                .css_classes(["dim-label"])
                .build(),
        );
        placeholder.append(
            &gtk::Label::builder()
                .label(i18n::t("preview.none"))
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["dim-label"])
                .build(),
        );
        self.body.append(&placeholder);
    }

    /// Render the preview for a concrete file entry.
    fn show_entry(&self, entry: &FileEntry) {
        self.clear();
        render_into(&self.body, entry, Some(240));
    }
}

/// Render a preview of `entry` into `target` (appended, not cleared first):
/// image/text/icon body, then the metadata grid. Shared by the side panel and
/// [`crate::quicklook`]'s fullscreen overlay so the two stay in sync.
///
/// `image_height`: fixed picture height for the narrow side panel, or `None`
/// to let the picture expand to fill the available space (Quick Look).
pub(crate) fn render_into(target: &gtk::Box, entry: &FileEntry, image_height: Option<i32>) {
    let ext = entry.extension();
    if ext.as_deref().is_some_and(|e| IMAGE_EXTS.contains(&e)) {
        target.append(&image_view(&entry.path, image_height));
    } else if let Some(text) = read_text(&entry.path) {
        target.append(&text_view(&text));
    } else if is_probably_binary(&entry.path) {
        target.append(&note(&i18n::t("preview.binary")));
    } else {
        target.append(&icon_view(entry.kind));
    }
    target.append(&metadata(entry));
}

/// A picture widget scaled to fit while keeping aspect. With a fixed height it
/// matches the narrow side panel; with `None` it expands to fill its parent.
fn image_view(path: &Path, height: Option<i32>) -> gtk::Picture {
    let picture = gtk::Picture::for_filename(path);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    match height {
        Some(h) => picture.set_height_request(h),
        None => picture.set_vexpand(true),
    }
    picture
}

/// A read-only, scrollable monospace view of `text`.
fn text_view(text: &str) -> gtk::ScrolledWindow {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(6)
        .right_margin(6)
        .top_margin(6)
        .bottom_margin(6)
        .build();
    view.buffer().set_text(text);
    gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .min_content_height(200)
        .css_classes(["orca-preview-text"])
        .child(&view)
        .build()
}

/// A large type icon, used when a file can be neither imaged nor read as text.
fn icon_view(kind: FileKind) -> gtk::Image {
    gtk::Image::builder()
        .icon_name(format::icon_name(kind))
        .pixel_size(64)
        .vexpand(true)
        .css_classes(["dim-label"])
        .build()
}

/// A wrapped, dimmed informational label.
fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .vexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build()
}

/// The metadata grid shown beneath the preview body.
fn metadata(entry: &FileEntry) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(12)
        .css_classes(["orca-preview-meta"])
        .build();

    let rows = [
        (i18n::t("preview.name"), entry.name.clone()),
        (i18n::t("preview.kind"), format::kind(entry.kind).to_owned()),
        (i18n::t("preview.size"), format::size(entry.size)),
        (
            i18n::t("preview.modified"),
            format::modified(entry.modified),
        ),
        (
            i18n::t("preview.perms"),
            format::permissions(entry.permissions),
        ),
    ];
    for (row, (key, value)) in rows.into_iter().enumerate() {
        let key_label = gtk::Label::builder()
            .label(key)
            .halign(gtk::Align::Start)
            .css_classes(["dim-label"])
            .build();
        let value_label = gtk::Label::builder()
            .label(value)
            .halign(gtk::Align::Start)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .hexpand(true)
            .xalign(0.0)
            .build();
        grid.attach(&key_label, 0, row as i32, 1, 1);
        grid.attach(&value_label, 1, row as i32, 1, 1);
    }
    grid
}

/// Read up to [`TEXT_PREVIEW_LIMIT`] bytes and return them as a string when the
/// content is valid UTF-8 with no NUL bytes (a cheap "is this text?" test).
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; TEXT_PREVIEW_LIMIT];
    let read = file.read(&mut buf).ok()?;
    buf.truncate(read);
    if buf.contains(&0) {
        return None;
    }
    String::from_utf8(buf).ok()
}

/// Whether the file's leading bytes look binary (contain a NUL). Used only to
/// pick the message shown when text decoding has already failed.
fn is_probably_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 1024];
    match file.read(&mut buf) {
        Ok(n) => buf[..n].contains(&0),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_utf8_text() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "hello orca").unwrap();
        assert_eq!(read_text(f.path()).as_deref(), Some("hello orca"));
        assert!(!is_probably_binary(f.path()));
    }

    #[test]
    fn rejects_binary_as_text() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&[0u8, 1, 2, 3, 0]).unwrap();
        assert!(read_text(f.path()).is_none());
        assert!(is_probably_binary(f.path()));
    }

    #[test]
    fn image_extensions_recognised() {
        assert!(IMAGE_EXTS.contains(&"png"));
        assert!(IMAGE_EXTS.contains(&"jpeg"));
        assert!(!IMAGE_EXTS.contains(&"txt"));
    }
}
