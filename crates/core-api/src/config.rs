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

use core_mail::TransportKind;

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
    /// HMAC shared secret for the inbound email webhook. Unset ⇒ the
    /// webhook endpoint is DISABLED (the documented none mode — never an
    /// open relay into ticketing).
    pub email_inbound_hmac_secret: Option<String>,
    /// Shared secret the worker must present on internal send-pipeline
    /// endpoints. Unset ⇒ those endpoints are disabled.
    pub email_internal_token: Option<String>,
    /// Outbound transport selection (none | smtp | postmark).
    pub email_transport: TransportKind,
    /// Malware scanner selection (none | clamd TCP).
    pub email_scan_clamd_addr: Option<String>,
    /// From-identity for Meridian-generated outbound mail.
    pub email_from_address: String,
    pub email_from_name: Option<String>,
    /// Bearer token guarding /api/internal/* machine endpoints used by the
    /// worker (§4.1 single-writer). Unset ⇒ the surface is disabled.
    pub internal_token: Option<String>,
    /// Object store selection for attachments (TASKLIST 2.14).
    pub object_store: core_store::StoreChoice,
    /// Per-file upload cap in bytes (default 25 MB; env can lower it).
    pub max_upload_bytes: i64,
    /// Migration connector (TASKLIST D3): per-principal request budget per
    /// minute on the /api/migration surface. Env
    /// `MERIDIAN_MIGRATION_RATE_LIMIT_PER_MIN` (default 600).
    pub migration_rate_limit_per_min: u32,
    /// Dispatch connector (TASKLIST Phase 3, Wave 9 lane 9): per-principal
    /// request budget per minute on the /api/dispatch-connector surface.
    /// Env `MERIDIAN_DISPATCH_RATE_LIMIT_PER_MIN` (default 600).
    pub dispatch_rate_limit_per_min: u32,
    /// Scheduling links (Wave 11 lane 4): per-token request budget per
    /// minute on the UNAUTHENTICATED public booking surface. Env
    /// `MERIDIAN_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN` (default 60; 0
    /// disables limiting).
    pub scheduling_links_rate_limit_per_min: u32,
    /// Native secrets vault (ADR 0006, Wave 16 lane 9 straggler sweep):
    /// per-ORG write-admission budget per fixed 60-second window across
    /// every secret-record/folder/grant mutation on `/api/secrets/*` and
    /// the chunked bulk-ingestion path. This is the noisy-neighbor floor:
    /// a runaway bulk importer consumes its own org's budget only.
    /// Env `MERIDIAN_VAULT_WRITE_BUDGET_PER_MIN` (default 600; 0
    /// disables limiting). Per-process counters — one API instance, one
    /// budget (the same posture as the migration dispatch budgets).
    pub vault_write_budget_per_min: u32,
    /// Plugin/skill artifact signing key (B1, Wave 16 lane 9, ADR 0009):
    /// the operator's ed25519 PUBLIC key (64 hex chars). When set, every
    /// registry/GitHub distribution artifact is cryptographically
    /// signature-verified against it (typed
    /// `distribution_signature_invalid` on failure); unset ⇒ structural
    /// verification only (dev posture — loudly logged at boot).
    /// Env `MERIDIAN_PLUGIN_SIGNING_KEY`.
    pub plugin_signing_key: Option<String>,
    /// Chunked bulk-secret ingestion (A2, Wave 16 lane 9): the rows/second
    /// pacing ceiling between per-row commits on the
    /// `POST /api/secrets/records/bulk` worker path. Env
    /// `MERIDIAN_VAULT_BULK_ROWS_PER_SEC` (default 50).
    pub vault_bulk_rows_per_sec: u32,
    /// Dev-tools gate (TASKLIST DX2): when `MERIDIAN_DEV_TOOLS=1` the
    /// `/api/dev/*` router mounts; otherwise those routes DO NOT EXIST (no
    /// registration at all, not a 403). Unset in production — never
    /// default-on.
    pub dev_tools: bool,
}

/// Parses OBJECT_STORE plus its companion vars. Unknown values degrade to
/// the documented local none mode rather than failing boot.
fn object_store_from_env() -> core_store::StoreChoice {
    match std::env::var("OBJECT_STORE").as_deref() {
        Ok("s3") => core_store::StoreChoice::S3(core_store::S3Config {
            endpoint: std::env::var("S3_ENDPOINT").ok().filter(|e| !e.is_empty()),
            bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "meridian".into()),
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key_id: std::env::var("S3_ACCESS_KEY_ID").unwrap_or_default(),
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY").unwrap_or_default(),
        }),
        _ => core_store::StoreChoice::Local(
            std::env::var("OBJECT_STORE_LOCAL_DIR").unwrap_or_else(|_| "./data/objects".into()),
        ),
    }
}

