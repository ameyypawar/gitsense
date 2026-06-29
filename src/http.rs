use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::tools::{AppState, GitSenseServer};

/// Build the shared MCP router.
///
/// Mounts `StreamableHttpService` at `/mcp`. Reused verbatim by Phase 8 (Shuttle).
///
/// DNS-rebinding protection comes from `StreamableHttpServerConfig::allowed_hosts`
/// (default: localhost / 127.0.0.1 / ::1 only). Override via `GITSENSE_ALLOWED_HOSTS`
/// (comma-separated) for public deployments.
pub fn build_router(state: Arc<AppState>) -> Router {
    let allowed_hosts: Vec<String> = std::env::var("GITSENSE_ALLOWED_HOSTS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
        .unwrap_or_else(|| vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]);

    let config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);

    let service = StreamableHttpService::<GitSenseServer, LocalSessionManager>::new(
        move || Ok(GitSenseServer::new(state.clone())),
        Default::default(),
        config,
    );

    Router::new().nest_service("/mcp", service)
}
