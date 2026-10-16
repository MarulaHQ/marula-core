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

/// Asymmetric access-token signing material (Wave 34 lane 6, threat-model
/// §A4 upgrade path). When present, every token this process issues is
/// signed with EdDSA (Ed25519) and stamped with `kid`; verification
/// prefers EdDSA and keeps accepting HS256 while `jwt_secret` is deployed
/// (rolling-migration posture — see `auth::jwt`).
#[derive(Clone)]
pub struct JwtSigningKey {
    /// PKCS#8 PEM of the Ed25519 private key (never logged).
    pub private_pem: String,
    /// PUBLIC key PEM for verification (SPKI).
    pub public_pem: String,
    /// `kid` stamped on every token header this process issues.
    pub kid: String,
}

impl std::fmt::Debug for JwtSigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtSigningKey")
            .field("kid", &self.kid)
            .field("private_pem", &"<redacted>")
            .field("public_pem", &"<redacted>")
            .finish()
    }
}

use core_mail::{TransportCredentials, TransportKind};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    /// Asymmetric signing upgrade (threat-model §A4, Wave 34 lane 6):
    /// `MARULA_JWT_EDDSA_PRIVATE_KEY` (inline PEM) or
    /// `MARULA_JWT_EDDSA_PRIVATE_KEY_FILE` (path) turns on EdDSA
    /// signing; `MARULA_JWT_EDDSA_PUBLIC_KEY` / `..._FILE` supplies the
    /// verification key (required with the private key); `MARULA_JWT_KID`
    /// names the key (default `m1`). Both PEMs are parsed at boot — a
    /// malformed key refuses to boot. Unset ⇒ HS256 on `jwt_secret`,
    /// exactly as before.
    pub jwt_signing: Option<JwtSigningKey>,
    /// Access token TTL in seconds (default 15 min).
    pub access_ttl_secs: i64,
    /// Refresh token TTL in seconds (default 30 days).
    pub refresh_ttl_secs: i64,
    pub cors_origin: Option<String>,
    /// Key for AES-256-GCM at-rest encryption of integration secrets
    /// (OIDC client secrets, TOTP secrets). Env `MARULA_SECRET_KEY`
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
    /// From-identity for Marula-generated outbound mail.
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
    /// `MARULA_MIGRATION_RATE_LIMIT_PER_MIN` (default 600).
    pub migration_rate_limit_per_min: u32,
    /// Dispatch connector (TASKLIST Phase 3, Wave 9 lane 9): per-principal
    /// request budget per minute on the /api/dispatch-connector surface.
    /// Env `MARULA_DISPATCH_RATE_LIMIT_PER_MIN` (default 600).
    pub dispatch_rate_limit_per_min: u32,
    /// Scheduling links (Wave 11 lane 4): per-token request budget per
    /// minute on the UNAUTHENTICATED public booking surface. Env
    /// `MARULA_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN` (default 60; 0
    /// disables limiting).
    pub scheduling_links_rate_limit_per_min: u32,
    /// Native secrets vault (ADR 0006, Wave 16 lane 9 straggler sweep):
    /// per-ORG write-admission budget per fixed 60-second window across
    /// every secret-record/folder/grant mutation on `/api/secrets/*` and
    /// the chunked bulk-ingestion path. This is the noisy-neighbor floor:
    /// a runaway bulk importer consumes its own org's budget only.
    /// Env `MARULA_VAULT_WRITE_BUDGET_PER_MIN` (default 600; 0
    /// disables limiting). Per-process counters — one API instance, one
    /// budget (the same posture as the migration dispatch budgets).
    pub vault_write_budget_per_min: u32,
    /// Plugin/skill artifact signing keys (B1, Wave 16 lane 9 → Wave 19
    /// lane 2 rotation full semantics, ADR 0009): the operator's trusted
    /// ed25519 PUBLIC keys (64 hex chars each) as a TRUST SET. Env
    /// `MARULA_PLUGIN_SIGNING_KEYS` (comma-separated `key_id=hex`
    /// entries) joins the legacy single-key `MARULA_PLUGIN_SIGNING_KEY`
    /// (merged as an implicit derived key-id). When the trust set is
    /// non-empty, every registry/GitHub distribution artifact is
    /// cryptographically signature-verified against it (typed refusals:
    /// `distribution_signature_invalid`, `distribution_signature_key_unknown`);
    /// an EMPTY trust set degrades to structural verification only (dev
    /// posture — loudly logged at boot). The DB registry
    /// (`plugin_signing_keys`) overrides this static set at request time.
    /// Env `MARULA_PLUGIN_SIGNING_KEYS`, `MARULA_PLUGIN_SIGNING_KEY`.
    pub plugin_signing_key: Option<String>,
    /// Wave 19 lane 2 (ADR 0009 rotation full semantics): ADDITIONAL
    /// trusted keys in the rotation trust set, comma-separated
    /// `key_id=public_key_hex` entries (key_id optional — `=…` entries
    /// derive the default first-16-hex id). Merged with the legacy
    /// `plugin_signing_key` into the static bootstrap trust set; the DB
    /// registry wins at request time. Env `MARULA_PLUGIN_SIGNING_KEYS`.
    pub plugin_signing_keys: Option<String>,
    /// Chunked bulk-secret ingestion (A2, Wave 16 lane 9): the rows/second
    /// pacing ceiling between per-row commits on the
    /// `POST /api/secrets/records/bulk` worker path. Env
    /// `MARULA_VAULT_BULK_ROWS_PER_SEC` (default 50).
    pub vault_bulk_rows_per_sec: u32,
    /// Dev-tools gate (TASKLIST DX2): when `MARULA_DEV_TOOLS=1` the
    /// `/api/dev/*` router mounts; otherwise those routes DO NOT EXIST (no
    /// registration at all, not a 403). Unset in production — never
    /// default-on.
    pub dev_tools: bool,
    /// Routing provider selection (Wave 32 lane 7, ADR 0014): operator
    /// infrastructure config, env-parsed like the email transport because
    /// credentials are operator secrets — never org settings.
    /// `ROUTING_PROVIDER` = `none` (default; built-in heuristic) | `osrm`
    /// | `ors` | `mapbox` | `contract`.
    pub routing: RoutingChoice,
    /// Trusted-proxy posture for real-client-IP resolution (Wave 47 lane
    /// A10): which direct peers may speak for another address, which
    /// peers are Cloudflare edges, and how much authority
    /// `CF-Connecting-IP` carries. Env:
    /// * `TRUSTED_PROXIES` — comma-separated CIDRs. UNSET ⇒ the narrow
    ///   default set (loopback + RFC1918 + IPv6 ULA), which keeps same-host
    ///   and in-VPC proxies working; an explicit list REPLACES it, which is
    ///   what an edge deployment does.
    /// * `CF_CONNECTING_IP` — `auto` (default: believe the header only
    ///   from a trusted Cloudflare range) | `true` (believe it from any
    ///   trusted peer) | `false` (ignore it).
    /// * `CF_IP_RANGES` — comma-separated CIDRs that EXTEND the versioned
    ///   Cloudflare range constant in `crate::client_ip` (Cloudflare
    ///   publishes new ranges between releases).
    ///
    /// Malformed entries are dropped loudly at boot and never fail it.
    /// See [`crate::client_ip`] for the resolution algorithm.
    pub trusted_proxies: crate::client_ip::TrustedProxies,
    /// App-pool ceiling (Wave 49 lane R1). Env `MARULA_DB_POOL_MAX`
    /// (default 20). A present-but-invalid value refuses boot; see
    /// [`parse_pool_max`].
    pub db_pool_max: u32,
    /// Reporting-pool ceiling (Wave 49 lane R1). Env
    /// `MARULA_REPORTING_POOL_MAX` (default 25). The reporting pool runs
    /// the app role under RLS, a different privilege profile from the app
    /// pool, so it carries its own ceiling.
    pub reporting_pool_max: u32,
}

