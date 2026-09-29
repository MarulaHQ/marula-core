//! End-to-end operational reports flows (TASKLIST R1) through the REAL
//! core-api router against a real Postgres.
//!
//! Purpose: prove the reports surface over HTTP — the eight kinds compile
//! and execute against the `ops.v1_*` read-model views, RBAC scoping
//! (manager = org-wide, everyone else structurally narrowed to own rows),
//! cursor pagination walk, CSV shape stability incl. quoting, summary
//! blocks, first-response edge semantics, and validation rejections.
//!
//! Non-purpose: SQL-level RLS probes and view fixture math (core-db
//! db_suite), compiler internals + proptests (core-reports unit tests).

use axum::Router;
use chrono::Utc;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

static RESET: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("reports e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    // db_suite-parity: one reset per process so reruns stay deterministic.
    RESET
        .get_or_init(|| async {
            let admin = connect_admin(&admin_url).await.expect("admin pool (reset)");
            sqlx::query("DROP SCHEMA public CASCADE")
                .execute(&admin)
                .await
                .ok();
            sqlx::query("CREATE SCHEMA public")
                .execute(&admin)
                .await
                .ok();
            core_db::run_migrations(&admin)
                .await
                .expect("migrations after reset");
        })
        .await;
    // The ROUTER keeps the established admin-pool harness convention of
    // sibling flow tests (auth/registration surfaces rely on it). Report
    // execution enforces real RLS separately: routes/reports derives an
    // app-role pool itself and binds app.org_id per request.
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let admin_url2 = admin_url.clone();
    config.database_url = admin_url;
    let reporting_url = admin_url2;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(30))
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
        .method(method.clone())
        .uri(uri);
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({e}); body={:?}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

/// Registers a fresh org; returns bearer tokens + the seeded board id.
async fn seed_org(app: &Router, slug: String) -> (String, Uuid, String) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Reports Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&bearer), None).await).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    (
        bearer,
        org_id,
        boards[0]["id"].as_str().unwrap().to_string(),
    )
}

/// Creates a second user through core_db with the builtin technician role,
/// then logs in over HTTP so scoping is exercised against REAL principals.
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let tech_role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Tech User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, tech_role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": "tech-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login must succeed");
    let tokens = body_json(response).await;
    tokens["access_token"].as_str().unwrap().to_string()
}

fn slug(suffix: &str) -> String {
    format!("rpt-{suffix}-{}", Uuid::now_v7().simple())
}

// --- shared fixture helpers ---------------------------------------------------

