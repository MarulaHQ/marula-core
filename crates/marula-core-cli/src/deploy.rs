//! The deploy descriptor (Wave 49 lane R7) — the interface marula-hq consumes.
//!
//! One image runs every role, and a deployment is parameterized by runtime env
//! vars. This module is the machine-readable description of that contract: the
//! image and its roles, the deployment profiles, and the full catalogue of
//! runtime parameters with their kinds, defaults and grouping.
//!
//! Rust is the single source of truth. `marula deploy describe --json` emits
//! the descriptor for hq to pin; `marula deploy profiles --write` regenerates
//! `deploy/profiles/*.yaml`, and a test asserts the checked-in files match this
//! code so the human-readable copy cannot drift from the contract.
//!
//! No hq logic lives here. marula-core states the interface; hq decides what to
//! render from it.

use std::path::{Path, PathBuf};

/// Schema version of the emitted descriptor. Bump only on a breaking shape
/// change, so a consumer can refuse a version it does not understand.
pub const SCHEMA_VERSION: u32 = 1;

/// The one image and the roles it can run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageSpec {
    pub name: &'static str,
    /// Role names, one per entrypoint (`BIN` build arg).
    pub roles: Vec<&'static str>,
    pub dockerfile: &'static str,
    /// Build arg that selects the role.
    pub role_arg: &'static str,
}

/// A process in a profile's topology.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Service {
    pub name: &'static str,
    /// The image role this service runs.
    pub role: &'static str,
    pub ports: Vec<u16>,
    /// Infrastructure it needs up first.
    pub needs: Vec<&'static str>,
}

/// A named deployment shape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Profile {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub services: Vec<Service>,
    /// Parameters this profile presets, name to value. Everything not listed
    /// keeps its catalogue default.
    pub sets: std::collections::BTreeMap<&'static str, &'static str>,
}

/// The kind of a runtime parameter, for a consumer to render a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    String,
    Integer,
    Boolean,
    Url,
    Secret,
}

/// One runtime parameter in the deployment surface.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Param {
    pub name: &'static str,
    pub group: &'static str,
    pub kind: ParamKind,
    /// `true` when the process refuses to boot without it (no default).
    pub required: bool,
    /// `None` when there is no default.
    pub default: Option<&'static str>,
    /// `true` for key material a consumer must mask.
    pub secret: bool,
    pub description: &'static str,
}

/// The whole descriptor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeployDescriptor {
    pub schema_version: u32,
    /// Core release this descriptor was built from; a consumer pins against it.
    pub core_version: &'static str,
    pub image: ImageSpec,
    pub profiles: Vec<Profile>,
    pub params: Vec<Param>,
    /// The retry matrix, published in the same vocabulary as the env group a
    /// consumer renders from it. The numbers are `core_domain::retry`'s — this
    /// is a projection, not a second copy.
    pub retries: RetrySpec,
}

/// Every retry preset and the budgets it sets, one entry per subsystem.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrySpec {
    /// The env that selects the preset (`MARULA_ROBUSTNESS`).
    pub preset_env: &'static str,
    /// The subsystem→lever env group, so a consumer can render an override
    /// beside each budget without hard-coding the pattern.
    pub lever_env_template: &'static str,
    pub presets: Vec<RetryPreset>,
}

/// One preset and what it does.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPreset {
    pub name: &'static str,
    pub policies: Vec<RetryPolicySpec>,
}

/// One subsystem's resolved budget.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPolicySpec {
    pub subsystem: &'static str,
    pub base_ms: u64,
    pub factor: u32,
    pub cap_ms: u64,
    /// Total tries including the first. `0` means the budget is unlimited.
    pub attempts: u32,
    pub jitter: &'static str,
    /// What a `409` means here.
    pub on_conflict: &'static str,
}

/// The retry section of the descriptor, built from `core_domain::retry`.
#[must_use]
pub fn retry_spec() -> RetrySpec {
    use core_domain::retry::{RobustnessPreset, Subsystem};

    let presets = RobustnessPreset::ALL
        .into_iter()
        .map(|preset| RetryPreset {
            name: preset.as_str(),
            policies: Subsystem::ALL
                .into_iter()
                .map(|subsystem| {
                    let policy = preset.policy(subsystem);
                    RetryPolicySpec {
                        subsystem: subsystem.as_str(),
                        base_ms: u64::try_from(policy.base.as_millis()).unwrap_or(u64::MAX),
                        factor: policy.factor,
                        cap_ms: u64::try_from(policy.cap.as_millis()).unwrap_or(u64::MAX),
                        attempts: policy.budget.limit().unwrap_or(0),
                        jitter: policy.jitter.as_str(),
                        on_conflict: policy.on_conflict.as_str(),
                    }
                })
                .collect(),
        })
        .collect();
    RetrySpec {
        preset_env: core_domain::retry::ROBUSTNESS_ENV,
        lever_env_template: "MARULA_RETRY_<SUBSYSTEM>_<LEVER>",
        presets,
    }
}

fn param(
    name: &'static str,
    group: &'static str,
    kind: ParamKind,
    default: Option<&'static str>,
    secret: bool,
    description: &'static str,
) -> Param {
    Param {
        name,
        group,
        kind,
        // Not derived from `secret`: most secrets are conditional (only
        // needed for the store, transport or provider they configure).
        required: false,
        default,
        secret,
        description,
    }
}

/// A parameter the process refuses to boot without. Today that is the two
/// boot-critical secrets, checked in `auth`/`secrets`; every other secret is
/// conditional on the feature it configures.
fn requires(
    name: &'static str,
    group: &'static str,
    kind: ParamKind,
    default: Option<&'static str>,
    secret: bool,
    description: &'static str,
) -> Param {
    Param {
        required: true,
        ..param(name, group, kind, default, secret, description)
    }
}

