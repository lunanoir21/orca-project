//! Git working-tree status, used to badge file items in the browser.
//!
//! [`git_status`] reports the status of every changed path under a directory's
//! enclosing repository. It is deliberately forgiving: a directory that is not
//! inside a git repository yields an empty map rather than an error, so callers
//! can call it unconditionally on whatever directory is being shown.
//!
//! `git2` is synchronous, so the work runs on a blocking task.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{OrcaError, Result};

/// The git status of a single working-tree path, collapsed from git's combined
/// index/worktree flags into the single state most useful for a UI badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStatus {
    /// Tracked and unchanged.
    Unmodified,
    /// Content differs from the index or HEAD.
    Modified,
    /// Newly staged (present in the index, not in HEAD).
    Added,
    /// Removed from the index or working tree.
    Deleted,
    /// Detected as a rename.
    Renamed,
    /// Present in the working tree but not tracked.
    Untracked,
    /// Matched a `.gitignore` rule.
    Ignored,
    /// Has unresolved merge conflicts.
    Conflicted,
}

/// Report the git status of paths under `dir`.
///
/// Returns a map from absolute path to [`GitStatus`]. If `dir` is not inside a
/// git repository (or the repository has no working tree), the map is empty.
///
/// # Errors
/// Returns [`OrcaError::Other`] if the repository exists but cannot be queried.
pub async fn git_status(dir: impl AsRef<Path>) -> Result<HashMap<PathBuf, GitStatus>> {
    let dir = dir.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || status_blocking(&dir))
        .await
        .map_err(|e| OrcaError::Other(format!("git status task failed: {e}")))?
}

fn status_blocking(dir: &Path) -> Result<HashMap<PathBuf, GitStatus>> {
    // Discover the repository enclosing `dir`. A missing repo is not an error.
    let repo = match git2::Repository::discover(dir) {
        Ok(repo) => repo,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(OrcaError::Other(format!("git discover failed: {e}"))),
    };

    // A bare repository has no working tree to report statuses against.
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return Ok(HashMap::new());
    };

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| OrcaError::Other(format!("git status query failed: {e}")))?;

    let mut map = HashMap::with_capacity(statuses.len());
    for entry in statuses.iter() {
        let Some(rel) = entry.path() else { continue };
        let abs = workdir.join(rel);
        map.insert(abs, classify(entry.status()));
    }
    Ok(map)
}

/// Collapse git's bitflag [`git2::Status`] into a single [`GitStatus`], in
/// descending order of significance for a badge.
fn classify(status: git2::Status) -> GitStatus {
    if status.is_conflicted() {
        GitStatus::Conflicted
    } else if status.is_index_renamed() || status.is_wt_renamed() {
        GitStatus::Renamed
    } else if status.is_index_new() {
        // Staged addition (in the index, not yet in HEAD).
        GitStatus::Added
    } else if status.is_index_deleted() || status.is_wt_deleted() {
        GitStatus::Deleted
    } else if status.is_index_modified() || status.is_wt_modified() {
        GitStatus::Modified
    } else if status.is_wt_new() {
        // In the working tree but not the index — untracked.
        GitStatus::Untracked
    } else if status.is_ignored() {
        GitStatus::Ignored
    } else {
        GitStatus::Unmodified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Initialise a repo with an initial commit so HEAD exists; returns the repo
    /// root. Configures a throwaway identity so commits succeed in CI.
    fn init_repo(root: &Path) -> git2::Repository {
        let repo = git2::Repository::init(root).expect("init");
        {
            let mut cfg = repo.config().expect("config");
            cfg.set_str("user.name", "Orca Test").expect("name");
            cfg.set_str("user.email", "test@orca.invalid")
                .expect("email");
        }
        repo
    }

    fn commit_all(repo: &git2::Repository, msg: &str) {
        let mut index = repo.index().expect("index");
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .expect("add");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let sig = repo.signature().expect("sig");
        let parents = match repo.head().ok().and_then(|h| h.target()) {
            Some(oid) => vec![repo.find_commit(oid).expect("parent")],
            None => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
            .expect("commit");
    }

    #[tokio::test]
    async fn non_repo_returns_empty_map() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("f.txt"), b"x").expect("write");
        let map = git_status(dir.path()).await.expect("status");
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn detects_untracked_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("tracked.txt"), b"v1").expect("write");
        commit_all(&repo, "init");
        // New file, never staged.
        fs::write(dir.path().join("new.txt"), b"hello").expect("write");

        let map = git_status(dir.path()).await.expect("status");
        assert_eq!(
            map.get(&dir.path().join("new.txt")).copied(),
            Some(GitStatus::Untracked)
        );
    }

    #[tokio::test]
    async fn detects_modified_tracked_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("tracked.txt"), b"v1").expect("write");
        commit_all(&repo, "init");
        // Modify the committed file.
        fs::write(dir.path().join("tracked.txt"), b"v2 changed").expect("write");

        let map = git_status(dir.path()).await.expect("status");
        assert_eq!(
            map.get(&dir.path().join("tracked.txt")).copied(),
            Some(GitStatus::Modified)
        );
    }

    #[tokio::test]
    async fn detects_ignored_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = init_repo(dir.path());
        fs::write(dir.path().join(".gitignore"), b"*.log\n").expect("write");
        commit_all(&repo, "init");
        fs::write(dir.path().join("debug.log"), b"noise").expect("write");

        let map = git_status(dir.path()).await.expect("status");
        assert_eq!(
            map.get(&dir.path().join("debug.log")).copied(),
            Some(GitStatus::Ignored)
        );
    }

    #[tokio::test]
    async fn detects_staged_addition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = init_repo(dir.path());
        fs::write(dir.path().join("seed.txt"), b"seed").expect("write");
        commit_all(&repo, "init");
        // Stage a brand-new file without committing.
        fs::write(dir.path().join("staged.txt"), b"data").expect("write");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("staged.txt")).expect("add");
        index.write().expect("write");

        let map = git_status(dir.path()).await.expect("status");
        assert_eq!(
            map.get(&dir.path().join("staged.txt")).copied(),
            Some(GitStatus::Added)
        );
    }
}
