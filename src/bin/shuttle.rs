//! Shuttle deployment entry point for gitsense.
//!
//! ## Deployed-repo strategy
//!
//! The headline tools (`blame_symbol`, `find_dead_code`) need a real Rust repo
//! with git history.  Shuttle containers have no meaningful `.git` of their own,
//! so at startup we clone a small public Rust crate into `/tmp/gitsense-target`:
//!
//! - Default: `dtolnay/anyhow` — ~7 source files, ~500 commits, fits easily
//!   in the 0.5 GB RAM budget and cold-starts in a few seconds.
//! - Override: set `GITSENSE_CLONE_URL` as a Shuttle secret to any public
//!   HTTPS git URL (prefer small crates; avoid multi-KLOC mono-repos).
//!
//! The clone is skipped on warm restarts (`.git` already present).
//!
//! ## Allowed-hosts
//!
//! `build_router` is called with an empty `allowed_hosts` list.  Per rmcp
//! semantics an empty list means "allow all inbound Host values", which is
//! required for shuttle's reverse-proxy TLS termination (the proxy rewrites
//! the Host header to the shuttle domain).  Operators can restrict this via
//! the `GITSENSE_ALLOWED_HOSTS` Shuttle secret (comma-separated hostnames).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gitsense::{http::build_router, index::SymbolIndex, tools::AppState};

/// Small public Rust crate used as the demo analysis target.
const DEFAULT_CLONE_URL: &str = "https://github.com/dtolnay/anyhow";

#[shuttle_runtime::main]
async fn shuttle_main() -> shuttle_axum::ShuttleAxum {
    let clone_url = std::env::var("GITSENSE_CLONE_URL")
        .unwrap_or_else(|_| DEFAULT_CLONE_URL.to_owned());

    let target = PathBuf::from("/tmp/gitsense-target");

    // Clone / reuse in a blocking thread (gix uses synchronous network I/O).
    let join = tokio::task::spawn_blocking({
        let url = clone_url.clone();
        let path = target.clone();
        move || clone_or_reuse(&url, &path)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {e}"))?;
    let repo_root = join?;

    // Build symbol + call-graph index (CPU-bound; blocking thread).
    let join = tokio::task::spawn_blocking({
        let root = repo_root.clone();
        move || SymbolIndex::build(&root)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {e}"))?;
    let index = join?;

    let state = Arc::new(AppState {
        index: Arc::new(index),
        repo_root,
    });

    // Empty allowed_hosts → rmcp allows all inbound Host values.
    // Operators can restrict via GITSENSE_ALLOWED_HOSTS shuttle secret.
    let allowed_hosts: Vec<String> = std::env::var("GITSENSE_ALLOWED_HOSTS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_default(); // empty = allow-all per rmcp semantics

    Ok(build_router(state, allowed_hosts).into())
}

/// Clone `url` into `target`, or reuse an existing clone if `.git` is present.
fn clone_or_reuse(url: &str, target: &Path) -> anyhow::Result<PathBuf> {
    if target.join(".git").exists() {
        tracing::info!(
            "gitsense-shuttle: reusing existing clone at {}",
            target.display()
        );
        return Ok(target.to_path_buf());
    }

    tracing::info!(
        "gitsense-shuttle: cloning {} -> {}",
        url,
        target.display()
    );

    let interrupt = AtomicBool::new(false);
    let mut prepare = gix::clone::PrepareFetch::new(
        url,
        target,
        gix::create::Kind::WithWorktree,
        Default::default(),
        Default::default(),
    )?;

    let (mut checkout, _fetch_outcome) =
        prepare.fetch_then_checkout(gix::progress::Discard, &interrupt)?;

    let (_repo, _checkout_outcome) =
        checkout.main_worktree(gix::progress::Discard, &interrupt)?;

    Ok(target.to_path_buf())
}
