//! Self-host services visibility (Wave 36 lane 3).
//!
//! Purpose: one owner/admin read surface answering the self-hoster's real
//! question — "does the operator ever see the ops stack?" It returns the
//! stack health sweep (database, Valkey, worker, and the API
//! itself), the ClamAV scan state with the recently quarantined messages
//! (RLS-scoped), which GlitchTip DSNs are configured per service (a boolean
//! per service — never the DSN), the external-UI links the operator set,
//! and the Garage CLI-first facts (volumes + bootstrap commands + the
//! admin-API docs link).
//!
//! Non-purpose: no writes, no secret values, no new status machinery — the
//! sweep calls each loop's existing `/healthz` seam and the same database
//! and Valkey the services already use. The raw clamd TCP PING stays a
//! documented probe (`docs/site/how-to/admin-self-host-services.md`), not a
//! built UI.

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// Every probe in the sweep is bounded; a hung dependency must never hold
/// the settings page open.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// Detections list cap for the pane.
const DETECTION_LIMIT: i64 = 20;

/// GET /api/v1/settings/self-host — the self-hoster visibility payload.
#[utoipa::path(operation_id = "settings_self_host_get", get,
    path = "/api/v1/settings/self-host", tag = "settings", security(("bearer" = [])),
    responses(
        (status = 200, body = SelfHostView),
        (status = 403, description = "requires an org owner or admin")))]
