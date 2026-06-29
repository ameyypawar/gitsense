use std::path::Path;

use anyhow::Context as _;

use crate::git::{BlameLine, BlameResult};

/// Compute blame for `[start_line, end_line]` (1-based, inclusive) in `file`.
///
/// `file` may be absolute or repo-relative; an absolute path is stripped to be
/// relative to `repo_root`.  The repo is opened fresh inside this function so
/// no `gix::Repository` (which is `!Send`) ever escapes the call frame.
pub fn blame_range(
    repo_root: &Path,
    file: &Path,
    start_line: usize,
    end_line: usize,
) -> anyhow::Result<BlameResult> {
    // ── Open repo ─────────────────────────────────────────────────────────
    let repo = gix::open(repo_root)
        .with_context(|| format!("opening repo at {}", repo_root.display()))?;

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
    }
}
