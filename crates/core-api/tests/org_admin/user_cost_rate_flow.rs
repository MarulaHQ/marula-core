//! Labour cost rates — the write surface the client-margin report needed
//! (migration 0188).
//!
//! The report could always cost labour; nothing could write a rate, so every
//! org's margin said "cost unknown". This suite drives the two routes through
//! the REAL router and then reads the money back out of the SAME read model
//! the report compiles (`ops.v1_client_margin`), so the assertion is that a
//! form submission changes what the report costs — not that a column changed.
//!
//! Proved here:
//!   1. set → read back → the cost leg costs the user's rate;
//!   2. clear (`null`) → the cost leg falls back to the org default;
//!   3. `null` and `0` are different answers (unset vs free);
//!   4. `PATCH /api/org`'s default is read-your-write (the profile is cached);
//!   5. negative and out-of-range rates are 422, and absent is 422 on the
//!      one-field route;
//!   6. own-rate read is allowed, another user's needs `financial:read`;
//!   7. another org's user is 404 for read and write.

use crate::common::{body_json, register_org_token, send, test_state};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

/// Seed one client with one 60-minute entry for `user`, and return the client
/// id. Written through the admin pool (table owner): the point under test is
/// the HTTP write surface, not how a ticket came to exist.
async fn seed_hour_of_work(state: &core_api::state::AppState, org: Uuid, user: Uuid) -> Uuid {
    let client = Uuid::now_v7();
    let board = Uuid::now_v7();
    let status = Uuid::now_v7();
    let ticket = Uuid::now_v7();
    let entry = Uuid::now_v7();
    let started = chrono::Utc::now() - chrono::Duration::hours(2);

    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org_and_user(&mut tx, org, user)
        .await
        .expect("scope");
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Margin Client')")
        .bind(client)
        .bind(org)
        .execute(&mut *tx)
        .await
        .expect("client");
    sqlx::query("INSERT INTO boards (id, org_id, name, is_default) VALUES ($1, $2, 'Board', true)")
        .bind(board)
        .bind(org)
        .execute(&mut *tx)
        .await
        .expect("board");
    sqlx::query(
        "INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
         VALUES ($1, $2, $3, 'Open', 'open', 0, true)",
    )
    .bind(status)
    .bind(board)
    .bind(org)
    .execute(&mut *tx)
    .await
    .expect("status");
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, category, client_id)
         VALUES ($1, $2, $3, $4, $5, 'Margin ticket', 'open', $6)",
    )
    .bind(ticket)
    .bind(org)
    .bind(format!("M-{}", &ticket.simple().to_string()[..8]))
    .bind(board)
    .bind(status)
    .bind(client)
    .execute(&mut *tx)
    .await
    .expect("ticket");
    sqlx::query(
        "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable)
         VALUES ($1, $2, $3, $4, $5, $6, 60, true)",
    )
    .bind(entry)
    .bind(org)
    .bind(ticket)
    .bind(user)
    .bind(started)
    .bind(started + chrono::Duration::hours(1))
    .execute(&mut *tx)
    .await
    .expect("time entry");
    tx.commit().await.expect("commit");
    client
}

/// What the client-margin read model says about one client: (cost, costed
/// minutes, uncosted minutes). This is the report's cost leg.
async fn margin(
    state: &core_api::state::AppState,
    org: Uuid,
    user: Uuid,
    client: Uuid,
) -> (i64, i64, i64) {
    use sqlx::Row;
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org_and_user(&mut tx, org, user)
        .await
        .expect("scope");
    let row = sqlx::query(
        "SELECT COALESCE(SUM(cost_minor), 0)::bigint AS cost,
                COALESCE(SUM(costed_minutes), 0)::bigint AS costed,
                COALESCE(SUM(uncosted_minutes), 0)::bigint AS uncosted
           FROM ops.v1_client_margin
          WHERE org_id = $1 AND client_id = $2",
    )
    .bind(org)
    .bind(client)
    .fetch_one(&mut *tx)
    .await
    .expect("margin rows");
    tx.commit().await.expect("commit");
    (row.get("cost"), row.get("costed"), row.get("uncosted"))
}