/// The full descriptor, built fresh (cheap, all static).
#[must_use]
pub fn descriptor() -> DeployDescriptor {
    DeployDescriptor {
        schema_version: SCHEMA_VERSION,
        core_version: env!("CARGO_PKG_VERSION"),
        image: ImageSpec {
            name: "marula-core",
            roles: vec!["core-api", "worker", "marula-core-cli"],
            dockerfile: "deploy/Dockerfile.rust",
            role_arg: "BIN",
        },
        profiles: profiles(),
        params: params(),
        retries: retry_spec(),
    }
}

fn profiles() -> Vec<Profile> {
    use std::collections::BTreeMap;

    let split = Profile {
        name: "split",
        title: "Split services",
        description: "The default topology: core-api and worker run as separate processes so each scales and restarts on its own. The compose stack ships this shape.",
        services: vec![
            Service {
                name: "core-api",
                role: "core-api",
                ports: vec![8080],
                needs: vec!["postgres", "redis"],
            },
            Service {
                name: "worker",
                role: "worker",
                ports: vec![8082],
                needs: vec!["postgres", "redis"],
            },
        ],
        sets: BTreeMap::new(),
    };

    let all_in_one = Profile {
        name: "all-in-one",
        title: "All-in-one (demo / one-box)",
        description: "One core-api process hosts the worker loops and serves the SSE live-push stream, so a small box runs a single application container. See docs/all-in-one.md.",
        services: vec![Service {
            name: "core-api",
            role: "core-api",
            ports: vec![8080, 8082],
            needs: vec!["postgres", "redis"],
        }],
        sets: BTreeMap::from([
            ("MARULA_ALL_IN_ONE", "1"),
            // One container, so the queue's retry window is short: there is no
            // second replica to absorb a long one, and the operator is the
            // only one watching.
            ("MARULA_ROBUSTNESS", "single_box"),
            (
                "MARULA_WORKER_DATABASE_URL",
                "postgres://marula:marula@postgres:5432/marula",
            ),
            ("EMAIL_POLL_SECS", "60"),
            ("QUEUE_IDLE_MS", "60000"),
            ("EVAL_INTERVAL_SECS", "300"),
            ("TIME_AUDIT_POLL_SECS", "300"),
            ("DELIVERY_POLL_SECS", "300"),
            ("USAGE_REFRESH_SECS", "900"),
            ("DIGEST_POLL_SECS", "3600"),
            ("RETENTION_SWEEP_SECS", "86400"),
        ]),
    };

    vec![split, all_in_one]
}

