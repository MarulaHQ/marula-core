//! Shared application state.
//!
//! Note: core-api never publishes to Redis directly — mutations only write
//! the transactional outbox; the worker is the single stream publisher (§4.2).

use crate::email::MailStack;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<crate::config::Config>,
    /// Stateless OIDC protocol client (discovery/exchange/JWKS).
    pub oidc: crate::auth::oidc_client::OidcHttpClient,
    /// Email edge: transport + scanner adapters built from config.
    pub mail: Arc<MailStack>,
    /// Object storage edge (attachments): local-fs none mode by default,
    /// S3/MinIO when configured. See crates/core-store.
    pub store: Arc<dyn core_store::ObjectStore>,
}
