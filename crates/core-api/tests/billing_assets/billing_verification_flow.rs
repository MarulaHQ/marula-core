//! End-to-end billing verification flows (Wave 39 lane 2, migration 0111)
//! driven through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the workspace read (V1) groups entries by ticket with
//! disposition badges, running totals, and the unclassified / draft /
//! non-billable counters; that adjustments (V2) require a reason, write
//! an audit row, and change what the ENGINE prices (a preview must
//! disagree with the raw minutes and agree with the adjusted ones); that
//! the invoiced lock holds on every action; that recategorization rides
//! the org taxonomy and refuses unknown slugs; that the close sign-off
//! (V8) re-verifies its own checklist claims against the live window
//! and surfaces overlapping run windows.
//!
//! Non-purpose: the approval state machine (approvals_flow.rs), the
//! invoice run pipeline (portal_v2_flow.rs), engine math (core-billing
//! unit tests).

use crate::common::{body_json, boot_with, register_org_token, send};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    boot_with(|_| {}).await
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Verification Co", slug).await
}

async fn default_board(app: &Router, bearer: &str) -> String {
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    boards
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["is_default"] == true)
        .expect("default board")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_ticket(app: &Router, bearer: &str, client_id: Option<&str>, title: &str) -> Value {
    let board_id = default_board(app, bearer).await;
    let mut payload = json!({
        "board_id": board_id,
        "title": title,
        "priority": "p3",
        "idempotency_key": Uuid::now_v7().to_string(),
    });
    if let Some(client) = client_id {
        payload["client_id"] = json!(client);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn create_entry(
    app: &Router,
    bearer: &str,
    ticket_id: &str,
    hours_ago: i64,
    minutes: i64,
    work_type: Option<&str>,
) -> String {
    let started = chrono::Utc::now() - chrono::Duration::hours(hours_ago);
    let mut payload = json!({
        "ticket_id": ticket_id,
        "started_at": started,
        "ended_at": started + chrono::Duration::minutes(minutes),
        "billable": true,
        "notes": "worked on the thing",
    });
    if let Some(wt) = work_type {
        payload["work_type"] = json!(wt);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries",
        Some(bearer),
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "entry create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn submit_and_approve(app: &Router, bearer: &str, entry_id: &str) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{entry_id}/submit"),
        Some(bearer),
        Some(json!({ "note": "for review" })),
    )
    .await;
    assert_eq!(response.status(), 200, "submit must succeed");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries/approvals/decide",
        Some(bearer),
        Some(json!({ "entry_ids": [entry_id], "decision": "approve" })),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "approve must succeed: {body}");
    assert_eq!(body["results"][0]["outcome"], json!("applied"), "{body}");
}

async fn verify(
    app: &Router,
    bearer: &str,
    contract_id: &str,
    from: &str,
    to: &str,
) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/billing/verification?contract_id={contract_id}&from={from}&to={to}"),
        Some(bearer),
        None,
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

async fn act(app: &Router, bearer: &str, entry_id: &str, payload: Value) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/billing/verification/entries/{entry_id}/action"),
        Some(bearer),
        Some(payload),
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

/// Seeds org → client → T&M contract with a 150.00/h default rate, and
/// returns (org_id, user_id, client_id, contract_id).
async fn seed_contract(app: &Router, state: &AppState, bearer: &str) -> (Uuid, Uuid, Uuid, Uuid) {
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(bearer), None).await).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Verify Client')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .unwrap();
    let today = chrono::Utc::now().date_naive();
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, currency, hourly_rate_minor, effective_from)
         VALUES ($1, $2, $3, 'C-VER-1', 'Verification T&M', 'time_and_materials', 'active', 'USD', 15000, $4)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(today - chrono::Duration::days(30))
    .execute(&state.pool)
    .await
    .unwrap();
    (org_id, user_id, client_id, contract_id)
}