pub async fn get(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SelfHostView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "self-host visibility is limited to org owners and admins".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;

    // --- detections + scan freshness: RLS-scoped reads ---
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let freshness = core_db::emails::scan_freshness(&mut *tx, org_id).await?;
    let messages =
        core_db::emails::list_recent_detections(&mut *tx, org_id, DETECTION_LIMIT).await?;
    let mut detections = Vec::with_capacity(messages.len());
    for message in messages {
        let attachments = core_db::attachments::list_for_email(&mut *tx, org_id, message.id)
            .await?
            .into_iter()
            .map(|a| DetectionFile {
                filename: a.filename,
                content_type: a.content_type,
                size_bytes: a.size_bytes,
                sha256: a.sha256,
            })
            .collect();
        detections.push(DetectionView {
            message_id: message.id,
            subject: message.subject,
            from_address: message.from_address,
            verdict: message.scan_status.as_str().to_string(),
            ingest_source: message.transport,
            occurred_at: message.created_at,
            attachments,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    // --- stack sweep (bounded) ---
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .unwrap_or_default();
    let worker_url = env_text("WORKER_INTERNAL_URL");
    let worker = probe_optional_http(&client, worker_url.as_deref()).await;
    let database = probe_database(&state).await;
    let valkey = probe_valkey(&state.config.redis_url).await;

    let services = vec![
        ServiceView {
            id: "core_api".into(),
            label: "core-api".into(),
            state: "up".into(),
            detail: Some("served this request".into()),
            loops: None,
        },
        ServiceView {
            id: "database".into(),
            label: "PostgreSQL".into(),
            state: database.0.into(),
            detail: database.1,
            loops: None,
        },
        ServiceView {
            id: "valkey".into(),
            label: "Valkey (Redis)".into(),
            state: valkey.state.into(),
            detail: valkey.detail,
            loops: None,
        },
        ServiceView {
            id: "worker".into(),
            label: "worker".into(),
            state: worker.state.into(),
            detail: worker.detail.clone(),
            loops: worker.body.clone(),
        },
    ];

    // --- GlitchTip DSN presence (never the DSN itself) ---
    let glitchtip_dsns = vec![
        DsnRow {
            service: "core-api".into(),
            state: if env_configured("GLITCHTIP_DSN_CORE_API") {
                "configured".into()
            } else {
                "unset".into()
            },
            detail: Some("read from this process's environment".into()),
        },
        DsnRow {
            service: "worker".into(),
            state: dsn_state(&worker.body),
            detail: Some(dsn_detail(&worker)),
        },
    ];

    let (s3_endpoint, s3_bucket) = match &state.config.object_store {
        core_store::StoreChoice::S3(cfg) => (
            cfg.endpoint.clone().filter(|e| !e.trim().is_empty()),
            Some(cfg.bucket.clone()),
        ),
        core_store::StoreChoice::Local(_) => (None, None),
    };

    Ok(Json(SelfHostView {
        scan: ScanView {
            provider: state.mail.scanner.name().to_string(),
            clamd_addr: state.config.email_scan_clamd_addr.clone(),
            last_scan_at: freshness.last_scan_at,
            last_detection_at: freshness.last_detection_at,
            detection_count: freshness.detection_count,
        },
        detections,
        services,
        glitchtip_dsns,
        links: SelfHostLinksView {
            glitchtip: env_text("MARULA_GLITCHTIP_URL"),
            grafana: env_text("MARULA_GRAFANA_URL"),
            tempo: env_text("MARULA_TEMPO_URL"),
            loki: env_text("MARULA_LOKI_URL"),
            otel_metrics: env_text("MARULA_OTEL_METRICS_URL"),
            uptime_kuma: env_text("MARULA_UPTIME_KUMA_URL"),
        },
        garage: GarageView {
            volumes: vec!["garagedata".into(), "garagemeta".into()],
            container_paths: vec!["/var/lib/garage/data".into(), "/var/lib/garage/meta".into()],
            bootstrap_commands: GARAGE_BOOTSTRAP_COMMANDS
                .iter()
                .map(|c| (*c).to_string())
                .collect(),
            admin_api_docs_url:
                "https://garagehq.deuxfleurs.fr/documentation/reference-manual/admin-api/".into(),
            s3_endpoint,
            s3_bucket,
        },
    }))
}

/// The shipped compose file's Garage bootstrap sequence, verbatim — the
/// pane and `docs/site/how-to/admin-self-host-services.md` cite the same
/// list so docs and UI cannot drift.
const GARAGE_BOOTSTRAP_COMMANDS: &[&str] = &[
    "docker compose --profile stage2 up -d garage",
    "docker compose --profile stage2 exec garage /garage status",
    "docker compose --profile stage2 exec garage /garage layout assign -z dc1 -c 1G <NODE_ID>",
    "docker compose --profile stage2 exec garage /garage layout apply --version 1",
    "docker compose --profile stage2 exec garage /garage bucket create marula",
    "docker compose --profile stage2 exec garage /garage key create marula",
    "docker compose --profile stage2 exec garage /garage bucket allow --read --write marula --key marula",
];

/// One probe outcome. `state` is the wire vocabulary the pane switches on:
/// `up` | `down` | `not_configured` | `unknown`.
struct ProbeResult {
    state: &'static str,
    detail: Option<String>,
    body: Option<serde_json::Value>,
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_configured(name: &str) -> bool {
    env_text(name).is_some()
}

/// A service's DSN state from its `/healthz` JSON: `configured` | `unset`,
/// or `unknown` when the service never reported it.
fn dsn_state(body: &Option<serde_json::Value>) -> String {
    match body
        .as_ref()
        .and_then(|v| v.get("glitchtip"))
        .and_then(|v| v.as_str())
    {
        Some("configured") => "configured".into(),
        Some("unset") => "unset".into(),
        _ => "unknown".into(),
    }
}

fn dsn_detail(probe: &ProbeResult) -> String {
    if probe.body.is_some() {
        "reported by the service's /healthz".into()
    } else {
        "unknown — the service's /healthz did not answer".into()
    }
}

/// Database reachability as observed by a fresh pool acquire.
async fn probe_database(state: &AppState) -> (&'static str, Option<String>) {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => ("up", Some("SELECT 1 answered".into())),
        Err(e) => ("down", Some(e.to_string())),
    }
}

/// Valkey reachability over the wire protocol: a bounded PING that must
/// answer `+PONG`. No client library — this is a probe, not a data path.
async fn probe_valkey(redis_url: &str) -> ProbeResult {
    let Some(addr) = redis_addr(redis_url) else {
        return ProbeResult {
            state: "unknown",
            detail: Some("REDIS_URL is not a parseable host[port] URL".into()),
            body: None,
        };
    };
    let probe = async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(&addr).await?;
        stream.write_all(b"PING\r\n").await?;
        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).await?;
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&buf[..n]).trim().to_string())
    };
    match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
        Ok(Ok(reply)) if reply.starts_with("+PONG") => ProbeResult {
            state: "up",
            detail: Some(format!("{addr} answered PING")),
            body: None,
        },
        Ok(Ok(reply)) => ProbeResult {
            state: "down",
            detail: Some(format!("{addr}: unexpected reply {reply:?}")),
            body: None,
        },
        Ok(Err(e)) => ProbeResult {
            state: "down",
            detail: Some(format!("{addr}: {e}")),
            body: None,
        },
        Err(_) => ProbeResult {
            state: "down",
            detail: Some(format!("{addr}: timed out")),
            body: None,
        },
    }
}