#[allow(clippy::too_many_lines)]
fn params() -> Vec<Param> {
    use ParamKind::{Boolean, Integer, Secret, String, Url};
    vec![
        // topology
        param(
            "MARULA_ALL_IN_ONE",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Host the worker loops inside core-api.",
        ),
        param(
            "BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8080"),
            false,
            "core-api HTTP listen address.",
        ),
        param(
            "MARULA_WORKER_BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8082"),
            false,
            "Worker health-endpoint address in all-in-one mode.",
        ),
        param(
            "PUBLIC_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:8080"),
            false,
            "Public base URL of the API, used to build OIDC redirect URIs.",
        ),
        param(
            "WEB_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:3000"),
            false,
            "Web app base URL; the SSO callback lands the browser here.",
        ),
        param(
            "CORS_ORIGIN",
            "topology",
            String,
            None,
            false,
            "Allowed browser origin for the API. Unset disables CORS.",
        ),
        // database
        param(
            "DATABASE_URL",
            "database",
            Url,
            Some("postgres://marula_app:marula_app@localhost:15432/marula"),
            false,
            "RLS-scoped app-role Postgres URL. Set it in production.",
        ),
        param(
            "MARULA_WORKER_DATABASE_URL",
            "database",
            Url,
            None,
            false,
            "Owner-role Postgres URL for the worker loops. Required when MARULA_ALL_IN_ONE=1.",
        ),
        param(
            "MARULA_DB_POOL_MAX",
            "database",
            Integer,
            Some("20"),
            false,
            "App-pool ceiling. Sizes both the API app pool and, in all-in-one, the worker pool.",
        ),
        param(
            "MARULA_REPORTING_POOL_MAX",
            "database",
            Integer,
            Some("25"),
            false,
            "Reporting-pool ceiling (app role under RLS).",
        ),
        param(
            "DELIVERY_POOL_MAX",
            "database",
            Integer,
            Some("5"),
            false,
            "Worker delivery reporting-pool ceiling.",
        ),
        // cache
        param(
            "REDIS_URL",
            "cache",
            Url,
            Some("redis://localhost:16379/0"),
            false,
            "Redis/Valkey URL for the outbox relay and stream consumers.",
        ),
        // secrets
        requires(
            "JWT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. Access/refresh token signing. Boot refuses without it outside dev.",
        ),
        requires(
            "MARULA_SECRET_KEY",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. AES-256 at-rest key for sealed integration secrets.",
        ),
        param(
            "MARULA_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Bearer token for /api/internal/* (worker to API machine surface).",
        ),
        param(
            "EMAIL_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Shared token for the email claim/report loop.",
        ),
        param(
            "EMAIL_INBOUND_HMAC_SECRET",
            "secrets",
            Secret,
            None,
            false,
            "HMAC secret for the inbound email webhook. Unset disables it.",
        ),
        param(
            "MARULA_PLUGIN_SIGNING_KEYS",
            "secrets",
            String,
            None,
            false,
            "Trusted plugin-signing public keys (key_id=hex, comma-separated).",
        ),
        // objects
        param(
            "OBJECT_STORE",
            "objects",
            String,
            Some("local"),
            false,
            "Attachment store: local or s3.",
        ),
        param(
            "OBJECT_STORE_LOCAL_DIR",
            "objects",
            String,
            Some("./data/objects"),
            false,
            "Local object-store directory.",
        ),
        param(
            "S3_ENDPOINT",
            "objects",
            Url,
            None,
            false,
            "S3/MinIO endpoint when OBJECT_STORE=s3.",
        ),
        param(
            "S3_BUCKET",
            "objects",
            String,
            Some("marula"),
            false,
            "S3 bucket name.",
        ),
        param(
            "S3_REGION",
            "objects",
            String,
            Some("us-east-1"),
            false,
            "S3 region.",
        ),
        param(
            "S3_ACCESS_KEY_ID",
            "objects",
            Secret,
            None,
            true,
            "S3 access key id.",
        ),
        param(
            "S3_SECRET_ACCESS_KEY",
            "objects",
            Secret,
            None,
            true,
            "S3 secret access key.",
        ),
        // email
        param(
            "EMAIL_TRANSPORT",
            "email",
            String,
            Some("none"),
            false,
            "Outbound transport: none, smtp, postmark or ses.",
        ),
        param(
            "EMAIL_FROM_ADDRESS",
            "email",
            String,
            Some("support@marula.local"),
            false,
            "From address on Marula-generated mail.",
        ),
        param(
            "EMAIL_FROM_NAME",
            "email",
            String,
            None,
            false,
            "From display name.",
        ),
        param(
            "POSTMARK_SERVER_TOKEN",
            "email",
            Secret,
            None,
            true,
            "Postmark server token.",
        ),
        param("SMTP_HOST", "email", String, None, false, "SMTP host."),
        param(
            "SMTP_PORT",
            "email",
            Integer,
            Some("25"),
            false,
            "SMTP port.",
        ),
        param(
            "SMTP_USERNAME",
            "email",
            String,
            None,
            false,
            "SMTP username.",
        ),
        param(
            "SMTP_PASSWORD",
            "email",
            Secret,
            None,
            true,
            "SMTP password.",
        ),
        param(
            "AWS_SES_ACCESS_KEY_ID",
            "email",
            Secret,
            None,
            true,
            "SES access key id.",
        ),
        param(
            "AWS_SES_SECRET_ACCESS_KEY",
            "email",
            Secret,
            None,
            true,
            "SES secret access key.",
        ),
        param(
            "AWS_SES_REGION",
            "email",
            String,
            Some("eu-west-1"),
            false,
            "SES region. Part of the endpoint, not a preference.",
        ),
        // auth
        param(
            "WEBAUTHN_RP_ID",
            "auth",
            String,
            Some("localhost"),
            false,
            "WebAuthn relying-party id (effective domain of the origin).",
        ),
        param(
            "WEBAUTHN_RP_ORIGIN",
            "auth",
            Url,
            Some("http://localhost:3000"),
            false,
            "WebAuthn relying-party origin.",
        ),
        param(
            "ACCESS_TTL_SECS",
            "auth",
            Integer,
            Some("900"),
            false,
            "Access-token TTL.",
        ),
        param(
            "REFRESH_TTL_SECS",
            "auth",
            Integer,
            Some("2592000"),
            false,
            "Refresh-token TTL.",
        ),
        // cadence
        param(
            "EMAIL_POLL_SECS",
            "cadence",
            Integer,
            Some("2"),
            false,
            "Outbound email idle poll interval. See docs/worker-cadence.md.",
        ),
        param(
            "QUEUE_IDLE_MS",
            "cadence",
            Integer,
            Some("500"),
            false,
            "General job-queue idle sleep.",
        ),
        param(
            "EVAL_INTERVAL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "SLA, project-billing, rotation and dedupe sweep interval.",
        ),
        param(
            "TIME_AUDIT_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Missing-time nudge sweep interval.",
        ),
        param(
            "DELIVERY_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Scheduled report delivery sweep interval.",
        ),
        param(
            "USAGE_REFRESH_SECS",
            "cadence",
            Integer,
            Some("300"),
            false,
            "Org usage refresh sweep interval.",
        ),
        param(
            "DIGEST_POLL_SECS",
            "cadence",
            Integer,
            Some("900"),
            false,
            "Daily digest sweep interval.",
        ),
        param(
            "RETENTION_SWEEP_SECS",
            "cadence",
            Integer,
            Some("21600"),
            false,
            "Workflow run-retention sweep interval.",
        ),
        // observability
        param(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "observability",
            Url,
            None,
            false,
            "OTLP endpoint. Unset runs the none mode (no export).",
        ),
        param(
            "GLITCHTIP_DSN_CORE_API",
            "observability",
            Secret,
            None,
            false,
            "GlitchTip DSN presence for the self-host pane.",
        ),
        // --- topology (added by Wave 52 lane R1: the descriptor omitted them) ---
        param(
            "RUN_MIGRATIONS",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Run startup migrations as this process. Dev only: the compose migrate one-shot owns them.",
        ),
        param(
            "MARULA_DEV_TOOLS",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Enable the dev-only fallbacks for JWT_SECRET and MARULA_SECRET_KEY. Never set in production.",
        ),
        param(
            "TRUSTED_PROXIES",
            "topology",
            String,
            None,
            false,
            "Comma-separated proxy CIDRs. Replaces the loopback plus RFC1918 default set.",
        ),
        param(
            "CF_CONNECTING_IP",
            "topology",
            String,
            Some("auto"),
            false,
            "Which CF-Connecting-IP port the client-IP resolver believes: auto, true or false.",
        ),
        param(
            "CF_IP_RANGES",
            "topology",
            String,
            None,
            false,
            "Extra Cloudflare CIDRs. Extends the versioned baseline, never replaces it.",
        ),
        param(
            "WORKER_ROLES",
            "topology",
            String,
            Some("all"),
            false,
            "Loop groups this worker runs: all, realtime, housekeeping, or both comma-separated.",
        ),
        param(
            "CORE_API_URL",
            "topology",
            Url,
            Some("http://localhost:8080"),
            false,
            "core-api base URL for the email and missing-time loops.",
        ),
        param(
            "INTERNAL_API_URL",
            "topology",
            Url,
            Some("http://127.0.0.1:8080"),
            false,
            "core-api base URL for the machine effect calls.",
        ),
        param(
            "WORKER_INTERNAL_URL",
            "topology",
            Url,
            None,
            false,
            "Worker health URL the self-host pane probes. Unset shows the pane as not configured.",
        ),

        // --- database and cache ---
        param(
            "MARULA_REF_CACHE_TTL_SECS",
            "database",
            Integer,
            Some("30"),
            false,
            "Backstop TTL for the reference cache; an invalidation event normally drops the entry first.",
        ),
        param(
            "MARULA_REF_CACHE_MAX_ENTRIES",
            "database",
            Integer,
            Some("4096"),
            false,
            "Entry ceiling per cache namespace, least-recently-used eviction.",
        ),
        param(
            "MARULA_MAX_UPLOAD_MB",
            "database",
            Integer,
            Some("25"),
            false,
            "Largest accepted attachment, in mebibytes.",
        ),
        param(
            "MARULA_EVENT_STREAM_MAXLEN",
            "cache",
            Integer,
            Some("50000"),
            false,
            "Entries kept in the event stream before the oldest are trimmed.",
        ),
        param(
            "MARULA_SSE_MAX_CONNECTIONS",
            "cache",
            Integer,
            Some("8"),
            false,
            "Live-push connections allowed per user.",
        ),
        param(
            "MARULA_SSE_REPLAY_BUFFER",
            "cache",
            Integer,
            Some("256"),
            false,
            "Live-push events buffered for replay to a reconnecting client.",
        ),
        param(
            "MARULA_PRESENCE_TTL_SECS",
            "cache",
            Integer,
            None,
            false,
            "Presence lease length. Unset keeps the built-in default.",
        ),
        param(
            "OUTBOX_RETENTION_DAYS",
            "cache",
            Integer,
            Some("7"),
            false,
            "Days a published outbox row is kept before the sweep removes it.",
        ),
        param(
            "OUTBOX_SWEEP_SECS",
            "cache",
            Integer,
            Some("21600"),
            false,
            "How often the outbox retention window is enforced.",
        ),

        // --- secrets and keys ---
        param(
            "MARULA_PLUGIN_SIGNING_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Single trusted plugin-signing key (key_id=hex). Superseded by MARULA_PLUGIN_SIGNING_KEYS.",
        ),
        param(
            "MARULA_JWT_EDDSA_PRIVATE_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Inline Ed25519 private key PEM for token signing. Set this or the _FILE form, not both.",
        ),
        param(
            "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
            "secrets",
            String,
            None,
            false,
            "Path to the Ed25519 private key PEM. Set this or the inline form, not both.",
        ),
        param(
            "MARULA_JWT_EDDSA_PUBLIC_KEY",
            "secrets",
            String,
            None,
            false,
            "Inline Ed25519 public key PEM. Required whenever a private key is configured.",
        ),
        param(
            "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
            "secrets",
            String,
            None,
            false,
            "Path to the Ed25519 public key PEM.",
        ),
        param(
            "MARULA_JWT_KID",
            "secrets",
            String,
            None,
            false,
            "Key id published in the token header so verifiers pick the right key.",
        ),
        param(
            "MARULA_STRIPE_SECRET_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Stripe API key for the billing provider adapter.",
        ),
        param(
            "MARULA_STRIPE_WEBHOOK_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Stripe webhook signing secret.",
        ),
        param(
            "GOOGLE_CALENDAR_CLIENT_ID",
            "secrets",
            String,
            None,
            false,
            "Delegated Google Calendar OAuth client id.",
        ),
        param(
            "GOOGLE_CALENDAR_CLIENT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Delegated Google Calendar OAuth client secret.",
        ),
        param(
            "MICROSOFT_GRAPH_CLIENT_ID",
            "secrets",
            String,
            None,
            false,
            "Delegated Microsoft Graph OAuth client id.",
        ),
        param(
            "MICROSOFT_GRAPH_CLIENT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Delegated Microsoft Graph OAuth client secret.",
        ),

        // --- email (remaining levers) ---
        param(
            "AWS_SES_ENDPOINT",
            "email",
            Url,
            None,
            false,
            "SES endpoint override. Unset derives it from the region.",
        ),
        param(
            "POSTMARK_BASE_URL",
            "email",
            Url,
            Some("https://api.postmarkapp.com"),
            false,
            "Postmark API base URL.",
        ),
        param(
            "EMAIL_SCAN",
            "email",
            String,
            Some("none"),
            false,
            "Inbound attachment scanning: none or clamd.",
        ),
        param(
            "CLAMD_ADDR",
            "email",
            String,
            Some("127.0.0.1:3310"),
            false,
            "clamd address when EMAIL_SCAN=clamd.",
        ),
        param(
            "EMAIL_CLAIM_BATCH",
            "email",
            Integer,
            Some("25"),
            false,
            "Outbound email rows claimed per cycle.",
        ),
        param(
            "EMAIL_LEASE_SECS",
            "email",
            Integer,
            Some("300"),
            false,
            "Lease seconds asked of the claim endpoint.",
        ),
        param(
            "DELIVERY_CLAIM_BATCH",
            "email",
            Integer,
            Some("20"),
            false,
            "Scheduled-delivery rows claimed per sweep.",
        ),

        // --- auth (remaining levers) ---
        param(
            "MFA_PENDING_TTL_SECS",
            "auth",
            Integer,
            Some("300"),
            false,
            "How long an MFA challenge stays valid.",
        ),
        param(
            "MARULA_DECISION_BOT_USER_ID",
            "auth",
            String,
            None,
            false,
            "Real member id decision runs are attributed to. Unset disables the decision subsystem.",
        ),

        // --- cadence (remaining levers) ---
        param(
            "QUEUE_BATCH",
            "cadence",
            Integer,
            Some("10"),
            false,
            "Jobs claimed per queue tick.",
        ),
        param(
            "QUEUE_STALE_SECS",
            "cadence",
            Integer,
            Some("300"),
            false,
            "Lease age after which a crashed claimant's job is re-armed.",
        ),
        param(
            "RELAY_BATCH",
            "cadence",
            Integer,
            Some("100"),
            false,
            "Outbox rows relayed per batch.",
        ),
        param(
            "RELAY_POLL_MS",
            "cadence",
            Integer,
            Some("5000"),
            false,
            "Backstop poll interval for the outbox relay.",
        ),
        param(
            "MARULA_RELAY_WAKEUP",
            "cadence",
            Boolean,
            Some("true"),
            false,
            "Listen on the outbox notification channel. Off restores pure polling.",
        ),

        // --- observability (remaining levers) ---
        param(
            "GLITCHTIP_DSN_WORKER",
            "observability",
            Secret,
            None,
            true,
            "GlitchTip DSN for the worker. Presence is shown in the self-host pane.",
        ),
        param(
            "MARULA_GLITCHTIP_URL",
            "observability",
            Url,
            None,
            false,
            "GlitchTip console URL for the operator links.",
        ),
        param(
            "MARULA_GRAFANA_URL",
            "observability",
            Url,
            None,
            false,
            "Grafana URL for the operator links.",
        ),
        param(
            "MARULA_LOKI_URL",
            "observability",
            Url,
            None,
            false,
            "Loki URL for the operator links.",
        ),
        param(
            "MARULA_OTEL_METRICS_URL",
            "observability",
            Url,
            None,
            false,
            "Metrics backend URL for the operator links.",
        ),
        param(
            "MARULA_TEMPO_URL",
            "observability",
            Url,
            None,
            false,
            "Tempo URL for the operator links.",
        ),
        param(
            "MARULA_UPTIME_KUMA_URL",
            "observability",
            Url,
            None,
            false,
            "Uptime Kuma URL for the operator links.",
        ),

        // --- routing ---
        param(
            "ROUTING_PROVIDER",
            "routing",
            String,
            Some("none"),
            false,
            "Distance provider: none, osrm, ors, contract or mapbox.",
        ),
        param(
            "ROUTING_OSRM_URL",
            "routing",
            Url,
            None,
            false,
            "OSRM base URL.",
        ),
        param(
            "ROUTING_ORS_URL",
            "routing",
            Url,
            None,
            false,
            "OpenRouteService base URL.",
        ),
        param(
            "ROUTING_ORS_API_KEY",
            "routing",
            Secret,
            None,
            true,
            "OpenRouteService API key.",
        ),
        param(
            "ROUTING_CONTRACT_URL",
            "routing",
            Url,
            None,
            false,
            "Contract routing service base URL.",
        ),
        param(
            "ROUTING_CONTRACT_ID",
            "routing",
            String,
            None,
            false,
            "Contract routing service account id.",
        ),
        param(
            "ROUTING_CONTRACT_TOKEN",
            "routing",
            Secret,
            None,
            true,
            "Contract routing service token.",
        ),
        param(
            "MAPBOX_ACCESS_TOKEN",
            "routing",
            Secret,
            None,
            true,
            "Mapbox access token.",
        ),
        param(
            "MAPBOX_PROFILE",
            "routing",
            String,
            None,
            false,
            "Mapbox routing profile (driving, driving-traffic, walking).",
        ),

        // --- rate limits and budgets ---
        param(
            "MARULA_MIGRATION_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Migration requests allowed per principal per minute.",
        ),
        param(
            "MARULA_DISPATCH_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Dispatch requests allowed per principal per minute.",
        ),
        param(
            "MARULA_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("60"),
            false,
            "Scheduling-link requests allowed per principal per minute.",
        ),
        param(
            "MARULA_VAULT_WRITE_BUDGET_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Vault writes allowed per org per minute.",
        ),
        param(
            "MARULA_VAULT_BULK_ROWS_PER_SEC",
            "limits",
            Integer,
            Some("50"),
            false,
            "Vault bulk-import rows per second.",
        ),

        // --- ai (model endpoints) ---
        param(
            "OPENAI_BASE_URL",
            "ai",
            Url,
            None,
            false,
            "OpenAI-compatible base URL. Unset keeps the built-in default.",
        ),
        param(
            "OLLAMA_BASE_URL",
            "ai",
            Url,
            None,
            false,
            "Ollama base URL for the local model option.",
        ),

        // --- retries (Wave 52 lane R1) ---
        //
        // One preset sets every budget; the per-subsystem group is the
        // operator's escape hatch and always wins over the preset. Every
        // default below is the `balanced` preset's value, so a descriptor
        // reader can see the shipped numbers without reading the code.
        param(
            "MARULA_ROBUSTNESS",
            "retries",
            String,
            Some("balanced"),
            false,
            "Retry preset for every subsystem: balanced, durable, best_effort or single_box.",
        ),
        param(
            "MARULA_RETRY_QUEUE_BASE_MS",
            "retries",
            Integer,
            Some("5000"),
            false,
            "Delay before the second try for the job queue, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_QUEUE_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for the job queue, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_QUEUE_ATTEMPTS",
            "retries",
            Integer,
            Some("10"),
            false,
            "Total tries for the job queue, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_QUEUE_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for the job queue: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_QUEUE_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for the job queue. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_EMAIL_BASE_MS",
            "retries",
            Integer,
            Some("30000"),
            false,
            "Delay before the second try for outbound email, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_EMAIL_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for outbound email, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_EMAIL_ATTEMPTS",
            "retries",
            Integer,
            Some("6"),
            false,
            "Total tries for outbound email, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_EMAIL_JITTER",
            "retries",
            String,
            Some("equal"),
            false,
            "Delay spread for outbound email: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_EMAIL_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for outbound email. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_WEBHOOK_BASE_MS",
            "retries",
            Integer,
            Some("30000"),
            false,
            "Delay before the second try for signed webhook delivery, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_WEBHOOK_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for signed webhook delivery, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_WEBHOOK_ATTEMPTS",
            "retries",
            Integer,
            Some("6"),
            false,
            "Total tries for signed webhook delivery, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_WEBHOOK_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for signed webhook delivery: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_WEBHOOK_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for signed webhook delivery. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_EFFECTS_BASE_MS",
            "retries",
            Integer,
            Some("100"),
            false,
            "Delay before the second try for the worker to core-api effect seam, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_EFFECTS_CAP_MS",
            "retries",
            Integer,
            Some("100"),
            false,
            "Ceiling on the computed delay for the worker to core-api effect seam, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_EFFECTS_ATTEMPTS",
            "retries",
            Integer,
            Some("1"),
            false,
            "Total tries for the worker to core-api effect seam, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_EFFECTS_JITTER",
            "retries",
            String,
            Some("none"),
            false,
            "Delay spread for the worker to core-api effect seam: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_EFFECTS_CONFLICT",
            "retries",
            String,
            Some("retry"),
            false,
            "Whether a 409 is retried for the worker to core-api effect seam. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_CONSUMER_BASE_MS",
            "retries",
            Integer,
            Some("250"),
            false,
            "Delay before the second try for stream consumer reconnect, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_CONSUMER_CAP_MS",
            "retries",
            Integer,
            Some("10000"),
            false,
            "Ceiling on the computed delay for stream consumer reconnect, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_CONSUMER_ATTEMPTS",
            "retries",
            Integer,
            Some("0"),
            false,
            "Total tries for stream consumer reconnect, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_CONSUMER_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for stream consumer reconnect: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_CONSUMER_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for stream consumer reconnect. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_RELAY_BASE_MS",
            "retries",
            Integer,
            Some("250"),
            false,
            "Delay before the second try for outbox relay and realtime listener reconnect, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_RELAY_CAP_MS",
            "retries",
            Integer,
            Some("10000"),
            false,
            "Ceiling on the computed delay for outbox relay and realtime listener reconnect, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_RELAY_ATTEMPTS",
            "retries",
            Integer,
            Some("0"),
            false,
            "Total tries for outbox relay and realtime listener reconnect, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_RELAY_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for outbox relay and realtime listener reconnect: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_RELAY_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for outbox relay and realtime listener reconnect. Retry only where a replay cannot double-apply.",
        ),
        param(
            "MARULA_RETRY_DECISION_BASE_MS",
            "retries",
            Integer,
            Some("200"),
            false,
            "Delay before the second try for the decision provider's upstream calls, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_DECISION_CAP_MS",
            "retries",
            Integer,
            Some("2000"),
            false,
            "Ceiling on the computed delay for the decision provider's upstream calls, in milliseconds.",
        ),
        param(
            "MARULA_RETRY_DECISION_ATTEMPTS",
            "retries",
            Integer,
            Some("3"),
            false,
            "Total tries for the decision provider's upstream calls, including the first. 0 never gives up.",
        ),
        param(
            "MARULA_RETRY_DECISION_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for the decision provider's upstream calls: none, full (0 to delay) or equal (half to delay).",
        ),
        param(
            "MARULA_RETRY_DECISION_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for the decision provider's upstream calls. Retry only where a replay cannot double-apply.",
        ),
    ]
}