#[tokio::test]
async fn workspace_groups_by_ticket_with_badges_counters_and_totals() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-rg-{}", Uuid::now_v7().simple())).await;
    let (_org, _user, client_id, contract_id) = seed_contract(&app, &state, &bearer).await;
    let client = client_id.to_string();

    // Two tickets: one with a classified approved entry + one
    // unclassified approved entry (prices at the contract default).
    let t1 = create_ticket(&app, &bearer, Some(&client), "Grouping ticket A").await;
    let t2 = create_ticket(&app, &bearer, Some(&client), "Grouping ticket B").await;
    let t1_id = t1["id"].as_str().unwrap().to_string();
    let t2_id = t2["id"].as_str().unwrap().to_string();

    let e1 = create_entry(&app, &bearer, &t1_id, 8, 60, Some("remote")).await;
    let e2 = create_entry(&app, &bearer, &t1_id, 9, 30, Some("on_site")).await;
    // No work_type: the unclassified counter must catch it.
    let e3 = create_entry(&app, &bearer, &t2_id, 10, 120, None).await;
    // Never approved: it must show, and it must not price.
    let draft_id = create_entry(&app, &bearer, &t2_id, 11, 30, Some("on_site")).await;
    submit_and_approve(&app, &bearer, &e1).await;
    submit_and_approve(&app, &bearer, &e2).await;
    submit_and_approve(&app, &bearer, &e3).await;

    let today = chrono::Utc::now().date_naive();
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let (status, ws) = verify(&app, &bearer, &contract_id.to_string(), &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    assert_eq!(ws["total_minor"], ws["preview_total_minor"], "foots: {ws}");
    assert_eq!(
        ws["counters"]["draft"],
        json!(1),
        "the draft is counted: {ws}"
    );
    let draft_row = ws["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["entries"].as_array().unwrap())
        .find(|e| e["id"] == json!(draft_id))
        .expect("the draft row still renders");
    assert_eq!(draft_row["approval_status"], json!("draft"), "{draft_row}");
    // ...and it is priced at zero, because nothing will bill it.
    assert_eq!(draft_row["cost_minor"], json!(0), "{draft_row}");

    // Grouped by ticket: two groups, not one flat list.
    let groups = ws["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 2, "one group per ticket: {ws}");
    let g1 = groups
        .iter()
        .find(|g| g["group_id"] == json!(t1_id))
        .expect("ticket A group");
    assert_eq!(g1["total_minutes"], json!(90), "60 + 30 minutes: {g1}");
    // 90 min at 150.00/h = 225.00 = 22_500 minor.
    assert_eq!(g1["total_cost_minor"], json!(22_500), "{g1}");
    let g2 = groups
        .iter()
        .find(|g| g["group_id"] == json!(t2_id))
        .expect("ticket B group");
    // 120 min at the DEFAULT rate (unclassified has no card row) = 300.00.
    assert_eq!(g2["total_cost_minor"], json!(30_000), "{g2}");

    // Counters: 4 entries, all billable; the unclassified one is the
    // blocker the close ritual exists to catch.
    assert_eq!(ws["counters"]["entries"], json!(4), "{ws}");
    assert_eq!(ws["counters"]["unclassified"], json!(1), "{ws}");
    assert_eq!(ws["counters"]["undispositioned"], json!(4), "{ws}");
    assert_eq!(
        ws["total_minor"],
        json!(52_500),
        "running total, draft excluded: {ws}"
    );

    // The workspace's own sum and the engine's preview agree — the
    // invariant that makes the workspace trustworthy.
    assert_eq!(
        ws["total_minor"], ws["preview_total_minor"],
        "workspace entry sum must foot to the engine total: {ws}"
    );
}

#[tokio::test]
async fn adjust_minutes_requires_reason_writes_audit_and_changes_pricing() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-adj-{}", Uuid::now_v7().simple())).await;
    let (org_id, _user, client_id, contract_id) = seed_contract(&app, &state, &bearer).await;
    let client = client_id.to_string();
    let ticket = create_ticket(&app, &bearer, Some(&client), "Adjustment ticket").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry = create_entry(&app, &bearer, &ticket_id, 8, 120, Some("remote")).await;
    submit_and_approve(&app, &bearer, &entry).await;

    // A reason is mandatory: the write is evidence, not a silent edit.
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "adjust_minutes", "minutes": 30 }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // Reducing works and lands the row.
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({
            "action": "adjust_minutes",
            "minutes": 30,
            "reason": "double-counted the switch rebuild",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["effective_minutes"], json!(30), "{body}");
    assert_eq!(body["disposition"], json!("approved"), "{body}");
    assert_eq!(body["adjustment"]["delta_minutes"], json!(-90), "{body}");
    assert_eq!(
        body["audit_action"],
        json!("billing_verification.adjust_minutes"),
        "{body}"
    );

    // Raising minutes is not a verification action.
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "adjust_minutes", "minutes": 200, "reason": "undercounted" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // The audit row exists (V9) and names the action.
    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_events
          WHERE org_id = $1 AND entity_id = $2 AND action LIKE 'billing_verification.%'",
    )
    .bind(org_id)
    .bind(entry.clone())
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert_eq!(
        audit,
        vec!["billing_verification.adjust_minutes".to_string()]
    );

    // The ENGINE prices the adjusted minutes: 30 min at 150.00/h =
    // 75.00 = 7_500 minor, where the raw 120 would have been 300.00.
    let today = chrono::Utc::now().date_naive();
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let (status, ws) = verify(&app, &bearer, &contract_id.to_string(), &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    assert_eq!(ws["total_minutes"], json!(30), "effective minutes: {ws}");
    assert_eq!(ws["total_minor"], json!(7_500), "adjusted pricing: {ws}");
    assert_eq!(ws["counters"]["adjusted"], json!(1), "{ws}");

    let entry_view = &ws["groups"][0]["entries"][0];
    assert_eq!(
        entry_view["minutes"],
        json!(120),
        "raw minutes kept: {entry_view}"
    );
    assert_eq!(entry_view["effective_minutes"], json!(30), "{entry_view}");
}

#[tokio::test]
async fn flip_billable_requires_typed_reason_and_recategorize_rides_the_taxonomy() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-flip-{}", Uuid::now_v7().simple())).await;
    let (_org, _user, client_id, contract_id) = seed_contract(&app, &state, &bearer).await;
    let client = client_id.to_string();
    let ticket = create_ticket(&app, &bearer, Some(&client), "Flip ticket").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry = create_entry(&app, &bearer, &ticket_id, 8, 60, Some("remote")).await;
    submit_and_approve(&app, &bearer, &entry).await;

    // Flipping to non-billable without a typed reason is a 422 (V3).
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "flip_billable", "reason": "covered by retainer" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({
            "action": "flip_billable",
            "reason": "this work sits inside the retainer block",
            "unbillable_reason": "in_retainer_block",
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["disposition"], json!("non_billable"), "{body}");

    // Recategorizing onto a live category slug succeeds; an unknown one
    // is refused rather than stored (lane 1's rule).
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "recategorize", "work_type": "travel" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "recategorize", "work_type": "nonsense_bucket" }),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // add_note annotates without dispositioning.
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "add_note", "note": "client confirmed the block covers March" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    // An unknown action is a typed validation, not a silent no-op.
    let (status, body) = act(&app, &bearer, &entry, json!({ "action": "obliterate" })).await;
    assert_eq!(status, 422, "{body}");

    let today = chrono::Utc::now().date_naive();
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let (status, ws) = verify(&app, &bearer, &contract_id.to_string(), &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    let view = &ws["groups"][0]["entries"][0];
    assert_eq!(view["work_type"], json!("travel"), "{view}");
    assert_eq!(
        view["unbillable_reason"],
        json!("in_retainer_block"),
        "{view}"
    );
    assert_eq!(
        view["cost_minor"],
        json!(0),
        "non-billable prices at zero: {view}"
    );
    assert_eq!(ws["counters"]["non_billable"], json!(1), "{ws}");
}

