//! Report authoring flow over the REAL core-api router (Wave 46 lane 3).
//!
//! Purpose: "describe it" → a report DRAFT through the shared AI admission path
//! and the shared transport seam — the model authors, the SAME `ReportSpec` +
//! `CardSpec` boundary the save and run routes use validates, and NOTHING is
//! written. The on-demand column tool round-trips; a bad model answer comes back
//! as a typed issue, never a stored row; AI admission refusals ride the existing
//! edge (no new admission path, no report-specific surface).
//!
//! Non-purpose: the LLM's real quality (no provider call — the answers are
//! deterministic `ScriptedTransport` turns), the registry itself (core-reports
//! suites), and the save route (report_defs). This file also emits the
//! deterministic measurement evidence.
//!
//! `MARULA_TEST_DATABASE_URL` (or `DATABASE_URL`) must point at a booted stack
//! (see `scripts/e2e-stack.sh`).

use crate::common::{body_json, boot_with, register_org_token, send};
use axum::Router;
use core_api::routes::chat_runner as ChatRunner;
use core_api::routes::report_authoring::{self, AuthoringSample};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

async fn test_state() -> (PgPool, Router) {
    let (state, app) = boot_with(|_| {}).await;
    (state.pool.clone(), app)
}

/// Registers an org whose slug is unique per run. The database outlives a run,
/// so a fixed slug answers 409 `org_slug_taken` on the second one.
async fn register_fresh_org(app: &Router, prefix: &str) -> String {
    let slug = format!("{prefix}-{}", Uuid::now_v7().simple());
    register_org_token(app, "Report AI Co", &slug).await
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

/// Catalog + org credential + surface grant + master switch: the complete
/// admit kit for the owner on `general_chat`. No report-specific surface — the
/// authoring edge is an AI surface like any other.
async fn seed_admit_kit(app: &Router, owner: &str, user_id: Uuid, hex: &str) {
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(owner),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "master switch on");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(owner),
        Some(json!({
            "provider": "openai",
            "model": "marula-test-chat",
            "label": "Report Authoring Test",
            "capabilities": ["chat", "completion", "tools"],
            "cost_weight": 1,
            "idempotency_key": format!("rcat-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "catalog seed");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(owner),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-report-authoring-test-secret",
            "label": "org openai",
            "idempotency_key": format!("rcred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "credential seed");
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(owner),
        Some(json!({
            "surface_key": "general_chat",
            "grantee_kind": "user",
            "user_id": user_id.to_string(),
            "idempotency_key": format!("rgrant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "surface grant");
}

async fn author(app: &Router, bearer: &str, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/reports/author",
        Some(bearer),
        Some(body),
    )
    .await
}

fn final_turn(prompt: u32, completion: u32, content: &str) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: content.to_string(),
        tool_calls: Vec::new(),
        prompt_tokens: prompt,
        completion_tokens: completion,
    }
}

fn columns_turn(id: &str, view: &str) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: String::new(),
        tool_calls: vec![ChatRunner::ToolCallReq {
            id: id.to_string(),
            name: report_authoring::VIEW_COLUMNS_TOOL.to_string(),
            arguments: json!({ "view": view }),
        }],
        prompt_tokens: 30,
        completion_tokens: 6,
    }
}

fn tickets_by_priority() -> Value {
    json!({
        "name": "Tickets by priority",
        "spec": {
            "view": "tickets",
            "filters": [],
            "group_by": ["priority"],
            "measures": [{ "agg": "count" }],
            "viz": "bar"
        },
        "card": { "window_days": 30 }
    })
}

async fn ledger_rows(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> Vec<Value> {
    let rows: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT surface, prompt_tokens, completion_tokens \
         FROM ai_usage_ledger WHERE org_id = $1 AND user_id = $2 ORDER BY created_at",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .expect("ledger read");
    rows.into_iter()
        .map(|(surface, prompt, completion)| {
            json!({ "surface": surface, "prompt_tokens": prompt, "completion_tokens": completion })
        })
        .collect()
}

#[tokio::test]
async fn describe_it_returns_a_validated_report_draft_and_writes_nothing() {
    let (pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "report-author-happy").await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "happy").await;
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![
            columns_turn("c1", "tickets"),
            final_turn(150, 90, &tickets_by_priority().to_string()),
        ])),
    );

    let response = author(
        &app,
        &owner,
        json!({
            "surface_key": "general_chat",
            "transcript": "Tickets by priority as a bar chart, last thirty days."
        }),
    )
    .await;
    assert_eq!(response.status(), 200, "authoring must succeed");
    let body = body_json(response).await;
    assert_eq!(body["valid"], json!(true), "draft must validate: {body}");
    assert_eq!(body["saved"], json!(false), "authoring never saves");
    // The contract's one derivation decided the shape; the model did not send it.
    assert_eq!(body["draft"]["shape"], json!("chart"));
    assert_eq!(body["draft"]["spec"]["view"], json!("tickets"));
    assert_eq!(body["draft"]["card"]["window_days"], json!(30));
    // The read-back is derived from the spec, so it says what the report does.
    assert_eq!(
        body["draft"]["reads"],
        json!(
            "A chart over tickets grouped by priority, showing a count of rows. It spans \
             the last 30 days, refreshes every hour, and sits half wide and regular tall."
        )
    );
    // The on-demand column tool was called and succeeded.
    assert_eq!(
        body["tool_calls"][0]["name"],
        json!(report_authoring::VIEW_COLUMNS_TOOL)
    );
    assert_eq!(body["tool_calls"][0]["argument"], json!("tickets"));
    assert_eq!(body["tool_calls"][0]["outcome"], json!("ok"));
    // A fresh description diffs against nothing → additions only.
    let diff = body["diff"].as_array().unwrap();
    assert!(diff
        .iter()
        .any(|d| d["op"] == json!("add") && d["path"] == json!("group_by/priority")));
    assert!(diff
        .iter()
        .any(|d| d["op"] == json!("add") && d["path"] == json!("card/window_days")));
    // Catalogue provenance is what the prompt actually carried.
    assert_eq!(body["catalogue"]["views"], json!(5));
    assert_eq!(body["catalogue"]["shapes"], json!(3));
    assert_eq!(body["catalogue"]["rules"], json!(3));
    assert!(body["catalogue"]["prompt_chars"].as_u64().unwrap() > 0);
    // Usage ledger row landed (caps bind across calls).
    let ledger = ledger_rows(&pool, org_id, user_id).await;
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["surface"], json!("general_chat"));
    assert_eq!(ledger[0]["prompt_tokens"], json!(180));
    assert_eq!(ledger[0]["completion_tokens"], json!(96));

    // NOTHING was stored: no definition exists for this org.
    let definitions = send(
        &app,
        axum::http::Method::GET,
        "/api/report-definitions",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(definitions.status(), 200);
    let body = body_json(definitions).await;
    assert_eq!(
        body["definitions"].as_array().map(Vec::len),
        Some(0),
        "authoring persists nothing: {body}"
    );
}

#[tokio::test]
async fn a_bad_model_answer_comes_back_typed_and_still_writes_nothing() {
    let (pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "report-author-bad").await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "bad").await;
    // A spec naming a view the registry does not have.
    let bad = json!({
        "name": "Broken",
        "spec": {
            "view": "invoices",
            "group_by": [],
            "measures": [{ "agg": "count" }],
            "viz": "stat"
        },
        "card": { "window_days": 30 }
    });
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![final_turn(
            90,
            40,
            &bad.to_string(),
        )])),
    );

    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "transcript": "something invalid" }),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the endpoint answers, it does not 5xx"
    );
    let body = body_json(response).await;
    assert_eq!(body["valid"], json!(false));
    assert_eq!(body["saved"], json!(false));
    assert!(body["draft"].is_null(), "no draft on a rejected answer");
    assert_eq!(body["issues"][0]["code"], json!("bad_spec_shape"));
    let ledger = ledger_rows(&pool, org_id, user_id).await;
    assert_eq!(ledger.len(), 1, "a rejected draft still costs tokens");
}