/// Creates one client + one ticket through the API; returns the ticket json.
async fn make_ticket(
    app: &Router,
    bearer: &str,
    board_id: &str,
    title: &str,
    assignee_id: Option<Uuid>,
    client_id: Option<Uuid>,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({
            "board_id": board_id,
            "title": title,
            "priority": "p3",
            "assignee_id": assignee_id,
            "client_id": client_id,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn make_client(app: &Router, bearer: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/clients",
        Some(bearer),
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");
    let v = body_json(response).await;
    v["id"].as_str().unwrap().parse().unwrap()
}

async fn post_entry(
    app: &Router,
    bearer: &str,
    ticket_id: Uuid,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    billable: bool,
) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries",
        Some(bearer),
        Some(json!({
            "ticket_id": ticket_id,
            "started_at": start.to_rfc3339(),
            "ended_at": end.to_rfc3339(),
            "billable": billable,
            // Wave 39 V3: a non-billable entry carries a typed reason.
            "unbillable_reason": if billable { Value::Null } else { json!("internal") },
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "time entry create must succeed");
}

async fn get_report(app: &Router, bearer: &str, qs: &str) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/reports/{qs}"),
        Some(bearer),
        None,
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

fn epoch(days: i64, hours: u32) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(2026, 6, 10, 0, 0, 0).unwrap()
        + chrono::Duration::days(days)
        + chrono::Duration::hours(i64::from(hours))
}

// --- RBAC scoping over real principals -----------------------------------------

#[tokio::test]
async fn manager_sees_org_wide_time_report_technician_is_scoped_to_self() {
    let (state, app) = test_state().await;
    let s = slug("rbac");
    let (owner_bearer, owner_org, board_id) = seed_org(&app, s.clone()).await;

    // Owner id from /api/me (for a distinct second group of entries).
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&owner_bearer),
            None,
        )
        .await,
    )
    .await;
    let owner_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();

    let tech_email = format!("tech@{s}.test");
    let tech_bearer = provision_technician(&state, &app, owner_org, &tech_email).await;
    let tech_me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&tech_bearer),
            None,
        )
        .await,
    )
    .await;
    let tech_id: Uuid = tech_me["user_id"].as_str().unwrap().parse().unwrap();

    let t_owner = make_ticket(
        &app,
        &owner_bearer,
        &board_id,
        "Owner work",
        Some(owner_id),
        None,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_tech = make_ticket(
        &app,
        &owner_bearer,
        &board_id,
        "Tech work",
        Some(tech_id),
        None,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    post_entry(
        &app,
        &owner_bearer,
        t_owner.parse().unwrap(),
        epoch(-1, 0),
        epoch(-1, 1),
        true,
    )
    .await;
    post_entry(
        &app,
        &tech_bearer,
        t_tech.parse().unwrap(),
        epoch(-1, 2),
        epoch(-1, 4),
        false, // 120m non-billable
    )
    .await;

    // Explicit window covering the seeded June entries (independent of the
    // run-time clock).
    let window = format!(
        "?from={}&to={}",
        epoch(-7, 0).date_naive(),
        epoch(1, 0).date_naive()
    );
    // Manager: BOTH rows visible with correct hand-computed math.
    let (status, body) = get_report(&app, &owner_bearer, &format!("time_by_user{window}")).await;
    assert_eq!(status, 200, "report status: {body}");
    assert_eq!(
        body["rows"].as_array().map(|r| r.len()),
        Some(2),
        "expected both groups org-wide: {body}"
    );
    assert_eq!(
        body["rows"].as_array().unwrap().len(),
        2,
        "org-wide rows: {body}"
    );
    let total = body["summary"]["total_minutes"].as_i64().unwrap();
    assert_eq!(total, 60 + 120);

    // Technician: exactly their own row even when passing ?user_id=owner.
    let qs = format!("time_by_user{window}&user_id={owner_id}");
    let (status, body) = get_report(&app, &tech_bearer, &qs).await;
    assert_eq!(status, 200);
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "self-scoped rows: {body}");
    assert_eq!(rows[0]["group_id"], json!(tech_id));
    assert_eq!(rows[0]["metric"], json!(120));
    assert_eq!(rows[0]["billable_minutes"], json!(0));

    // billable filter narrows manager view too.
    let (_s, b2) = get_report(
        &app,
        &owner_bearer,
        &format!("time_by_user{window}&billable=true"),
    )
    .await;
    assert_eq!(b2["rows"].as_array().unwrap().len(), 1);
}

// --- cursor pagination ---------------------------------------------------------