/// Which distance/duration provider the process registers (Wave 32 lane 7).
/// `None` = NONE-mode: the built-in per-mile heuristic answers and
/// suggestion-grade features stay silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingChoice {
    None,
    /// Self-hosted OSRM table/route server.
    Osrm {
        base_url: String,
    },
    /// Self-hosted or hosted OpenRouteService.
    Ors {
        base_url: String,
        api_key: Option<String>,
    },
    /// Mapbox (first-party in-core provider).
    Mapbox {
        access_token: String,
        profile: String,
    },
    /// An out-of-tree pack speaking the Marula Routing Contract
    /// (`docs/routing-providers.md`).
    Contract {
        id: String,
        base_url: String,
        token: Option<String>,
    },
}

/// Parses OBJECT_STORE plus its companion vars. Unknown values degrade to
/// the documented local none mode rather than failing boot.
fn object_store_from_env() -> core_store::StoreChoice {
    match std::env::var("OBJECT_STORE").as_deref() {
        Ok("s3") => core_store::StoreChoice::S3(core_store::S3Config {
            endpoint: std::env::var("S3_ENDPOINT").ok().filter(|e| !e.is_empty()),
            bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "marula".into()),
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key_id: std::env::var("S3_ACCESS_KEY_ID").unwrap_or_default(),
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY").unwrap_or_default(),
        }),
        _ => core_store::StoreChoice::Local(
            std::env::var("OBJECT_STORE_LOCAL_DIR").unwrap_or_else(|_| "./data/objects".into()),
        ),
    }
}

