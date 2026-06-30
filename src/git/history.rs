use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::git::blame::{blame_last_ts_with_repo, blame_range};

/// Return the Unix timestamp (seconds) of the most recent commit that touched
/// the given `line_range` (1-based inclusive) in `file`.
///
/// Cheapest path: delegates to `blame_range` which already computes
/// `last_timestamp`.  No separate rev-walk needed.
pub fn last_touched(
    repo_root: &Path,
    file: &Path,
    line_range: (usize, usize),
) -> anyhow::Result<i64> {
    let result = blame_range(repo_root, file, line_range.0, line_range.1).with_context(|| {
        format!(
            "last_touched: blame failed for '{}' lines {}..={}",
            file.display(),
            line_range.0,
            line_range.1,
        )
    })?;
    Ok(result.last_timestamp)
}

/// Open the repo once and return the last-touched Unix timestamp for each
/// `(file_rel, line_range)` pair.  `None` is returned for any item where
/// blame fails (new file, empty range, no commits, etc.).
///
/// # Purpose
/// Used by `find_dead_code` to enrich many candidates in a single
/// `spawn_blocking` closure with a single `gix::open` rather than one per
/// candidate.  The `!Send` `gix::Repository` is created and dropped entirely
/// inside this synchronous function.
pub fn last_touched_all(repo_root: &Path, items: &[(PathBuf, (usize, usize))]) -> Vec<Option<i64>> {
    if items.is_empty() {
        return Vec::new();
    }
    let repo = match gix::open(repo_root) {
        Ok(r) => r,
        Err(_) => return vec![None; items.len()],
    };
    items
        .iter()
        .map(|(file, lr)| blame_last_ts_with_repo(&repo, file, lr.0, lr.1).ok())
        .collect()
}

/// Count how many commits in HEAD's history touched each file in `files`.
///
/// # Algorithm (v0 — simplified, documented)
///
/// For each of the first `COMMIT_CAP` commits reachable from HEAD (first-parent
/// only, to keep this O(n) on `main`-style branches):
///  1. Retrieve the commit's tree.
///  2. Retrieve the first parent's tree (or treat it as empty for root commits).
///  3. For each target file, look up its blob OID in both trees.
///  4. If the blob OID differs (or the file exists only in one tree), count
///     the commit as "touching" that file.
///
/// # Limitations
///  - Only follows the first parent.  Merge commits that bring in a file via
///    a non-first parent are undercounted.
///  - Capped at 500 commits.  Files with older initial commits are still
///    counted, but their creation commit is missed if it is beyond the cap.
///  - No rename tracking.  A file renamed from `old.rs` to `new.rs` appears
///    to have been deleted and re-created.
///
/// # Returns
///
/// Pairs `(file, count)` sorted by count descending.  Files with zero commits
/// in the walked range are omitted.
pub fn file_churn(repo_root: &Path, files: &[PathBuf]) -> anyhow::Result<Vec<(PathBuf, usize)>> {
    // Cap the revwalk to bound latency on large repositories.
    const COMMIT_CAP: usize = 500;

    let repo =
        gix::open(repo_root).with_context(|| format!("opening repo at {}", repo_root.display()))?;

    let head_commit = match repo.head_commit() {
        Ok(c) => c,
        // Empty repo — return zero counts rather than an error.
        Err(_) => return Ok(vec![]),
    };

    // Build relative paths for comparison (strip repo_root prefix if absolute).
    let rel_paths: Vec<(PathBuf, PathBuf)> = files
        .iter()
        .map(|f| {
            let rel = if f.is_absolute() {
                f.strip_prefix(repo_root)
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| f.clone())
            } else {
                f.clone()
            };
            (f.clone(), rel)
        })
        .collect();

    let mut counts: HashMap<PathBuf, usize> = HashMap::new();

    let walk = head_commit
        .ancestors()
        .first_parent_only()
        .all()
        .context("building commit walk")?;

    for (idx, info_res) in walk.enumerate() {
        if idx >= COMMIT_CAP {
            break;
        }

        let info = info_res.context("walking commit graph")?;
        let commit = info.object().context("loading commit object")?;
        let commit_tree = commit.tree().context("loading commit tree")?;

        // Load first-parent's tree; None for root commits.
        let first_parent_id = commit.parent_ids().next();
        let parent_tree_opt: Option<gix::Tree<'_>> = match first_parent_id {
            Some(pid) => {
                let parent_obj = pid.object().context("loading parent commit")?;
                let parent_commit = parent_obj.into_commit();
                Some(parent_commit.tree().context("loading parent tree")?)
            }
            None => None,
        };

        for (orig_path, rel_path) in &rel_paths {
            // Look up the blob OID in this commit's tree.
            let current_oid = commit_tree
                .lookup_entry_by_path(rel_path)
                .with_context(|| format!("looking up '{}' in commit tree", rel_path.display()))?
                .map(|e| e.object_id());

            // Look up the blob OID in the parent's tree.
            let parent_oid = match &parent_tree_opt {
                Some(pt) => pt
                    .lookup_entry_by_path(rel_path)
                    .with_context(|| format!("looking up '{}' in parent tree", rel_path.display()))?
                    .map(|e| e.object_id()),
                None => None,
            };

            if current_oid != parent_oid {
                *counts.entry(orig_path.clone()).or_insert(0) += 1;
            }
        }
    }

    // Sort by count descending.
    let mut result: Vec<(PathBuf, usize)> = counts.into_iter().collect();
    result.sort_by_key(|b| std::cmp::Reverse(b.1));
    Ok(result)
}