/// Which malware scanner the inbound path uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanChoice {
    None,
    Clamd(String),
}

impl Config {
    /// Dev JWT secret — the compose/dev default. The desktop local profile
    /// pins it so the demo seed's at-rest seal key derives identically.
    const DEV_JWT_SECRET: &'static str = "dev-only-insecure-secret-change-me";

    /// Local/desktop profile (desktop crate, ADR 0005): a clean
    /// constructor for the double-clickable test app — every external
    /// integration is off (no SMTP/Postmark, no inbound webhook, no
    /// S3/MinIO, no internal worker token, no OTel), dev tooling is on,
    /// and the API binds loopback only. Callers inject the embedded
    /// Postgres URL, API port, and object-store directory.
    pub fn from_local(database_url: String, api_port: u16, object_dir: std::path::PathBuf) -> Self {
        let jwt_secret = Config::DEV_JWT_SECRET.to_string();
        Self {
            database_url,
            // core-api never connects to Redis in-process (the realtime
            // hub rides Postgres LISTEN); the field exists for compose
            // wiring, so an unreachable placeholder documents that.
            redis_url: "redis://127.0.0.1:1/0".into(),
            bind_addr: format!("127.0.0.1:{api_port}"),
            jwt_secret,
            access_ttl_secs: 900,
            refresh_ttl_secs: 60 * 60 * 24 * 30,
            cors_origin: None,
            secret_key: derive_dev_key(Config::DEV_JWT_SECRET),
            mfa_pending_ttl_secs: 300,
            webauthn_rp_id: "localhost".into(),
            webauthn_rp_origin: "tauri://localhost".into(),
            public_base_url: format!("http://127.0.0.1:{api_port}"),
            web_base_url: "tauri://localhost".into(),
            email_inbound_hmac_secret: None,
            email_internal_token: None,
            email_transport: TransportKind::None,
            email_scan_clamd_addr: None,
            email_from_address: "support@meridian.local".into(),
            email_from_name: None,
            internal_token: None,
            object_store: core_store::StoreChoice::Local(object_dir.to_string_lossy().into_owned()),
            max_upload_bytes: core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
            migration_rate_limit_per_min: 600,
            dispatch_rate_limit_per_min: 600,
            scheduling_links_rate_limit_per_min: 60,
            vault_write_budget_per_min: 600,
            vault_bulk_rows_per_sec: 50,
            plugin_signing_key: None,
            dev_tools: true,
        }
    }

    #[must_use]
    pub fn scan_choice(&self) -> ScanChoice {
        match &self.email_scan_clamd_addr {
            Some(addr) => ScanChoice::Clamd(addr.clone()),
            None => ScanChoice::None,
        }
    }