/// Parses the routing provider selection plus its companion vars (Wave 32
/// lane 7). Unset / unknown values degrade to the documented NONE mode
/// (built-in heuristic) rather than failing boot — a missing routing
/// server never takes scheduling down. A named provider with an empty
/// required credential degrades the same way, loudly.
fn routing_from_env() -> RoutingChoice {
    match std::env::var("ROUTING_PROVIDER").as_deref() {
        Ok("osrm") => {
            let base_url = std::env::var("ROUTING_OSRM_URL").unwrap_or_default();
            if base_url.trim().is_empty() {
                tracing::warn!(
                    "ROUTING_PROVIDER=osrm without ROUTING_OSRM_URL — staying in NONE mode"
                );
                return RoutingChoice::None;
            }
            RoutingChoice::Osrm { base_url }
        }
        Ok("ors") => {
            let base_url = std::env::var("ROUTING_ORS_URL").unwrap_or_default();
            if base_url.trim().is_empty() {
                tracing::warn!(
                    "ROUTING_PROVIDER=ors without ROUTING_ORS_URL — staying in NONE mode"
                );
                return RoutingChoice::None;
            }
            RoutingChoice::Ors {
                base_url,
                api_key: std::env::var("ROUTING_ORS_API_KEY")
                    .ok()
                    .filter(|v| !v.trim().is_empty()),
            }
        }
        Ok("mapbox") => {
            let access_token = std::env::var("MAPBOX_ACCESS_TOKEN").unwrap_or_default();
            if access_token.trim().is_empty() {
                tracing::warn!(
                    "ROUTING_PROVIDER=mapbox without MAPBOX_ACCESS_TOKEN — staying in NONE mode"
                );
                return RoutingChoice::None;
            }
            RoutingChoice::Mapbox {
                access_token,
                profile: std::env::var("MAPBOX_PROFILE")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "driving".into()),
            }
        }
        Ok("contract") => {
            let base_url = std::env::var("ROUTING_CONTRACT_URL").unwrap_or_default();
            if base_url.trim().is_empty() {
                tracing::warn!(
                    "ROUTING_PROVIDER=contract without ROUTING_CONTRACT_URL — staying in NONE mode"
                );
                return RoutingChoice::None;
            }
            RoutingChoice::Contract {
                id: std::env::var("ROUTING_CONTRACT_ID")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "pack".into()),
                base_url,
                token: std::env::var("ROUTING_CONTRACT_TOKEN")
                    .ok()
                    .filter(|v| !v.trim().is_empty()),
            }
        }
        Ok("none") | Ok("") | Err(_) => RoutingChoice::None,
        Ok(other) => {
            tracing::warn!(
                provider = other,
                "unknown ROUTING_PROVIDER — staying in NONE mode"
            );
            RoutingChoice::None
        }
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
            jwt_signing: None,
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
            email_from_address: "support@marula.local".into(),
            email_from_name: None,
            internal_token: None,
            object_store: core_store::StoreChoice::Local(object_dir.to_string_lossy().into_owned()),
            max_upload_bytes: core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
            db_pool_max: core_db::DEFAULT_POOL_MAX,
            reporting_pool_max: 25,
            migration_rate_limit_per_min: 600,
            dispatch_rate_limit_per_min: 600,
            scheduling_links_rate_limit_per_min: 60,
            vault_write_budget_per_min: 600,
            vault_bulk_rows_per_sec: 50,
            plugin_signing_key: None,
            plugin_signing_keys: None,
            dev_tools: true,
            // Desktop/test profile: NONE mode — never dials a routing
            // service the operator did not configure.
            routing: RoutingChoice::None,
            // Loopback-only bind: the default trust set (loopback +
            // RFC1918) is already correct for a same-host proxy.
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
        }
    }

    #[must_use]
    pub fn scan_choice(&self) -> ScanChoice {
        match &self.email_scan_clamd_addr {
            Some(addr) => ScanChoice::Clamd(addr.clone()),
            None => ScanChoice::None,
        }
    }

    /// App-pool sizing handed to `core_db::connect_with` (Wave 49 lane R1).
    #[must_use]
    pub fn pool_config(&self) -> core_db::PoolConfig {
        core_db::PoolConfig {
            max_connections: self.db_pool_max,
            acquire_timeout: std::time::Duration::from_secs(core_db::DEFAULT_ACQUIRE_TIMEOUT_SECS),
        }
    }

    /// First-boot sanity check for the trusted-proxy posture (Wave 47 lane
    /// A10). WARNS, never fails — the same posture as the banned-defaults
    /// and plugin-signing-key checks in the binary's startup sequence: an
    /// operator must be told about the spoofing implication, not locked
    /// out of booting while they read about it.
    ///
    /// Fires when the bind is internet-reachable while only the narrow
    /// default trust set is configured. The default set trusts loopback and
    /// RFC1918 space, which is right for compose and in-VPC proxies and
    /// wrong for an edge deployment: a Cloudflare (or any public) peer is
    /// then untrusted, so `X-Forwarded-For` from that path carries no
    /// authority and every real client collapses onto the edge IP for the
    /// per-IP throttle keys. Naming the fix in the warning is the point.
    pub fn warn_trusted_proxy_posture(&self) {
        if !self.trusted_proxies.is_default() {
            return;
        }
        if !crate::client_ip::bind_is_internet_reachable(&self.bind_addr) {
            return;
        }
        tracing::warn!(
            bind = %self.bind_addr,
            "TRUSTED_PROXIES is unset on an internet-reachable bind: the default trust set \
             (loopback + RFC1918) is in force, so proxy headers from PUBLIC peers are ignored \
             and every request resolves to its direct peer address. Behind Cloudflare or any \
             public load balancer that poisons the per-IP throttle keys (the per-actor keys stay \
             authoritative) — set TRUSTED_PROXIES to your edge CIDRs (Cloudflare's are listed in \
             crates/core-api/src/client_ip.rs) and, for Cloudflare, keep CF_CONNECTING_IP=auto."
        );
    }

    /// Returns the error that closes the confirmed 2026-09-16
    /// secrets-audit finding 'fallback-secrets': production boot refuses
    /// weak/absent secrets INSTEAD of silently falling back.
    ///
    /// Fail-closed rules (threat model §A3):
    /// * `JWT_SECRET` MUST exist and be exactly 64 hex chars.
    /// * `MARULA_SECRET_KEY` (64 hex chars) is required in production -
    ///   the at-rest seal key NEVER silently derives from JWT_SECRET.
    ///
    /// Production detection: absence of the explicit dev markers. The dev
    /// fallback REMAINS available behind `MARULA_DEV_TOOLS=1` (explicit
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
        let dev_tools = std::env::var("MARULA_DEV_TOOLS").as_deref() == Ok("1") || dev_fallback;
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
                    "JWT_SECRET is not set — refusing to boot                      (MARULA_DEV_TOOLS=1 enables the dev fallback)"
                        .into(),
                )
            }
        };
        let secret_key = match std::env::var("MARULA_SECRET_KEY") {
            Ok(hex_key) if is_64_hex(&hex_key) => {
                let mut bytes = [0u8; 32];
                if hex::decode_to_slice(&hex_key, &mut bytes).is_err() {
                    return Err(
                        "MARULA_SECRET_KEY is not valid hex (64 hex chars expected)"
                        .into(),
                    );
                }
                SecretKey(bytes)
            }
            Ok(_) => {
                return Err(
                    "MARULA_SECRET_KEY must be exactly 64 hex characters"
                        .into(),
                )
            }
            // Dev fallback explicitly behind the dev profile: the derive
            // exists ONLY so the demo seed's at-rest seal key matches.
            Err(_) if dev_tools => {
                tracing::warn!(
                    "MARULA_DEV_TOOLS=1: at-rest secret encryption derives from JWT_SECRET (dev fallback — never ship)"
                );
                derive_dev_key(&jwt_secret)
            }
            _ => {
                return Err(
                    "MARULA_SECRET_KEY is not set — at-rest secret encryption needs an explicit key (64 hex chars). MARULA_DEV_TOOLS=1 enables the dev fallback."
                        .into(),
                )
            }
        };
        let jwt_signing = jwt_signing_from_env()?;
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://marula_app:marula_app@localhost:15432/marula".into()
            }),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:16379/0".into()),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            jwt_secret,
            jwt_signing,
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
                TransportCredentials {
                    smtp: smtp_from_env(),
                    postmark: postmark_from_env(),
                    ses: ses_from_env(),
                },
            ),
            email_scan_clamd_addr: match std::env::var("EMAIL_SCAN").as_deref() {
                Ok("clamd") => {
                    Some(std::env::var("CLAMD_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into()))
                }
                _ => None,
            },
            email_from_address: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "support@marula.local".into()),
            email_from_name: std::env::var("EMAIL_FROM_NAME").ok(),
            internal_token: std::env::var("MARULA_INTERNAL_TOKEN").ok(),
            object_store: object_store_from_env(),
            max_upload_bytes: std::env::var("MARULA_MAX_UPLOAD_MB")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES),
            db_pool_max: parse_pool_max("MARULA_DB_POOL_MAX", core_db::DEFAULT_POOL_MAX)?,
            reporting_pool_max: parse_pool_max("MARULA_REPORTING_POOL_MAX", 25)?,
            migration_rate_limit_per_min: std::env::var("MARULA_MIGRATION_RATE_LIMIT_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            dispatch_rate_limit_per_min: std::env::var("MARULA_DISPATCH_RATE_LIMIT_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            scheduling_links_rate_limit_per_min: std::env::var(
                "MARULA_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60),
            vault_write_budget_per_min: std::env::var("MARULA_VAULT_WRITE_BUDGET_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            vault_bulk_rows_per_sec: std::env::var("MARULA_VAULT_BULK_ROWS_PER_SEC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            plugin_signing_key: std::env::var("MARULA_PLUGIN_SIGNING_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            plugin_signing_keys: std::env::var("MARULA_PLUGIN_SIGNING_KEYS")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            dev_tools,
            routing: routing_from_env(),
            trusted_proxies: crate::client_ip::TrustedProxies::from_env(),
        })
    }
}

