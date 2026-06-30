use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::tools::{AppState, GitSenseServer};

/// Build the shared MCP router.
///
/// Mounts `StreamableHttpService` at `/mcp`. Callers supply `allowed_hosts`
/// directly so both the local binary and the Shuttle entry can configure it:
/// - Local HTTP mode: reads `GITSENSE_ALLOWED_HOSTS` env, defaults to localhost.
/// - Shuttle entry:   passes an empty vec (rmcp allows ALL hosts when the list
///   is empty — correct for shuttle's reverse-proxy termination).
///
/// Operators can restrict the Shuttle deployment by setting the
/// `GITSENSE_ALLOWED_HOSTS` secret on shuttle.dev.
pub fn build_router(state: Arc<AppState>, allowed_hosts: Vec<String>) -> Router {
    let config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);

    let service = StreamableHttpService::<GitSenseServer, LocalSessionManager>::new(
        move || Ok(GitSenseServer::new(state.clone())),
        Default::default(),
        config,
    );

    Router::new().nest_service("/mcp", service)
}
