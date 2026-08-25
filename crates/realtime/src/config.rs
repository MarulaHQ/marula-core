//! Realtime service configuration, loaded from the environment.
//!
//! Purpose: the handful of settings this service needs, with defaults that
//! mirror `core_api::config::Config` so `docker compose up` needs zero env
//! changes. Variable names are identical to core-api's where they overlap.
//!
//! Non-purpose: no file/config-store loading and no secret management — the
//! JWT secret is trusted from the environment exactly as core-api does.

/// Runtime settings for the WebSocket hub process.
#[derive(Debug, Clone)]
pub struct RealtimeConfig {
    /// Redis connection URL; the stream is `core_events::EVENT_STREAM`.
    pub redis_url: String,
    /// HTTP/WebSocket listen address (`BIND_ADDR`, default `0.0.0.0:8081`;
    /// core-api owns 8080).
    pub bind_addr: String,
    /// HS256 secret for verifying access tokens — must match core-api's.
    pub jwt_secret: String,
}

impl RealtimeConfig {
    /// Reads `REDIS_URL`, `BIND_ADDR`, `JWT_SECRET` from the process
    /// environment, falling back to dev defaults.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same resolution against any key lookup; keeps tests free of global
    /// environment mutation.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            redis_url: lookup("REDIS_URL")
                .unwrap_or_else(|| "redis://localhost:16379/0".to_string()),
            bind_addr: lookup("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8081".to_string()),
            jwt_secret: lookup("JWT_SECRET")
                .unwrap_or_else(|| "dev-only-insecure-secret-change-me".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_mirror_core_api() {
        let cfg = RealtimeConfig::from_lookup(|_| None);
        assert_eq!(cfg.redis_url, "redis://localhost:16379/0");
        assert_eq!(cfg.bind_addr, "0.0.0.0:8081");
        assert_eq!(cfg.jwt_secret, "dev-only-insecure-secret-change-me");
    }

    #[test]
    fn env_values_override_defaults() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("REDIS_URL", "redis://r:1".into());
        vars.insert("BIND_ADDR", "127.0.0.1:9999".into());
        vars.insert("JWT_SECRET", "s3cret".into());
        let cfg = RealtimeConfig::from_lookup(|k| vars.get(k).cloned());
        assert_eq!(cfg.redis_url, "redis://r:1");
        assert_eq!(cfg.bind_addr, "127.0.0.1:9999");
        assert_eq!(cfg.jwt_secret, "s3cret");
    }
}