/// The STATIC bootstrap signing-key trust set (Wave 19 lane 2 rotation
/// semantics): the legacy single key merged with the `key_id=hex` list,
/// de-duplicated by key_id; malformed entries are logged loudly and
/// dropped (never a boot failure — an operator fixing a typo must not
/// take the core down; the DB registry remains the live source of intent
/// and the env set feeds resilience when the DB registry is unreachable
/// or still empty pre-runbook). An EMPTY result disables cryptographic
/// enforcement (the legacy dev posture, logged at boot).
pub fn bootstrap_signing_keys(
    config: &Config,
) -> Vec<plugin_host::distribution::TrustedSigningKey> {
    use plugin_host::distribution::TrustedSigningKey;
    let mut keys: Vec<TrustedSigningKey> = Vec::new();
    let mut push = |key_id: Option<&str>, public_key_hex: &str| {
        let pk = public_key_hex.trim().to_lowercase();
        if pk.len() != 64 || !pk.bytes().all(|b| b.is_ascii_hexdigit()) {
            eprintln!(
                "[plugins] MARULA signing key dropped: public key must be 64 hex chars (got {} chars)",
                pk.len()
            );
            return;
        }
        let key_id = key_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| pk.chars().take(16).collect());
        if keys.iter().any(|k| k.key_id == key_id) {
            return;
        }
        keys.push(TrustedSigningKey {
            key_id,
            public_key: pk,
        });
    };
    if let Some(legacy) = config.plugin_signing_key.as_deref() {
        push(None, legacy);
    }
    if let Some(list) = config.plugin_signing_keys.as_deref() {
        for entry in list.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            match entry.split_once('=') {
                Some((key_id, pk)) => push(Some(key_id), pk),
                // A bare hex entry rides the derived default id.
                None => push(None, entry),
            }
        }
    }
    keys
}

