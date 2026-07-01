// ─────────────────────────────────────────────────────────────────────────────
// !Send boundary — read before touching this module
//
// `gix::Repository` is !Send because it contains Rc internally.
// Every public function in this module MUST:
//   1. Be synchronous.
//   2. Accept an OWNED `repo_root: &Path` (or PathBuf) as its first arg and
//      open the repo inside the function body via `gix::open(repo_root)`.
//   3. Do ALL gix work inside the function body and return only OWNED, Send
//      plain structs (defined below).
//   4. Never return any gix type — not Repository, not Tree, not Id, nothing.
//
// Async callers (Phase 6) will wrap these in `tokio::task::spawn_blocking`.
// `spawn_blocking` requires `Send + 'static` closures. These sync fns satisfy
// that requirement: the closure captures only `Send` data (PathBuf, usize,
// etc.) and returns only `Send` data. The gix types live entirely within the
// lexical scope of the sync fn and are dropped before the fn returns.
//
// Gotcha: calling `drop(repo)` before an `.await` is NOT sufficient in general
// because the compiler cannot see through it in all cases. Lexical scope
// ending is the reliable mechanism — keep every gix value local.
// ─────────────────────────────────────────────────────────────────────────────

use serde::Serialize;

/// Per-hunk blame attribution for a line (or contiguous range of lines) in a
/// file.  One `BlameLine` corresponds to one `BlameEntry` returned by gix.
#[derive(Debug, Clone, Serialize)]
pub struct BlameLine {
    /// Author name from the commit that introduced these lines.
    pub author: String,
    /// 7-character hex prefix of the commit id.
    pub commit_short: String,
    /// ISO-ish date string, e.g. "2025-11-03".
    pub date: String,
    /// Unix timestamp (seconds since epoch) of the committer time; used for
    /// "days ago" arithmetic by callers.
    pub timestamp: i64,
    /// First line of the commit message (summary).
    pub message_summary: String,
}

/// The result of a `blame_range` call.
///
/// `last_*` fields identify the **most recently committed** hunk among all
/// hunks that overlap the requested line range.  This is what callers use
/// when they want a single "freshness" signal for a symbol.
#[derive(Debug, Clone, Serialize)]
pub struct BlameResult {
    /// One entry per blame hunk overlapping the requested range.
    pub lines: Vec<BlameLine>,

    // Convenience fields for the most-recent commit in the range.
    pub last_author: String,
    pub last_commit_short: String,
    pub last_date: String,
    pub last_timestamp: i64,
    pub last_message: String,

    /// `true` when the on-disk worktree content of the blamed file differs
    /// from the blob committed at HEAD, or the path is absent from HEAD
    /// entirely (new/untracked file). `blame_range` always blames the
    /// committed HEAD blob, never on-disk content — when this is `true`,
    /// `lines` reflects HEAD, and line numbers/attribution may not match
    /// what's currently on disk. See #6.
    pub worktree_dirty: bool,
}

pub mod blame;
pub mod history;
