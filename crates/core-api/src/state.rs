//! Shared application state.
//!
//! Note: core-api never publishes to Redis directly — mutations only write
//! the transactional outbox; the worker is the single stream publisher (§4.2).

use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<crate::config::Config>,
    /// Stateless OIDC protocol client (discovery/exchange/JWKS).
    pub oidc: crate::auth::oidc_client::OidcHttpClient,
}
