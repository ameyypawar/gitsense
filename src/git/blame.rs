use std::path::Path;

use anyhow::Context as _;

use crate::git::{BlameLine, BlameResult};

/// Compute blame for `[start_line, end_line]` (1-based, inclusive) using an
/// already-open `repo`.  `file_rel` must be repo-relative (forward-slash
/// components are acceptable on all platforms).
///
/// Returns only the last-touched Unix timestamp — cheaper than constructing
/// the full [`BlameResult`].  Used by batch callers that open the repo once
/// and blame many files without reopening.
///
/// # Safety / !Send note
/// The caller must ensure this is invoked inside a `spawn_blocking` closure.
/// `gix::Repository` is `!Send`; the `!Send` value never escapes the closure.
pub(crate) fn blame_last_ts_with_repo(
    repo: &gix::Repository,
    file_rel: &Path,
    start_line: usize,
    end_line: usize,
) -> anyhow::Result<i64> {
    let head_id = repo
        .head_id()
        .map_err(|e| anyhow::anyhow!("repository has no commits: {e}"))?;

    let file_str: String = file_rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/");
    let file_bstring: gix::bstr::BString = file_str.as_bytes().to_vec().into();

    let ranges = gix::blame::BlameRanges::from_one_based_inclusive_range(
        start_line as u32..=end_line as u32,
    )
    .map_err(|e| anyhow::anyhow!("invalid line range {start_line}..={end_line}: {e}"))?;

    let options = gix::repository::blame_file::Options {
        ranges,
        ..Default::default()
    };

    let outcome = repo
        .blame_file(file_bstring.as_ref(), head_id, options)
        .with_context(|| {
            format!(
                "blame_file failed for '{}' lines {start_line}..={end_line}",
                file_str
            )
        })?;

    let mut last_ts = i64::MIN;
    for entry in &outcome.entries {
        let commit = repo
            .find_object(entry.commit_id)
            .with_context(|| format!("finding commit {}", entry.commit_id))?
            .into_commit();
        let ts = commit.time()?.seconds;
        if ts > last_ts {
            last_ts = ts;
        }
    }

    if last_ts == i64::MIN {
        anyhow::bail!(
            "no blame entries for '{}' lines {start_line}..={end_line}",
            file_str
        );
    }
    Ok(last_ts)
}

/// Returns `true` when the on-disk worktree content of `file_rel` differs
/// from the blob recorded for it in `head_id`'s tree, or `file_rel` is absent
/// from that tree entirely (new/untracked file).
///
/// `blame_file` (called by [`blame_range`]) always blames the committed HEAD
/// blob, never on-disk content (gix does not support blaming a dirty
/// worktree blob). This check lets callers detect the skew instead of
/// silently mis-attributing lines — see #6.
///
/// Errors (failure to read the worktree file, corrupt tree, hashing failure)
/// are treated as `Ok(true)` by the caller via `.unwrap_or(true)`: when
/// staleness can't be proven false, the conservative answer is "assume
/// dirty" rather than silently claiming the file is clean.
fn worktree_is_dirty(
    repo: &gix::Repository,
    repo_root: &Path,
    file_rel: &Path,
    head_id: gix::Id<'_>,
) -> anyhow::Result<bool> {
    let head_tree = head_id
        .object()
        .with_context(|| format!("resolving HEAD object {head_id}"))?
        .into_commit()
        .tree()
        .with_context(|| format!("loading HEAD tree for {head_id}"))?;

    let head_blob_id = head_tree
        .lookup_entry_by_path(file_rel)
        .with_context(|| format!("looking up '{}' in HEAD tree", file_rel.display()))?
        .map(|entry| entry.object_id());

    let worktree_bytes = match std::fs::read(repo_root.join(file_rel)) {
        Ok(bytes) => bytes,
        // Missing from the worktree (e.g. deleted since HEAD) — can't compare
        // content, so report dirty rather than silently treating it as clean.
        Err(_) => return Ok(true),
    };

    let worktree_blob_id =
        gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, &worktree_bytes)
            .with_context(|| format!("hashing worktree content of '{}'", file_rel.display()))?;

    Ok(head_blob_id != Some(worktree_blob_id))
}

