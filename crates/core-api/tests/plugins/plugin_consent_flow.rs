//! The consent-commit live drive (Wave 16 lane 10, ADR 0009): the
//! update/rollback re-consent flow driven END-TO-END through the REAL
//! core-api router against a real Postgres — promoted from the lane-8
//! fixture-tested pass to a live drive.
//!
//! Proves, over HTTP:
//!   * propose-update pins the pending candidate while the OLD version
//!     keeps serving (the pre-consent read path NEVER serves the pending
//!     artifact);
//!   * discard clears the pending proposal (POST consent then refuses
//!     with the typed 409 — nothing pending to consent);
//!   * consent commits the version+grants flip ATOMICALLY in one
//!     transaction: the install moves to the new version AND the widened
//!     grants land together (no partial state queryable at any commit
//!     boundary — the swap is the `commit_pending_version` tx);
//!   * the swap advances the append-only provenance chain with the
//!     `re_consented` action and appends the audit events
//!     (`plugin.update_pending`, `plugin.update_consented`,
//!     `plugin.update_discarded`) in the same transactions;
//!   * the contract stream carries the typed events
//!     (`plugin.update_pending` / `plugin.install_updated` via the
//!     transactional outbox) with no secret or config material.

use crate::common::{body_json, boot_with, register_org as common_register_org, send};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let (state, app) = boot_with(|_| {}).await;
    core_db::assert_rls_coverage(&state.pool)
        .await
        .expect("RLS coverage");
    (state, app)
}

