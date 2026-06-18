//! Bulk rename engine: regex find/replace with template variables, a dry-run
//! preview, conflict detection and a safe two-phase apply.
//!
//! A [`BulkRenameRule`] describes how to turn each input file name into a new
//! one. [`preview_rename`] computes the results without touching disk (it only
//! *reads* to detect pre-existing targets); [`apply_rename`] performs the
//! renames, refusing the whole batch if any conflict is detected.
//!
//! Template variables expanded in the replacement string:
//! - `{name}` — the original file stem (name without its extension)
//! - `{ext}`  — the original extension (without the dot; empty if none)
//! - `{n}`    — the per-file counter, zero-padded to [`BulkRenameRule::counter_width`]
//! - `{date}` — today's date as `YYYY-MM-DD`
//!
//! When [`BulkRenameRule::find`] is set, the expanded template is used as the
//! regex *replacement* (so capture groups like `$1` work) applied to the
//! original name; when it is `None`, the expanded template becomes the entire
//! new name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::error::{OrcaError, Result};

/// A bulk-rename transformation applied to a list of paths.
#[derive(Debug, Clone)]
pub struct BulkRenameRule {
    /// Optional regex matched against the original file name. `None` means the
    /// expanded template replaces the whole name.
    pub find: Option<String>,
    /// Replacement template. May contain the `{name}`/`{ext}`/`{n}`/`{date}`
    /// variables and, when `find` is set, regex capture references (`$1`).
    pub replace: String,
    /// Compile `find` case-insensitively.
    pub case_insensitive: bool,
    /// First value of the `{n}` counter.
    pub counter_start: usize,
    /// Amount the counter advances per file.
    pub counter_step: usize,
    /// Zero-pad width for `{n}` (e.g. width 3 renders `7` as `007`).
    pub counter_width: usize,
}

impl Default for BulkRenameRule {
    fn default() -> Self {
        Self {
            find: None,
            replace: String::new(),
            case_insensitive: false,
            counter_start: 1,
            counter_step: 1,
            counter_width: 1,
        }
    }
}

/// Why a proposed new name cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameConflict {
    /// The new name is empty.
    Empty,
    /// The new name contains a path separator or NUL byte.
    InvalidName,
    /// Two or more inputs map to the same new name.
    DuplicateTarget,
    /// The target already exists on disk and is not itself part of this batch.
    TargetExists,
}

/// One row of a [`preview_rename`] result.
#[derive(Debug, Clone)]
pub struct RenamePreview {
    /// The original path.
    pub original: PathBuf,
    /// The proposed new file name (final component only).
    pub new_name: String,
    /// The proposed new full path (same parent directory as `original`).
    pub new_path: PathBuf,
    /// `Some` if this row cannot be applied as-is.
    pub conflict: Option<RenameConflict>,
}

/// The outcome of one successful rename in [`apply_rename`].
#[derive(Debug, Clone)]
pub struct RenameResult {
    /// Where the file used to be.
    pub original: PathBuf,
    /// Where the file now is.
    pub new_path: PathBuf,
}

/// Compute the new name for each path under `rule`, flagging conflicts, without
/// renaming anything.
///
/// The returned vector is parallel to `paths`. Existence checks stat the
/// filesystem but never modify it.
///
/// # Errors
/// Returns [`OrcaError::InvalidPath`] if `rule.find` is not a valid regex.
pub fn preview_rename(paths: &[PathBuf], rule: &BulkRenameRule) -> Result<Vec<RenamePreview>> {
    let regex = compile_find(rule)?;
    let date = today_string();

    // First pass: build proposed names.
    let mut previews: Vec<RenamePreview> = Vec::with_capacity(paths.len());
    let mut counter = rule.counter_start;
    for path in paths {
        let (stem, ext) = split_name(path);
        let counter_str = format_counter(counter, rule.counter_width);
        let template = expand_vars(&rule.replace, &stem, &ext, &counter_str, &date);

        let new_name = match &regex {
            Some(re) => {
                let original_name = file_name_str(path);
                re.replace_all(&original_name, template.as_str())
                    .into_owned()
            }
            None => template,
        };

        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let new_path = parent.join(&new_name);
        previews.push(RenamePreview {
            original: path.clone(),
            new_name,
            new_path,
            conflict: None,
        });
        counter = counter.saturating_add(rule.counter_step);
    }

    detect_conflicts(paths, &mut previews);
    Ok(previews)
}

