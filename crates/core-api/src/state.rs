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
    /// Email edge: transport + scanner adapters built from config.
    pub mail: Arc<MailStack>,
}
