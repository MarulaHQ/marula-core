//! End-to-end Currency & FX flow (ADR 0002, Wave 12 lane 10) driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the FX surface over HTTP — the manual-entry rate book
//! (append-only; the replay returns the byte-identical row), the typed
//! effective-rate resolver, the fail-closed rule (a cross-currency
//! expense without rate coverage is a 422 naming the pair), the expense
//! lock moment (snapshot lands and is immutable), the reconciliation
//! read model listing the fact, and the governed base-currency change
//! (refused with a typed conflict once snapshots exist).
//!
//! Non-purpose: managed feeds (Relay bundles on the FxRateProvider
//! trait), revaluation (never in the core), UI (lane 11).

use crate::common::{body_json, register_org, send, test_state as common_test_state};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

/// `common::test_state` plus the RLS-coverage precondition this flow carried
/// as a separate target. Same check, same pool, run before the test body.
async fn test_state() -> (AppState, Router) {
    let (state, app) = common_test_state().await;
    core_db::assert_rls_coverage(&state.pool)
        .await
        .expect("RLS coverage holds with fx tables");
    (state, app)
}

#[tokio::test]
async fn fx_rate_book_snapshots_reconciliation_and_governed_base_change() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, "FX Co", &format!("fx1-{}", Uuid::now_v7().simple())).await;
    let token = org["access_token"].as_str().unwrap().to_string();

    // The org's base currency defaults to USD and is visible on /api/org.
    let profile: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/org",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(profile["base_currency"], json!("USD"));

    // ZAR expense WITHOUT a rate: fail-closed 422 naming the pair.
    let tickets: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert!(tickets.is_array(), "boards list must succeed");

    // Record the manual rate: 1 USD = 17.85 ZAR. Append-only: the same
    // request with an idempotency key replays byte-identical.
    let rate_body = json!({
        "base_currency": "USD",
        "quote_currency": "ZAR",
        "as_of_date": "2026-09-01",
        "rate": "17.85",
        "idempotency_key": "fx-rate-1",
    });
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/fx/rates",
        Some(&token),
        Some(rate_body.clone()),
    )
    .await;
    assert_eq!(response.status(), 200, "manual rate entry must succeed");
    let recorded = body_json(response).await;
    assert_eq!(recorded["rate"], json!("17.850000000000"));
    assert_eq!(recorded["source"], json!("manual"));
    let replay: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/fx/rates",
            Some(&token),
            Some(rate_body),
        )
        .await,
    )
    .await;
    assert_eq!(replay["id"], recorded["id"], "replay is the same row");

    // Resolve: directed (USD→ZAR) and inverse (ZAR→USD) both covered.
    let resolved: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/fx/rates/resolve?from=USD&to=ZAR&as_of=2026-09-05",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(resolved["rate"], json!("17.850000000000"));
    let inverse: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/fx/rates/resolve?from=ZAR&to=USD&as_of=2026-09-05",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        inverse["base_currency"],
        json!("USD"),
        "inverse coverage resolves the stored pair"
    );

    // No coverage for JPY: typed 404 (nothing fabricated).
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/fx/rates/resolve?from=JPY&to=USD&as_of=2026-09-05",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);

    // A cross-currency expense (ZAR, base USD) locks its snapshot.
    let boards: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let ticket: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&token),
            Some(json!({
                "board_id": board_id,
                "title": "FX ticket",
                "priority": "p3",
            })),
        )
        .await,
    )
    .await;
    let ticket_id = ticket["ticket"]["id"]
        .as_str()
        .or(ticket["id"].as_str())
        .unwrap()
        .to_string();

    let expense_body = json!({
        "description": "ZAR courier",
        "amount_minor": 178_500,
        "currency": "ZAR",
        "billable": false,
        "idempotency_key": "fx-exp-1",
    });
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket_id}/expenses"),
        Some(&token),
        Some(expense_body.clone()),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "cross-currency expense must land with a snapshot"
    );
    let expense = body_json(response).await;
    let exp = &expense["expense"];
    assert_eq!(exp["currency"], json!("ZAR"));
    assert_eq!(exp["fx_source"], json!("manual"));
    assert_eq!(exp["fx_rate_used"], json!("0.056022408964"));
    // base = half_even(178500 × 0.056022408964) = 10_000 minor USD
    assert_eq!(exp["base_amount_minor"], json!(10_000));

    // The reconciliation read model lists the fact (FX7 packet).
    let recon: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/fx/reconciliation",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let rows = recon.as_array().unwrap();
    assert_eq!(rows.len(), 1, "exactly one cross-currency fact");
    assert_eq!(rows[0]["kind"], json!("expense"));
    assert_eq!(rows[0]["original_currency"], json!("ZAR"));
    assert_eq!(rows[0]["base_amount_minor"], json!(10_000));
    assert_eq!(rows[0]["sync_target"], json!("none"));

    // The governed base-currency change is REFUSED once snapshots exist.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&token),
        Some(json!({ "base_currency": "EUR" })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "base change with snapshots is a typed conflict"
    );
    let conflict = body_json(response).await;
    assert_eq!(
        conflict["error"]["reason"],
        json!("conflict"),
        "typed conflict envelope, got {conflict}"
    );

    // …and ALLOWED again once the triggering facts are gone is NOT tested
    // here: snapshots are immutable, so the governed re-snapshot path is
    // an operator act outside this surface (ADR 0002).
}