    /// Returns the error that closes the confirmed 2026-09-16
    /// secrets-audit finding 'fallback-secrets': production boot refuses
    /// weak/absent secrets INSTEAD of silently falling back.
    ///
    /// Fail-closed rules (threat model §A3):
    /// * `JWT_SECRET` MUST exist and be exactly 64 hex chars.
    /// * `MERIDIAN_SECRET_KEY` (64 hex chars) is required in production -
    ///   the at-rest seal key NEVER silently derives from JWT_SECRET.
    ///
    /// Production detection: absence of the explicit dev markers. The dev
    /// fallback REMAINS available behind `MERIDIAN_DEV_TOOLS=1` (explicit
    /// dev profile, TASKLIST DX2 posture) and `from_local` — where the
    /// same DEV_JWT_SECRET context applies (ADR 0005). Failures return
    /// Err with operator-copy that names the missing variable.
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_inner(false)
    }

    /// EXPLICIT non-boot alias for integration tests (and the desktop
    /// shell, which pins its own dev secret): same parsing as
    /// [`Config::from_env`] but the dev fallback is the DECLARED posture
    /// — a test env without JWT_SECRET still boots with the loud dev
    /// secret, while production boot (from_env) hard-fails (§A3).
    pub fn from_env_allow_dev_fallback() -> Result<Self, String> {
        Self::from_env_inner(true)
    }

    fn from_env_inner(dev_fallback: bool) -> Result<Self, String> {
        let dev_tools = std::env::var("MERIDIAN_DEV_TOOLS").as_deref() == Ok("1") || dev_fallback;
        let jwt_secret = match std::env::var("JWT_SECRET") {
            Ok(secret) if is_64_hex(&secret) => secret,
            Ok(_) => {
                return Err(
                    "JWT_SECRET must be exactly 64 hex characters                      (generate: openssl rand -hex 32)"
                        .into(),
                )
            }
            Err(_) if dev_fallback => {
                // Explicit dev/test profile keeps the compose/dev default;
                // production boot (from_env) never reaches this arm.
                Config::DEV_JWT_SECRET.to_string()
            }
            Err(_) if dev_tools => {
                Config::DEV_JWT_SECRET.to_string()
            }
            Err(_) => {
                return Err(
                    "JWT_SECRET is not set — refusing to boot                      (MERIDIAN_DEV_TOOLS=1 enables the dev fallback)"
                        .into(),
                )
            }
        };
        let secret_key = match std::env::var("MERIDIAN_SECRET_KEY") {
            Ok(hex_key) if is_64_hex(&hex_key) => {
                let mut bytes = [0u8; 32];
                if hex::decode_to_slice(&hex_key, &mut bytes).is_err() {
                    return Err(
                        "MERIDIAN_SECRET_KEY is not valid hex (64 hex chars expected)"
                        .into(),
                    );
                }
                SecretKey(bytes)
            }
            Ok(_) => {
                return Err(
                    "MERIDIAN_SECRET_KEY must be exactly 64 hex characters"
                        .into(),
                )
            }
            // Dev fallback explicitly behind the dev profile: the derive
            // exists ONLY so the demo seed's at-rest seal key matches.
            Err(_) if dev_tools => {
                tracing::warn!(
                    "MERIDIAN_DEV_TOOLS=1: at-rest secret encryption derives from JWT_SECRET (dev fallback — never ship)"
                );
                derive_dev_key(&jwt_secret)
            }
            _ => {
                return Err(
                    "MERIDIAN_SECRET_KEY is not set — at-rest secret encryption needs an explicit key (64 hex chars). MERIDIAN_DEV_TOOLS=1 enables the dev fallback."
                        .into(),
                )
            }
        };
        Ok(Self {
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
            email_inbound_hmac_secret: std::env::var("EMAIL_INBOUND_HMAC_SECRET").ok(),
            email_internal_token: std::env::var("EMAIL_INTERNAL_TOKEN").ok(),
            email_transport: TransportKind::parse(
                &std::env::var("EMAIL_TRANSPORT").unwrap_or_default(),
                smtp_from_env(),
                postmark_from_env(),
            ),
            email_scan_clamd_addr: match std::env::var("EMAIL_SCAN").as_deref() {
                Ok("clamd") => {
                    Some(std::env::var("CLAMD_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into()))
                }
                _ => None,
            },
            email_from_address: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "support@meridian.local".into()),
            email_from_name: std::env::var("EMAIL_FROM_NAME").ok(),
            internal_token: std::env::var("MERIDIAN_INTERNAL_TOKEN").ok(),
            object_store: object_store_from_env(),
            max_upload_bytes: std::env::var("MERIDIAN_MAX_UPLOAD_MB")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES),
            migration_rate_limit_per_min: std::env::var("MERIDIAN_MIGRATION_RATE_LIMIT_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            dispatch_rate_limit_per_min: std::env::var("MERIDIAN_DISPATCH_RATE_LIMIT_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            scheduling_links_rate_limit_per_min: std::env::var(
                "MERIDIAN_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60),
            vault_write_budget_per_min: std::env::var("MERIDIAN_VAULT_WRITE_BUDGET_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            vault_bulk_rows_per_sec: std::env::var("MERIDIAN_VAULT_BULK_ROWS_PER_SEC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            plugin_signing_key: std::env::var("MERIDIAN_PLUGIN_SIGNING_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            dev_tools,
        })
    }
}

/// The audit's hex discipline: exactly 64 hex chars = 32 bytes.
fn is_64_hex(v: &str) -> bool {
    v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit())
}

/// Single source of truth for the dev fallback derivation lives in
/// core-db::demo (the CLI seeder must derive the SAME key or the demo
/// secrets it seals are unopenable by this API).
fn derive_dev_key(jwt_secret: &str) -> SecretKey {
    SecretKey(core_db::demo::derive_dev_seal_key(jwt_secret))
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
            email_inbound_hmac_secret: std::env::var("EMAIL_INBOUND_HMAC_SECRET").ok(),
            email_internal_token: std::env::var("EMAIL_INTERNAL_TOKEN").ok(),
            email_transport: TransportKind::parse(
                &std::env::var("EMAIL_TRANSPORT").unwrap_or_default(),
                smtp_from_env(),
                postmark_from_env(),
            ),
            email_scan_clamd_addr: match std::env::var("EMAIL_SCAN").as_deref() {
                Ok("clamd") => {
                    Some(std::env::var("CLAMD_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into()))
                }
                _ => None,
            },
            email_from_address: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "support@meridian.local".into()),
            email_from_name: std::env::var("EMAIL_FROM_NAME").ok(),
            internal_token: std::env::var("MERIDIAN_INTERNAL_TOKEN").ok(),
            object_store: object_store_from_env(),
            max_upload_bytes: std::env::var("MERIDIAN_MAX_UPLOAD_MB")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES),
            migration_rate_limit_per_min: 600,
            dispatch_rate_limit_per_min: 600,
            scheduling_links_rate_limit_per_min: 60,
            vault_write_budget_per_min: 600,
            vault_bulk_rows_per_sec: 50,
            plugin_signing_key: None,
            dev_tools: false,
        }
    }
}

