pub mod params;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content},
    tool, tool_handler, tool_router, ErrorData, ServerHandler,
};
use serde::Serialize;

use crate::graph::{self, Direction};
use crate::index::model::SymbolKind;
use crate::index::SymbolIndex;

use params::{
    BlameSymbolParams, CallGraphParams, FindDeadCodeParams, FindReferencesParams,
    RepoOverviewParams, SearchSymbolsParams,
};

// ── Shared state ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub index: Arc<SymbolIndex>,
    pub repo_root: PathBuf,
}

// ── Server type ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GitSenseServer {
    state: Arc<AppState>,
}

impl GitSenseServer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

// ── Error helper ──────────────────────────────────────────────────────────────

fn to_mcp_err(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

// ── Kind string → SymbolKind ──────────────────────────────────────────────────

fn parse_kind(s: &str) -> Option<SymbolKind> {
    match s.to_lowercase().as_str() {
        "fn" => Some(SymbolKind::Fn),
        "method" => Some(SymbolKind::Method),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "trait" => Some(SymbolKind::Trait),
        "impl" => Some(SymbolKind::Impl),
        "mod" => Some(SymbolKind::Mod),
        "const" => Some(SymbolKind::Const),
        "macro" => Some(SymbolKind::Macro),
        "other" => Some(SymbolKind::Other),
        _ => None,
    }
}

// ── Direction string → Direction ──────────────────────────────────────────────

fn parse_direction(s: &str) -> Direction {
    match s.to_lowercase().as_str() {
        "callees" => Direction::Callees,
        "callers" => Direction::Callers,
        _ => Direction::Both,
    }
}

// ── Output serialisation helper ───────────────────────────────────────────────

fn json_result<T: Serialize>(val: &T) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string_pretty(val)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// ── Tool implementations ──────────────────────────────────────────────────────

#[tool_router]
impl GitSenseServer {
    /// Search for symbols by name and/or kind across the indexed Rust repo.
    ///
    /// Performs a case-insensitive substring match on symbol names and an
    /// optional exact kind filter.  Note: struct/enum/type-alias nodes are all
    /// tagged as `Struct` by tree-sitter-tags — filter by `struct` to reach
    /// all of them.
    ///
    /// Accepted `kind` values: fn | method | struct | enum | trait | impl | mod
    /// | const | macro | other.
    #[tool(
        description = "Search for symbols by name substring and/or kind in the indexed Rust repo. \
        Case-insensitive name match. Accepted kind values: fn | method | struct | enum | trait | \
        impl | mod | const | macro | other. Note: struct/enum/type-alias nodes all tag as 'Struct' \
        (tree-sitter-tags limitation). Returns definitions with file, line, visibility, and docs."
    )]
    async fn search_symbols(
        &self,
        Parameters(p): Parameters<SearchSymbolsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let index = Arc::clone(&self.state.index);
        let kind = p.kind.as_deref().and_then(parse_kind);
        let name = p.name.clone();

        let defs = index.search_symbols(name.as_deref(), kind);
        json_result(&defs)
    }

    /// Find all recorded call-site references for a symbol name.
    #[tool(
        description = "Find all recorded call-site references for an exact symbol name. \
        Returns file + line locations of every reference captured by tree-sitter. \
        Note: dynamic dispatch, trait objects, and macro-expanded calls may not be captured."
    )]
    async fn find_references(
        &self,
        Parameters(p): Parameters<FindReferencesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let index = Arc::clone(&self.state.index);
        let refs = index.find_references(&p.name);
        json_result(&refs)
    }

    /// Build a call graph rooted at a function or method.
    ///
    /// CAVEAT: name-based resolution — overloads, closures, and
    /// macro-expanded calls may be mis-attributed.  Results are approximate.
    /// Runs on a blocking-task thread since BFS traversal is CPU-bound.
    #[tool(
        description = "Build a call graph rooted at a Rust function or method. \
        direction: callees | callers | both (default: both). max_hops default: 3, \
        hard-capped at 32 regardless of the requested value. \
        CAVEAT: name-based resolution — overloads, closures, and macro-expanded calls may \
        be mis-attributed or missing. Cycles are detected and reported. Graph may be truncated \
        when max_hops is reached."
    )]
    async fn call_graph(
        &self,
        Parameters(p): Parameters<CallGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        /// Hard ceiling for `max_hops` to bound BFS traversal cost.
        const MAX_HOPS: usize = 32;

        let index = Arc::clone(&self.state.index);
        let direction = p
            .direction
            .as_deref()
            .map(parse_direction)
            .unwrap_or(Direction::Both);
        let max_hops = p.max_hops.unwrap_or(3).min(MAX_HOPS);
        let name = p.name.clone();

        let result =
            tokio::task::spawn_blocking(move || graph::build(&index, &name, max_hops, direction))
                .await
                .map_err(|e| to_mcp_err(anyhow::anyhow!("spawn_blocking join error: {e}")))?;
        json_result(&result)
    }

    /// Show git blame attribution for a named symbol, using actual commit history.
    ///
    /// Git history is the unique differentiator: see exactly who last touched
    /// a function's body and in which commit.
    #[tool(
        description = "Show git blame attribution for a named Rust symbol using actual commit history. \
        Resolves the symbol to its definition's line range, then runs git blame over that range. \
        Returns per-hunk blame (author, commit, date, message) plus convenience last_author / \
        last_commit_short / last_date fields identifying the most recently committed hunk. \
        Returns an error if the symbol is not found or the repo has no history."
    )]
    async fn blame_symbol(
        &self,
        Parameters(p): Parameters<BlameSymbolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let index = Arc::clone(&self.state.index);
        let repo_root = self.state.repo_root.clone();
        let name = p.name.clone();

        // Resolve to first Fn/Method def; fall back to any def.
        let defs = index.definitions(&name);
        if defs.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "no symbol named '{}' found in the index",
                name
            ))]));
        }
        let def = defs
            .iter()
            .find(|d| matches!(d.kind, SymbolKind::Fn | SymbolKind::Method))
            .unwrap_or(&defs[0])
            .clone();

        // Convert file to repo-relative path for git blame.
        let file_rel = def
            .location
            .file
            .strip_prefix(&index.repo_root)
            .unwrap_or(&def.location.file)
            .to_path_buf();

        let (start, end) = def.line_range;

        let result = tokio::task::spawn_blocking(move || {
            crate::git::blame::blame_range(&repo_root, &file_rel, start, end)
        })
        .await
        .map_err(|e| to_mcp_err(anyhow::anyhow!("spawn_blocking join error: {e}")))?
        .map_err(to_mcp_err)?;

        json_result(&result)
    }

    /// Find potentially dead (unreferenced) symbols, enriched with git-history age.
    ///
    /// APPROXIMATE: misses dynamic dispatch, trait objects, macros, and any
    /// `pub` item consumed by external crates.  Use as a triage signal, not ground truth.
    #[tool(
        description = "Find potentially unused (unreferenced) Rust symbols, enriched with git-history age. \
        APPROXIMATE: misses dynamic dispatch, trait objects, macros, and externally-referenced \
        pub items. include_pub (default false) includes pub items. limit (default 50, max 200) caps results. \
        Results sorted: non-pub first, then by days since last git touch (oldest first — safest to delete). \
        Uses git blame for age; symbols where blame fails appear with null days_since_last_touch."
    )]
    async fn find_dead_code(
        &self,
        Parameters(p): Parameters<FindDeadCodeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        /// Hard ceiling for `limit` to prevent runaway git work on the
        /// unauthenticated endpoint.
        const MAX_DEAD_CODE_LIMIT: usize = 200;

        let index = Arc::clone(&self.state.index);
        let repo_root = self.state.repo_root.clone();
        let include_pub = p.include_pub.unwrap_or(false);
        // Clamp client-supplied limit; saturating_mul avoids overflow when
        // assembling the candidate pre-fetch window.
        let limit = p.limit.unwrap_or(50).min(MAX_DEAD_CODE_LIMIT);

        // Collect unreferenced defs (cheap, no I/O).
        let unreferenced: Vec<_> = index
            .unreferenced_defs()
            .into_iter()
            .filter(|d| include_pub || !d.is_pub)
            .cloned()
            .collect();

        // Over-sample by 4× so we have room to sort by age and still return
        // `limit` results after truncation.  saturating_mul guards overflow.
        let candidates: Vec<_> = unreferenced
            .into_iter()
            .take(limit.saturating_mul(4))
            .collect();

        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        #[derive(Serialize)]
        struct DeadEntry {
            name: String,
            kind: String,
            file: String,
            line: usize,
            is_pub: bool,
            days_since_last_touch: Option<i64>,
        }

        // Build the blame-input list: repo-relative path + line range.
        // All candidates are serialised into owned Send types before entering
        // spawn_blocking, so no gix type is captured by the closure.
        let blame_items: Vec<(std::path::PathBuf, (usize, usize))> = candidates
            .iter()
            .map(|def| {
                let file_rel = def
                    .location
                    .file
                    .strip_prefix(&index.repo_root)
                    .unwrap_or(&def.location.file)
                    .to_path_buf();
                (file_rel, def.line_range)
            })
            .collect();

        // Single spawn_blocking: opens the repo once, blames every candidate
        // inside, returns owned Vec<Option<i64>>.  The !Send gix::Repository
        // is created and dropped entirely within this closure.
        let timestamps: Vec<Option<i64>> = tokio::task::spawn_blocking(move || {
            crate::git::history::last_touched_all(&repo_root, &blame_items)
        })
        .await
        .map_err(|e| to_mcp_err(anyhow::anyhow!("spawn_blocking join error: {e}")))?;

        let mut entries: Vec<DeadEntry> = candidates
            .iter()
            .zip(timestamps.iter())
            .map(|(def, ts_opt)| {
                let days = ts_opt.map(|ts| (now_secs - ts) / 86400);
                DeadEntry {
                    name: def.name.clone(),
                    kind: format!("{:?}", def.kind),
                    file: def
                        .location
                        .file
                        .strip_prefix(&index.repo_root)
                        .unwrap_or(&def.location.file)
                        .to_string_lossy()
                        .into_owned(),
                    line: def.location.line,
                    is_pub: def.is_pub,
                    days_since_last_touch: days,
                }
            })
            .collect();

        // Sort: non-pub first; within same pub tier, oldest (largest days) first.
        entries.sort_by(|a, b| {
            a.is_pub.cmp(&b.is_pub).then_with(|| {
                // Entries with None days (blame failed) sort last.
                match (b.days_since_last_touch, a.days_since_last_touch) {
                    (Some(bd), Some(ad)) => bd.cmp(&ad),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
        });

        entries.truncate(limit);
        json_result(&entries)
    }

    /// High-level repo overview: symbol counts, module list, and hottest files by churn.
    #[tool(
        description = "High-level overview of the indexed Rust repo: symbol counts by kind, \
        module names, and hottest files by git churn (number of commits that touched each file, \
        capped at 500 commits of history). Useful as a starting point for exploring an unfamiliar \
        codebase or for identifying high-churn areas."
    )]
    async fn repo_overview(
        &self,
        Parameters(_p): Parameters<RepoOverviewParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let index = Arc::clone(&self.state.index);
        let repo_root = self.state.repo_root.clone();

        // Stats are pure index, no I/O.
        let stats = index.stats();

        // Collect file paths for churn analysis.
        let files: Vec<PathBuf> = index.file_paths().into_iter().cloned().collect();

        let churn = tokio::task::spawn_blocking(move || {
            crate::git::history::file_churn(&repo_root, &files)
        })
        .await
        .map_err(|e| to_mcp_err(anyhow::anyhow!("spawn_blocking join error: {e}")))?
        .map_err(to_mcp_err)?;

        // Convert churn paths to relative strings for readability.
        let churn_display: Vec<(String, usize)> = churn
            .into_iter()
            .take(20)
            .map(|(p, n)| (p.to_string_lossy().into_owned(), n))
            .collect();

        #[derive(Serialize)]
        struct Overview {
            file_count: usize,
            def_count: usize,
            ref_count: usize,
            by_kind: Vec<(String, usize)>,
            modules: Vec<String>,
            hottest_files: Vec<(String, usize)>,
        }

        let out = Overview {
            file_count: stats.file_count,
            def_count: stats.def_count,
            ref_count: stats.ref_count,
            by_kind: stats.by_kind,
            modules: stats.modules,
            hottest_files: churn_display,
        };

        json_result(&out)
    }
}

// ── ServerHandler impl (auto-generates call_tool / list_tools / get_info) ────

#[tool_handler(
    name = "gitsense",
    instructions = "GitSense analyzes a single Rust repository with git-history awareness. \
        Tools: \
        (1) search_symbols — find Rust symbols by name substring / kind across all .rs files; \
        (2) find_references — locate every call-site reference to a named symbol; \
        (3) call_graph — build a caller/callee graph rooted at a function (name-based, approximate); \
        (4) blame_symbol — show git blame attribution for a symbol's body (who last touched it and when); \
        (5) find_dead_code — find unreferenced symbols enriched with git age (oldest untouched = safest to delete); \
        (6) repo_overview — high-level counts, module list, and hottest files by git churn. \
        All analysis is approximate: dynamic dispatch, trait objects, and macro-expanded calls may be missed."
)]
impl ServerHandler for GitSenseServer {}