#[tokio::test]
async fn editing_diffs_against_the_current_draft_instead_of_nothing() {
    let (_pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "report-author-edit").await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "edit").await;
    // Same report plus a second dimension and a narrower window.
    let proposal = json!({
        "name": "Tickets by priority",
        "spec": {
            "view": "tickets",
            "filters": [],
            "group_by": ["priority", "status_name"],
            "measures": [{ "agg": "count" }],
            "viz": "bar"
        },
        "card": { "window_days": 7 }
    });
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![final_turn(
            120,
            70,
            &proposal.to_string(),
        )])),
    );

    let response = author(
        &app,
        &owner,
        json!({
            "surface_key": "general_chat",
            "transcript": "Also break it down by status, and only the last week.",
            "name": "Tickets by priority",
            "spec": {
                "view": "tickets",
                "filters": [],
                "group_by": ["priority"],
                "measures": [{ "agg": "count" }],
                "viz": "bar"
            },
            "card": { "window_days": 30 }
        }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["valid"], json!(true), "{body}");
    let diff = body["diff"].as_array().unwrap();
    // The added dimension is an add; the one that was already there is not in
    // the diff at all.
    assert!(diff
        .iter()
        .any(|d| d["op"] == json!("add") && d["path"] == json!("group_by/status name")));
    assert!(
        !diff.iter().any(|d| d["path"] == json!("group_by/priority")),
        "an unchanged dimension is not a change: {diff:?}"
    );
    let window = diff
        .iter()
        .find(|d| d["path"] == json!("card/window_days"))
        .expect("the window change is in the diff");
    assert_eq!(window["op"], json!("change"));
    assert_eq!(window["before"], json!(30));
    assert_eq!(window["after"], json!(7));
    // The unchanged parts are absent too.
    assert!(!diff.iter().any(|d| d["path"] == json!("view")));
}

