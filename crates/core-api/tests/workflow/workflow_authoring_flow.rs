//! Workflow authoring flow over the REAL core-api router (Wave 43 lane 17).
//!
//! Purpose: "describe it" → a valid workflow DRAFT through the shared AI
//! admission path and the shared transport seam — the model authors, the
//! SAME `WorkflowDocument` boundary validates, and NOTHING is written. The
//! on-demand schema tool round-trips; a bad model answer comes back as a
//! typed issue, never a stored row; AI admission refusals ride the existing
//! lane-1 edge (no new admission path).
//!
//! Non-purpose: the LLM's real quality (no provider call — the answers are
//! deterministic `ScriptedTransport` turns, the BYOK-injection posture), the
//! document validator itself (core-domain suites), and the save route
//! (lane 16). This file also emits the deterministic measurement evidence.
//!
//! `MARULA_TEST_DATABASE_URL` (or `DATABASE_URL`) must point at a booted
//! stack (see `scripts/e2e-stack.sh`).

use crate::common::{body_json, boot_with, register_org_token, send};
use axum::Router;
use core_api::routes::chat_runner as ChatRunner;
use core_api::routes::workflow_authoring::{self, AuthoringSample};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

async fn test_state() -> (PgPool, Router) {
    let (state, app) = boot_with(|_| {}).await;
    (state.pool.clone(), app)
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Workflow AI Co", slug).await
}

/// Registers an org whose slug is unique per run. The database outlives a run,
/// so a fixed slug answers 409 `org_slug_taken` on the second one.
async fn register_fresh_org(app: &Router, prefix: &str) -> String {
    let slug = format!("{prefix}-{}", Uuid::now_v7().simple());
    register_org(app, &slug).await
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
/// admit kit for the owner on `general_chat`. No workflow-specific surface —
/// the authoring edge is an AI surface like any other.
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
            "label": "Authoring Test",
            "capabilities": ["chat", "completion", "tools"],
            "cost_weight": 1,
            "idempotency_key": format!("cat-{hex}"),
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
            "secret": "sk-authoring-test-secret",
            "label": "org openai",
            "idempotency_key": format!("cred-{hex}"),
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
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "surface grant");
}

async fn author(app: &Router, bearer: &str, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/workflows/author",
        Some(bearer),
        Some(body),
    )
    .await
}

fn two_node_document() -> Value {
    json!({
        "version": "1",
        "nodes": [
            { "id": "raise", "node_type": "ticket.setPriority", "params": { "priority": "p1" } },
            { "id": "notify", "node_type": "notification.send", "params": {
                "userIds": ["00000000-0000-0000-0000-000000000001"],
                "kind": "escalation",
                "title": "P1 ticket raised"
            } }
        ],
        "edges": [{ "from": "raise", "to": "notify" }]
    })
}

fn envelope(document: Value) -> String {
    json!({
        "name": "Escalate a P1",
        "trigger_event": "ticket.created",
        "document": document
    })
    .to_string()
}

fn final_turn(prompt: u32, completion: u32, content: &str) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: content.to_string(),
        tool_calls: Vec::new(),
        prompt_tokens: prompt,
        completion_tokens: completion,
    }
}

fn schema_turn(id: &str, node_id: &str) -> ChatRunner::ModelTurn {
    ChatRunner::ModelTurn {
        content: String::new(),
        tool_calls: vec![ChatRunner::ToolCallReq {
            id: id.to_string(),
            name: workflow_authoring::NODE_SCHEMA_TOOL.to_string(),
            arguments: json!({ "node_id": node_id }),
        }],
        prompt_tokens: 30,
        completion_tokens: 6,
    }
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
async fn describe_it_returns_a_validated_draft_with_a_diff_and_writes_nothing() {
    let (pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "author-happy").await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "happy").await;
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![
            schema_turn("c1", "ticket.setPriority"),
            final_turn(140, 80, &envelope(two_node_document())),
        ])),
    );

    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "prompt": "When a P1 ticket arrives, raise it and notify on-call." }),
    )
    .await;
    assert_eq!(response.status(), 200, "authoring must succeed");
    let body = body_json(response).await;
    assert_eq!(body["valid"], json!(true), "draft must validate: {body}");
    assert_eq!(body["saved"], json!(false), "authoring never saves");
    assert!(body["draft"]["nodes"].as_array().unwrap().len() == 2);
    assert_eq!(body["trigger_event"], json!("ticket.created"));
    // The on-demand schema tool was called and succeeded.
    assert_eq!(
        body["tool_calls"][0]["name"],
        json!(workflow_authoring::NODE_SCHEMA_TOOL)
    );
    assert_eq!(body["tool_calls"][0]["outcome"], json!("ok"));
    // A fresh generation diffs against the empty document → additions only.
    let diff = body["diff"].as_array().unwrap();
    assert!(diff
        .iter()
        .any(|d| d["op"] == json!("add") && d["path"] == json!("nodes/raise")));
    // Registry provenance.
    assert!(body["catalogue"]["node_count"].as_u64().unwrap() > 0);
    // Usage ledger row landed (caps bind across calls).
    let ledger = ledger_rows(&pool, org_id, user_id).await;
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["surface"], json!("general_chat"));
    assert_eq!(ledger[0]["prompt_tokens"], json!(170));
    assert_eq!(ledger[0]["completion_tokens"], json!(86));
}