/// Apply `rule` to `paths`, renaming files on disk.
///
/// Refuses the entire batch (renaming nothing) if [`preview_rename`] reports any
/// conflict, so a bulk rename is all-or-nothing. Renames go through unique
/// temporary names first, so cycles and swaps (`a`→`b`, `b`→`a`) are handled
/// correctly.
///
/// # Errors
/// - [`OrcaError::InvalidPath`] if the regex is invalid or any new name is unusable.
/// - [`OrcaError::AlreadyExists`] / I/O errors surfaced from the rename syscalls.
pub async fn apply_rename(paths: &[PathBuf], rule: &BulkRenameRule) -> Result<Vec<RenameResult>> {
    let previews = preview_rename(paths, rule)?;
    if let Some(bad) = previews.iter().find(|p| p.conflict.is_some()) {
        return Err(OrcaError::InvalidPath(format!(
            "bulk rename aborted: {:?} for {}",
            bad.conflict.as_ref().expect("checked"),
            bad.new_name
        )));
    }

    // Phase 1: move every source to a unique temporary sibling so no later
    // rename can collide with a not-yet-moved source.
    let mut temps: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(previews.len());
    for (i, p) in previews.iter().enumerate() {
        // Skip no-op renames (name unchanged) to avoid needless churn.
        if p.original == p.new_path {
            continue;
        }
        let parent = p.original.parent().unwrap_or_else(|| Path::new(""));
        let tmp = parent.join(format!(".orca-rename-tmp-{i}-{}", std::process::id()));
        tokio::fs::rename(&p.original, &tmp)
            .await
            .map_err(|e| OrcaError::from_io(&p.original, e))?;
        temps.push((tmp, p.new_path.clone()));
    }

    // Phase 2: move each temp to its final destination.
    let mut results = Vec::with_capacity(previews.len());
    for (tmp, dest) in temps {
        tokio::fs::rename(&tmp, &dest)
            .await
            .map_err(|e| OrcaError::from_io(&dest, e))?;
        results.push(RenameResult {
            original: tmp,
            new_path: dest,
        });
    }
    // Repair `original` fields (phase 1 lost them); map by destination order.
    let mut out = Vec::with_capacity(previews.len());
    let mut idx = 0;
    for p in &previews {
        if p.original == p.new_path {
            continue;
        }
        out.push(RenameResult {
            original: p.original.clone(),
            new_path: results[idx].new_path.clone(),
        });
        idx += 1;
    }
    Ok(out)
}

/// Compile the optional find regex, honouring the case-insensitive flag.
fn compile_find(rule: &BulkRenameRule) -> Result<Option<Regex>> {
    match &rule.find {
        None => Ok(None),
        Some(pat) => {
            let re = regex::RegexBuilder::new(pat)
                .case_insensitive(rule.case_insensitive)
                .build()
                .map_err(|e| OrcaError::InvalidPath(format!("invalid find regex {pat:?}: {e}")))?;
            Ok(Some(re))
        }
    }
}

/// Split a path's file name into `(stem, extension)`, where extension excludes
/// the dot and is empty when there is none (and for dotfiles like `.bashrc`).
fn split_name(path: &Path) -> (String, String) {
    let name = file_name_str(path);
    match name.rfind('.') {
        // Leading dot (dotfile) is part of the stem, not an extension marker.
        Some(idx) if idx > 0 => (name[..idx].to_string(), name[idx + 1..].to_string()),
        _ => (name, String::new()),
    }
}