/// `redis://[user:pass@]host[:port][/db]` → `host:port` (default 6379).
fn redis_addr(redis_url: &str) -> Option<String> {
    let rest = redis_url.split_once("://").map(|(_, rest)| rest)?;
    let host_port = rest.split(['/', '?']).next()?;
    let host_port = host_port.rsplit('@').next()?;
    if host_port.is_empty() {
        return None;
    }
    Some(if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:6379")
    })
}

/// GET `{base}/healthz` with the bounded client; a missing base URL is the
/// honest `not_configured` state, not a failure.
async fn probe_optional_http(client: &reqwest::Client, base: Option<&str>) -> ProbeResult {
    let Some(base) = base else {
        return ProbeResult {
            state: "not_configured",
            detail: Some("health URL is not set for this process".into()),
            body: None,
        };
    };
    let target = format!("{}/healthz", base.trim_end_matches('/'));
    match client.get(&target).send().await {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let body = serde_json::from_str::<serde_json::Value>(&text).ok();
            let reported_ok = body
                .as_ref()
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str())
                .map(|s| s == "ok")
                .unwrap_or(true);
            if status.is_success() && reported_ok {
                ProbeResult {
                    state: "up",
                    detail: Some(format!("{target}: {status}")),
                    body,
                }
            } else {
                ProbeResult {
                    state: "down",
                    detail: Some(format!("{target}: {status}")),
                    body,
                }
            }
        }
        Err(e) => ProbeResult {
            state: "down",
            detail: Some(format!("{target}: {e}")),
            body: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct SelfHostView {
    pub scan: ScanView,
    pub detections: Vec<DetectionView>,
    pub services: Vec<ServiceView>,
    pub glitchtip_dsns: Vec<DsnRow>,
    pub links: SelfHostLinksView,
    pub garage: GarageView,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScanView {
    /// `clamd` (scanner wired) or `none` (scanning disabled).
    pub provider: String,
    /// Configured clamd TCP address — a host:port, never a credential.
    pub clamd_addr: Option<String>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub last_scan_at: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub last_detection_at: Option<DateTime<Utc>>,
    pub detection_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DetectionView {
    #[schema(value_type = uuid::Uuid)]
    pub message_id: Uuid,
    pub subject: Option<String>,
    pub from_address: String,
    /// Recorded scan verdict (`infected`); fail-closed quarantines ride the
    /// same status, so the pane words it as "quarantined", not "virus".
    pub verdict: String,
    /// Ledger transport label — the ingest source (`webhook/clamd`, …).
    pub ingest_source: String,
    #[schema(value_type = String, format = "date-time")]
    pub occurred_at: DateTime<Utc>,
    /// Files that rode the message; the v1 scanner returns one verdict per
    /// message (body plus attachment bytes), so these are what was scanned
    /// with it, not per-file verdicts.
    pub attachments: Vec<DetectionFile>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DetectionFile {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceView {
    pub id: String,
    pub label: String,
    /// `up` | `down` | `not_configured` | `unknown`.
    pub state: String,
    pub detail: Option<String>,
    /// The raw `/healthz` body when the service speaks JSON (the worker's
    /// per-loop liveness flags ride here); null otherwise.
    pub loops: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DsnRow {
    pub service: String,
    /// `configured` | `unset` | `unknown` — never the DSN itself.
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SelfHostLinksView {
    pub glitchtip: Option<String>,
    pub grafana: Option<String>,
    pub tempo: Option<String>,
    pub loki: Option<String>,
    pub otel_metrics: Option<String>,
    /// Uptime Kuma's status-page UI (Wave 34 claims close-out: the section
    /// 2 observability row that had not shipped). Same link-out shape as
    /// Grafana — a standalone operator UI in the observability profile
    /// that Marula never reads from, so the pane can only offer the link
    /// and never report on it.
    pub uptime_kuma: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GarageView {
    /// Named volumes in the shipped compose file (the compose project
    /// prefix applies at runtime).
    pub volumes: Vec<String>,
    /// Mount points inside the Garage container.
    pub container_paths: Vec<String>,
    /// The documented bootstrap sequence, verbatim.
    pub bootstrap_commands: Vec<String>,
    pub admin_api_docs_url: String,
    /// S3 endpoint the API is configured against, when S3 mode is on.
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
}