#[tokio::test]
async fn time_by_user_walks_keyset_pages_with_limit_one() {
    let (state, app) = test_state().await;
    let s = slug("cursor");
    let (owner_bearer, owner_org, board_id) = seed_org(&app, s.clone()).await;
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&owner_bearer),
            None,
        )
        .await,
    )
    .await;
    let owner_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();

    let tech_bearer = provision_technician(&state, &app, owner_org, &format!("t2@{s}.test")).await;
    let tech_me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&tech_bearer),
            None,
        )
        .await,
    )
    .await;
    let tech_id: Uuid = tech_me["user_id"].as_str().unwrap().parse().unwrap();

    for user in [owner_id, tech_id] {
        let t = make_ticket(&app, &owner_bearer, &board_id, "walk", Some(user), None).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        post_entry(
            &app,
            &owner_bearer,
            t.parse().unwrap(),
            epoch(-2, 0),
            epoch(-2, 1),
            true,
        )
        .await;
        post_entry(
            &app,
            &owner_bearer,
            t.parse().unwrap(),
            epoch(-2, 3),
            epoch(-2, 5),
            true,
        )
        .await;
    }

    // Page 1 (limit=1): the heaviest group (tie broken by group_id ASC —
    // owner uuid sorts below technician's? deterministically ordered either way).
    let window = format!(
        "from={}&to={}",
        epoch(-7, 0).date_naive(),
        epoch(1, 0).date_naive()
    );
    let (p1s, p1) = get_report(
        &app,
        &owner_bearer,
        &format!("time_by_user?{window}&limit=1"),
    )
    .await;
    assert_eq!(p1s, 200, "{p1}");
    assert_eq!(p1["rows"].as_array().unwrap().len(), 1, "{p1}");
    let first_group = p1["rows"][0]["group_id"].clone();
    let cursor1 = p1["next_cursor"]
        .as_str()
        .expect("page 1 must continue")
        .to_string();

    // Page 2: cursor decoded, different group returned, exhausted.
    let (s2, p2) = get_report(
        &app,
        &owner_bearer,
        &format!("time_by_user?{window}&limit=1&cursor={cursor1}"),
    )
    .await;
    assert_eq!(s2, 200, "cursor decode must succeed: {p2}");
    assert_ne!(
        p2["rows"][0]["group_id"], first_group,
        "page 2 advances the keyset"
    );
    assert!(p2["next_cursor"].is_null() || p2["next_cursor"] == json!(""));

    // Garbage cursors fail closed with a validation error.
    let (gs, _gb) = get_report(
        &app,
        &owner_bearer,
        &format!("time_by_user?{window}&cursor=AAAAjunk"),
    )
    .await;
    assert_eq!(gs, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

// --- SLA outcomes + breaches -----------------------------------------------------

#[tokio::test]
async fn sla_attainment_and_breaches_reflect_outcome_columns() {
    let (state, app) = test_state().await;
    let s = slug("sla");
    let (bearer, org, board_id) = seed_org(&app, s.clone()).await;

    let t_met = make_ticket(&app, &bearer, &board_id, "resolved on time", None, None).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_breached = make_ticket(&app, &bearer, &board_id, "breached open", None, None).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Default policy id + status id for a legal close move. All direct
    // fixture SQL runs inside org-scoped transactions (RLS applies through
    // the app role — same as production traffic).
    let mut tx = state.pool.begin().await.expect("fixture tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let policy_id: Uuid =
        sqlx::query_scalar("SELECT id FROM sla_policies WHERE org_id = $1 LIMIT 1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await
            .unwrap();

    // MET outcome: resolved before due.
    sqlx::query(
        "INSERT INTO ticket_sla_state
           (ticket_id, org_id, policy_id, first_response_due, first_responded_at,
            resolution_due, breached)
         VALUES ($1,$2,$3,$4,$5,$6,false)
         ON CONFLICT (ticket_id) DO UPDATE SET
           policy_id = $3, first_response_due = $4, first_responded_at = $5,
           resolution_due = $6, breached = false",
    )
    .bind(t_met.parse::<Uuid>().unwrap())
    .bind(org)
    .bind(policy_id)
    .bind(epoch(0, 4))
    .bind(epoch(0, 1))
    .bind(epoch(0, 12))
    .execute(&mut *tx)
    .await
    .unwrap();

    // Breached: no resolution yet, breach flag set; creation pinned into
    // the queried window (report anchors breaches on ticket_created_at).
    sqlx::query("UPDATE tickets SET created_at = $2 WHERE id = $1")
        .bind(t_breached.parse::<Uuid>().unwrap())
        .bind(epoch(0, 2))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO ticket_sla_state
           (ticket_id, org_id, policy_id, resolution_due, breached)
         VALUES ($1,$2,$3,$4,true)
         ON CONFLICT (ticket_id) DO UPDATE SET
           policy_id = $3, resolution_due = $4, breached = true",
    )
    .bind(t_breached.parse::<Uuid>().unwrap())
    .bind(org)
    .bind(policy_id)
    .bind(epoch(0, 12))
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Close the "met" ticket through the workflow so resolved_at exists.
    let mut tx = state.pool.begin().await.expect("status tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let resolved_status: Uuid = sqlx::query_scalar(
        "SELECT s.id FROM board_statuses s JOIN boards b ON b.id = s.board_id
         WHERE b.id = $1 AND s.category = 'closed' LIMIT 1",
    )
    .bind(board_id.parse::<Uuid>().unwrap())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{t_met}/status"),
        Some(&bearer),
        Some(json!({ "status_id": resolved_status })),
    )
    .await;
    assert_eq!(response.status(), 200, "close must succeed");
    // force-resolve instantly via SQL so it lands inside our window math:
    let mut tx = state.pool.begin().await.expect("resolve tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    sqlx::query("UPDATE tickets SET resolved_at = $2 WHERE id = $1")
        .bind(t_met.parse::<Uuid>().unwrap())
        .bind(epoch(0, 8))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Attainment over that day resolves exactly one judged row.
    let qs = format!(
        "sla_attainment?from={}&to={}",
        epoch(0, 0).date_naive(),
        epoch(0, 0).date_naive()
    );
    let (st, body) = get_report(&app, &bearer, &qs).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["summary"]["resolutions_total"], json!(1));
    assert_eq!(body["summary"]["resolutions_met"], json!(1));
    assert_eq!(body["summary"]["attainment_pct"], json!(100.0));
    assert_eq!(body["rows"][0]["metric"], json!(1));

    // Breaches listing contains exactly the flagged ticket.
    let qs = format!(
        "sla_breaches?from={}&to={}",
        epoch(-7, 0).date_naive(),
        epoch(1, 0).date_naive()
    );
    let (st, body) = get_report(&app, &bearer, &qs).await;
    assert_eq!(st, 200);
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["ticket_id"], json!(t_breached));
    assert_eq!(body["summary"]["breaches_total"], json!(1));
}

