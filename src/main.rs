use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use gitsense::{
    http::build_router,
    index::SymbolIndex,
    tools::{AppState, GitSenseServer},
};
use rmcp::ServiceExt;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "gitsense-mcp",
    about = "GitSense MCP server — Rust repo analysis with git-history awareness"
)]
struct Cli {
    /// Path to the Rust repository to analyse.
    #[arg(long, env = "REPO_PATH", default_value = ".")]
    repo_path: PathBuf,

    /// Transport to use: stdio or http.
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// HTTP port (used when --transport http).
    #[arg(long, default_value_t = 8080)]
    port: u16,
}

// ── Shared state construction ─────────────────────────────────────────────────

async fn build_state(repo_path: PathBuf) -> anyhow::Result<Arc<AppState>> {
    let repo_path = repo_path.canonicalize().unwrap_or(repo_path);
    tracing::info!("building index for {}", repo_path.display());
    let t0 = std::time::Instant::now();
    let index = SymbolIndex::build(&repo_path)?;
    tracing::info!(
        "index ready in {:.1}s ({} defs)",
        t0.elapsed().as_secs_f32(),
        index.stats().def_count,
    );
    Ok(Arc::new(AppState {
        index: Arc::new(index),
        repo_root: repo_path,
    }))
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
            let state = build_state(cli.repo_path).await?;
            let allowed_hosts: Vec<String> = std::env::var("GITSENSE_ALLOWED_HOSTS")
                .ok()
                .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
                .unwrap_or_else(|| vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]);
            let router = build_router(state, allowed_hosts);
            let addr = format!("0.0.0.0:{}", cli.port);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            eprintln!("gitsense-mcp listening on http://0.0.0.0:{}/mcp", cli.port);
            tracing::info!("gitsense-mcp listening on http://0.0.0.0:{}/mcp", cli.port);
            axum::serve(listener, router).await?;
        }
        "stdio" => {
            let state = build_state(cli.repo_path).await?;
            let server = GitSenseServer::new(state);
            tracing::info!("gitsense-mcp listening on stdio");
            let service = server.serve(rmcp::transport::stdio()).await?;
            service.waiting().await?;
        }
        other => {
            anyhow::bail!("unknown transport '{}'; supported: stdio, http", other);
        }
    }

    Ok(())
}