/// The registered owner's token and user id for a fresh org.
async fn register_owner(app: &Router, slug: &str) -> (String, Uuid) {
    let token = register_org_token(app, "Cost Rate Co", slug).await;
    let directory = send(
        app,
        axum::http::Method::GET,
        "/api/org/users",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(directory.status(), 200);
    let users = body_json(directory).await;
    let id: Uuid = users["users"][0]["id"]
        .as_str()
        .expect("owner id")
        .parse()
        .expect("uuid");
    (token, id)
}

async fn patch_rate(
    app: &Router,
    token: &str,
    user: Uuid,
    body: Value,
) -> axum::response::Response {
    send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/users/{user}/cost-rate"),
        Some(token),
        Some(body),
    )
    .await
}

async fn get_rate(app: &Router, token: &str, user: Uuid) -> axum::response::Response {
    send(
        app,
        axum::http::Method::GET,
        &format!("/api/users/{user}/cost-rate"),
        Some(token),
        None,
    )
    .await
}

#[tokio::test]
async fn a_rate_can_be_set_read_cleared_and_the_report_follows() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("cost-rate-{}", Uuid::now_v7().simple());
    let (owner, owner_id) = register_owner(&app, &slug).await;
    let org: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await
        .expect("org id");

    let client = seed_hour_of_work(&state, org, owner_id).await;

    // --- unset before anything is written ----------------------------------
    let response = get_rate(&app, &owner, owner_id).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await,
        json!({ "cost_rate_minor": Value::Null }),
        "a fresh org has no rate, and null is the shape that says so"
    );
    let (cost, costed, uncosted) = margin(&state, org, owner_id, client).await;
    assert_eq!((cost, costed, uncosted), (0, 0, 60), "uncosted, not free");

    // --- set: 6000 minor units per hour ------------------------------------
    let response = patch_rate(&app, &owner, owner_id, json!({ "cost_rate_minor": 6000 })).await;
    assert_eq!(response.status(), 200, "set must succeed");
    assert_eq!(body_json(response).await["cost_rate_minor"], json!(6000));
    let response = get_rate(&app, &owner, owner_id).await;
    assert_eq!(body_json(response).await["cost_rate_minor"], json!(6000));
    // One hour at 6000/hour costs 6000 — the same prorating the invoice uses.
    let (cost, costed, uncosted) = margin(&state, org, owner_id, client).await;
    assert_eq!((cost, costed, uncosted), (6000, 60, 0), "the user's rate");

    // --- the org default, and read-your-write on the cached profile --------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&owner),
        Some(json!({ "default_cost_rate_minor": 3000 })),
    )
    .await;
    assert_eq!(response.status(), 200, "org default must be settable");
    assert_eq!(
        body_json(response).await["default_cost_rate_minor"],
        json!(3000)
    );
    // The profile is cached under the org-settings namespace; a GET right
    // after the PATCH must not serve the pre-write default.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/org",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["default_cost_rate_minor"],
        json!(3000),
        "the cached profile was invalidated by the write"
    );

    // Absent leaves it alone — the other half of the tri-state, on the route
    // that patches several fields at once.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&owner),
        Some(json!({ "name": "Cost Rate Co" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["default_cost_rate_minor"],
        json!(3000),
        "a PATCH that does not mention the rate must not clear it"
    );

    // The user's own rate still wins over the default.
    let (cost, _, _) = margin(&state, org, owner_id, client).await;
    assert_eq!(cost, 6000);

    // --- clear: the cost leg falls back to the org default ------------------
    let response = patch_rate(&app, &owner, owner_id, json!({ "cost_rate_minor": null })).await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["cost_rate_minor"], Value::Null);
    let (cost, costed, uncosted) = margin(&state, org, owner_id, client).await;
    assert_eq!(
        (cost, costed, uncosted),
        (3000, 60, 0),
        "cleared means fall back to the org default, not zero"
    );

    // --- zero is a rate, not "unset" ---------------------------------------
    let response = patch_rate(&app, &owner, owner_id, json!({ "cost_rate_minor": 0 })).await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["cost_rate_minor"], json!(0));
    let (cost, costed, uncosted) = margin(&state, org, owner_id, client).await;
    assert_eq!(
        (cost, costed, uncosted),
        (0, 60, 0),
        "zero is costed at zero, and it is NOT the uncosted state"
    );

    // Clear the default too: both unset means uncosted again.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&owner),
        Some(json!({ "default_cost_rate_minor": null })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["default_cost_rate_minor"],
        Value::Null
    );
    let response = patch_rate(&app, &owner, owner_id, json!({ "cost_rate_minor": null })).await;
    assert_eq!(response.status(), 200);
    let (_, costed, uncosted) = margin(&state, org, owner_id, client).await;
    assert_eq!((costed, uncosted), (0, 60), "unknown cost is never zero");
}

