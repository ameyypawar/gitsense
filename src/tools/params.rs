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
/// be one of): `fn`, `method`, `struct`, `enum`, `trait`, `mod`, `macro`,
/// `other`.
///
/// Note: `enum` is distinguished from `struct` via the enclosing item node;
/// union/type-alias definitions still surface as `struct`. `impl` and
/// `const` are NOT accepted — tree-sitter-rust's tags query never emits a
/// named definition tag for `impl` blocks or `const` items, so those
/// filters would always return empty (#8).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchSymbolsParams {
    /// Optional case-insensitive substring to match against symbol names.
    pub name: Option<String>,
    /// Optional exact kind filter: fn | method | struct | enum | trait | mod | macro | other.
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
    /// Optional file path (matched against the end of the definition's
    /// path, e.g. `"src/foo.rs"` or just `"foo.rs"`) to disambiguate the
    /// root when multiple definitions share `name` (#8).
    pub file: Option<String>,
    /// Optional line number (1-based) to further disambiguate the root;
    /// must fall within the candidate definition's line range. Only
    /// consulted together with `file`.
    pub line: Option<usize>,
}

// ── blame_symbol ──────────────────────────────────────────────────────────────

/// Parameters for `blame_symbol`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BlameSymbolParams {
    /// Exact symbol name to look up blame information for.
    pub name: String,
    /// Optional file path (matched against the end of the definition's
    /// path, e.g. `"src/foo.rs"` or just `"foo.rs"`) to disambiguate when
    /// multiple definitions share `name` (#8).
    pub file: Option<String>,
    /// Optional line number (1-based) to further disambiguate; must fall
    /// within the candidate definition's line range. Only consulted
    /// together with `file`.
    pub line: Option<usize>,
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