#[tokio::test]
async fn a_current_draft_that_is_not_saveable_is_refused_before_the_model_call() {
    let (_pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "report-author-base").await;
    // No admit kit on purpose: the base check must run first, so this answers
    // 422 rather than the 403 an un-admitted AI call would give.
    let response = author(
        &app,
        &owner,
        json!({
            "surface_key": "general_chat",
            "transcript": "anything",
            "spec": {
                "view": "tickets",
                "filters": [],
                "group_by": [],
                "measures": [],
                "viz": "table"
            },
            "card": { "window_days": 30 }
        }),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], json!("validation_failed"));

    // Half a draft is a caller bug, not a fresh generation.
    let response = author(
        &app,
        &owner,
        json!({
            "surface_key": "general_chat",
            "transcript": "anything",
            "card": { "window_days": 30 }
        }),
    )
    .await;
    assert_eq!(response.status(), 422);

    // A pasted document is not a description: refused typed, never truncated.
    let long = "x".repeat(report_authoring::MAX_TRANSCRIPT_CHARS + 1);
    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "transcript": long }),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], json!("validation_failed"));
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains(&report_authoring::MAX_TRANSCRIPT_CHARS.to_string()));
}

#[tokio::test]
async fn ai_admission_is_the_existing_path() {
    let (_pool, app) = test_state().await;
    // No admit kit: AI features are off by default → typed refusal, not a run.
    let owner = register_fresh_org(&app, "report-author-refused").await;
    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "transcript": "anything" }),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "an un-admitted AI call is refused typed on the shared edge"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], json!("ai_features_disabled"));
}