/// JSON rendering of the descriptor, optionally narrowed to one profile.
pub fn describe_json(profile: Option<&str>) -> anyhow::Result<String> {
    let mut desc = descriptor();
    if let Some(name) = profile {
        let kept: Vec<Profile> = desc
            .profiles
            .iter()
            .filter(|p| p.name == name)
            .cloned()
            .collect();
        anyhow::ensure!(
            !kept.is_empty(),
            "unknown profile {name:?} — known: {}",
            desc.profiles
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        desc.profiles = kept;
    }
    Ok(serde_json::to_string_pretty(&desc)?)
}

/// A short human summary for `marula deploy describe` without `--json`.
#[must_use]
pub fn describe_summary() -> String {
    let d = descriptor();
    let mut out = String::new();
    out.push_str(&format!(
        "marula-core {} — deploy descriptor schema {}\n",
        d.core_version, d.schema_version
    ));
    out.push_str(&format!(
        "image {} (roles: {}) via {} [{}]\n\n",
        d.image.name,
        d.image.roles.join(", "),
        d.image.dockerfile,
        d.image.role_arg
    ));
    for p in &d.profiles {
        out.push_str(&format!("profile {} — {}\n", p.name, p.title));
        for s in &p.services {
            out.push_str(&format!(
                "  service {:<10} role {:<12} ports {:?} needs {:?}\n",
                s.name, s.role, s.ports, s.needs
            ));
        }
        if !p.sets.is_empty() {
            out.push_str(&format!("  presets {} parameters\n", p.sets.len()));
        }
    }
    out.push_str(&format!(
        "\nretries: preset via {} ({} presets, {} subsystems, {} levers each)\n",
        d.retries.preset_env,
        d.retries.presets.len(),
        d.retries.presets.first().map_or(0, |p| p.policies.len()),
        core_domain::retry::Lever::ALL.len(),
    ));
    out.push_str(&format!(
        "\n{} runtime parameters; emit JSON with `marula deploy describe --json`\n",
        d.params.len()
    ));
    out
}

/// The repo root, from this crate's manifest dir.
#[cfg(test)]
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Env names read by a source tree, discovered by scanning the code.
///
/// The scan is deliberately dumb: find a call that looks like an env read, then
/// take the first all-caps string literal in the same statement (up to the next
/// `;` or `{`, so a literal on a later line cannot be misread as this call's
/// argument). Test modules are skipped — a fixture's `"AKIAEXAMPLE"` is not a
/// lever.
///
/// Its failure mode is a false positive, which surfaces as "add a descriptor row
/// or explain in `NON_LEVER_ENV`". That is the right side to err on: a missing
/// row is the bug this test exists for.
#[cfg(test)]
fn env_names_in(root: &Path) -> std::collections::BTreeSet<String> {
    const SIGNALS: [&str; 9] = [
        "std::env::var(",
        "env::var(",
        "env_text(",
        "env_configured(",
        "lookup(",
        "parse_pool_max(",
        "parse_bounded_u64(",
        "dsn_state(",
        "env(",
    ];
    let mut names = std::collections::BTreeSet::new();
    for path in rust_files(root) {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        // A test's own values are not levers.
        let source = match source.find("#[cfg(test)]") {
            Some(cut) => &source[..cut],
            None => source.as_str(),
        };
        for signal in SIGNALS {
            let mut from = 0;
            while let Some(offset) = source[from..].find(signal) {
                let after = from + offset + signal.len();
                let end = source.len().min(after + 160);
                let window = &source[after..end];
                let stop = window.find([';', '{']).unwrap_or(window.len());
                if let Some(name) = first_caps_literal(&window[..stop]) {
                    if !NON_LEVER_ENV.contains(&name.as_str()) {
                        names.insert(name);
                    }
                }
                from = after;
            }
        }
    }
    names
}

#[cfg(test)]
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The first `"SCREAMING_SNAKE"` literal in a fragment, if any.
#[cfg(test)]
fn first_caps_literal(fragment: &str) -> Option<String> {
    let mut rest = fragment;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else {
            return None;
        };
        let candidate = &after[..close];
        let looks_like_env = candidate.len() >= 3
            && candidate.starts_with(|c: char| c.is_ascii_uppercase())
            && candidate
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if looks_like_env {
            return Some(candidate.to_string());
        }
        rest = &after[close + 1..];
    }
    None
}