/// The final path component as an owned `String` (lossy for non-UTF-8).
fn file_name_str(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Substitute the `{name}`/`{ext}`/`{n}`/`{date}` variables in `template`.
fn expand_vars(template: &str, name: &str, ext: &str, counter: &str, date: &str) -> String {
    template
        .replace("{name}", name)
        .replace("{ext}", ext)
        .replace("{n}", counter)
        .replace("{date}", date)
}

/// Render the counter zero-padded to at least `width` digits.
fn format_counter(value: usize, width: usize) -> String {
    format!("{value:0width$}")
}

/// Today's local date as `YYYY-MM-DD`, falling back to UTC if the local offset
/// is unavailable.
fn today_string() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let date = now.date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// Fill in the `conflict` field of each preview: invalid names, duplicate
/// targets within the batch, and pre-existing on-disk targets.
fn detect_conflicts(inputs: &[PathBuf], previews: &mut [RenamePreview]) {
    // Count how many previews map to each destination path to find duplicates.
    let mut dest_counts: HashMap<&Path, usize> = HashMap::new();
    for p in previews.iter() {
        *dest_counts.entry(p.new_path.as_path()).or_insert(0) += 1;
    }
    // Set of source paths so we can tell "target already exists" apart from
    // "target is another input that will itself be moved away".
    let input_set: std::collections::HashSet<&Path> = inputs.iter().map(PathBuf::as_path).collect();

    // Collect duplicate dests up front (borrow ends before the &mut loop).
    let duplicates: std::collections::HashSet<PathBuf> = dest_counts
        .iter()
        .filter(|(_, &c)| c > 1)
        .map(|(p, _)| p.to_path_buf())
        .collect();

    for p in previews.iter_mut() {
        p.conflict = if p.new_name.is_empty() {
            Some(RenameConflict::Empty)
        } else if p.new_name.contains('/') || p.new_name.contains('\0') {
            Some(RenameConflict::InvalidName)
        } else if duplicates.contains(&p.new_path) {
            Some(RenameConflict::DuplicateTarget)
        } else if p.new_path != p.original
            && !input_set.contains(p.new_path.as_path())
            && p.new_path.exists()
        {
            Some(RenameConflict::TargetExists)
        } else {
            None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn paths(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| {
                let p = dir.join(n);
                fs::File::create(&p).expect("create");
                p
            })
            .collect()
    }

    #[test]
    fn split_name_handles_extension_and_dotfiles() {
        assert_eq!(
            split_name(Path::new("/a/photo.JPG")),
            ("photo".to_string(), "JPG".to_string())
        );
        assert_eq!(
            split_name(Path::new("/a/Makefile")),
            ("Makefile".to_string(), String::new())
        );
        assert_eq!(
            split_name(Path::new("/a/.bashrc")),
            (".bashrc".to_string(), String::new())
        );
    }

    #[test]
    fn template_counter_and_vars_expand() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["a.txt", "b.txt"]);
        let rule = BulkRenameRule {
            replace: "file_{n}.{ext}".to_string(),
            counter_start: 1,
            counter_width: 3,
            ..Default::default()
        };
        let prev = preview_rename(&ps, &rule).expect("preview");
        assert_eq!(prev[0].new_name, "file_001.txt");
        assert_eq!(prev[1].new_name, "file_002.txt");
        assert!(prev.iter().all(|p| p.conflict.is_none()));
    }

    #[test]
    fn regex_find_replace_with_capture_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["IMG_1234.jpg"]);
        let rule = BulkRenameRule {
            find: Some(r"IMG_(\d+)".to_string()),
            replace: "photo-$1".to_string(),
            ..Default::default()
        };
        let prev = preview_rename(&ps, &rule).expect("preview");
        assert_eq!(prev[0].new_name, "photo-1234.jpg");
    }

    #[test]
    fn detects_duplicate_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["a.txt", "b.txt"]);
        let rule = BulkRenameRule {
            replace: "same.txt".to_string(),
            ..Default::default()
        };
        let prev = preview_rename(&ps, &rule).expect("preview");
        assert_eq!(prev[0].conflict, Some(RenameConflict::DuplicateTarget));
        assert_eq!(prev[1].conflict, Some(RenameConflict::DuplicateTarget));
    }

    #[test]
    fn detects_preexisting_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["a.txt"]);
        fs::File::create(dir.path().join("taken.txt")).expect("create");
        let rule = BulkRenameRule {
            replace: "taken.txt".to_string(),
            ..Default::default()
        };
        let prev = preview_rename(&ps, &rule).expect("preview");
        assert_eq!(prev[0].conflict, Some(RenameConflict::TargetExists));
    }

    #[test]
    fn invalid_regex_errors() {
        let rule = BulkRenameRule {
            find: Some("(".to_string()),
            ..Default::default()
        };
        let err = preview_rename(&[], &rule).expect_err("bad regex");
        assert!(matches!(err, OrcaError::InvalidPath(_)));
    }

    #[tokio::test]
    async fn apply_renames_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["one.txt", "two.txt"]);
        let rule = BulkRenameRule {
            replace: "f{n}.{ext}".to_string(),
            counter_width: 2,
            ..Default::default()
        };
        let res = apply_rename(&ps, &rule).await.expect("apply");
        assert_eq!(res.len(), 2);
        assert!(dir.path().join("f01.txt").exists());
        assert!(dir.path().join("f02.txt").exists());
        assert!(!dir.path().join("one.txt").exists());
    }

    #[tokio::test]
    async fn apply_shift_into_existing_source_via_two_phase() {
        // A "shift" maps each file onto a name currently held by another file in
        // the batch (1->2, 2->3). The intermediate temp phase is what lets this
        // succeed; a naive one-pass rename would clobber file 2 before moving it.
        let dir = tempfile::tempdir().expect("tempdir");
        let f1 = dir.path().join("1.txt");
        let f2 = dir.path().join("2.txt");
        fs::write(&f1, b"ONE").expect("write");
        fs::write(&f2, b"TWO").expect("write");
        let rule = BulkRenameRule {
            replace: "{n}.{ext}".to_string(),
            counter_start: 2, // 1.txt -> 2.txt, 2.txt -> 3.txt
            ..Default::default()
        };
        // Preview: dest 2.txt is another input (in-batch) so no conflict.
        let prev = preview_rename(&[f1.clone(), f2.clone()], &rule).expect("preview");
        assert!(prev.iter().all(|p| p.conflict.is_none()));

        let res = apply_rename(&[f1, f2], &rule).await.expect("apply");
        assert_eq!(res.len(), 2);
        assert_eq!(fs::read(dir.path().join("2.txt")).expect("read"), b"ONE");
        assert_eq!(fs::read(dir.path().join("3.txt")).expect("read"), b"TWO");
        assert!(!dir.path().join("1.txt").exists());
    }

    #[tokio::test]
    async fn apply_refuses_on_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ps = paths(dir.path(), &["a.txt", "b.txt"]);
        let rule = BulkRenameRule {
            replace: "dup.txt".to_string(),
            ..Default::default()
        };
        let err = apply_rename(&ps, &rule).await.expect_err("conflict");
        assert!(matches!(err, OrcaError::InvalidPath(_)));
        // Nothing renamed.
        assert!(dir.path().join("a.txt").exists());
        assert!(dir.path().join("b.txt").exists());
    }
}