#[tokio::test]
async fn invalid_rates_are_refused_before_the_column_throws() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("cost-rate-bad-{}", Uuid::now_v7().simple());
    let (owner, owner_id) = register_owner(&app, &slug).await;

    for negative in [-1_i64, -60_000] {
        let response = patch_rate(
            &app,
            &owner,
            owner_id,
            json!({ "cost_rate_minor": negative }),
        )
        .await;
        assert_eq!(response.status(), 422, "{negative} must be a typed refusal");
    }
    // The column is `integer`; a bigger number is refused by validation, not
    // by a database error surfacing as 500.
    let response = patch_rate(
        &app,
        &owner,
        owner_id,
        json!({ "cost_rate_minor": i64::from(i32::MAX) + 1 }),
    )
    .await;
    assert_eq!(response.status(), 422, "out of range is a 422");

    // A float is not minor units. JSON has one number type, but a fractional
    // value is not an integer and must not be silently truncated.
    let response = patch_rate(&app, &owner, owner_id, json!({ "cost_rate_minor": 12.5 })).await;
    assert_eq!(response.status(), 422, "minor units are integers");

    // The one-field route refuses an empty body rather than reading it as
    // "no change" — absence is a client bug here.
    let response = patch_rate(&app, &owner, owner_id, json!({})).await;
    assert_eq!(response.status(), 422, "absent field is refused");

    // Same rules on the org default.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        "/api/org",
        Some(&owner),
        Some(json!({ "default_cost_rate_minor": -5 })),
    )
    .await;
    assert_eq!(response.status(), 422, "the org default cannot be negative");

    // Nothing above landed.
    let response = get_rate(&app, &owner, owner_id).await;
    assert_eq!(body_json(response).await["cost_rate_minor"], Value::Null);
}

#[tokio::test]
async fn another_orgs_user_is_not_reachable() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug_a = format!("cost-rate-a-{}", Uuid::now_v7().simple());
    let slug_b = format!("cost-rate-b-{}", Uuid::now_v7().simple());
    let (owner_a, _) = register_owner(&app, &slug_a).await;
    let (_owner_b, user_b) = register_owner(&app, &slug_b).await;

    let response = get_rate(&app, &owner_a, user_b).await;
    assert_eq!(response.status(), 404, "a foreign user id is not a read");
    let response = patch_rate(&app, &owner_a, user_b, json!({ "cost_rate_minor": 1 })).await;
    assert_eq!(response.status(), 404, "nor a write");
}

#[tokio::test]
async fn own_rate_is_readable_and_another_users_needs_the_money_read() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("cost-rate-perm-{}", Uuid::now_v7().simple());
    let (owner, owner_id) = register_owner(&app, &slug).await;

    // A technician: no financial read, no user administration.
    let invite = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(&owner),
        Some(json!({
            "email": format!("tech@{slug}.test"),
            "display_name": "Tech",
            "role": "technician",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200);
    let invited = body_json(invite).await;
    let tech_id: Uuid = invited["id"].as_str().expect("tech id").parse().unwrap();
    let setup_token = invited["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200);
    let tech = body_json(accept).await["access_token"]
        .as_str()
        .expect("tech token")
        .to_string();

    // Own rate: yes, even without the money grant.
    let response = get_rate(&app, &tech, tech_id).await;
    assert_eq!(response.status(), 200, "a member reads their own rate");
    assert_eq!(body_json(response).await["cost_rate_minor"], Value::Null);

    // Somebody else's rate: no.
    let response = get_rate(&app, &tech, owner_id).await;
    assert_eq!(
        response.status(),
        403,
        "another user's rate is a financial read"
    );

    // And a technician cannot write any rate, including their own.
    let response = patch_rate(&app, &tech, tech_id, json!({ "cost_rate_minor": 1 })).await;
    assert_eq!(response.status(), 403, "writing is user administration");
}