/// The deterministic measurement evidence. Runs two corpora through the REAL
/// registry validation and the real compile path, and — when a lane names its
/// evidence directory in `MARULA_EVIDENCE_DIR` — writes the report beneath it.
/// Without that variable it writes nothing: the report is lane evidence, which
/// is company material and must never land in this public tree.
#[tokio::test]
async fn measurement_report_is_recorded() {
    let realistic = vec![
        AuthoringSample {
            name: "chart".into(),
            candidate: tickets_by_priority().to_string(),
        },
        AuthoringSample {
            name: "stat".into(),
            candidate: json!({
                "name": "Total hours",
                "spec": {
                    "view": "time_entries",
                    "filters": [],
                    "group_by": [],
                    "measures": [{ "agg": "sum", "column": "minutes" }],
                    "viz": "stat"
                },
                "card": { "window_days": 7 }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "table-with-filter".into(),
            candidate: json!({
                "name": "P1 tickets",
                "spec": {
                    "view": "tickets",
                    "filters": [{ "column": "priority", "op": "eq", "value": { "text": "p1" } }],
                    "group_by": [],
                    "measures": [{ "agg": "count" }],
                    "viz": "table"
                },
                "card": { "window_days": 1 }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "fenced-chart".into(),
            candidate: format!("```json\n{}\n```", tickets_by_priority()),
        },
    ];
    let boundary = vec![
        AuthoringSample {
            name: "unknown-view".into(),
            candidate: json!({
                "spec": { "view": "invoices", "group_by": [], "measures": [{ "agg": "count" }], "viz": "stat" },
                "card": { "window_days": 30 }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "unknown-column".into(),
            candidate: json!({
                "spec": {
                    "view": "tickets",
                    "group_by": ["nonsense"],
                    "measures": [{ "agg": "count" }],
                    "viz": "bar"
                },
                "card": { "window_days": 30 }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "stat-with-dimension".into(),
            candidate: json!({
                "spec": {
                    "view": "tickets",
                    "group_by": ["priority"],
                    "measures": [{ "agg": "count" }],
                    "viz": "stat"
                },
                "card": { "window_days": 30 }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "window-wider-than-refresh".into(),
            candidate: json!({
                "spec": {
                    "view": "tickets",
                    "group_by": ["priority"],
                    "measures": [{ "agg": "count" }],
                    "viz": "bar"
                },
                "card": { "window_days": 30, "refresh": { "every_minutes": 60, "reach_days": 7 } }
            })
            .to_string(),
        },
        AuthoringSample {
            name: "prose-only".into(),
            candidate: "I am not able to build that.".into(),
        },
    ];

    let now = chrono::Utc::now();
    let scope = core_reports::Scope::OrgWide;
    let realistic_measure = report_authoring::measure_corpus(&realistic, scope, now);
    let boundary_measure = report_authoring::measure_corpus(&boundary, scope, now);
    let (prompt_chars, column_chars, views) = report_authoring::measure_prompt_footprint();

    // The boundary MUST accept every realistic draft and reject every
    // malformed one — this is the registry holding, not the model.
    assert_eq!(
        realistic_measure.boundary_valid, realistic_measure.total,
        "every realistic draft validates: {realistic_measure:?}"
    );
    assert!(
        boundary_measure.boundary_rejected >= 4,
        "the boundary rejects each malformation class: {boundary_measure:?}"
    );

    let report = json!({
        "lane": "p46-authoring-draft",
        "catalogue": {
            "views": views,
            "shapes": core_reports::shape_catalogue().len(),
            "rules": core_reports::rules().len(),
        },
        "prompt_footprint": {
            "prompt_chars": prompt_chars,
            "column_inventory_chars": column_chars,
            "prompt_with_columns_chars": prompt_chars + column_chars,
            "column_share_pct": ((column_chars as f64
                / (prompt_chars + column_chars) as f64) * 100.0).round(),
            "note": "The prompt does not grow when a column is added; the inventory \
                     column does. With five views the rules prose dominates, so \
                     inlining today would cost the share printed, and that share \
                     rises with every column."
        },
        "realistic_corpus": realistic_measure,
        "boundary_corpus": boundary_measure,
        "note": "Deterministic (ScriptedTransport) corpus through the REAL \
                 registry validation and the real compile path. `realistic_corpus` \
                 = plausible model answers (all must validate). `boundary_corpus` = \
                 deliberate malformations (the boundary must reject each). This \
                 measures the boundary and the prompt footprint, not a live \
                 model's rate.",
    });
    let Some(dir) = std::env::var_os("MARULA_EVIDENCE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join("qa-p46-authoring-draft/measurement.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("evidence dir");
    }
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap())
        .expect("write measurement");
}
