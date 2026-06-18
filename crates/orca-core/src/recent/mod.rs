//! XDG recent documents (`recently-used.xbel`).
//!
//! Reads and writes the freedesktop bookmark file at
//! `$XDG_DATA_HOME/recently-used.xbel`. The file is shared with other
//! applications, so writes preserve every bookmark block we do not own verbatim
//! and only regenerate the one entry being added/updated. Writes are atomic
//! (temp file + rename) to avoid corrupting a file other apps may read.

use std::path::{Path, PathBuf};

use crate::error::{OrcaError, Result};
use crate::util;

/// A single recent-documents entry parsed from the bookmark file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    /// The decoded filesystem path the `file://` URI refers to.
    pub path: PathBuf,
    /// The raw `file://` URI as stored in the bookmark file.
    pub uri: String,
    /// MIME type recorded for the entry, if any.
    pub mime_type: Option<String>,
    /// When the entry was first added (ISO-8601 string as stored), if present.
    pub added: Option<String>,
    /// When the entry was last modified (ISO-8601 string), if present.
    pub modified: Option<String>,
    /// When the entry was last visited (ISO-8601 string), if present.
    pub visited: Option<String>,
}

/// The XDG application name recorded in our bookmark registrations.
const APP_NAME: &str = "orca";

/// Resolve the path to `recently-used.xbel` under the XDG data directory.
fn recent_file() -> Result<PathBuf> {
    let data = dirs::data_dir()
        .ok_or_else(|| OrcaError::Other("could not resolve XDG data directory".into()))?;
    Ok(data.join("recently-used.xbel"))
}

/// Return up to `limit` recent entries, most-recently-visited first.
///
/// Entries are sorted by their `visited` timestamp descending (ISO-8601 strings
/// sort chronologically); entries without a visited timestamp sort last.
///
/// # Errors
/// I/O errors reading the bookmark file (a missing file yields an empty list).
pub async fn get_recent(limit: usize) -> Result<Vec<RecentEntry>> {
    let path = recent_file()?;
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OrcaError::from_io(&path, e)),
    };

    let mut entries: Vec<RecentEntry> = bookmark_blocks(&text)
        .iter()
        .filter_map(|block| parse_entry(block))
        .collect();

    entries.sort_by(|a, b| b.visited.cmp(&a.visited));
    entries.truncate(limit);
    Ok(entries)
}

/// Add (or refresh) `path` in the recent documents list.
///
/// If an entry for the same path exists, its `visited`/`modified` timestamps and
/// application visit count are updated; otherwise a new bookmark is appended. All
/// other applications' bookmarks are preserved unchanged.
///
/// # Errors
/// I/O errors, or if the path cannot be made absolute.
pub async fn add_recent(path: impl AsRef<Path>) -> Result<()> {
    let abs =
        std::path::absolute(path.as_ref()).map_err(|e| OrcaError::from_io(path.as_ref(), e))?;
    let uri = format!("file://{}", util::percent_encode_path(&abs));
    let now = now_rfc3339_utc();

    let file = recent_file()?;
    let existing = match tokio::fs::read_to_string(&file).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(OrcaError::from_io(&file, e)),
    };

    // Collect every bookmark block that is not ours (different path), verbatim.
    let mut kept: Vec<String> = Vec::new();
    let mut added_stamp = now.clone();
    let mut count = 1u32;
    for block in bookmark_blocks(&existing) {
        match parse_entry(&block) {
            Some(entry) if entry.path == abs => {
                // Preserve the original added timestamp; bump the visit count.
                if let Some(a) = entry.added {
                    added_stamp = a;
                }
                count = extract_count(&block).unwrap_or(0) + 1;
            }
            _ => kept.push(block),
        }
    }

    let our_block = render_bookmark(&uri, &added_stamp, &now, count);
    kept.push(our_block);

    let document = render_document(&kept);
    write_atomic(&file, &document).await
}

