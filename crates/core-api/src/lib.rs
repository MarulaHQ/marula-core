//! core-api — the ONLY database writer (§4.1). Axum HTTP API surface.

pub mod auth;
pub mod config;
pub mod email;
pub mod error;
pub mod org_tx;
pub mod routes;
pub mod state;

use axum::Router;
use state::AppState;

pub fn router(state: AppState) -> Router {
    routes::build_router(state)
}
