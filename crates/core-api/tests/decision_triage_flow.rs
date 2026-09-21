//! End-to-end decision-triage flow through the REAL core-api router
//! (Wave 23 lane 3, ADR 0010 §3 #1–#4 + #9).
//!
//! Purpose: the worker seam (`POST /internal/decision/fan-out`) riding
//! the landed decision edge — scripted provider answers (no keys, no
//! network), answers + thresholds surfaced for the workbench card, the
//! alert decision-ack idempotence, the workbench suggestions read model
//! (act/pending classification), and the OLD-SCHOOL GUARANTEE: the org
//! master switch OFF ⇒ typed quiet + ZERO ledger rows + ZERO
//! decision_answers rows (the surface acts exactly as it would without
//! a decision model).
//!
//! Non-purpose: stream-consumption mechanics (worker crate unit tests),
//! the automations `ai_decision` evaluation (worker + core-domain unit
//! tests), the UI card itself (vision review + vitest).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "test-internal-token";

async fn test_state() -> (AppState, PgPool) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("decision e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    config.internal_token = Some(INTERNAL_TOKEN.to_string());
    let reporting_url = admin_url.clone();
    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    (state, pool)
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("x-internal-token", INTERNAL_TOKEN);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Decision Triage Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .into()
}

async fn me_ids(app: &Router, bearer: &str) -> (Uuid, Uuid) {
    let response = send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    (
        me["user_id"].as_str().unwrap().parse().unwrap(),
        me["org_id"].as_str().unwrap().parse().unwrap(),
    )
}

/// Deterministic scripted decision provider (no network, no keys).
struct ScriptedProvider {
    answers: Vec<core_decision::provider::Answer>,
}

#[async_trait::async_trait]
impl core_decision::provider::DecisionProvider for ScriptedProvider {
    fn kind(&self) -> &'static str {
        "scripted_test"
    }
    async fn answer(
        &self,
        _req: core_decision::provider::DecisionRequest,
    ) -> Result<core_decision::provider::DecisionResponse, core_decision::provider::DecisionError>
    {
        use core_decision::provider::{DecisionResponse, QuestionUsage};
        Ok(DecisionResponse {
            answers: self.answers.clone(),
            usage: QuestionUsage {
                prompt_tokens: 100,
                completion_tokens: 0,
            },
        })
    }
}

fn noul(id: &str, probability: f64) -> core_decision::provider::Answer {
    core_decision::provider::Answer::Noul {
        id: id.into(),
        probability,
    }
}

fn score(id: &str, level: usize, width: usize) -> core_decision::provider::Answer {
    core_decision::provider::Answer::Score {
        id: id.into(),
        level,
        distribution: {
            let mut dist = vec![0.0; width];
            dist[level.min(width - 1)] = 1.0;
            dist
        },
        confidence: 0.9,
    }
}

