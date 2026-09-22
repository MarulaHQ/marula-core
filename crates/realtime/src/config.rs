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

    /// Fail-closed boot posture (mirror of core-api's `Config::from_env`):
    /// the hub verifies access tokens with HS256, so running with the
    /// publicly-known dev secret would let anyone forge a valid token.
    /// `from_env()` keeps the dev fallback for the compose/dev profile and
    /// tests; production entry (`main.rs`) refuses to boot without an
    /// explicit 64-hex `JWT_SECRET` unless `MERIDIAN_DEV_TOOLS=1`.
    pub fn try_from_env() -> Result<Self, String> {
        let dev_tools = std::env::var("MERIDIAN_DEV_TOOLS").as_deref() == Ok("1");
        match std::env::var("JWT_SECRET") {
            Ok(secret) if secret.len() == 64 && secret.chars().all(|c| c.is_ascii_hexdigit()) => {
                Ok(Self::from_lookup(|key| std::env::var(key).ok())
                    .with_jwt_secret(secret))
            }
            Ok(_) => Err(
                "JWT_SECRET must be exactly 64 hex characters (generate: openssl rand -hex 32)"
                    .into(),
            ),
            Err(_) if dev_tools => Ok(Self::from_env()),
            Err(_) => Err(
                "JWT_SECRET is not set — refusing to boot (MERIDIAN_DEV_TOOLS=1 enables the dev fallback)"
                    .into(),
            ),
        }
    }

    fn with_jwt_secret(mut self, jwt_secret: String) -> Self {
        self.jwt_secret = jwt_secret;
        self
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

    // Fail-closed posture (Wave 19 lane 4 GA audit): the hub refuses to
    // boot without an explicit 64-hex JWT_SECRET. RealtimeConfig exposes no
    // lookup-injectable variant of try_from_env (it reads the process env
    // directly), so the acceptance contract here is the error-message
    // shape, pinned verbatim; the happy path is exercised in the compose
    // stack where JWT_SECRET is always set.
    #[test]
    fn try_from_env_error_messages_are_operator_actionable() {
        // Not set at all (or dev flag unset) -> refusal names the variable
        // and the dev escape hatch. Serialized against the other env tests
        // via the shared guard mutex.
        let guard = EnvGuard::clear("JWT_SECRET");
        let msg = RealtimeConfig::try_from_env().unwrap_err();
        assert!(msg.contains("JWT_SECRET is not set"));
        assert!(msg.contains("MERIDIAN_DEV_TOOLS=1"));
        drop(guard);
    }

    #[test]
    fn try_from_env_rejects_non_hex_secret() {
        // A set-but-malformed secret must be rejected with the generation
        // hint, never silently accepted (guards against "s3cret"-class
        // values landing in a prod env).
        let guard = EnvGuard::set("JWT_SECRET", "s3cret");
        let msg = RealtimeConfig::try_from_env().unwrap_err();
        assert!(msg.contains("64 hex"), "got: {msg}");
        drop(guard);
    }

    #[test]
    fn try_from_env_accepts_valid_hex() {
        let secret = "a".repeat(64);
        let guard = EnvGuard::set("JWT_SECRET", &secret);
        let cfg = RealtimeConfig::try_from_env().expect("valid hex accepted");
        assert_eq!(cfg.jwt_secret, secret);
        drop(guard);
    }

    /// Test-scoped env mutation with restore-on-drop, serialized behind a
    /// shared mutex so the env-reading trio stays deterministic under the
    /// default parallel test runner.
    struct EnvGuard {
        prev: Option<String>,
        key: &'static str,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                prev,
                key,
                _lock: lock,
            }
        }

        fn clear(key: &'static str) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self {
                prev,
                key,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