#[cfg(test)]
mod signing_keys_tests {
    use super::*;

    #[test]
    fn empty_config_yields_an_empty_trust_set() {
        let config = Config::test_with_secret("s");
        assert!(bootstrap_signing_keys(&config).is_empty());
    }

    #[test]
    fn legacy_and_list_entries_merge_deduplicated() {
        let pk_a = "a".repeat(64);
        let mut config = Config::test_with_secret("s");
        config.plugin_signing_key = Some(pk_a.clone());
        config.plugin_signing_keys = Some(format!("ops-1={pk_a},ops-2={}", "b".repeat(64)));
        let keys = bootstrap_signing_keys(&config);
        // Dedup is by KEY ID, not by key material: the same public key
        // under two identities (legacy derived id + a named id) is TWO
        // trust identities (an artifact may cite either).
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0].key_id, pk_a.chars().take(16).collect::<String>());
        assert_eq!(keys[1].key_id, "ops-1");
        assert_eq!(keys[2].key_id, "ops-2");
    }

    #[test]
    fn malformed_entries_dropped_loudly_without_failing_boot() {
        let mut config = Config::test_with_secret("s");
        config.plugin_signing_keys = Some("ops-bad=zzz,ops-good=".to_string() + &"c".repeat(64));
        let keys = bootstrap_signing_keys(&config);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_id, "ops-good");
    }
}