/// Values read through a computed variable, so the scan cannot see them. Every
/// entry still has to be a descriptor parameter — that is the assertion.
#[cfg(test)]
const DYNAMIC_ENV_READS: [&str; 8] = [
    // `read_key(inline_var, file_var)` in core-api's EdDSA resolution.
    "MARULA_JWT_EDDSA_PRIVATE_KEY",
    "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
    "MARULA_JWT_EDDSA_PUBLIC_KEY",
    "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
    // `oauth_client_config(provider)` picks the pair by provider name.
    "GOOGLE_CALENDAR_CLIENT_ID",
    "GOOGLE_CALENDAR_CLIENT_SECRET",
    "MICROSOFT_GRAPH_CLIENT_ID",
    "MICROSOFT_GRAPH_CLIENT_SECRET",
];

/// Values the processes read that are not operator levers, each for a reason.
/// Keep this list short: it is the only place a lever can hide.
#[cfg(test)]
const NON_LEVER_ENV: [&str; 1] = [
    // Provided by the container runtime for consumer naming; telling an
    // operator to set it would be noise.
    "HOSTNAME",
];

/// Regenerates `deploy/profiles/*.yaml` from this module. Returns the paths
/// written.
pub fn write_profiles(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for profile in profiles() {
        let path = dir.join(format!("{}.yaml", profile.name));
        std::fs::write(&path, profile_yaml(&profile))?;
        written.push(path);
    }
    Ok(written)
}