/// Compute blame for `[start_line, end_line]` (1-based, inclusive) in `file`.
///
/// `file` may be absolute or repo-relative; an absolute path is stripped to be
/// relative to `repo_root`.  The repo is opened fresh inside this function so
/// no `gix::Repository` (which is `!Send`) ever escapes the call frame.
///
/// Blame always runs against HEAD, not the on-disk worktree (see
/// `worktree_is_dirty`) — the returned [`BlameResult::worktree_dirty`] flags
/// when the file has uncommitted changes, so line numbers may not correspond
/// to what's currently on disk (#6).
pub fn blame_range(
    repo_root: &Path,
    file: &Path,
    start_line: usize,
    end_line: usize,
) -> anyhow::Result<BlameResult> {
    // ── Open repo ─────────────────────────────────────────────────────────
    let repo =
        gix::open(repo_root).with_context(|| format!("opening repo at {}", repo_root.display()))?;

    // ── Resolve HEAD ──────────────────────────────────────────────────────
    let head_id = repo
        .head_id()
        .map_err(|e| anyhow::anyhow!("repository has no commits to blame: {e}"))?;

    // ── Build gix-relative path (slash-separated) ─────────────────────────
    let file_rel = if file.is_absolute() {
        file.strip_prefix(repo_root).with_context(|| {
            format!(
                "file '{}' is not under repo_root '{}'",
                file.display(),
                repo_root.display()
            )
        })?
    } else {
        file
    };

    // Join with '/' — gix blame requires slash-separated repo-relative paths.
    let file_str: String = file_rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/");

    let file_bstring: gix::bstr::BString = file_str.as_bytes().to_vec().into();

    // ── Build blame options (1-based inclusive range) ─────────────────────
    // `BlameRanges::from_one_based_inclusive_range` handles the 1-based →
    // 0-based exclusive conversion internally, so we pass the user's range
    // (start_line..=end_line, 1-based) directly.
    let ranges = gix::blame::BlameRanges::from_one_based_inclusive_range(
        start_line as u32..=end_line as u32,
    )
    .map_err(|e| anyhow::anyhow!("invalid line range {start_line}..={end_line}: {e}"))?;

    let options = gix::repository::blame_file::Options {
        ranges,
        ..Default::default()
    };

    // ── Run blame ─────────────────────────────────────────────────────────
    let outcome = repo
        .blame_file(file_bstring.as_ref(), head_id, options)
        .with_context(|| {
            format!(
                "blame_file failed for '{}' in '{}'",
                file_str,
                repo_root.display()
            )
        })?;

    if outcome.entries.is_empty() {
        anyhow::bail!(
            "no blame entries for '{}' lines {start_line}..={end_line} \
             (file may be empty or the range is out of bounds)",
            file_str
        );
    }

    // ── Detect worktree/HEAD skew (#6) ────────────────────────────────────
    let worktree_dirty = worktree_is_dirty(&repo, repo_root, file_rel, head_id).unwrap_or(true);

    // ── Build result ──────────────────────────────────────────────────────
    let mut lines: Vec<BlameLine> = Vec::with_capacity(outcome.entries.len());
    let mut last_timestamp = i64::MIN;
    let mut last_author = String::new();
    let mut last_commit_short = String::new();
    let mut last_date = String::new();
    let mut last_message = String::new();

    for entry in &outcome.entries {
        // Resolve commit — always a commit object, so into_commit() is safe.
        let commit = repo
            .find_object(entry.commit_id)
            .with_context(|| format!("finding commit {}", entry.commit_id))?
            .into_commit();

        // Author name
        let author_name = format!("{}", commit.author()?.name);

        // Committer timestamp (gix blame walks by committer time, consistent
        // with `git blame`).
        let time = commit.time()?;
        let timestamp = time.seconds;

        // Human-readable date (YYYY-MM-DD), falls back to unix seconds if the
        // timezone is malformed.
        let date = time.format_or_unix(gix::date::time::format::SHORT);

        // Commit message summary (first line).
        let message_summary = format!("{}", commit.message()?.summary());

        // 7-char short hash — no ODB lookup needed, just truncate.
        let commit_short = entry.commit_id.to_hex_with_len(7).to_string();

        if timestamp > last_timestamp {
            last_timestamp = timestamp;
            last_author = author_name.clone();
            last_commit_short = commit_short.clone();
            last_date = date.clone();
            last_message = message_summary.clone();
        }

        lines.push(BlameLine {
            author: author_name,
            commit_short,
            date,
            timestamp,
            message_summary,
        });
    }

    Ok(BlameResult {
        lines,
        last_author,
        last_commit_short,
        last_date,
        last_timestamp,
        last_message,
        worktree_dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Gate test: proves real gix blame output against gitsense's own history.
    ///
    /// Skipped when there is no `.git` directory (e.g. fresh clone from a
    /// tarball, CI shallow clone without `.git`, or a non-git checkout).
    #[test]
    fn blame_range_gate() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if !repo_root.join(".git").exists() {
            // Not a git repo — skip gracefully.
            return;
        }

        let result = blame_range(&repo_root, Path::new("src/main.rs"), 1, 3)
            .expect("blame_range should succeed on src/main.rs lines 1-3");

        assert!(
            !result.last_author.is_empty(),
            "last_author must be non-empty; got {:?}",
            result.last_author
        );
        assert!(
            !result.last_commit_short.is_empty(),
            "last_commit_short must be non-empty; got {:?}",
            result.last_commit_short
        );
        assert!(
            result.last_timestamp > 0,
            "last_timestamp must be a positive unix epoch; got {}",
            result.last_timestamp
        );

        // #6: don't assert a specific `worktree_dirty` value here — a local
        // dev checkout may legitimately have uncommitted edits to
        // src/main.rs, which would make a hard-coded expectation flaky.
        // Just prove the field exists and serializes.
        let json = serde_json::to_value(&result).expect("BlameResult must serialize to JSON");
        assert!(
            json.get("worktree_dirty").is_some(),
            "worktree_dirty must be present in serialized BlameResult; got {json:?}"
        );
    }

    /// #6: deterministic dirty-vs-clean detection using a throwaway temp repo
    /// (not gitsense's own checkout), so this test never depends on the
    /// state of the dev tree it runs in.
    ///
    /// Requires a `git` binary on PATH to build the fixture — guaranteed on
    /// the CI runner (which itself checked this repo out via git) and on any
    /// realistic dev machine.
    #[test]
    fn blame_range_detects_dirty_worktree() {
        use std::process::Command;

        let repo_root = std::env::temp_dir().join(format!(
            "gitsense-blame-dirty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&repo_root).expect("create temp repo dir");

        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo_root)
                .status()
                .expect("git must be on PATH to run this test");
            assert!(status.success(), "`git {}` failed", args.join(" "));
        };

        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "gitsense test"]);

        let file_path = repo_root.join("hello.rs");
        std::fs::write(&file_path, "fn hello() {}\n").expect("write initial file");
        run(&["add", "hello.rs"]);
        run(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "initial commit",
        ]);

        // Clean worktree: on-disk content matches the committed HEAD blob.
        let clean = blame_range(&repo_root, Path::new("hello.rs"), 1, 1)
            .expect("blame_range should succeed on a clean worktree");
        assert!(
            !clean.worktree_dirty,
            "freshly committed, unmodified file must not be flagged dirty"
        );

        // Dirty worktree: modify the file on disk without committing.
        std::fs::write(&file_path, "fn hello() { /* uncommitted edit */ }\n")
            .expect("modify file on disk");
        let dirty = blame_range(&repo_root, Path::new("hello.rs"), 1, 1)
            .expect("blame_range should still succeed against HEAD despite dirty worktree");
        assert!(
            dirty.worktree_dirty,
            "file with uncommitted on-disk changes must be flagged dirty"
        );

        let _ = std::fs::remove_dir_all(&repo_root);
    }
}