/// Clear all recent documents by writing an empty bookmark file.
///
/// # Errors
/// I/O errors writing the bookmark file.
pub async fn clear_recent() -> Result<()> {
    let file = recent_file()?;
    write_atomic(&file, &render_document(&[])).await
}

// --- XBEL rendering -------------------------------------------------------

/// Render a full xbel document wrapping the given bookmark blocks.
fn render_document(blocks: &[String]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<xbel version=\"1.0\"\n      \
xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      \
xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for block in blocks {
        out.push_str(block);
        if !block.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("</xbel>\n");
    out
}

/// Render a single `<bookmark>` block we own, with the standard freedesktop
/// metadata and an application registration for Orca.
fn render_bookmark(uri: &str, added: &str, visited: &str, count: u32) -> String {
    format!(
        "  <bookmark href=\"{uri}\" added=\"{added}\" modified=\"{visited}\" visited=\"{visited}\">\n\
\x20   <info>\n\
\x20     <metadata owner=\"http://freedesktop.org\">\n\
\x20       <mime:mime-type type=\"application/octet-stream\"/>\n\
\x20       <bookmark:applications>\n\
\x20         <bookmark:application name=\"{app}\" exec=\"&apos;{app} %u&apos;\" modified=\"{visited}\" count=\"{count}\"/>\n\
\x20       </bookmark:applications>\n\
\x20     </metadata>\n\
\x20   </info>\n\
\x20 </bookmark>",
        uri = xml_escape(uri),
        added = xml_escape(added),
        visited = xml_escape(visited),
        app = APP_NAME,
        count = count,
    )
}

/// XML-escape a string for use in element text or double-quoted attributes.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Reverse [`xml_escape`] for the five predefined XML entities.
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// --- XBEL parsing ---------------------------------------------------------

/// Split a document into raw `<bookmark>...</bookmark>` blocks. Bookmarks do not
/// nest in xbel, so a simple scan is sufficient and preserves inner content
/// (including other apps' metadata) byte-for-byte.
fn bookmark_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<bookmark ") {
        let after = &rest[start..];
        if let Some(end_rel) = after.find("</bookmark>") {
            let end = end_rel + "</bookmark>".len();
            blocks.push(after[..end].to_string());
            rest = &after[end..];
        } else {
            break;
        }
    }
    blocks
}

/// Parse a bookmark block into a [`RecentEntry`], or `None` if it has no usable
/// `file://` href.
fn parse_entry(block: &str) -> Option<RecentEntry> {
    let open_tag = &block[..block.find('>')?];
    let uri = xml_unescape(&attr(open_tag, "href")?);
    let path = uri.strip_prefix("file://").map(|p| {
        // Strip an optional host part is unnecessary for local file URIs.
        PathBuf::from(util::percent_decode(p))
    })?;

    let added = attr(open_tag, "added").map(|s| xml_unescape(&s));
    let modified = attr(open_tag, "modified").map(|s| xml_unescape(&s));
    let visited = attr(open_tag, "visited").map(|s| xml_unescape(&s));
    let mime_type = mime_of(block);

    Some(RecentEntry {
        path,
        uri,
        mime_type,
        added,
        modified,
        visited,
    })
}

/// Extract a double-quoted attribute value from an opening tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Find the first `<mime:mime-type type="...">` value within a block.
fn mime_of(block: &str) -> Option<String> {
    let idx = block.find("mime-type")?;
    attr(&block[idx..], "type")
}

/// Extract the application visit `count` from a bookmark block, if present.
fn extract_count(block: &str) -> Option<u32> {
    let idx = block.find("count=\"")?;
    attr(&block[idx..], "count").and_then(|c| c.parse().ok())
}

// --- helpers --------------------------------------------------------------