#[tokio::test]
async fn invoiced_entries_are_locked_against_every_verification_action() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-lock-{}", Uuid::now_v7().simple())).await;
    let (org_id, user_id, client_id, contract_id) = seed_contract(&app, &state, &bearer).await;
    let client = client_id.to_string();
    let ticket = create_ticket(&app, &bearer, Some(&client), "Lock ticket").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry = create_entry(&app, &bearer, &ticket_id, 8, 60, Some("remote")).await;
    submit_and_approve(&app, &bearer, &entry).await;

    // Consume the entry with an issued invoice → the lock.
    let invoice_id = Uuid::now_v7();
    let today = chrono::Utc::now().date_naive();
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency, issued_at, due_on, total_minor, created_by)
         VALUES ($1, $2, $3, $4, 'INV-VER-1', 'issued', 'USD', $5, $5, 0, $6)",
    )
    .bind(invoice_id)
    .bind(org_id)
    .bind(client_id)
    .bind(contract_id)
    .bind(today)
    .bind(user_id)
    .execute(&state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(invoice_id)
    .bind(entry.parse::<Uuid>().unwrap())
    .execute(&state.pool)
    .await
    .unwrap();

    for payload in [
        json!({ "action": "adjust_minutes", "minutes": 0, "reason": "too late" }),
        json!({ "action": "flip_billable", "reason": "x", "unbillable_reason": "warranty" }),
        json!({ "action": "recategorize", "work_type": "travel" }),
        json!({ "action": "add_note", "note": "sneaky" }),
    ] {
        let (status, body) = act(&app, &bearer, &entry, payload).await;
        assert_eq!(status, 409, "invoiced entries are locked: {body}");
    }

    // The workspace still SHOWS the locked entry — locked means
    // read-only, not invisible.
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let (status, ws) = verify(&app, &bearer, &contract_id.to_string(), &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    assert_eq!(ws["counters"]["invoiced"], json!(1), "{ws}");
    assert_eq!(
        ws["groups"][0]["entries"][0]["invoiced"],
        json!(true),
        "{ws}"
    );
}

#[tokio::test]
async fn signoff_reverifies_the_checklist_and_warns_on_run_overlap() {
    let (state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-sign-{}", Uuid::now_v7().simple())).await;
    let (_org, _user, client_id, contract_id) = seed_contract(&app, &state, &bearer).await;
    let client = client_id.to_string();
    let ticket = create_ticket(&app, &bearer, Some(&client), "Signoff ticket").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry = create_entry(&app, &bearer, &ticket_id, 8, 60, Some("remote")).await;
    submit_and_approve(&app, &bearer, &entry).await;

    let today = chrono::Utc::now().date_naive();
    let from = (today - chrono::Duration::days(30)).to_string();
    let to = (today + chrono::Duration::days(1)).to_string();
    let contract = contract_id.to_string();

    // Claiming "every entry dispositioned" while one is not is a 422 —
    // the sign-off re-verifies its own claims.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/billing/verification/signoff",
        Some(&bearer),
        Some(json!({
            "contract_id": contract,
            "from": from,
            "to": to,
            "entries_dispositioned": true,
            "zero_unclassified": true,
            "preview_seen": true,
        })),
    )
    .await;
    let status = response.status().as_u16();
    let body = body_json(response).await;
    assert_eq!(status, 422, "{body}");

    // A partial checklist cannot be signed off at all.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/billing/verification/signoff",
        Some(&bearer),
        Some(json!({
            "contract_id": contract,
            "from": from,
            "to": to,
            "entries_dispositioned": false,
            "zero_unclassified": true,
            "preview_seen": true,
        })),
    )
    .await;
    let status = response.status().as_u16();
    let body = body_json(response).await;
    assert_eq!(status, 422, "{body}");

    // Dispose the entry, then the sign-off lands.
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({ "action": "approve", "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    // Already approved: the queue has nothing to decide, which is a
    // typed conflict, not a silent success.
    assert_eq!(status, 409, "{body}");

    // Disposition it through the adjustment path (a reviewed entry).
    let (status, body) = act(
        &app,
        &bearer,
        &entry,
        json!({
            "action": "adjust_minutes",
            "minutes": 60,
            "reason": "reviewed, no change to the minutes",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/billing/verification/signoff",
        Some(&bearer),
        Some(json!({
            "contract_id": contract,
            "from": from,
            "to": to,
            "entries_dispositioned": true,
            "zero_unclassified": true,
            "preview_seen": true,
            "reviewer_note": "March looks right",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status_code = response.status().as_u16();
    let body = body_json(response).await;
    assert_eq!(status_code, 200, "{body}");
    assert_eq!(
        body["signoff"]["entries_dispositioned"],
        json!(true),
        "{body}"
    );
    assert_eq!(
        body["signoff"]["reviewer_note"],
        json!("March looks right"),
        "{body}"
    );

    // The recorded sign-off rides the workspace read back.
    let (status, ws) = verify(&app, &bearer, &contract, &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    assert_eq!(ws["signoff"]["zero_unclassified"], json!(true), "{ws}");

    // A run over the signed window CLAIMS the sign-off, so "was this
    // period verified?" stays one lookup (this is the seam the run
    // composer hook owns — a regression here is silent, so it is
    // asserted explicitly).
    let run = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/billing/invoice-runs",
            Some(&bearer),
            Some(json!({
                "from": from,
                "to": to,
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(run["run"]["invoice_count"], json!(1), "{run}");
    let run_id: Uuid = run["run"]["id"].as_str().unwrap().parse().unwrap();
    let (status, ws) = verify(&app, &bearer, &contract, &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    assert_eq!(
        ws["signoff"]["run_id"],
        json!(run_id.to_string()),
        "the issued run claims the sign-off: {ws}"
    );
    // The same window now overlaps that run, so the warning names it.
    let overlaps = ws["overlapping_runs"].as_array().expect("overlaps");
    assert!(
        overlaps
            .iter()
            .any(|o| o["run_id"] == json!(run_id.to_string())),
        "the claimed run appears in the overlap warning: {ws}"
    );

    // An overlapping issued run surfaces as a WARNING, not a block.
    let extra_run = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoice_runs (id, org_id, number, period_from, period_to)
         VALUES ($1, $2, 'RUN-1', $3, $4)",
    )
    .bind(extra_run)
    .bind(_org)
    .bind(today - chrono::Duration::days(10))
    .bind(today)
    .execute(&state.pool)
    .await
    .unwrap();
    let (status, ws) = verify(&app, &bearer, &contract, &from, &to).await;
    assert_eq!(status, 200, "{ws}");
    let overlaps = ws["overlapping_runs"].as_array().expect("overlaps");
    assert!(
        overlaps.iter().any(|o| o["number"] == json!("RUN-1")),
        "the hand-seeded overlap is named: {ws}"
    );
}

#[tokio::test]
async fn idempotent_action_replays_the_stored_result() {
    let (_state, app) = test_state().await;
    let bearer = register_org(&app, &format!("ver-idem-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &bearer).await;
    let ticket = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            Some(json!({
                "board_id": board,
                "title": "Idempotency ticket",
                "priority": "p3",
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    let entry = create_entry(&app, &bearer, &ticket_id, 8, 60, Some("remote")).await;
    submit_and_approve(&app, &bearer, &entry).await;

    let key = Uuid::now_v7().to_string();
    let payload = json!({
        "action": "adjust_minutes",
        "minutes": 45,
        "reason": "trimmed the travel tail",
        "idempotency_key": key,
    });
    let (first_status, first) = act(&app, &bearer, &entry, payload.clone()).await;
    assert_eq!(first_status, 200, "{first}");
    let (second_status, second) = act(&app, &bearer, &entry, payload).await;
    assert_eq!(second_status, 200, "{second}");
    assert_eq!(
        first["effective_minutes"], second["effective_minutes"],
        "a replay returns the stored result"
    );
    assert_eq!(
        first["adjustment"]["id"], second["adjustment"]["id"],
        "{second}"
    );
}
