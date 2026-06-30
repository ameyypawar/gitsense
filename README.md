# gitsense

**Git-history-aware code intelligence for Rust, served over MCP.**

[![CI](https://github.com/ameyypawar/gitsense/actions/workflows/ci.yml/badge.svg)](https://github.com/ameyypawar/gitsense/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

---

gitsense is a single-binary MCP server that gives AI agents structural and historical insight into a Rust codebase. It combines **tree-sitter** (symbol extraction) with **gitoxide/gix** (pure-Rust git, no system git or libgit2 required) to answer questions a static symbol index alone cannot: who last touched this function, how long ago, and which symbols have sat untouched long enough that deleting them is safe. It speaks the official [Model Context Protocol](https://modelcontextprotocol.io/) over both stdio and Streamable HTTP, and deploys to a live HTTPS URL on Shuttle with a single command.

---

## Demo

`blame_symbol` + `find_dead_code` against [`dtolnay/anyhow`](https://github.com/dtolnay/anyhow) (~500 commits of history):

![gitsense demo](demo/gitsense-demo.gif)

---

## Tools

| Tool | What it does | Example question |
|------|-------------|-----------------|
| `search_symbols` | Case-insensitive substring search over all Rust symbol definitions (fn, method, struct, enum, trait, impl, mod, const, macro). | "Find all types named `Context`." |
| `find_references` | Every call-site reference to a named symbol captured by tree-sitter. | "Where is `blame_range` called?" |
| `call_graph` | Caller/callee graph rooted at a function, up to `max_hops` deep with cycle detection. Name-based — results are approximate. | "What calls `build_router` and what does it call?" |
| **`blame_symbol`** | **Resolves a symbol to its line range, runs `gix` blame over that range, and returns per-hunk attribution (author, commit hash, date, message) plus a `last_author`/`last_date` summary.** | "Who last touched `SymbolIndex::build` and in which commit?" |
| **`find_dead_code`** | **Finds unreferenced symbols, then ranks them by `days_since_last_touch` (oldest first — safest to delete). Non-pub items surfaced first.** | "What functions can I safely delete? Sort by how long they've been untouched." |
| `repo_overview` | Symbol counts by kind, module list, and hottest files by commit churn (capped at 500 commits). | "Give me a high-level map of this repo." |

The git-history angle sets `blame_symbol` and `find_dead_code` apart from structural-only tools: rather than reporting "this looks unused," gitsense tells you *how long* it has been unused and *when* it was last committed.

---

## Quickstart

### Add to Claude Code / Claude Desktop (stdio)

Build the binary first:

```bash
cargo build --release
```

Then register the MCP server (replace the repo path with the Rust project you want to analyse):

```bash
claude mcp add gitsense -- /path/to/gitsense/target/release/gitsense-mcp \
  --repo-path /path/to/your/rust/repo
```

Alternatively, set `REPO_PATH` in your environment and omit the flag:

```bash
REPO_PATH=/path/to/your/rust/repo \
  claude mcp add gitsense -- /path/to/gitsense/target/release/gitsense-mcp
```

Claude Code picks up the server on the next session start. You can now ask it to call `search_symbols`, `blame_symbol`, etc. directly.

### HTTP / remote transport

Run gitsense as an HTTP server (Streamable HTTP, endpoint at `/mcp`):

```bash
./target/release/gitsense-mcp \
  --transport http \
  --repo-path /path/to/your/rust/repo \
  --port 8080
```

Then add it as a remote MCP server in Claude Code:

```bash
claude mcp add --transport http gitsense http://localhost:8080/mcp
```

After you deploy to Shuttle (see below), the URL will look like:

```
https://<your-app>.shuttle.app/mcp
```

---

## Live demo / hosted instance

The hosted instance (once deployed) indexes [`dtolnay/anyhow`](https://github.com/dtolnay/anyhow) — ~7 source files, ~500 commits of history, well-suited to demonstrating the git-history tools.

Example: blame the `chain` method in anyhow:

```json
{
  "tool": "blame_symbol",
  "arguments": { "name": "chain" }
}
```

Expected response includes `last_author`, `last_commit_short`, `last_date`, and per-hunk attribution for every line in the function body.

---

## Comparison

| Feature | gitsense | Serena | Sourcegraph MCP | narsil-mcp |
|---------|----------|--------|-----------------|------------|
| Symbol search | ✓ tree-sitter | ✓ tree-sitter | ✓ Sourcegraph index | ✓ |
| Find references | ✓ | ✓ | ✓ | ✓ |
| Call graph | ✓ (name-based) | ✗ | ✗ | ✗ |
| Per-symbol blame | ✓ | ✗ | ✗ | ✓ (system git) |
| Git-aged dead code | ✓ | ✗ | ✗ | ✗ |
| Git backend | **gitoxide/gix (pure Rust)** | none | Sourcegraph indexing | system git |
| Deployable as HTTPS MCP | ✓ Shuttle | ✗ | ✓ sourcegraph.com | unclear |

Honest note: narsil-mcp also does per-symbol blame; gitsense's distinguishing claims are the **pure-Rust gitoxide backend** (no system-git/libgit2 dependency — single static binary) and the **git-aged dead-code ranking** (zero-reference symbols sorted safest-to-delete-first by commit age). Table reflects publicly visible features and may be incomplete.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Rust repository                        │
└──────────────────────────┬──────────────────────────────────┘
                           │
          ┌────────────────┴─────────────────┐
          │                                  │
   gix walk (blame,                  tree-sitter-tags
   commit history,                   (symbol defs +
   file churn)                       call-site refs)
          │                                  │
          └────────────────┬─────────────────┘
                           │
                    SymbolIndex
              (in-memory, cross-file:
               defs_by_name, refs_by_name,
               tags_by_file, all_defs)
                           │
              ┌────────────┴────────────┐
              │       Tools layer       │
              │  search_symbols         │
              │  find_references        │
              │  call_graph             │
              │  blame_symbol  ─────────┼──► spawn_blocking ──► gix
              │  find_dead_code ────────┼──► spawn_blocking ──► gix
              │  repo_overview ─────────┼──► spawn_blocking ──► gix
              └────────────┬────────────┘
                           │
                    rmcp 1.8 SDK
                           │
             ┌─────────────┴──────────────┐
             │                            │
           stdio                Streamable HTTP
        (Claude Code,             (/mcp endpoint,
         Claude Desktop)          Shuttle / local)
```

**`spawn_blocking` boundary:** `gix::Repository` is `!Send` (contains `Rc` internally). Every git operation — blame, history walk, file churn — runs inside a `tokio::task::spawn_blocking` closure and drops before any `.await` point. The async tool handlers never hold a live `gix::Repository`.

### Module map

```
src/
  main.rs           CLI (gitsense-mcp binary): --repo-path, --transport, --port
  lib.rs            Library root (re-exports git, graph, http, index, tools)
  http.rs           build_router: StreamableHttpService mounted at /mcp
  bin/
    shuttle.rs      Shuttle entry (gitsense binary): clone-or-reuse + build_router
  git/
    blame.rs        blame_range: gix blame over a line range
    history.rs      last_touched, file_churn: gix rev-walk
  index/
    mod.rs          SymbolIndex: build, search_symbols, unreferenced_defs, stats
    parse.rs        RustTagger: tree-sitter-tags extraction
    walk.rs         collect_rust_files: walkdir over .rs files
  graph/
    mod.rs          call graph builder with cycle detection
  tools/
    mod.rs          GitSenseServer: 6 #[tool] handlers
    params.rs       Input param structs for each tool
```

---

## Build and run

```bash
# Build release binary
cargo build --release

# Run over a local Rust repo (stdio, default)
./target/release/gitsense-mcp --repo-path /path/to/rust/repo

# Run as HTTP server (Streamable HTTP at /mcp)
./target/release/gitsense-mcp \
  --transport http \
  --repo-path /path/to/rust/repo \
  --port 8080

# Restrict inbound Host values in HTTP mode (comma-separated)
GITSENSE_ALLOWED_HOSTS=myapp.example.com \
  ./target/release/gitsense-mcp --transport http --repo-path /path/to/rust/repo
```

---

## Deploy to Shuttle

gitsense ships a second binary (`gitsense`) wired to `shuttle-axum`. On startup it clones a small Rust repo into `/tmp/gitsense-target` (default: `dtolnay/anyhow`), builds the symbol index, and serves the MCP endpoint at `/mcp`.

```bash
# Install the Shuttle CLI
cargo install cargo-shuttle

# Log in to shuttle.dev
shuttle login

# Deploy (from the gitsense repo root)
shuttle deploy
```

**Secrets / environment variables** (set via `shuttle secret`):

| Variable | Default | Purpose |
|----------|---------|---------|
| `GITSENSE_CLONE_URL` | `https://github.com/dtolnay/anyhow` | HTTPS git URL of the repo to index |
| `GITSENSE_ALLOWED_HOSTS` | *(empty — allow all)* | Comma-separated Host values to accept |

Prefer small crates (< a few thousand commits) for the Shuttle free tier (0.5 GB RAM). Cold-start on `dtolnay/anyhow` is a few seconds.

---

## Limitations (v0)

- **Rust only.** tree-sitter-tags language files for Python, TypeScript, Go are planned but not yet wired. v0 is intentionally single-language.
- **Approximate call graph.** Name-based resolution — overloads, closures, and macro-expanded calls may be mis-attributed or missing.
- **Approximate dead code.** `find_dead_code` misses dynamic dispatch (trait objects), macro-generated items, and `pub` items consumed by external crates. Treat it as a triage signal, not a guarantee.
- **File churn simplified.** `repo_overview` walks first-parent only, capped at 500 commits, with no rename tracking.
- **Demo-repo scale.** The Shuttle deployment is designed for small-to-medium crates. Mono-repos or crates with tens of thousands of commits will hit the 0.5 GB RAM ceiling.

**Roadmap:** multi-language support (Python, TypeScript); rename tracking in churn; persistent index (avoid cold-start reparse); workspace-aware multi-crate indexing.

---

## License

MIT — see [LICENSE](LICENSE).
