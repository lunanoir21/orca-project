//! Filename and file-content search.
//!
//! Both entry points stream their results: matches are delivered through a
//! [`tokio::sync::mpsc::Receiver`] as they are found, so a caller can render
//! partial results and stop early by dropping the receiver or cancelling the
//! shared [`CancelToken`].
//!
//! - [`search_by_name`] matches against entry names (substring) or against the
//!   path / name as a glob (`*.rs`, `src/**/*.toml`).
//! - [`search_by_content`] greps file contents using ripgrep's matcher and
//!   searcher crates, skipping binary files automatically.
//!
//! The bare signatures in the roadmap (`search_by_name(root, query)`) are
//! extended with a [`SearchOptions`] argument and a [`CancelToken`]: the options
//! are the only place the case-insensitive / exclude-hidden / glob switches can
//! live, and the token mirrors the cancellation model used by directory
//! listing.

use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use tokio::sync::mpsc::{self, Receiver};
use walkdir::WalkDir;

use crate::error::{OrcaError, Result};
use crate::fs::{entry_from_path, is_hidden_name, CancelToken};
use crate::types::FileEntry;

/// Bound on the result channel. Caps in-flight, un-consumed matches so a slow
/// consumer applies backpressure to the traversal rather than letting memory
/// grow without limit.
const RESULT_CHANNEL_CAP: usize = 256;

/// Tunables shared by [`search_by_name`] and [`search_by_content`].
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Match without regard to case. Applies to name substrings, name globs and
    /// content regexes alike.
    pub case_insensitive: bool,
    /// Skip dot-prefixed entries and prune dot-prefixed directories.
    pub exclude_hidden: bool,
    /// Interpret the name query as a glob pattern rather than a substring.
    /// Ignored by [`search_by_content`].
    pub glob: bool,
    /// Follow symlinks while traversing. Off by default to avoid escaping the
    /// search root and to sidestep symlink cycles.
    pub follow_symlinks: bool,
    /// Maximum traversal depth (the root itself is depth 0). `None` is unbounded.
    pub max_depth: Option<usize>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        // File-manager-friendly defaults: case-insensitive, hide dotfiles, plain
        // substring matching, don't chase symlinks.
        Self {
            case_insensitive: true,
            exclude_hidden: true,
            glob: false,
            follow_symlinks: false,
            max_depth: None,
        }
    }
}

/// How a single name query should be tested against an entry.
enum NameMatcher {
    /// Lower-cased substring (the query is pre-lowered when case-insensitive).
    Substring { needle: String, fold_case: bool },
    /// Glob applied to the entry's relative path (when the pattern has a `/`).
    GlobPath(GlobMatcher),
    /// Glob applied to the entry's file name (when the pattern has no `/`).
    GlobName(GlobMatcher),
}

impl NameMatcher {
    fn build(query: &str, opts: &SearchOptions) -> Result<Self> {
        if opts.glob {
            let glob = GlobBuilder::new(query)
                .case_insensitive(opts.case_insensitive)
                .literal_separator(true)
                .build()
                .map_err(|e| OrcaError::InvalidPath(format!("invalid glob {query:?}: {e}")))?
                .compile_matcher();
            // A pattern containing a separator is matched against the path
            // relative to the root (so `src/**/*.toml` works); a separator-free
            // pattern like `*.rs` is matched against the bare file name so it
            // hits at any depth.
            if query.contains('/') {
                Ok(NameMatcher::GlobPath(glob))
            } else {
                Ok(NameMatcher::GlobName(glob))
            }
        } else if opts.case_insensitive {
            Ok(NameMatcher::Substring {
                needle: query.to_lowercase(),
                fold_case: true,
            })
        } else {
            Ok(NameMatcher::Substring {
                needle: query.to_string(),
                fold_case: false,
            })
        }
    }

    /// `rel` is the entry path relative to the search root; `name` is its final
    /// component.
    fn is_match(&self, rel: &Path, name: &str) -> bool {
        match self {
            NameMatcher::Substring { needle, fold_case } => {
                if *fold_case {
                    name.to_lowercase().contains(needle)
                } else {
                    name.contains(needle)
                }
            }
            NameMatcher::GlobName(g) => g.is_match(name),
            NameMatcher::GlobPath(g) => g.is_match(rel),
        }
    }
}