/// The audit's hex discipline: exactly 64 hex chars = 32 bytes.
fn is_64_hex(v: &str) -> bool {
    v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit())
}

/// EdDSA signing/verification key resolution (threat-model §A4). Both
/// halves come from env (inline PEM or a mounted file); the private key
/// without a public key is a boot refusal, because the process must be
/// able to verify what it issues. Parsing goes through the SAME
/// constructors signing/verification use, so a bad PEM fails at boot
/// with operator copy instead of at the first login.
fn jwt_signing_from_env() -> Result<Option<JwtSigningKey>, String> {
    fn read_key(inline_var: &str, file_var: &str) -> Result<Option<String>, String> {
        let inline = std::env::var(inline_var)
            .ok()
            .filter(|v| !v.trim().is_empty());
        let file = std::env::var(file_var)
            .ok()
            .filter(|v| !v.trim().is_empty());
        match (inline, file) {
            (Some(_), Some(_)) => Err(format!(
                "set EITHER {inline_var} (inline PEM) OR {file_var} (path), not both"
            )),
            (Some(pem), None) => Ok(Some(pem)),
            (None, Some(path)) => {
                let path = path.trim().to_string();
                std::fs::read_to_string(&path)
                    .map(Some)
                    .map_err(|e| format!("{file_var}: cannot read {path}: {e}"))
            }
            (None, None) => Ok(None),
        }
    }

    let private_pem = read_key(
        "MARULA_JWT_EDDSA_PRIVATE_KEY",
        "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
    )?;
    let public_pem = read_key(
        "MARULA_JWT_EDDSA_PUBLIC_KEY",
        "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
    )?;
    let (private_pem, public_pem) = match (private_pem, public_pem) {
        (None, None) => return Ok(None),
        (Some(_), None) => {
            return Err(
                "MARULA_JWT_EDDSA_PRIVATE_KEY is set without MARULA_JWT_EDDSA_PUBLIC_KEY — \
                 the process must be able to verify the tokens it signs (generate the pair with: \
                 openssl genpkey -algorithm ED25519)"
                    .into(),
            )
        }
        (None, Some(_)) => {
            return Err(
                "MARULA_JWT_EDDSA_PUBLIC_KEY is set without MARULA_JWT_EDDSA_PRIVATE_KEY — \
                 core-api both signs and verifies; the public-only posture belongs to realtime"
                    .into(),
            )
        }
        (Some(private), Some(public)) => (private, public),
    };
    jsonwebtoken::EncodingKey::from_ed_pem(private_pem.as_bytes()).map_err(|e| {
        format!("MARULA_JWT_EDDSA_PRIVATE_KEY is not a valid Ed25519 PKCS#8 PEM: {e}")
    })?;
    jsonwebtoken::DecodingKey::from_ed_pem(public_pem.as_bytes()).map_err(|e| {
        format!("MARULA_JWT_EDDSA_PUBLIC_KEY is not a valid Ed25519 public-key PEM: {e}")
    })?;
    let kid = std::env::var("MARULA_JWT_KID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "m1".into());
    Ok(Some(JwtSigningKey {
        private_pem,
        public_pem,
        kid,
    }))
}

/// Single source of truth for the dev fallback derivation lives in
/// core-db::demo (the CLI seeder must derive the SAME key or the demo
/// secrets it seals are unopenable by this API).
/// Parses a pool-ceiling env var (Wave 49 lane R1).
///
/// Fail-closed on a present-but-invalid value: a wrong pool size is a
/// capacity incident, not a cosmetic one, so garbage refuses boot rather
/// than silently reverting to the default.
fn parse_pool_max(var: &str, default: u32) -> Result<u32, String> {
    parse_pool_max_value(std::env::var(var).ok(), default, var)
}

fn parse_pool_max_value(raw: Option<String>, default: u32, var: &str) -> Result<u32, String> {
    match raw {
        None => Ok(default),
        Some(v) => match v.trim().parse::<u32>() {
            Ok(n) if (1..=1024).contains(&n) => Ok(n),
            Ok(n) => Err(format!("{var} must be between 1 and 1024 (got {n})")),
            Err(_) => Err(format!("{var} must be a positive integer (got {v:?})")),
        },
    }
}

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
            jwt_signing: None,
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
                TransportCredentials {
                    smtp: smtp_from_env(),
                    postmark: postmark_from_env(),
                    ses: ses_from_env(),
                },
            ),
            email_scan_clamd_addr: match std::env::var("EMAIL_SCAN").as_deref() {
                Ok("clamd") => {
                    Some(std::env::var("CLAMD_ADDR").unwrap_or_else(|_| "127.0.0.1:3310".into()))
                }
                _ => None,
            },
            email_from_address: std::env::var("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|_| "support@marula.local".into()),
            email_from_name: std::env::var("EMAIL_FROM_NAME").ok(),
            internal_token: std::env::var("MARULA_INTERNAL_TOKEN").ok(),
            object_store: object_store_from_env(),
            max_upload_bytes: std::env::var("MARULA_MAX_UPLOAD_MB")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES),
            db_pool_max: core_db::DEFAULT_POOL_MAX,
            reporting_pool_max: 25,
            migration_rate_limit_per_min: 600,
            dispatch_rate_limit_per_min: 600,
            scheduling_links_rate_limit_per_min: 60,
            vault_write_budget_per_min: 600,
            vault_bulk_rows_per_sec: 50,
            plugin_signing_key: None,
            plugin_signing_keys: None,
            dev_tools: false,
            routing: RoutingChoice::None,
            trusted_proxies: crate::client_ip::TrustedProxies::from_env(),
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