/// The checked-in YAML for one profile, as `profile_yaml` renders it. Used by
/// the drift test.
#[cfg(test)]
#[must_use]
pub fn checked_in_profile_yaml(name: &str) -> Option<String> {
    let path = repo_root()
        .join("deploy/profiles")
        .join(format!("{name}.yaml"));
    std::fs::read_to_string(path).ok()
}

/// Deterministic YAML for one profile. The shape is fixed and simple, so a
/// small emitter beats a YAML dependency here.
#[must_use]
pub fn profile_yaml(p: &Profile) -> String {
    fn q(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
    let mut s = String::new();
    s.push_str("# Generated by `marula deploy profiles --write` (Wave 49 lane R7).\n");
    s.push_str("# The machine-readable contract is `marula deploy describe --json`;\n");
    s.push_str("# see docs/deploy-descriptor.md. Do not edit by hand.\n");
    s.push_str(&format!("name: {}\n", p.name));
    s.push_str(&format!("title: {}\n", q(p.title)));
    s.push_str(&format!("description: {}\n", q(p.description)));
    s.push_str("services:\n");
    for svc in &p.services {
        s.push_str(&format!("  - name: {}\n", svc.name));
        s.push_str(&format!("    role: {}\n", svc.role));
        let ports: Vec<String> = svc.ports.iter().map(u16::to_string).collect();
        s.push_str(&format!("    ports: [{}]\n", ports.join(", ")));
        s.push_str(&format!("    needs: [{}]\n", svc.needs.join(", ")));
    }
    s.push_str("sets:\n");
    for (k, v) in &p.sets {
        s.push_str(&format!("  {k}: {}\n", q(v)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn param_names_are_unique_and_groups_are_known() {
        let names: Vec<&str> = params().iter().map(|p| p.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate parameter name");

        let groups: std::collections::HashSet<&str> = params().iter().map(|p| p.group).collect();
        for known in [
            "topology",
            "database",
            "cache",
            "secrets",
            "objects",
            "email",
            "auth",
            "cadence",
            "observability",
            "routing",
            "limits",
            "ai",
            "retries",
        ] {
            assert!(groups.contains(known), "group {known} missing");
        }
    }

    /// Every env value core-api or worker reads must appear in the descriptor.
    ///
    /// The gap this closes: `marula deploy describe` listed 49 parameters
    /// while the two processes read roughly a hundred, so an operator
    /// deploying from the descriptor could not discover the knobs that decide
    /// whether the queue retries, whether the relay listens, or whether the
    /// process boots at all.
    ///
    /// A name reaches the descriptor one of two ways: the scan finds it in
    /// source, or it is listed in [`DYNAMIC_ENV_READS`] because it is read
    /// through a computed variable. Both sets are checked, so "make the test
    /// pass" is not a way to hide a lever.
    #[test]
    fn every_env_value_the_processes_read_has_a_descriptor_entry() {
        let mut discovered = BTreeSet::new();
        for root in ["crates/core-api/src", "crates/worker/src"] {
            discovered.extend(env_names_in(&repo_root().join(root)));
        }
        assert!(
            discovered.len() > 80,
            "the scan found only {} env names — it is broken, not the descriptor",
            discovered.len()
        );

        for name in DYNAMIC_ENV_READS {
            assert!(
                params().iter().any(|p| p.name == name),
                "dynamic read {name} is not in the descriptor"
            );
        }

        let mut missing: Vec<&str> = discovered
            .iter()
            .map(String::as_str)
            .filter(|name| !NON_LEVER_ENV.contains(name))
            .filter(|name| !params().iter().any(|p| p.name == *name))
            .collect();
        missing.sort_unstable();
        assert!(
            missing.is_empty(),
            "{} env values are read by core-api or worker but absent from \
             `marula deploy describe`: {missing:?}\n\
             Add a descriptor entry (group it with its siblings) or, if it is \
             not an operator lever, say why in NON_LEVER_ENV.",
            missing.len()
        );
    }

    /// The retry matrix is the product decision; the descriptor has to publish
    /// it. Every lever the resolver accepts is a descriptor parameter and
    /// every descriptor parameter in the `retries` group is a lever the
    /// resolver reads — otherwise the escape hatch and its documentation can
    /// drift.
    #[test]
    fn every_retry_lever_is_in_the_descriptor_and_vice_versa() {
        let levers: BTreeSet<&str> = core_domain::retry::env_names().into_iter().collect();
        let described: BTreeSet<&str> = params()
            .iter()
            .filter(|p| p.group == "retries")
            .map(|p| p.name)
            .collect();
        assert_eq!(
            levers, described,
            "the retry levers and the descriptor's `retries` group disagree"
        );
        assert!(levers.contains(core_domain::retry::ROBUSTNESS_ENV));
    }

    /// The retry section of the JSON speaks in the same vocabulary as the env:
    /// a consumer renders the preset name and the lever names, not Rust type
    /// names.
    #[test]
    fn the_json_publishes_the_retry_presets_and_the_resolved_defaults() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let retries = &v["retries"];
        assert_eq!(retries["preset_env"], core_domain::retry::ROBUSTNESS_ENV);
        let presets = retries["presets"].as_array().expect("presets array");
        assert_eq!(presets.len(), 4);
        let names: Vec<&str> = presets
            .iter()
            .map(|p| p["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec!["balanced", "durable", "best_effort", "single_box"]
        );

        let balanced = presets
            .iter()
            .find(|p| p["name"] == "balanced")
            .expect("balanced preset");
        let policies = balanced["policies"].as_array().expect("policies");
        assert_eq!(policies.len(), 7);
        let queue = policies
            .iter()
            .find(|p| p["subsystem"] == "queue")
            .expect("queue policy");
        // The shipped queue budget, readable without the source.
        assert_eq!(queue["attempts"], 10);
        assert_eq!(queue["cap_ms"], 3_600_000);
        assert_eq!(queue["jitter"], "full");
    }

    #[test]
    fn only_the_boot_critical_secrets_are_required() {
        // `required` means the process refuses to boot without it. Most secrets
        // are conditional on the feature they configure (an S3 key, a Postmark
        // token), so they are not required here. The two boot-critical secrets
        // are, and neither may carry a default.
        let required: Vec<&str> = params()
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        assert_eq!(required, vec!["JWT_SECRET", "MARULA_SECRET_KEY"]);
        for p in params() {
            if p.required {
                assert!(
                    p.default.is_none(),
                    "{} is required but has a default",
                    p.name
                );
            }
        }
    }

    #[test]
    fn every_profile_preset_names_a_real_param() {
        let known: std::collections::HashSet<&str> = params().iter().map(|p| p.name).collect();
        for profile in profiles() {
            for name in profile.sets.keys() {
                assert!(
                    known.contains(name),
                    "profile {} presets unknown param {name}",
                    profile.name
                );
            }
        }
    }

    #[test]
    fn all_in_one_is_the_profile_the_roster_describes() {
        let p = profiles()
            .into_iter()
            .find(|p| p.name == "all-in-one")
            .expect("all-in-one profile");
        assert_eq!(p.sets.get("MARULA_ALL_IN_ONE"), Some(&"1"));
        assert!(
            p.sets.contains_key("MARULA_WORKER_DATABASE_URL"),
            "all-in-one must preset the owner-role URL"
        );
        // One process, so one service, exposing the API and worker-health ports.
        assert_eq!(p.services.len(), 1);
        assert_eq!(p.services[0].ports, vec![8080, 8082]);
    }

    #[test]
    fn split_profile_has_one_service_per_role() {
        let p = profiles()
            .into_iter()
            .find(|p| p.name == "split")
            .expect("split profile");
        let roles: Vec<&str> = p.services.iter().map(|s| s.role).collect();
        assert_eq!(roles, vec!["core-api", "worker"]);
        assert!(p.sets.is_empty(), "split ships the defaults");
    }

    #[test]
    fn describe_json_is_valid_and_pins_the_schema() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["profiles"].as_array().map(Vec::len), Some(2));
        assert!(!v["core_version"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn describe_json_narrows_to_a_named_profile() {
        let json = describe_json(Some("all-in-one")).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["profiles"].as_array().map(Vec::len), Some(1));
        assert_eq!(v["profiles"][0]["name"], "all-in-one");

        assert!(describe_json(Some("nope")).is_err());
    }

    #[test]
    fn checked_in_profile_files_match_this_code() {
        // The contract lives in Rust. The checked-in YAML is a copy, so a drift
        // here means someone edited one side only.
        for profile in profiles() {
            let expected = profile_yaml(&profile);
            let actual = checked_in_profile_yaml(profile.name).unwrap_or_else(|| {
                panic!(
                    "deploy/profiles/{}.yaml is missing — run `marula deploy profiles --write`",
                    profile.name
                )
            });
            assert_eq!(
                actual, expected,
                "deploy/profiles/{}.yaml drifted from the descriptor — run `marula deploy profiles --write`",
                profile.name
            );
        }
    }
}
