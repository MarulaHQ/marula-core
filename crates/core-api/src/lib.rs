//! core-api — the ONLY database writer (§4.1). Axum HTTP API surface.

pub mod ai;
pub mod all_in_one;
pub mod api_scopes;
pub mod auth;
pub mod cache;
pub mod cache_invalidation;
pub mod client_ip;
pub mod config;
pub mod crypto;
pub mod dns_verify;
pub mod email;
pub mod error;
pub mod http_version;
pub mod instant;
pub mod mcp_catalog;
pub mod org_tx;
pub mod otel_http;
pub mod outbound;
pub mod plugin_http;
pub mod realtime;
pub mod routes;
pub mod routing;
pub mod scan_backfill;
pub mod scan_gate;
pub mod services;
pub mod state;
pub mod vault_scale;

use axum::Router;
use state::AppState;

pub fn router(state: AppState) -> Router {
    routes::build_router(state)
}