/// Search the tree rooted at `root` for entries whose name matches `query`.
///
/// Returns immediately with a [`Receiver`] of matches; traversal runs on a
/// blocking task and stops when `cancel` fires or the receiver is dropped.
///
/// # Errors
/// - [`OrcaError::NotADirectory`] if `root` is not a directory.
/// - [`OrcaError::InvalidPath`] if `query` is requested as a glob but malformed.
#[must_use = "the returned receiver is the only way to read search results"]
pub fn search_by_name(
    root: impl AsRef<Path>,
    query: &str,
    opts: &SearchOptions,
    cancel: CancelToken,
) -> Result<Receiver<FileEntry>> {
    let root = validated_root(root.as_ref())?;
    let matcher = NameMatcher::build(query, opts)?;
    let opts = opts.clone();

    let (tx, rx) = mpsc::channel(RESULT_CHANNEL_CAP);
    tokio::task::spawn_blocking(move || {
        for entry in walker(&root, &opts) {
            if cancel.is_cancelled() {
                break;
            }
            let path = entry.path();
            let rel = path.strip_prefix(&root).unwrap_or(path);
            let name = entry.file_name().to_string_lossy();
            if !matcher.is_match(rel, &name) {
                continue;
            }
            match entry_from_path(path) {
                Ok(file) => {
                    if tx.blocking_send(file).is_err() {
                        break; // receiver dropped — caller stopped reading
                    }
                }
                Err(err) => {
                    tracing::warn!(path = ?path, error = %err, "skipping unreadable match");
                }
            }
        }
    });

    Ok(rx)
}