fn smtp_from_env() -> Option<(String, u16, Option<String>, Option<String>)> {
    let host = std::env::var("SMTP_HOST").ok()?;
    let port = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(25);
    Some((
        host,
        port,
        std::env::var("SMTP_USERNAME").ok(),
        std::env::var("SMTP_PASSWORD").ok(),
    ))
}

fn postmark_from_env() -> Option<(String, String)> {
    let token = std::env::var("POSTMARK_SERVER_TOKEN").ok()?;
    Some((
        token,
        std::env::var("POSTMARK_BASE_URL").unwrap_or_else(|_| "https://api.postmarkapp.com".into()),
    ))
}
#[cfg(test)]
mod hardening_tests {
    use super::*;

    /// The from_env probes mutate the process env: serialized + restored
    /// here so parallel tests stay deterministic.
    static ENV_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct CleanEnv {
        state: Vec<(String, Option<String>)>,
        _held: std::sync::MutexGuard<'static, ()>,
    }

    impl CleanEnv {
        /// Holds the env-mutation lock for the WHOLE test body, then
        /// restores every snapped variable on drop (parallel safety).
        fn snapshot() -> Self {
            let mutex = ENV_MUTEX.get_or_init(|| std::sync::Mutex::new(()));
            let _held = mutex.lock().unwrap();
            let mut state = Vec::new();
            for key in ["JWT_SECRET", "MERIDIAN_SECRET_KEY", "MERIDIAN_DEV_TOOLS"] {
                state.push((key.to_string(), std::env::var(key).ok()));
            }
            CleanEnv { state, _held }
        }
    }

    impl Drop for CleanEnv {
        fn drop(&mut self) {
            for (key, value) in &self.state {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn is_64_hex_is_what_the_hard_gate_uses() {
        assert!(!is_64_hex("a".repeat(32).as_str()));
        assert!(is_64_hex("f".repeat(64).as_str()));
        assert!(!is_64_hex("0".repeat(63).as_str()));
        assert!(!is_64_hex("z".repeat(64).as_str()));
    }

    #[test]
    fn production_boot_fails_closed_without_jwt_secret() {
        let _guard = CleanEnv::snapshot();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("MERIDIAN_DEV_TOOLS");
        std::env::remove_var("MERIDIAN_SECRET_KEY");
        let err = Config::from_env().expect_err("production boot must hard-fail");
        assert!(
            err.contains("JWT_SECRET"),
            "operator copy names the variable: {err}"
        );
    }

    #[test]
    fn production_boot_fails_closed_on_short_jwt_secret() {
        let _guard = CleanEnv::snapshot();
        std::env::set_var("JWT_SECRET", "short");
        std::env::remove_var("MERIDIAN_DEV_TOOLS");
        let err = Config::from_env().expect_err("short secret must fail");
        assert!(
            err.contains("64 hex"),
            "operator copy names the discipline: {err}"
        );
    }

    #[test]
    fn production_boot_requires_explicit_secret_key() {
        let _guard = CleanEnv::snapshot();
        std::env::set_var("JWT_SECRET", "f".repeat(64));
        std::env::remove_var("MERIDIAN_SECRET_KEY");
        std::env::remove_var("MERIDIAN_DEV_TOOLS");
        let err = Config::from_env().expect_err("missing secret key must fail");
        assert!(
            err.contains("MERIDIAN_SECRET_KEY"),
            "operator copy names the variable: {err}"
        );
    }

    #[test]
    fn dev_profile_keeps_the_explicit_fallback() {
        let _guard = CleanEnv::snapshot();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("MERIDIAN_SECRET_KEY");
        std::env::set_var("MERIDIAN_DEV_TOOLS", "1");
        let cfg = Config::from_env().expect("dev profile boots");
        assert_eq!(cfg.jwt_secret, Config::DEV_JWT_SECRET);
    }

    #[test]
    fn test_alias_keeps_the_explicit_dev_fallback_even_without_dev_tools() {
        let _guard = CleanEnv::snapshot();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("MERIDIAN_SECRET_KEY");
        std::env::remove_var("MERIDIAN_DEV_TOOLS");
        let cfg = Config::from_env_allow_dev_fallback().expect("test alias boots");
        assert_eq!(cfg.jwt_secret, Config::DEV_JWT_SECRET);
    }
}