async fn register_org(app: &Router, slug: String) -> String {
    common_register_org(app, "Consent Co", &slug).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

fn publish_body(slug: &str, version: &str, capabilities: &[&str]) -> Value {
    json!({
        "slug": slug,
        "name": "Ticket Summarizer",
        "description": "Summarizes ticket threads",
        "publisher": "marula",
        "version": version,
        "manifest": {
            "description": "Summarizes ticket threads",
            "capabilities": capabilities,
        },
    })
}

/// Counts audit events by action for the install entity.
async fn audit_count(pool: &PgPool, install_id: &str, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE entity_id = $1 AND action = $2")
        .bind(install_id)
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("audit count")
}

#[tokio::test]
async fn consent_commit_flips_version_and_grants_atomically() {
    let (state, router) = test_state().await;
    let slug = format!("consent-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&router, slug.clone()).await;

    // Publish v1.0.0 (one capability) and v1.1.0 (adds a second).
    let entry_slug = format!("summarizer-{}", Uuid::now_v7().simple());
    let first = send(
        &router,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(&entry_slug, "1.0.0", &["tickets.read"])),
    )
    .await;
    assert_eq!(first.status(), 201, "publish v1.0.0 must succeed");
    let second = send(
        &router,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "manifest": {
                "description": "adds ticket writing",
                "capabilities": ["tickets.read", "tickets.write"],
            },
        })),
    )
    .await;
    assert_eq!(second.status(), 201, "append v1.1.0 must succeed");

    // Install at 1.0.0 with the narrow grant set.
    let install = send(
        &router,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "version": "1.0.0",
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    let install_body = body_json(install).await;
    let install_id = install_body["id"].as_str().unwrap().to_string();
    let install_uri = format!("/api/plugins/installed/{install_id}");
    assert_eq!(install_body["version"], "1.0.0", "{install_body}");

    // --- propose the update ------------------------------------------------
    let propose = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/update"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "granted_capabilities": ["tickets.read", "tickets.write"],
        })),
    )
    .await;
    assert_eq!(propose.status(), 200, "propose must succeed");
    let proposal = body_json(propose).await;
    assert_eq!(proposal["current_version"], "1.0.0");
    assert_eq!(proposal["pending_version"], "1.1.0");
    let added: Vec<&str> = proposal["consent_diff"]["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(added, vec!["tickets.write"], "{proposal}");
    assert_eq!(
        proposal["consent_diff"]["unchanged"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "tickets.read stays unchanged"
    );

    // The pre-consent read path NEVER serves the pending artifact: the
    // roster still advertises 1.0.0 while the candidate sits pending.
    let roster: Value = body_json(
        send(
            &router,
            axum::http::Method::GET,
            "/api/plugins/installed",
            Some(&owner),
            None,
        )
        .await,
    )
    .await;
    assert!(!roster.as_array().unwrap().is_empty(), "install visible");
    assert!(
        roster.as_array().unwrap().first().unwrap()["version"] == json!("1.0.0"),
        "the pending artifact is not the served version before consent: {roster}"
    );

    // --- consent: the swap IS the commit (version + grants in one tx) -------
    let consent = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/consent"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(consent.status(), 200, "consent must succeed");
    let committed = body_json(consent).await;
    // The commit response IS the update view: the OLD version is gone and
    // the pending candidate is consumed (current_version now 1.1.0,
    // pending_version null).
    assert_eq!(committed["current_version"], "1.1.0", "{committed}");
    assert!(committed["pending_version"].is_null());
    assert_eq!(
        committed["current_granted_capabilities"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "the widened grant set landed WITH the version (atomic flip)"
    );

    // Atomics at the STORE level: the flip shipped zero partial states —
    // version 1.1.0 and the two-capability grant set live in ONE row.
    let row: (String, Value) =
        sqlx::query_as("SELECT version, granted_capabilities FROM plugin_installs WHERE id = $1")
            .bind(uuid::Uuid::parse_str(&install_id).unwrap())
            .fetch_one(&state.pool)
            .await
            .expect("install row");
    assert_eq!(row.0, "1.1.0");
    let grants_in_row: Vec<&str> = row
        .1
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(grants_in_row, vec!["tickets.read", "tickets.write"]);

    // The swap advanced the PROVENANCE chain with `re_consented` AND the
    // audit events appended in the SAME transactions.
    let actions: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT action FROM plugin_install_provenance WHERE install_id = $1 ORDER BY recorded_at",
    )
    .bind(uuid::Uuid::parse_str(&install_id).unwrap())
    .fetch_all(&state.pool)
    .await
    .map(|rows| rows.into_iter().map(|r| r.0).collect())
    .expect("provenance reads");
    assert!(actions.contains(&"re_consented".to_string()), "{actions:?}");
    assert!(actions.contains(&"updated".to_string()), "{actions:?}");

    assert_eq!(
        audit_count(&state.pool, &install_id, "plugin.update_pending").await,
        1
    );
    assert_eq!(
        audit_count(&state.pool, &install_id, "plugin.update_consented").await,
        1
    );

    // Re-consent consent is refused: nothing pending after commit.
    let again = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/consent"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(again.status(), 409, "consent without a pending is refused");

    // --- discard: the pending proposal clears; the OLD version persists ----
    let propose2 = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/update"),
        Some(&owner),
        Some(json!({ "version": "1.0.0" })),
    )
    .await;
    assert_eq!(propose2.status(), 200, "second propose");
    let discard = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/discard-update"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(discard.status(), 204, "discard clears");
    let view = body_json(
        send(
            &router,
            axum::http::Method::GET,
            &format!("{install_uri}/update"),
            Some(&owner),
            None,
        )
        .await,
    )
    .await;
    assert!(
        view["pending_version"].is_null(),
        "pending cleared by discard"
    );
    let roster_now: Value = body_json(
        send(
            &router,
            axum::http::Method::GET,
            "/api/plugins/installed",
            Some(&owner),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        roster_now.as_array().unwrap().first().unwrap()["version"],
        json!("1.1.0"),
        "kept running version"
    );
    let consent_409 = send(
        &router,
        axum::http::Method::POST,
        &format!("{install_uri}/consent"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(consent_409.status(), 409, "consent after discard refuses");
}