#[tokio::test]
async fn a_bad_model_answer_comes_back_typed_and_still_writes_nothing() {
    let (_pool, app) = test_state().await;
    let owner = register_fresh_org(&app, "author-bad").await;
    let (user_id, org_id) = me_ids(&app, &owner).await;
    seed_admit_kit(&app, &owner, user_id, "bad").await;
    let bad = json!({
        "name": "Broken",
        "trigger_event": "ticket.created",
        "document": { "version": "1",
            "nodes": [{ "id": "a", "node_type": "ticket.doesNotExist", "params": {} }],
            "edges": [] }
    })
    .to_string();
    ChatRunner::seed_transport_for_org(
        org_id,
        Arc::new(ChatRunner::ScriptedTransport::new(vec![final_turn(
            90, 40, &bad,
        )])),
    );

    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "prompt": "do something invalid" }),
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
    assert_eq!(body["issues"][0]["code"], json!("unknown_node_type"));
}

#[tokio::test]
async fn ai_admission_is_the_existing_path() {
    let (_pool, app) = test_state().await;
    // No admit kit: AI features are off by default → typed refusal, not a run.
    let owner = register_fresh_org(&app, "author-refused").await;
    let response = author(
        &app,
        &owner,
        json!({ "surface_key": "general_chat", "prompt": "anything" }),
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

/// The deterministic measurement evidence (spec E). Runs two corpora through
/// the REAL registry validation and, when a lane names its evidence directory in
/// `MARULA_EVIDENCE_DIR`, writes the report beneath it. Without that variable it
/// writes nothing: the report is lane evidence, which is company material and
/// must never land in this public tree.
#[tokio::test]
async fn measurement_report_is_recorded() {
    let realistic = vec![
        AuthoringSample {
            name: "two-step".into(),
            candidate: envelope(two_node_document()),
        },
        AuthoringSample {
            name: "single-stop".into(),
            candidate: envelope(json!({ "version": "1",
                "nodes": [{ "id": "end", "node_type": "flow.stop", "params": {} }], "edges": [] })),
        },
        AuthoringSample {
            name: "branch".into(),
            candidate: envelope(json!({ "version": "1",
                "nodes": [
                    { "id": "check", "node_type": "field.equals", "params": { "field": "ticket.priority", "value": "p1" } },
                    { "id": "branch", "node_type": "flow.branch", "params": { "condition": "check" } },
                    { "id": "yes", "node_type": "notification.send", "params": { "userIds": ["00000000-0000-0000-0000-000000000001"], "kind": "escalation", "title": "P1" } },
                    { "id": "no", "node_type": "flow.stop", "params": {} }
                ],
                "edges": [
                    { "from": "check", "to": "branch" },
                    { "from": "branch", "to": "yes", "label": "true" },
                    { "from": "branch", "to": "no", "label": "false" }
                ] })),
        },
        AuthoringSample {
            name: "fenced-valid".into(),
            candidate: format!("```json\n{}\n```", envelope(two_node_document())),
        },
    ];
    let boundary = vec![
        AuthoringSample {
            name: "unknown-node".into(),
            candidate: json!({ "document": { "version": "1",
                "nodes": [{ "id": "a", "node_type": "ticket.nope", "params": {} }], "edges": [] } })
            .to_string(),
        },
        AuthoringSample {
            name: "missing-required-param".into(),
            candidate: json!({ "document": { "version": "1",
                "nodes": [{ "id": "a", "node_type": "ticket.setPriority", "params": {} }], "edges": [] } })
            .to_string(),
        },
        AuthoringSample {
            name: "trigger-in-body".into(),
            candidate: json!({ "document": { "version": "1",
                "nodes": [{ "id": "a", "node_type": "trigger.ticket", "params": { "eventType": "ticket.created" } }], "edges": [] } })
            .to_string(),
        },
        AuthoringSample {
            name: "dangling-edge".into(),
            candidate: json!({ "document": { "version": "1",
                "nodes": [{ "id": "a", "node_type": "flow.stop", "params": {} }],
                "edges": [{ "from": "a", "to": "ghost" }] } })
            .to_string(),
        },
        AuthoringSample {
            name: "prose-only".into(),
            candidate: "I am not able to build that.".into(),
        },
    ];

    let realistic_measure = workflow_authoring::measure_corpus(&realistic);
    let boundary_measure = workflow_authoring::measure_corpus(&boundary);
    let (compact, inline, nodes) = workflow_authoring::measure_prompt_footprint();

    // The boundary MUST accept every realistic draft and reject every
    // malformed one — this is the schema boundary holding, not the model.
    assert_eq!(
        realistic_measure.schema_valid, realistic_measure.total,
        "every realistic draft validates"
    );
    assert!(
        boundary_measure.schema_rejected >= 4,
        "the boundary rejects each malformation class"
    );
    assert!(
        inline > compact,
        "on-demand schemas cost less prompt than inlining them"
    );

    let report = json!({
        "lane": "p43-workflow-ai-authoring",
        "catalogue": {
            "version": core_domain::node_registry::CATALOGUE_VERSION,
            "digest": core_domain::node_registry::catalogue_digest(),
            "node_count": nodes,
        },
        "prompt_footprint": {
            "compact_chars": compact,
            "inline_schemas_chars": inline,
            "reduction_pct": ((inline - compact) as f64 / inline as f64 * 100.0).round(),
        },
        "realistic_corpus": realistic_measure,
        "boundary_corpus": boundary_measure,
        "note": "Deterministic (ScriptedTransport) corpus through the REAL \
                 registry validation. `realistic_corpus` = plausible model \
                 answers (all must validate). `boundary_corpus` = deliberate \
                 malformations (the boundary must reject each). This measures \
                 the boundary and the prompt footprint, not a live model's rate.",
    });
    let Some(dir) = std::env::var_os("MARULA_EVIDENCE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join("qa-p43-workflow-ai-authoring/measurement.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("evidence dir");
    }
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap())
        .expect("write measurement");
}