/// Search the tree rooted at `root` for files whose contents match the regex
/// `query`. Binary files are detected and skipped.
///
/// Returns immediately with a [`Receiver`]; each matching file is emitted once,
/// as a [`FileEntry`]. Traversal runs on a blocking task and stops when `cancel`
/// fires or the receiver is dropped.
///
/// # Errors
/// - [`OrcaError::NotADirectory`] if `root` is not a directory.
/// - [`OrcaError::InvalidPath`] if `query` is not a valid regex.
#[must_use = "the returned receiver is the only way to read search results"]
pub fn search_by_content(
    root: impl AsRef<Path>,
    query: &str,
    opts: &SearchOptions,
    cancel: CancelToken,
) -> Result<Receiver<FileEntry>> {
    let root = validated_root(root.as_ref())?;
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(opts.case_insensitive)
        .build(query)
        .map_err(|e| OrcaError::InvalidPath(format!("invalid regex {query:?}: {e}")))?;
    let opts = opts.clone();

    let (tx, rx) = mpsc::channel(RESULT_CHANNEL_CAP);
    tokio::task::spawn_blocking(move || {
        let mut searcher = SearcherBuilder::new()
            // Treat a file as binary (and skip it) at the first NUL byte; this
            // keeps grep off media/executables without reading them whole.
            .binary_detection(BinaryDetection::quit(0))
            .build();

        for entry in walker(&root, &opts) {
            if cancel.is_cancelled() {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !file_contains(&mut searcher, &matcher, path) {
                continue;
            }
            match entry_from_path(path) {
                Ok(file) => {
                    if tx.blocking_send(file).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::warn!(path = ?path, error = %err, "skipping unreadable match");
                }
            }
        }
    });

    Ok(rx)
}

/// Run `searcher` over `path` and report whether `matcher` hits at least once.
/// Stops at the first match (the sink returns `false`) and treats any I/O or
/// decode error as "no match" after logging it, so one bad file cannot abort the
/// whole search.
fn file_contains(
    searcher: &mut grep_searcher::Searcher,
    matcher: &RegexMatcher,
    path: &Path,
) -> bool {
    let mut found = false;
    let sink = UTF8(|_lineno, _line| {
        found = true;
        Ok(false) // one hit is enough; stop scanning this file
    });
    if let Err(err) = searcher.search_path(matcher, path, sink) {
        tracing::warn!(path = ?path, error = %err, "content search skipped file");
        return false;
    }
    found
}

/// Validate that `root` exists and is a directory.
fn validated_root(root: &Path) -> Result<std::path::PathBuf> {
    let meta = std::fs::metadata(root).map_err(|e| OrcaError::from_io(root, e))?;
    if !meta.is_dir() {
        return Err(OrcaError::NotADirectory(root.to_path_buf()));
    }
    Ok(root.to_path_buf())
}

/// Build a [`WalkDir`] iterator honouring the depth / symlink / hidden options,
/// pruning hidden directories so their subtrees are skipped entirely.
fn walker(root: &Path, opts: &SearchOptions) -> impl Iterator<Item = walkdir::DirEntry> {
    let mut walk = WalkDir::new(root).follow_links(opts.follow_symlinks);
    if let Some(depth) = opts.max_depth {
        walk = walk.max_depth(depth);
    }
    let exclude_hidden = opts.exclude_hidden;
    walk.into_iter()
        .filter_entry(move |e| {
            // Never prune the root, even if its own name is dot-prefixed.
            if e.depth() == 0 {
                return true;
            }
            if exclude_hidden {
                if let Some(name) = e.file_name().to_str() {
                    return !is_hidden_name(name);
                }
            }
            true
        })
        .filter_map(|res| match res {
            Ok(entry) => Some(entry),
            Err(err) => {
                tracing::warn!(error = %err, "skipping unreadable path during search");
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write file");
    }

    async fn collect(mut rx: Receiver<FileEntry>) -> Vec<String> {
        let mut names = Vec::new();
        while let Some(e) = rx.recv().await {
            names.push(e.name);
        }
        names.sort();
        names
    }

    #[tokio::test]
    async fn name_substring_is_case_insensitive_by_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("README.md"), "x");
        write(&dir.path().join("notes.txt"), "x");

        let rx = search_by_name(
            dir.path(),
            "readme",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["README.md"]);
    }

    #[tokio::test]
    async fn name_substring_recurses_subdirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("sub")).expect("mkdir");
        write(&dir.path().join("sub/target.log"), "x");

        let rx = search_by_name(
            dir.path(),
            "target",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["target.log"]);
    }

    #[tokio::test]
    async fn glob_name_matches_extension_at_any_depth() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("src")).expect("mkdir");
        write(&dir.path().join("src/main.rs"), "x");
        write(&dir.path().join("top.rs"), "x");
        write(&dir.path().join("readme.md"), "x");

        let opts = SearchOptions {
            glob: true,
            ..Default::default()
        };
        let rx = search_by_name(dir.path(), "*.rs", &opts, CancelToken::new()).expect("search");
        assert_eq!(collect(rx).await, vec!["main.rs", "top.rs"]);
    }

    #[tokio::test]
    async fn glob_path_with_separator_matches_relative_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("src/inner")).expect("mkdir");
        write(&dir.path().join("src/inner/a.toml"), "x");
        write(&dir.path().join("src/b.toml"), "x");
        write(&dir.path().join("root.toml"), "x");

        let opts = SearchOptions {
            glob: true,
            ..Default::default()
        };
        let rx =
            search_by_name(dir.path(), "src/**/*.toml", &opts, CancelToken::new()).expect("search");
        assert_eq!(collect(rx).await, vec!["a.toml", "b.toml"]);
    }

    #[tokio::test]
    async fn excludes_hidden_by_default_and_includes_when_asked() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join(".hidden.txt"), "x");
        write(&dir.path().join("visible.txt"), "x");

        let rx = search_by_name(
            dir.path(),
            ".txt",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["visible.txt"]);

        let opts = SearchOptions {
            exclude_hidden: false,
            ..Default::default()
        };
        let rx = search_by_name(dir.path(), ".txt", &opts, CancelToken::new()).expect("search");
        assert_eq!(collect(rx).await, vec![".hidden.txt", "visible.txt"]);
    }

    #[tokio::test]
    async fn hidden_directory_is_pruned() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join(".git")).expect("mkdir");
        write(&dir.path().join(".git/config.txt"), "x");
        write(&dir.path().join("keep.txt"), "x");

        let rx = search_by_name(
            dir.path(),
            ".txt",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["keep.txt"]);
    }

    #[tokio::test]
    async fn content_search_finds_matching_files_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("hit.txt"), "alpha\nNEEDLE here\nbeta\n");
        write(&dir.path().join("miss.txt"), "nothing relevant\n");

        let rx = search_by_content(
            dir.path(),
            "NEEDLE",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["hit.txt"]);
    }

    #[tokio::test]
    async fn content_search_is_case_insensitive_by_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("f.txt"), "Hello World\n");

        let rx = search_by_content(
            dir.path(),
            "hello",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["f.txt"]);
    }

    #[tokio::test]
    async fn content_search_skips_binary_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        // NUL byte triggers binary detection; the literal bytes won't be grepped.
        fs::write(dir.path().join("bin.dat"), b"AB\x00NEEDLE").expect("write");
        write(&dir.path().join("text.txt"), "NEEDLE\n");

        let rx = search_by_content(
            dir.path(),
            "NEEDLE",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect("search");
        assert_eq!(collect(rx).await, vec!["text.txt"]);
    }

    #[tokio::test]
    async fn invalid_regex_is_rejected_eagerly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = search_by_content(
            dir.path(),
            "(",
            &SearchOptions::default(),
            CancelToken::new(),
        )
        .expect_err("bad regex");
        assert!(matches!(err, OrcaError::InvalidPath(_)));
    }

    #[tokio::test]
    async fn non_directory_root_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("f");
        write(&file, "x");
        let err = search_by_name(&file, "x", &SearchOptions::default(), CancelToken::new())
            .expect_err("not a dir");
        assert!(matches!(err, OrcaError::NotADirectory(_)));
    }

    #[tokio::test]
    async fn cancellation_stops_traversal() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..50 {
            write(&dir.path().join(format!("f{i}.txt")), "x");
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        let rx =
            search_by_name(dir.path(), "f", &SearchOptions::default(), cancel).expect("search");
        // Pre-cancelled: the blocking task should send nothing (or stop at once).
        let names = collect(rx).await;
        assert!(
            names.len() < 50,
            "cancellation should curtail results, got {}",
            names.len()
        );
    }
}
