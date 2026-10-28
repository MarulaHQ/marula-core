//! The CI merge workbench + Jev dedupe workflow flows (Wave 22 lane 2)
//! through the REAL core-api router.
//!
//! Purpose:
//! 1. The workbench read model: side-by-side cards across the queues'
//!    full tier state, the field-by-field picker rows, the threshold
//!    context, and the decision slot HONESTLY ABSENT when the org has no
//!    decision model (ADR 0010 degrade-quiet — never a 5xx, never a dead
//!    card).
//! 2. Defer/restore/accept/reject + batch: park without deciding,
//!    un-defer, batch accept/reject/defer N with per-candidate audit and
//!    no silent partials.
//! 3. The Jev dedupe workflow: the typed enable gate
//!    (`dedupe_decision_not_configured`), manual runs that record
//!    suggestion-grade asks and stamp the report, and report-only mode
//!    that NEVER merges.

use crate::common::{
    body_json, register_org_token, send_headers as send, test_state as boot_state,
};
use axum::Router;
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> AppState {
    boot_state().await.0
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Workbench Co", slug).await
}

struct Ctx {
    app: Router,
    bearer: String,
    client_id: String,
}

fn hdr(ctx: &Ctx) -> [(&'static str, String); 1] {
    [("authorization", ctx.bearer.clone())]
}

async fn setup() -> Ctx {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("ciwb-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &[("authorization", format!("Bearer {token}"))],
            Some(json!({ "name": "Workbench Systems" })),
        )
        .await,
    )
    .await;
    Ctx {
        app,
        bearer: format!("Bearer {token}"),
        client_id: client["id"].as_str().unwrap().to_string(),
    }
}

