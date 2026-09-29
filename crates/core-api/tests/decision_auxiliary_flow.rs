//! End-to-end decision-AUXILIARY flow through the REAL core-api router
//! (Wave 23 lane 4, ADR 0010 §3 #5–#8 + ADR 0011 §4).
//!
//! Purpose: the auxiliary surfaces riding the landed decision edge —
//! `decision_search_rerank` (the similar-tickets read model over the
//! existing Postgres index), the org-facing admission for
//! `decision_billing_audit` / `decision_record_dedupe`, the lane-3
//! `decision_contract_scope` seam completion (its registry + question-set
//! rows were missing until 0092), and the `decision_loop_termination`
//! quiet posture. Scripted provider answers (no keys, no network).
//!
//! Old-school guarantee asserted again for the NEW surfaces: master
//! switch OFF ⇒ retrieval-only similar-tickets + ZERO ledger rows + ZERO
//! decision_answers rows.

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("decision auxiliary e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
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
            "org_name": "Decision Auxiliary Flow Co",
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

/// Deterministic scripted decision provider (no network, no keys) —
/// answers every ask with the SAME scripted set (the surfaces ask
/// different question maps; a mismatched question id reads as unanswered,
/// which is exactly the honest-gate behavior).
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

/// Master switch ON + catalog pair + sealed credential + per-surface
/// config (live, pinned typesafe/jev-1.13.0). `grant_surfaces` are the
/// grant-targeted (kind `decision`) keys; internal keys are core-called
/// and take no grants.
async fn enable_ai(
    app: &Router,
    pool: &PgPool,
    bearer: &str,
    org_id: Uuid,
    user_id: Uuid,
    hex: &str,
    grant_surfaces: &[&str],
) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(bearer),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch must flip");

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
            "enabled": true,
            "idempotency_key": format!("cat-{hex}")
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "catalog curation");

    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(bearer),
        Some(json!({
            "provider": "typesafe",
            "kind": "api_key",
            "secret": "ts-secret-never-surfaces-w23l4",
            "label": "org typesafe",
            "idempotency_key": format!("cred-{hex}")
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "credential seal");

    for surface in grant_surfaces {
        let response = send(
            app,
            axum::http::Method::PUT,
            "/api/settings/ai/grants",
            Some(bearer),
            Some(json!({
                "surface_key": surface,
                "grantee_kind": "user",
                "user_id": user_id.to_string(),
                "idempotency_key": format!("grant-{hex}-{surface}")
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "grant {surface}");
    }

    let mut tx = pool.begin().await.unwrap();
    for surface in grant_surfaces {
        upsert_live_surface(&mut tx, org_id, surface, user_id).await;
    }
    tx.commit().await.unwrap();
}

async fn upsert_live_surface(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    surface: &str,
    user_id: Uuid,
) {
    core_db::decision::upsert_surface_config(
        tx,
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

async fn default_board(app: &Router, bearer: &str) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"].as_str().unwrap().parse().unwrap()
}

async fn create_ticket(app: &Router, bearer: &str, board: Uuid, title: &str, body: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({
            "board_id": board,
            "title": title,
            "body": body,
            "priority": "p2"
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn seed_ticket_index_row(
    pool: &PgPool,
    org_id: Uuid,
    ticket_id: Uuid,
    title: &str,
    body: &str,
) {
    sqlx::query(
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body, client_scope)
             VALUES ($1, $2, 'ticket', $3, NULL, $4, $5, NULL)
             ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE
               SET title = EXCLUDED.title, body = EXCLUDED.body"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(ticket_id)
    .bind(title)
    .bind(body)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn similar_tickets_off_switch_is_retrieval_only_zero_decision_rows() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("similar-off-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;

    let board = default_board(&app, &owner).await;
    let anchor = create_ticket(&app, &owner, board, "Printer on fire", "sparks and smoke").await;
    let other = create_ticket(
        &app,
        &owner,
        board,
        "Printer sparks and smoke",
        "the fire came back",
    )
    .await;
    seed_ticket_index_row(&pool, org_id, anchor, "Printer on fire", "sparks and smoke").await;
    seed_ticket_index_row(
        &pool,
        org_id,
        other,
        "Printer sparks and smoke",
        "the fire came back",
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{anchor}/similar-tickets"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    assert_eq!(view["candidates"].as_array().unwrap().len(), 1);
    let candidate = &view["candidates"][0];
    assert_eq!(candidate["ticket_id"], json!(other));
    assert_eq!(candidate["relates"], Value::Null, "no decision answers");
    assert_eq!(candidate["mode"], Value::Null);
    assert!(candidate["retrieval_score"].as_f64().unwrap() > 0.0);

    let (ledger, answers) = counts(&pool, org_id).await;
    let _ = user_id;
    assert_eq!(
        (ledger, answers),
        (0, 0),
        "old-school org: zero decision side effects"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn similar_tickets_live_reranks_and_records_answers() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("similar-on-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    enable_ai(&app, &pool, &owner, org_id, user_id, &hex, &[]).await;
    {
        let mut tx = pool.begin().await.unwrap();
        upsert_live_surface(&mut tx, org_id, "decision_search_rerank", user_id).await;
        tx.commit().await.unwrap();
    }
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            answers: vec![noul("relates", 0.9)],
        }),
    );

    let board = default_board(&app, &owner).await;
    let anchor = create_ticket(&app, &owner, board, "Printer on fire", "sparks and smoke").await;
    let strong = create_ticket(
        &app,
        &owner,
        board,
        "Printer sparks and smoke again",
        "the fire came back",
    )
    .await;
    for (id, title, body) in [
        (anchor, "Printer on fire", "sparks and smoke"),
        (
            strong,
            "Printer sparks and smoke again",
            "the fire came back",
        ),
    ] {
        seed_ticket_index_row(&pool, org_id, id, title, body).await;
    }

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{anchor}/similar-tickets"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let candidates = view["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "retrieval top-k minus the anchor");
    let candidate = &candidates[0];
    assert_eq!(candidate["ticket_id"], json!(strong));
    assert_eq!(candidate["relates"], 0.9);
    assert_eq!(candidate["mode"], "live");
    assert_eq!(candidate["state"], "act", "0.9 ≥ act 0.7");

    // Every candidate ask rode the ledger + the shadow store.
    let (ledger, answers) = counts(&pool, org_id).await;
    assert_eq!(ledger, 1, "one ledger row per candidate ask");
    assert_eq!(answers, 1, "one recorded run per candidate");

    // The rerank reorders ONLY on live answers; a medium answer relabels
    // the candidate `pending` (0.55 ≥ review 0.5, < act 0.7) — never acts.
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            answers: vec![noul("relates", 0.55)],
        }),
    );
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{anchor}/similar-tickets"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let candidates = view["candidates"].as_array().unwrap();
    assert_eq!(
        candidates[0]["state"], "pending",
        "0.55 ≥ review 0.5, < act 0.7"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_fan_out_admits_billing_dedupe_and_contract_scope_surfaces() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("aux-fanout-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    enable_ai(
        &app,
        &pool,
        &owner,
        org_id,
        user_id,
        &hex,
        &[
            "decision_billing_audit",
            "decision_record_dedupe",
            "decision_contract_scope",
        ],
    )
    .await;
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            answers: vec![
                noul("billable", 0.9),
                noul("matches_work", 0.8),
                noul("duplicate", 0.95),
                noul("after_hours_surcharge", 0.8),
            ],
        }),
    );

    async fn fan_out(
        app: Router,
        org_id: Uuid,
        user_id: Uuid,
        surface: &str,
        entity_type: &str,
        state: Value,
    ) -> (axum::http::StatusCode, Value) {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/internal/decision/fan-out",
            None,
            Some(json!({
                "org_id": org_id,
                "user_id": user_id,
                "surface_key": surface,
                "entity_type": entity_type,
                "entity_id": Uuid::now_v7(),
                "state": state,
                "request_id": format!("req-{}", Uuid::now_v7()),
                "inventories": {}
            })),
        )
        .await;
        (response.status(), body_json(response).await)
    }

    // billing audit: per-time-entry Noul pair through the full admission
    // path (grant + catalog + credential + starter set from 0090).
    let (status, run) = fan_out(
        app.clone(),
        org_id,
        user_id,
        "decision_billing_audit",
        "time_entry",
        json!({
            "entry": { "description": "resolved the printer driver issue" },
            "ticket": { "subject": "Printer broken", "body": "sparks" },
            "elapsed_sum": 45
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(run["status"], "ok", "billing audit admits: {run:?}");
    assert_eq!(run["answers"]["billable"]["probability"], 0.9);

    // record dedupe: the residual pairwise Noul (suggestion-grade).
    let (status, run) = fan_out(
        app.clone(),
        org_id,
        user_id,
        "decision_record_dedupe",
        "ci_merge_candidate",
        json!({
            "left": { "name": "EXCH-01", "email": "" },
            "right": { "name": "exch01", "email": "" }
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(run["status"], "ok", "record dedupe admits: {run:?}");
    assert_eq!(run["answers"]["duplicate"]["probability"], 0.95);

    // contract scope: lane 3's surface — its registry + question-set rows
    // land with 0092, so the fan-out admits (it went Unconfigured before).
    let (status, run) = fan_out(
        app.clone(),
        org_id,
        user_id,
        "decision_contract_scope",
        "ticket",
        json!({
            "contract": { "summary": "CT-1 Managed IT; included_minutes=600" },
            "request": { "description": "new office Wi-Fi APs" }
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        run["status"], "ok",
        "contract scope admits after 0092: {run:?}"
    );
    assert_eq!(run["answers"]["after_hours_surcharge"]["probability"], 0.8);

    let (_, answers) = counts(&pool, org_id).await;
    assert_eq!(answers, 3, "three recorded runs, one per surface");
}

#[tokio::test(flavor = "multi_thread")]
async fn loop_termination_quiet_is_continue_and_starter_set_resolves() {
    let (state, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("loop-quiet-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let owner = register_org(&app, &slug).await;
    let (user_id, org_id) = me_ids(&app, &owner).await;

    // Unconfigured: the step boundary is typed-quiet → Continue (the
    // loop runs exactly as it would without a decision model).
    let verdict = core_api::routes::loop_termination::check(
        &state,
        core_api::routes::loop_termination::LoopTerminationAsk {
            org_id,
            user_id,
            goal_criteria: "the printer prints a test page",
            progress_excerpt: "reinstalled the driver",
            task_created_at: chrono::Utc::now(),
            steps_completed: 1,
            hard_iteration_cap: 8,
            request_id: format!("loop-{}", Uuid::now_v7()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        verdict,
        core_api::routes::loop_termination::LoopTermination::Continue
    );
    let (ledger, answers) = counts(&pool, org_id).await;
    assert_eq!((ledger, answers), (0, 0));

    // Configured live + a satisfied Noul at a boundary UNDER the cap.
    let member = user_id;
    enable_ai(&app, &pool, &owner, org_id, member, &hex, &[]).await;
    {
        let mut tx = pool.begin().await.unwrap();
        upsert_live_surface(&mut tx, org_id, "decision_loop_termination", member).await;
        tx.commit().await.unwrap();
    }
    core_api::routes::decision_edge::seed_provider_for_org(
        org_id,
        Arc::new(ScriptedProvider {
            answers: vec![noul("goal_satisfied", 0.95)],
        }),
    );
    let verdict = core_api::routes::loop_termination::check(
        &state,
        core_api::routes::loop_termination::LoopTerminationAsk {
            org_id,
            user_id: member,
            goal_criteria: "the printer prints a test page",
            progress_excerpt: "test page printed cleanly",
            task_created_at: chrono::Utc::now(),
            steps_completed: 2,
            hard_iteration_cap: 8,
            request_id: format!("loop-{}", Uuid::now_v7()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        verdict,
        core_api::routes::loop_termination::LoopTermination::ExitEarly {
            probability: 0.95,
            act: 0.9
        }
    );
    let (_, answers) = counts(&pool, org_id).await;
    assert_eq!(answers, 1, "the boundary ask is recorded");
    let _ = &app;
}
