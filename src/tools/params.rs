/// Parameter structs for all 6 GitSense MCP tools.
///
/// All structs derive `serde::Deserialize` (for JSON-RPC argument parsing) and
/// `schemars::JsonSchema` (so rmcp can emit a JSON Schema for each tool's
/// `inputSchema` field).  We use `rmcp::schemars` to guarantee the exact same
/// schemars version rmcp resolved internally, avoiding type-ID mismatches.
use rmcp::schemars;
use serde::Deserialize;

// ── search_symbols ────────────────────────────────────────────────────────────

/// Parameters for `search_symbols`.
///
/// Accepted `kind` strings (case-insensitive prefix match not required — must
/// be one of): `fn`, `method`, `struct`, `enum`, `trait`, `impl`, `mod`,
/// `const`, `macro`, `other`.
///
/// Note: tree-sitter-tags reports struct/enum/type-aliases all as `Struct`; the
/// `kind` filter reflects the internal tag, not the Rust syntax keyword.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsParams {
    /// Optional case-insensitive substring to match against symbol names.
    pub name: Option<String>,
    /// Optional exact kind filter: fn | method | struct | enum | trait | impl | mod | const | macro | other.
    pub kind: Option<String>,
}

// ── find_references ───────────────────────────────────────────────────────────

/// Parameters for `find_references`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindReferencesParams {
    /// Exact symbol name to look up call-site references for.
    pub name: String,
}

// ── call_graph ────────────────────────────────────────────────────────────────

/// Parameters for `call_graph`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CallGraphParams {
    /// Root symbol name (must be a function or method).
    pub name: String,
    /// Maximum hops to traverse.  Defaults to 3.
    pub max_hops: Option<usize>,
    /// Direction: `callees` | `callers` | `both`.  Defaults to `both`.
    pub direction: Option<String>,
}

// ── blame_symbol ──────────────────────────────────────────────────────────────

/// Parameters for `blame_symbol`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BlameSymbolParams {
    /// Exact symbol name to look up blame information for.
    pub name: String,
}

// ── find_dead_code ────────────────────────────────────────────────────────────

/// Parameters for `find_dead_code`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindDeadCodeParams {
    /// When `true`, include `pub` items in results (default: false).
    pub include_pub: Option<bool>,
    /// Maximum number of results to return (default: 50).
    pub limit: Option<usize>,
}

// ── repo_overview ─────────────────────────────────────────────────────────────

/// Parameters for `repo_overview` (no inputs needed).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RepoOverviewParams {}