/// Enables the org master switch, curates the catalog pair, seals an
/// org credential, and enables the given decision surfaces (live mode,
/// pinned typesafe/jev-1.13.0) — the full admission path.
async fn enable_ai_with_token(
    app: &Router,
    pool: &PgPool,
    bearer: &str,
    org_id: Uuid,
    user_id: Uuid,
    surfaces: &[&str],
) {
    // 1. master switch ON
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(bearer),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch must flip");

    // 2. catalog pair with the decision capability
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(bearer),
        Some(json!({
            "provider": "typesafe",
            "model": "jev-1.13.0",
            "label": "Jev pinned",
            "capabilities": ["decision"],
            "cost_weight": 1,
            "enabled": true
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "catalog curation");

    // 3. sealed org credential for typesafe
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(bearer),
        Some(json!({
            "provider": "typesafe",
            "kind": "api_key",
            "secret": "ts-secret-never-surfaces-w23",
            "label": "org typesafe",
            "idempotency_key": format!("cred-{}", Uuid::now_v7())
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "credential seal");

    // 4. grant the native decision surfaces to the calling user
    for surface in surfaces {
        let response = send(
            app,
            axum::http::Method::PUT,
            "/api/settings/ai/grants",
            Some(bearer),
            Some(json!({
                "surface_key": surface,
                "grantee_kind": "user",
                "user_id": user_id.to_string()
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "grant {surface}");
    }

    // 5. surface configs: enabled + live + pinned model (starter sets)
    let mut tx = pool.begin().await.unwrap();
    for surface in surfaces {
        core_db::decision::upsert_surface_config(
            &mut tx,
            &core_db::decision::DecisionSurfaceConfig {
                org_id,
                surface_key: surface.to_string(),
                starter_version: 1,
                customized: false,
                config: None,
                mode: "live".into(),
                model_provider: Some("typesafe".into()),
                model_id: Some("jev-1.13.0".into()),
                enabled: true,
            },
            user_id,
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn counts(pool: &PgPool, org_id: Uuid) -> (i64, i64) {
    let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM ai_usage_ledger WHERE org_id = $1")
        .bind(org_id)
        .fetch_one(pool)
        .await
        .unwrap();
    let answers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM decision_answers WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(pool)
            .await
            .unwrap();
    (ledger, answers)
}

async fn fan_out(app: &Router, org_id: Uuid, user_id: Uuid, surface: &str, entity: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/internal/decision/fan-out",
        None,
        Some(json!({
            "org_id": org_id,
            "user_id": user_id,
            "surface_key": surface,
            "entity_type": "ticket",
            "entity_id": entity,
            "state": { "ticket": { "subject": "Printer on fire", "body": "sparks" } },
            "request_id": format!("req-{}", Uuid::now_v7()),
            "inventories": {}
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "fan-out is degrade-never-5xx");
    body_json(response).await
}

#[tokio::test(flavor = "multi_thread")]
async fn old_school_guarantee_switch_off_is_typed_quiet_zero_side_effects() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("decision-off-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (_, org_id) = me_ids(&app, &owner).await;
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            answers: vec![noul("actionable", 0.99)],
        }),
    );
    let run = fan_out(
        &app,
        org_id,
        Uuid::now_v7(),
        "decision_ticket_triage",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(run["status"], "quiet");
    assert_eq!(run["reason"], "ai_features_disabled");
    let (ledger, answers) = counts(&pool, org_id).await;
    assert_eq!(
        (ledger, answers),
        (0, 0),
        "zero side effects with switch OFF"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn triage_fan_out_records_answers_thresholds_and_ledger() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("decision-on-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    enable_ai_with_token(
        &app,
        &pool,
        &owner,
        org_id,
        user_id,
        &["decision_ticket_triage"],
    )
    .await;
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            // actionable 0.9 (≥ 0.8 act), urgency level 4 (≥ 4.0 act —
            // level is an INDEX so 4 == level 5), frustrated 0.3 (ignore),
            // board choice with confidence 0.9.
            answers: vec![
                noul("actionable", 0.9),
                score("urgency", 4, 5),
                noul("frustrated", 0.3),
                core_decision::provider::Answer::Choice {
                    id: "board".into(),
                    picked: 0,
                    distribution: vec![1.0],
                    confidence: 0.9,
                },
            ],
        }),
    );

    // A real ticket the suggestions read model can point at.
    let nav = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&owner),
            None,
        )
        .await,
    )
    .await;
    let board_id = nav[0]["id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&owner),
        Some(json!({
            "board_id": board_id,
            "title": "Printer on fire",
            "body": "sparks and smoke",
            "priority": "p1"
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create");
    let ticket_id = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .unwrap();

    let run = fan_out(
        &app,
        org_id,
        user_id,
        "decision_ticket_triage",
        json!(ticket_id),
    )
    .await;
    assert_eq!(run["status"], "ok", "live fan-out: {run:?}");
    assert_eq!(run["mode"], "live");
    assert_eq!(run["answers"]["actionable"]["probability"], 0.9);
    let thresholds = run["thresholds"].clone();
    assert_eq!(thresholds["actionable"]["act"], 0.8);
    assert_eq!(thresholds["urgency"]["act"], 4.0);

    let (ledger, answers) = counts(&pool, org_id).await;
    assert_eq!(ledger, 1, "every decision call rides the usage ledger");
    assert_eq!(answers, 1, "shadow store records the run");

    // Workbench suggestions read model: classified act/pending per
    // thresholds; board gates on confidence.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_id}/decision-suggestions"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let surfaces = view["suggestions"].as_array().unwrap();
    assert_eq!(surfaces.len(), 1);
    let triage = &surfaces[0];
    assert_eq!(triage["surface"], "decision_ticket_triage");
    assert_eq!(triage["mode"], "live");
    let by_question: std::collections::HashMap<&str, &Value> = triage["answers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["question_id"].as_str().unwrap(), a))
        .collect();
    assert_eq!(by_question["actionable"]["state"], "act");
    assert_eq!(by_question["urgency"]["state"], "act");
    assert_eq!(by_question["frustrated"]["state"], "ignore");
    assert_eq!(by_question["board"]["state"], "act", "confidence 0.9 ≥ 0.7");
}

#[tokio::test(flavor = "multi_thread")]
async fn alert_decision_ack_is_idempotent_above_thresholds() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("decision-alert-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    let _ = (user_id, owner);

    // Seed one OPEN alert directly (the ingest HMAC path is its own
    // flow's business; this lane exercises the decision ack).
    let alert_id = Uuid::now_v7();
    let source_id = Uuid::now_v7();
    let board_id: Uuid = sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1 LIMIT 1")
        .bind(org_id)
        .fetch_one(&pool)
        .await
        .expect("org has a board");
    sqlx::query(
        r#"INSERT INTO alert_sources (id, org_id, name, source_ref, secret_encrypted, board_id)
            VALUES ($1, $2, 'test-source', $3, '\x00', $4)"#,
    )
    .bind(source_id)
    .bind(org_id)
    .bind(format!("src-{}", source_id))
    .bind(board_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key,
                               title, description, severity, occurred_at, attributes, status)
            VALUES ($1, $2, $3, $4, $4, 'Disk full', 'C: 99%', 'critical', now(), '{}', 'open')"#,
    )
    .bind(alert_id)
    .bind(org_id)
    .bind(source_id)
    .bind(format!("dedup-{}", alert_id))
    .execute(&pool)
    .await
    .unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        "/internal/alerts/decision-ack",
        None,
        Some(json!({
            "org_id": org_id,
            "alert_id": alert_id,
            "actor_id": user_id,
            "request_id": format!("decision-alert-{}", alert_id)
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["acked"], true);

    // Idempotent replay: already acknowledged ⇒ acked=false, no error.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/internal/alerts/decision-ack",
        None,
        Some(json!({
            "org_id": org_id,
            "alert_id": alert_id,
            "actor_id": user_id,
            "request_id": format!("decision-alert-replay-{}", alert_id)
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["acked"], false);

    let status: String = sqlx::query_scalar("SELECT status FROM alerts WHERE id = $1")
        .bind(alert_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "acknowledged");
}