/// Current UTC time formatted as RFC-3339 `YYYY-MM-DDThh:mm:ssZ`.
fn now_rfc3339_utc() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// Write `contents` to `path` atomically (write temp file in the same directory,
/// then rename over the target).
async fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| OrcaError::InvalidPath(format!("no parent for {path:?}")))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| OrcaError::from_io(parent, e))?;
    let tmp = path.with_extension("xbel.tmp");
    tokio::fs::write(&tmp, contents)
        .await
        .map_err(|e| OrcaError::from_io(&tmp, e))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| OrcaError::from_io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::ENV_LOCK;

    #[test]
    fn xml_escape_unescape_roundtrip() {
        let s = "a&b<c>\"d'e";
        assert_eq!(xml_unescape(&xml_escape(s)), s);
    }

    #[test]
    fn parses_glib_style_document() {
        let doc = "<?xml version=\"1.0\"?>\n<xbel>\n\
  <bookmark href=\"file:///home/u/a.txt\" added=\"2024-01-01T00:00:00Z\" \
modified=\"2024-01-02T00:00:00Z\" visited=\"2024-01-02T00:00:00Z\">\n\
    <info><metadata owner=\"http://freedesktop.org\">\
<mime:mime-type type=\"text/plain\"/></metadata></info>\n  </bookmark>\n\
  <bookmark href=\"file:///home/u/b%20c.txt\" visited=\"2024-02-01T00:00:00Z\">\n\
  </bookmark>\n</xbel>\n";
        let blocks = bookmark_blocks(doc);
        assert_eq!(blocks.len(), 2);

        let a = parse_entry(&blocks[0]).expect("a");
        assert_eq!(a.path, PathBuf::from("/home/u/a.txt"));
        assert_eq!(a.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(a.visited.as_deref(), Some("2024-01-02T00:00:00Z"));

        let b = parse_entry(&blocks[1]).expect("b");
        assert_eq!(b.path, PathBuf::from("/home/u/b c.txt"));
    }

    #[tokio::test]
    async fn add_get_clear_cycle() {
        let _guard = ENV_LOCK.lock().await;
        let home = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_DATA_HOME", home.path());

        let work = tempfile::tempdir().expect("work");
        let f = work.path().join("doc.txt");
        std::fs::write(&f, b"x").expect("write");

        add_recent(&f).await.expect("add");
        let recents = get_recent(10).await.expect("get");
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].path, std::path::absolute(&f).expect("abs"));

        // Adding the same path again keeps a single entry.
        add_recent(&f).await.expect("add2");
        assert_eq!(get_recent(10).await.expect("get").len(), 1);

        clear_recent().await.expect("clear");
        assert!(get_recent(10).await.expect("get").is_empty());

        std::env::remove_var("XDG_DATA_HOME");
    }

    #[tokio::test]
    async fn preserves_foreign_bookmarks_and_orders_by_visited() {
        let _guard = ENV_LOCK.lock().await;
        let home = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_DATA_HOME", home.path());

        // Seed a file with a foreign bookmark we must not clobber.
        let seeded = render_document(&[
            "  <bookmark href=\"file:///foreign/old.txt\" visited=\"2020-01-01T00:00:00Z\">\n  </bookmark>".to_string(),
        ]);
        let file = recent_file().expect("file");
        std::fs::create_dir_all(file.parent().unwrap()).expect("mkdir");
        std::fs::write(&file, seeded).expect("seed");

        let work = tempfile::tempdir().expect("work");
        let f = work.path().join("new.txt");
        std::fs::write(&f, b"y").expect("write");
        add_recent(&f).await.expect("add");

        let recents = get_recent(10).await.expect("get");
        assert_eq!(recents.len(), 2, "foreign bookmark must survive");
        // Our just-added entry (recent UTC) sorts before the 2020 foreign one.
        assert_eq!(recents[0].path, std::path::absolute(&f).expect("abs"));
        assert_eq!(recents[1].path, PathBuf::from("/foreign/old.txt"));

        std::env::remove_var("XDG_DATA_HOME");
    }
}
