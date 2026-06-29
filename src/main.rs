use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use gitsense::{
    index::SymbolIndex,
    tools::{AppState, GitSenseServer},
};
use rmcp::ServiceExt;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "gitsense-mcp", about = "GitSense MCP server — Rust repo analysis with git-history awareness")]
struct Cli {
    /// Path to the Rust repository to analyse.
    #[arg(long, env = "REPO_PATH", default_value = ".")]
    repo_path: PathBuf,

    /// Transport to use: stdio (Phase 6) or http (Phase 7, not yet implemented).
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// HTTP port (unused until Phase 7).
    #[arg(long, default_value_t = 8080)]
    port: u16,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise tracing to STDERR so stdout stays clean for the stdio MCP channel.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.transport.as_str() {
        "http" => {
            anyhow::bail!("HTTP transport not implemented yet — added in Phase 7");
        }
        "stdio" => {}
        other => {
            anyhow::bail!("unknown transport '{}'; supported: stdio", other);
        }
    }

    let repo_path = cli
        .repo_path
        .canonicalize()
        .unwrap_or(cli.repo_path.clone());

    // Build the symbol index (sync, potentially slow — log timing).
    tracing::info!("building index for {}", repo_path.display());
    let t0 = std::time::Instant::now();
    let index = SymbolIndex::build(&repo_path)?;
    tracing::info!(
        "index ready in {:.1}s ({} defs)",
        t0.elapsed().as_secs_f32(),
        index.stats().def_count,
    );

    let state = Arc::new(AppState {
        index: Arc::new(index),
        repo_root: repo_path,
    });

    let server = GitSenseServer::new(state);

    tracing::info!("gitsense-mcp listening on stdio");

    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;

    Ok(())
}
