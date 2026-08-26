//! Environment-driven configuration. Defaults suit `docker compose up`.
//!
//! Stage-2 auth hardening additions: the at-rest secret-encryption key,
//! MFA pending-token TTL, and WebAuthn relying-party identity. The
//! encryption key is a single app-level key from env — envelope
//! encryption (per-org data keys under a master key) is a documented
//! upgrade path, tracked as a known gap.

/// AES-256 key material for encrypting provider secrets at rest.
#[derive(Debug, Clone, Copy)]
pub struct SecretKey(pub [u8; 32]);

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
    /// Key for AES-256-GCM at-rest encryption of integration secrets
    /// (OIDC client secrets, TOTP secrets). Env `MERIDIAN_SECRET_KEY`
    /// (64 hex chars). Falls back to a value derived from `jwt_secret`
    /// for dev only — loudly logged; rotate to a real key before prod.
    pub secret_key: SecretKey,
    /// TTL of the short-lived `mfa_pending` JWT bridging password
    /// success and second-factor completion (default 5 minutes).
    pub mfa_pending_ttl_secs: i64,
    /// WebAuthn relying-party id (effective domain of the browser origin).
    pub webauthn_rp_id: String,
    /// WebAuthn relying-party origin — must match the browser origin exactly.
    pub webauthn_rp_origin: String,
    /// Public base URL of this API, used to build OIDC redirect URIs.
    pub public_base_url: String,
    /// Web app base URL — SSO callback lands the browser here.
    pub web_base_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
            // Dev default; compose sets a real one. Never do this in prod.
            "dev-only-insecure-secret-change-me".into()
        });
        let secret_key = match std::env::var("MERIDIAN_SECRET_KEY") {
            Ok(hex_key) if hex_key.len() == 64 => {
                let mut bytes = [0u8; 32];
                if hex::decode_to_slice(&hex_key, &mut bytes).is_err() {
                    tracing::warn!(
                        "MERIDIAN_SECRET_KEY is not valid hex; deriving from jwt_secret"
                    );
                    derive_dev_key(&jwt_secret)
                } else {
                    SecretKey(bytes)
                }
            }
            _ => {
                tracing::warn!(
                    "MERIDIAN_SECRET_KEY unset — at-rest secret encryption derives its key \
                     from JWT_SECRET. Set MERIDIAN_SECRET_KEY (64 hex chars) for production."
                );
                derive_dev_key(&jwt_secret)
            }
        };
        Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://meridian_app:meridian_app@localhost:15432/meridian".into()
            }),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:16379/0".into()),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            jwt_secret,
            access_ttl_secs: std::env::var("ACCESS_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            refresh_ttl_secs: std::env::var("REFRESH_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60 * 60 * 24 * 30),
            cors_origin: std::env::var("CORS_ORIGIN").ok(),
            secret_key,
            mfa_pending_ttl_secs: std::env::var("MFA_PENDING_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            webauthn_rp_id: std::env::var("WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into()),
            webauthn_rp_origin: std::env::var("WEBAUTHN_RP_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            public_base_url: std::env::var("PUBLIC_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into()),
            web_base_url: std::env::var("WEB_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
        }
    }
}

fn derive_dev_key(jwt_secret: &str) -> SecretKey {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"meridian-at-rest-secret-key:");
    hasher.update(jwt_secret.as_bytes());
    SecretKey(hasher.finalize().into())
}

#[cfg(test)]
impl Config {
    /// Deterministic config for unit tests (no env reads).
    pub fn test_with_secret(secret: &str) -> Self {
        Self {
            database_url: String::new(),
            redis_url: String::new(),
            bind_addr: String::new(),
            jwt_secret: secret.to_string(),
            access_ttl_secs: 900,
            refresh_ttl_secs: 86_400,
            cors_origin: None,
            secret_key: derive_dev_key(secret),
            mfa_pending_ttl_secs: 300,
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "http://localhost:3000".into(),
            public_base_url: "http://localhost:8080".into(),
            web_base_url: "http://localhost:3000".into(),
        }
    }
}
