//! Environment-driven configuration. Defaults suit `docker compose up`.

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    /// Access token TTL in seconds (default 15 min).
    pub access_ttl_secs: i64,
    /// Refresh token TTL in seconds (default 30 days).
    pub refresh_ttl_secs: i64,
    pub cors_origin: Option<String>,
    /// Bearer token guarding /api/internal/* machine endpoints used by the
    /// worker (§4.1 single-writer). Unset ⇒ the surface is disabled.
    pub internal_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://meridian_app:meridian_app@localhost:15432/meridian".into()
            }),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:16379/0".into()),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_else(|_| {
                // Dev default; compose sets a real one. Never do this in prod.
                "dev-only-insecure-secret-change-me".into()
            }),
            access_ttl_secs: std::env::var("ACCESS_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            refresh_ttl_secs: std::env::var("REFRESH_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60 * 60 * 24 * 30),
            cors_origin: std::env::var("CORS_ORIGIN").ok(),
            internal_token: std::env::var("MERIDIAN_INTERNAL_TOKEN").ok(),
        }
    }
}