/// Feeds two devices whose partial-MAC overlap scores 0.6 (below the
/// 0.85 threshold ⇒ a fresh item + ONE pending candidate), like the CI
/// flow's ambiguous case.
async fn queue_candidate(ctx: &Ctx, tag: &str) -> String {
    let first = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &hdr(ctx),
            Some(json!({
                "source_system": "ninjaone", "external_id": format!("X-{tag}-1"),
                "client_id": ctx.client_id, "tag": format!("DEV-{tag}"), "name": "Device A",
                "identity_keys": { "macs": ["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:02"] },
                "observed_fields": {},
                "idempotency_key": format!("feed-{tag}-1"),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(first["created_item"], true);
    let second = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/config-items/feed-upsert",
            &hdr(ctx),
            Some(json!({
                "source_system": "auvix", "external_id": format!("Y-{tag}-2"),
                "client_id": ctx.client_id, "tag": format!("DEV-{tag}"), "name": "Device B",
                "identity_keys": { "macs": ["aa:bb:cc:dd:ee:02"] },
                "observed_fields": {},
                "idempotency_key": format!("feed-{tag}-2"),
            })),
        )
        .await,
    )
    .await;
    let queued = second["queued_candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        queued.len(),
        1,
        "partial MAC overlap queues a candidate: {second}"
    );
    queued[0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ci_workbench_shows_side_by_side_and_batch_laws_hold() {
    let ctx = setup().await;
    let candidate_id = queue_candidate(&ctx, "W").await;
    let bearer = hdr(&ctx);

    // The workbench read: side-by-side cards, picker rows, threshold
    // context, and the HONESTLY-ABSENT decision slot.
    let wb = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            &format!("/api/v1/config-items/merge-candidates/{candidate_id}/workbench"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(wb["candidate"]["id"].as_str().unwrap(), candidate_id);
    assert_eq!(wb["primary"]["tag"], "DEV-W");
    assert_eq!(wb["candidate_side"]["tag"], "DEV-W");
    assert!(!wb["primary"]["source_links"].as_array().unwrap().is_empty());
    assert!(
        wb["decision"].is_null(),
        "unconfigured decision model renders NOTHING (never a dead card)"
    );
    assert!(wb["auto_merge_threshold"].as_f64().unwrap() > 0.0);
    assert!(
        wb["related_pending"].is_array(),
        "related-pending shape present"
    );

    // Defer parks the candidate; restore brings it back; a decided
    // candidate refuses re-decision (typed conflict).
    let deferred = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/config-items/merge-candidates/{candidate_id}/defer"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(deferred["status"], "deferred");
    let restored = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/config-items/merge-candidates/{candidate_id}/restore"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(restored["status"], "pending");

    // Batch ACCEPT: the audited fold (candidate flips to accepted; each
    // merge rides its own split substrate).
    let batch = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/config-items/merge-candidates/batch",
            &bearer,
            Some(json!({ "ids": [candidate_id], "action": "accept" })),
        )
        .await,
    )
    .await;
    assert_eq!(batch["accepted"], 1);
    assert_eq!(batch["failed"], 0);
    let merge = &batch["outcomes"][0]["merge"];
    assert_eq!(merge["status"], "applied");
    let merge_id = merge["id"].as_str().unwrap().to_string();

    // Undo: the split endpoint restores the absorbed item.
    let split = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            &format!("/api/v1/config-items/merges/{merge_id}/split"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(split["status"], "undone");

    // A decided candidate is skipped by a later batch, never re-decided.
    let batch2 = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/config-items/merge-candidates/batch",
            &bearer,
            Some(json!({ "ids": [candidate_id], "action": "reject" })),
        )
        .await,
    )
    .await;
    assert_eq!(batch2["skipped"], 1, "decided rows never flip silently");

    // An unknown action and an empty id list are typed 422s.
    let bad = send(
        &ctx.app,
        axum::http::Method::POST,
        "/api/v1/config-items/merge-candidates/batch",
        &bearer,
        Some(json!({ "ids": [candidate_id], "action": "nuke" })),
    )
    .await;
    assert_eq!(bad.status(), 422);
    let empty = send(
        &ctx.app,
        axum::http::Method::POST,
        "/api/v1/config-items/merge-candidates/batch",
        &bearer,
        Some(json!({ "ids": [], "action": "defer" })),
    )
    .await;
    assert_eq!(empty.status(), 422);
}

#[tokio::test]
async fn jev_dedupe_workflow_gate_report_and_report_only_never_merges() {
    let ctx = setup().await;
    let bearer = hdr(&ctx);
    let candidate_id = queue_candidate(&ctx, "J").await;

    // THE GATE: this org has no decision model configured — enabling is
    // impossible quietly (typed 409, ADR 0011 §3).
    let refused = send(
        &ctx.app,
        axum::http::Method::PUT,
        "/api/v1/dedupe/workflow",
        &bearer,
        Some(json!({
            "enabled": true,
            "confidence_threshold": 0.8,
            "mode": "report_only",
            "interval_hours": null,
            "notify_user_ids": [],
            "entity_kinds": ["config_items"],
        })),
    )
    .await;
    assert_eq!(
        refused.status(),
        409,
        "unconfigured model refuses the enable, typed"
    );

    // A DISABLED workflow CAN be stored (the operator drafts the config;
    // the enable gate fires only when enabled=true).
    let stored = body_json(
        send(
            &ctx.app,
            axum::http::Method::PUT,
            "/api/v1/dedupe/workflow",
            &bearer,
            Some(json!({
                "enabled": false,
                "confidence_threshold": 0.8,
                "mode": "report_only",
                "interval_hours": null,
                "notify_user_ids": [],
                "entity_kinds": ["config_items"],
            })),
        )
        .await,
    )
    .await;
    assert_eq!(stored["workflow"]["enabled"], false);
    assert_eq!(stored["decision_configured"], false);

    // The manual run: asks fire (suggestion-grade), the decision slot on
    // the workbench STAYS quiet (no model), and NOTHING merges — the
    // candidate remains pending for human review.
    let run = body_json(
        send(
            &ctx.app,
            axum::http::Method::POST,
            "/api/v1/dedupe/workflow/run",
            &bearer,
            Some(json!({ "dry_run": false })),
        )
        .await,
    )
    .await;
    let report = &run["report"];
    assert_eq!(report["queues_scanned"].as_array().unwrap().len(), 1);
    assert_eq!(report["scanned"], 1, "run body: {run}");
    assert_eq!(report["asked"], 1);
    assert_eq!(
        report["auto_merged"], 0,
        "report-only mode NEVER merges — even an above-threshold answer would stay queued"
    );
    let outcome = &report["outcomes"][0];
    assert_eq!(outcome["queue"], "config_items");
    assert_eq!(outcome["disposition"], "quiet");

    // The run report stamped the workflow row.
    let view = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            "/api/v1/dedupe/workflow",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert!(
        view["workflow"]["last_result"].is_object(),
        "run report stamped: {view}"
    );
    assert!(view["workflow"]["last_run_at"].is_string());

    // The candidate is STILL pending after the pass (suggestion-grade).
    let pendings = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            "/api/v1/config-items/merge-candidates?status=pending",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(pendings.as_array().cloned().unwrap_or_default().len(), 1);

    // The workbench render stays silent about the ask (quiet run).
    let wb = body_json(
        send(
            &ctx.app,
            axum::http::Method::GET,
            &format!("/api/v1/config-items/merge-candidates/{candidate_id}/workbench"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert!(
        wb.get("decision").is_none_or(|d| d.is_null()),
        "unconfigured model: no decision evidence, honestly absent"
    );
    let _ = candidate_id;
}