/// SES v2 credentials, or `None` when the pair is incomplete.
///
/// BOTH the key id and the secret are required, and a half-configured SES
/// resolves to `None` (which `TransportKind::parse` turns into none-mode)
/// rather than to a partially-signed adapter. An access key id with no
/// secret cannot sign anything, and an adapter that 403s on every send is a
/// worse outcome than one that keeps the mail queued and says why.
fn ses_from_env() -> Option<(String, String, String, Option<String>)> {
    let access_key_id = std::env::var("AWS_SES_ACCESS_KEY_ID").ok()?;
    let secret_access_key = std::env::var("AWS_SES_SECRET_ACCESS_KEY").ok()?;
    if access_key_id.trim().is_empty() || secret_access_key.trim().is_empty() {
        return None;
    }
    // Region is part of the endpoint, not a preference, so it has a default
    // rather than being optional: a missing region that defaulted to
    // "whatever" would send to the wrong data centre.
    let region = std::env::var("AWS_SES_REGION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "eu-west-1".into());
    // Optional override for a VPC endpoint or a local capture server.
    let endpoint = std::env::var("AWS_SES_ENDPOINT")
        .ok()
        .filter(|v| !v.trim().is_empty());
    Some((access_key_id, secret_access_key, region, endpoint))
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
            for key in [
                "JWT_SECRET",
                "MARULA_SECRET_KEY",
                "MARULA_DEV_TOOLS",
                "MARULA_JWT_EDDSA_PRIVATE_KEY",
                "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
                "MARULA_JWT_EDDSA_PUBLIC_KEY",
                "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
                "MARULA_JWT_KID",
            ] {
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
        std::env::remove_var("MARULA_DEV_TOOLS");
        std::env::remove_var("MARULA_SECRET_KEY");
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
        std::env::remove_var("MARULA_DEV_TOOLS");
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
        std::env::remove_var("MARULA_SECRET_KEY");
        std::env::remove_var("MARULA_DEV_TOOLS");
        let err = Config::from_env().expect_err("missing secret key must fail");
        assert!(
            err.contains("MARULA_SECRET_KEY"),
            "operator copy names the variable: {err}"
        );
    }

    #[test]
    fn dev_profile_keeps_the_explicit_fallback() {
        let _guard = CleanEnv::snapshot();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("MARULA_SECRET_KEY");
        std::env::set_var("MARULA_DEV_TOOLS", "1");
        let cfg = Config::from_env().expect("dev profile boots");
        assert_eq!(cfg.jwt_secret, Config::DEV_JWT_SECRET);
    }

    #[test]
    fn test_alias_keeps_the_explicit_dev_fallback_even_without_dev_tools() {
        let _guard = CleanEnv::snapshot();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("MARULA_SECRET_KEY");
        std::env::remove_var("MARULA_DEV_TOOLS");
        let cfg = Config::from_env_allow_dev_fallback().expect("test alias boots");
        assert_eq!(cfg.jwt_secret, Config::DEV_JWT_SECRET);
    }

    // --- EdDSA signing upgrade env contract (Wave 34 lane 6) -------------

    #[test]
    fn eddsa_private_key_without_public_key_refuses_boot() {
        let _guard = CleanEnv::snapshot();
        std::env::set_var("JWT_SECRET", "f".repeat(64));
        std::env::set_var("MARULA_SECRET_KEY", "e".repeat(64));
        std::env::set_var("MARULA_JWT_EDDSA_PRIVATE_KEY", "not-a-pem");
        std::env::remove_var("MARULA_JWT_EDDSA_PUBLIC_KEY");
        std::env::remove_var("MARULA_JWT_EDDSA_PUBLIC_KEY_FILE");
        let err = Config::from_env().expect_err("private without public must refuse");
        assert!(
            err.contains("MARULA_JWT_EDDSA_PUBLIC_KEY"),
            "operator copy names the missing variable: {err}"
        );
    }

    #[test]
    fn eddsa_public_key_without_private_key_refuses_boot() {
        let _guard = CleanEnv::snapshot();
        std::env::set_var("JWT_SECRET", "f".repeat(64));
        std::env::set_var("MARULA_SECRET_KEY", "e".repeat(64));
        std::env::set_var("MARULA_JWT_EDDSA_PUBLIC_KEY", "not-a-pem");
        std::env::remove_var("MARULA_JWT_EDDSA_PRIVATE_KEY");
        std::env::remove_var("MARULA_JWT_EDDSA_PRIVATE_KEY_FILE");
        let err = Config::from_env().expect_err("public without private must refuse");
        assert!(
            err.contains("MARULA_JWT_EDDSA_PRIVATE_KEY"),
            "operator copy names the missing variable: {err}"
        );
    }

    #[test]
    fn eddsa_malformed_private_pem_refuses_boot_with_the_variable_name() {
        let _guard = CleanEnv::snapshot();
        std::env::set_var("JWT_SECRET", "f".repeat(64));
        std::env::set_var("MARULA_SECRET_KEY", "e".repeat(64));
        std::env::set_var("MARULA_JWT_EDDSA_PRIVATE_KEY", "not-a-pem");
        std::env::set_var("MARULA_JWT_EDDSA_PUBLIC_KEY", "not-a-pem");
        let err = Config::from_env().expect_err("malformed PEM must refuse");
        assert!(
            err.contains("MARULA_JWT_EDDSA_PRIVATE_KEY"),
            "operator copy names the malformed variable: {err}"
        );
    }
}

#[cfg(test)]
mod pool_max_tests {
    use super::*;

    /// Wave 49 lane R1: the pool-ceiling env parse is fail-closed. An
    /// absent value takes the default; a present but invalid one refuses
    /// boot rather than silently reverting (a wrong pool size is a
    /// capacity incident).
    #[test]
    fn pool_max_parsing_is_fail_closed() {
        assert_eq!(
            parse_pool_max_value(None, 20, "MARULA_DB_POOL_MAX").unwrap(),
            20
        );
        assert_eq!(
            parse_pool_max_value(Some("5".into()), 20, "MARULA_DB_POOL_MAX").unwrap(),
            5
        );
        assert_eq!(
            parse_pool_max_value(Some("  64 ".into()), 20, "MARULA_DB_POOL_MAX").unwrap(),
            64
        );
        // 0, out-of-range, and non-numeric all refuse, naming the variable.
        for bad in ["0", "1025", "abc", "-1"] {
            let err = parse_pool_max_value(Some(bad.into()), 20, "MARULA_DB_POOL_MAX")
                .expect_err("invalid pool ceiling must refuse");
            assert!(
                err.contains("MARULA_DB_POOL_MAX"),
                "operator copy names the variable: {err}"
            );
        }
    }
}