// --- first-response edge semantics ------------------------------------------------

#[tokio::test]
async fn first_response_ignores_portal_comments_and_reports_unanswered_nulls() {
    let (state, app) = test_state().await;
    let s = slug("fr");
    let (bearer, org, board_id) = seed_org(&app, s.clone()).await;

    let t_unanswered = make_ticket(&app, &bearer, &board_id, "no reply yet", None, None).await
        ["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_portal_first = make_ticket(&app, &bearer, &board_id, "client posted first", None, None)
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Pin creations into the queried window; portal-style reply
    // (client_visible, author_id NULL) is NOT a staff response even when it
    // lands before any staff comment. All direct SQL inside org-scoped txs.
    let mut tx = state.pool.begin().await.expect("fixture tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    for t in [&t_unanswered, &t_portal_first] {
        sqlx::query("UPDATE tickets SET created_at = $2 WHERE id = $1")
            .bind(t.parse::<Uuid>().unwrap())
            .bind(epoch(0, 0))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO ticket_comments
           (id, org_id, ticket_id, author_id, kind, body, created_at)
         VALUES ($1,$2,$3,NULL,'client_visible','portal shout', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(t_portal_first.parse::<Uuid>().unwrap())
    .bind(epoch(0, 2))
    .execute(&mut *tx)
    .await
    .unwrap();

    // Staff internal comment at hour 6 → counts as first response.
    sqlx::query(
        "INSERT INTO ticket_comments
           (id, org_id, ticket_id, author_id, kind, body, created_at)
         SELECT $1,$2,$3,id,'internal','we are on it',$4 FROM users WHERE org_id=$2 LIMIT 1",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(t_portal_first.parse::<Uuid>().unwrap())
    .bind(epoch(0, 6))
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let qs = format!(
        "first_response?from={}&to={}",
        epoch(-7, 0).date_naive(),
        epoch(1, 0).date_naive()
    );
    let (st, body) = get_report(&app, &bearer, &qs).await;
    assert_eq!(st, 200);
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{body}");

    let by_number: std::collections::HashMap<String, &Value> = rows
        .iter()
        .map(|r| (r["ticket_id"].as_str().unwrap().to_string(), r))
        .collect();

    // Unanswered ticket carries NULLs end-to-end.
    let unanswered = by_number[&t_unanswered];
    assert!(unanswered["first_staff_comment_at"].is_null());
    assert!(unanswered["first_response_secs"].is_null());

    // Portal comment at h2 excluded; staff at h6 measured instead.
    let portal_row = by_number[&t_portal_first];
    // serde's chrono RFC3339 encoding renders UTC offsets as 'Z'.
    assert_eq!(
        portal_row["first_staff_comment_at"],
        json!("2026-06-10T06:00:00Z")
    );
    assert_eq!(
        portal_row["first_response_secs"],
        json!((epoch(0, 6) - epoch(0, 0)).num_seconds())
    );

    // Summary split counts match the fixture exactly.
    assert_eq!(body["summary"]["tickets_total"], json!(2));
    assert_eq!(body["summary"]["answered_count"], json!(1));
    assert_eq!(body["summary"]["unanswered_count"], json!(1));
}

// --- aging + throughput + CSV + validation ----------------------------------------

#[tokio::test]
async fn aging_buckets_and_throughput_series() {
    let (_state, app) = test_state().await;
    let s = slug("aging");
    let (bearer, _org, board_id) = seed_org(&app, s.clone()).await;

    make_ticket(&app, &bearer, &board_id, "fresh", None, None).await;
    make_ticket(&app, &bearer, &board_id, "oldie", None, None).await;
    // Age the second ticket artificially to land in the over-30d bucket.
    sqlx::query("UPDATE tickets SET created_at = now() - interval '45 days' WHERE title = 'oldie'")
        .execute(&_state.pool)
        .await
        .unwrap();

    let as_of = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (st, body) = get_report(&app, &bearer, &format!("ticket_aging?as_of={as_of}")).await;
    assert_eq!(st, 200);
    let buckets: Vec<&Value> = body["rows"].as_array().unwrap().iter().collect();
    assert_eq!(
        buckets
            .iter()
            .map(|b| b["bucket"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["under_1d", "over_30d"]
    );
    assert_eq!(buckets[0]["open_count"], json!(1));
    let oldest = buckets[1]["oldest_age_days"].as_i64().unwrap();
    assert!(
        (40..=46).contains(&oldest),
        "oldest bucket age ~45d, got {oldest}"
    );

    // Throughput resolves both arms without error and summary is present.
    let (st, body) = get_report(&app, &bearer, "ticket_throughput").await;
    assert_eq!(st, 200);
    assert!(!body["rows"].as_array().unwrap().is_empty());
    assert!(body["summary"]["created_total"].is_i64());
}

#[tokio::test]
async fn csv_shape_is_stable_and_quotes_specials() {
    let (state, app) = test_state().await;
    let s = slug("csv");
    let (bearer, _org, board_id) = seed_org(&app, s.clone()).await;

    // Client name WITH a comma proves quoting stability in labels.
    let comma_client = make_client(&app, &bearer, "Beta, Inc & Sons").await;
    let t = make_ticket(
        &app,
        &bearer,
        &board_id,
        "comma csv",
        None,
        Some(comma_client),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    post_entry(
        &app,
        &bearer,
        t.parse().unwrap(),
        epoch(-1, 0),
        epoch(-1, 2),
        true,
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!(
            "/api/reports/time_by_client?format=csv&from={}&to={}",
            epoch(-7, 0).date_naive(),
            epoch(1, 0).date_naive()
        ),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let content_type = response.headers()["content-type"].to_str().unwrap();
    assert!(content_type.starts_with("text/csv"), "{content_type}");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut lines = text.trim_end_matches('\n').split('\n');
    let header = lines.next().unwrap();
    assert_eq!(header, "group_id,label,metric,billable_minutes,entry_count");
    let row_line = lines.next().unwrap();
    assert!(
        row_line.contains("\"Beta, Inc & Sons\""),
        "quoting must wrap the embedded comma: {row_line}"
    );
    assert!(
        row_line.contains(",120,"),
        "metric column carries 120 minutes"
    );
    assert!(row_line.ends_with(",1"), "entry_count tail");
    assert!(lines.next().is_none(), "exactly one data row");
    let _ = state.pool;
}

#[tokio::test]
async fn validation_and_auth_rejections_are_typed() {
    let (_state, app) = test_state().await;
    let s = slug("valid");
    let (bearer, _org, _board) = seed_org(&app, s.clone()).await;

    let cases: &[(&str, axum::http::StatusCode)] = &[
        (
            "time_by_user?from=2026-06-10&to=2026-06-01",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ), // inverted
        (
            "time_by_user?from=2025-01-01&to=2027-01-01",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ), // >366d span? 367+?
        (
            "time_by_user?limit=0",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "time_by_user?format=xml",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("definitely_not_a_kind", axum::http::StatusCode::NOT_FOUND),
        ("sla_attainment", axum::http::StatusCode::OK), // sane baseline
    ];
    for (qs, expected) in cases {
        let (st, _body) = get_report(&app, &bearer, qs).await;
        assert_eq!(&st, expected, "query {qs}");
    }

    // Unauthenticated call never reaches the compiler.
    let resp = send(
        &app,
        axum::http::Method::GET,
        "/api/reports/time_by_user",
        None,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

    // Window boundary precision: 2025 has 365 days, so Jan 1 → Dec 31
    // inclusive == 365 ≤ 366 must PASS; stepping one more day spills past
    // the cap and must fail.
    let legal = "time_by_user?from=2025-01-01&to=2025-12-31";
    let (st_legal, _) = get_report(&app, &bearer, legal).await;
    assert_eq!(
        st_legal,
        axum::http::StatusCode::OK,
        "365-day window is legal"
    );
    let at_cap = "time_by_user?from=2025-01-01&to=2026-01-01"; // exactly 366 inclusive
    let (st_cap, _) = get_report(&app, &bearer, at_cap).await;
    assert_eq!(
        st_cap,
        axum::http::StatusCode::OK,
        "366-day window is the cap"
    );
    let spill = "time_by_user?from=2025-01-01&to=2026-01-02"; // 367 days inclusive
    let (st_spill, _) = get_report(&app, &bearer, spill).await;
    assert_eq!(
        st_spill,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "367-day window exceeds the cap"
    );
}
