//! core-api — the ONLY database writer (§4.1). Axum HTTP API surface.

pub mod ai;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod email;
pub mod error;
pub mod instant;
pub mod org_tx;
pub mod otel_http;
pub mod realtime;
pub mod routing;
pub mod routes;
pub mod state;
pub mod vault_scale;

use axum::Router;
use state::AppState;

pub fn router(state: AppState) -> Router {
    routes::build_router(state)
}
