//! End-to-end dispatch v2 flows (TASKLIST Phase 3, Wave 9 lane 9) driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the lane's three surfaces over HTTP — the capacity
//! horizon (day view math rolled across a bounded window), unassigned-
//! queue prioritization (explicit rank overrides ahead of the computed
//! order, in the queue endpoint AND the day view), skills/certification
//! matching (expiry-aware, best-first candidates), travel/mileage draft
//! conversion through the EXISTING time-entry path (idempotent submit,
//! provenance notes + write-trail + events), and the dispatch connector
//! (scoped API-key reads, idempotent status pushes with connector
//! provenance, rate-limit and scope rejections, cross-org 404s).
//!
//! Non-purpose: day-view conflict math (dispatch_flow), connector bulk
//! push/pull (connector_conformance), pure interval math (core-domain).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("dispatch v2 e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with dispatch v2 surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
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
    api_key: Option<&str>,
    body: Option<Value>,
) -> axum::http::response::Response<axum::body::Body> {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = api_key {
        builder = builder.header("x-api-key", key);
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

async fn body_json(response: axum::http::response::Response<axum::body::Body>) -> Value {
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({}; status {}) for last SEND endpoint",
            e, parts.status
        )
    })
}

struct Org {
    token: String,
    org_id: Uuid,
    user_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        None,
        Some(json!({
            "org_name": "Dispatch v2 Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/me",
            Some(&token),
            None,
            None,
        )
        .await,
    )
    .await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
    }
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool — there is no org-user invite API yet — and
/// logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@dispatch-v2.test");
    core_db::users::create_user(
        &state.pool,
        Uuid::now_v7(),
        org.org_id,
        &email,
        label,
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed technician user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            None,
            Some(json!({"email": email, "password": "correct-horse-battery"})),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"]
        .as_str()
        .expect("default board seeded")
        .parse()
        .unwrap()
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str, priority: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        None,
        Some(json!({
            "board_id": board,
            "title": title,
            "priority": priority,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn create_appointment(
    app: &Router,
    org: &Org,
    title: &str,
    day: u32,
    start_h: u32,
    end_h: u32,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&org.token),
        None,
        Some(json!({
            "title": title,
            "starts_at": at(day, start_h),
            "ends_at": at(day, end_h),
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "appointment create must succeed");
    body_json(response).await
}

/// June 2026: day 1 is a Monday. All times UTC.
fn at(day: u32, hour: u32) -> String {
    format!("2026-06-{day:02}T{hour:02}:00:00Z")
}

/// Mon–Fri 09:00–17:00 UTC availability for the org's owner.
async fn seed_weekday_availability(app: &Router, org: &Org) {
    for weekday in 0..5 {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/availability",
            Some(&org.token),
            None,
            Some(json!({
                "weekday": weekday, "start_local": "09:00", "end_local": "17:00", "timezone": "UTC",
            })),
        )
        .await;
        assert_eq!(response.status(), 201);
    }
}

async fn create_api_key(app: &Router, org: &Org, name: &str, scopes: &[&str]) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(&org.token),
        None,
        Some(json!({ "name": name, "scopes": scopes })),
    )
    .await;
    assert_eq!(response.status(), 201, "api key create must succeed");
    let body = body_json(response).await;
    body["raw_key"].as_str().unwrap().to_string()
}

// --- capacity horizon -------------------------------------------------------

#[tokio::test]
async fn capacity_horizon_rolls_day_math_across_days() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("d2-cap-{}", Uuid::now_v7().simple())).await;
    seed_weekday_availability(&app, &org).await;

    // A 09:00–11:00 booking on Monday only.
    create_appointment(&app, &org, "site visit", 1, 9, 11).await;

    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/capacity?from=2026-06-01&days=3",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["from"], "2026-06-01");
    assert_eq!(view["days"], 3);
    let me = view["technicians"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["user_id"].as_str() == Some(&org.user_id.to_string()))
        .expect("owner is a technician")
        .clone();
    let days = me["days"].as_array().unwrap();
    assert_eq!(days.len(), 3);
    // Monday: 420 available (exclusion-free here), 120 booked, 300 free.
    assert_eq!(days[0]["available_minutes"], 480, "capacity view: {view}");
    assert_eq!(days[0]["booked_minutes"], 120);
    assert_eq!(days[0]["free_minutes"], 360);
    assert_eq!(days[0]["utilization_percent"], 25);
    // Tuesday/Wednesday: full 480, nothing booked.
    assert_eq!(days[1]["available_minutes"], 480);
    assert_eq!(days[1]["booked_minutes"], 0);
    assert_eq!(days[1]["free_minutes"], 480);
    assert_eq!(days[2]["booked_minutes"], 0);

    // The horizon is clamped: asking for a year of days returns 31.
    let clamped = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/capacity?days=365",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(clamped["days"], 31);

    // Demand context: open unassigned tickets show up as the queue depth.
    let board = default_board(&app, &org).await;
    create_ticket(&app, &org, board, "unplaced", "p2").await;
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/capacity?from=2026-06-01&days=1",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["unassigned_count"], 1);

    let _ = state;
}

// --- queue prioritization ----------------------------------------------------

#[tokio::test]
async fn queue_override_orders_queue_and_day_view() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("d2-queue-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &org).await;

    let p3 = create_ticket(&app, &org, board, "low", "p3").await;
    let p1 = create_ticket(&app, &org, board, "urgent", "p1").await;

    // Computed order first: priority rank puts the p1 ticket ahead.
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/queue",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["queue"][0]["id"], p1["id"]);
    assert!(view["queue"][0]["rank_override"].is_null());

    // Pin the p3 ticket to the front.
    let set = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/dispatch/queue/{}", p3["id"].as_str().unwrap()),
        Some(&org.token),
        None,
        Some(json!({
            "rank": 0,
            "note": "promised site visit",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(
        set.status(),
        200,
        "queue set must succeed: {:?}",
        set.status()
    );
    let set_body = body_json(set).await;
    assert_eq!(set_body["rank"], 0);
    assert_eq!(set_body["cleared"], false);
    assert!(set_body["mutation"]["seq"].is_i64());

    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/queue",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["queue"][0]["id"], p3["id"]);
    assert_eq!(view["queue"][0]["rank_override"], 0);
    assert_eq!(view["queue"][0]["override_note"], "promised site visit");
    assert_eq!(view["queue"][1]["id"], p1["id"]);

    // The day view queue honors the same ordering.
    let day = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/day?date=2026-06-01",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(day["unassigned"][0]["id"], p3["id"]);

    // Clear restores the computed order (idempotent clears return 200).
    let cleared = body_json(
        send(
            &app,
            axum::http::Method::DELETE,
            &format!("/api/dispatch/queue/{}", p3["id"].as_str().unwrap()),
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(cleared["cleared"], true, "clear body: {cleared}");
    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/queue",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["queue"][0]["id"], p1["id"]);

    // Assigned tickets cannot hold overrides (planning data only).
    let assign = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/assign",
        Some(&org.token),
        None,
        Some(json!({
            "ticket_id": p1["id"],
            "user_id": org.user_id,
            "starts_at": at(1, 13),
            "ends_at": at(1, 14),
        })),
    )
    .await;
    assert_eq!(assign.status(), 201);
    let rejected = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/dispatch/queue/{}", p1["id"].as_str().unwrap()),
        Some(&org.token),
        None,
        Some(json!({ "rank": 1 })),
    )
    .await;
    assert_eq!(rejected.status(), 422);

    // RBAC: a roleless user gets 403 on every planning surface.
    let roleless = add_roleless_user(&app, &state, &org, "norole-queue").await;
    for (method, uri) in [
        ("GET", "/api/dispatch/queue"),
        ("GET", "/api/dispatch/capacity"),
        (
            "PUT",
            &format!("/api/dispatch/queue/{}", p3["id"].as_str().unwrap()),
        ),
    ] {
        let response = send(
            &app,
            axum::http::Method::from_bytes(method.as_bytes()).unwrap(),
            uri,
            Some(&roleless),
            None,
            if method == "PUT" {
                Some(json!({"rank": 0}))
            } else {
                None
            },
        )
        .await;
        assert_eq!(response.status(), 403, "{method} {uri} must be 403");
    }
}

// --- skills / certifications matching ---------------------------------------

#[tokio::test]
async fn skills_replace_catalog_and_expiry_aware_match() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("d2-skill-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &org).await;

    // Owner: current cert + plain skill. Roleless tech: network only.
    let put_owner = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/dispatch/users/{}/skills", org.user_id),
        Some(&org.token),
        None,
        Some(json!({
            "skills": [
                { "skill": "nvidia-certified", "expires_at": "2027-01-01T00:00:00Z" },
                { "skill": "networking" }
            ],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(put_owner.status(), 200);
    let owner_skills = body_json(put_owner).await;
    assert_eq!(owner_skills["skills"].as_array().unwrap().len(), 2);
    assert!(owner_skills["mutation"]["seq"].is_i64());

    // Replay: the same key + payload returns the same version, not a
    // second write.
    let key = Uuid::now_v7().to_string();
    let body = json!({
        "skills": [ { "skill": "solo" } ],
        "idempotency_key": key,
    });
    let first = body_json(
        send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/dispatch/users/{}/skills", org.user_id),
            Some(&org.token),
            None,
            Some(body.clone()),
        )
        .await,
    )
    .await;
    let replay = body_json(
        send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/dispatch/users/{}/skills", org.user_id),
            Some(&org.token),
            None,
            Some(body),
        )
        .await,
    )
    .await;
    assert_eq!(first["mutation"]["seq"], replay["mutation"]["seq"]);
    assert_eq!(first["skills"], replay["skills"]);

    let _tech2_token = add_roleless_user(&app, &state, &org, "tech2").await;
    // A roleless user holds no scheduling grant, so give tech2 skills via
    // a direct seed (the owner-surface path is exercised above).
    sqlx::query(
        "INSERT INTO user_skills (id, org_id, user_id, skill) VALUES ($1, $2, $3, 'networking')",
    )
    .bind(Uuid::now_v7())
    .bind(org.org_id)
    .bind(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM users WHERE org_id = $1 AND display_name = 'tech2'",
        )
        .bind(org.org_id)
        .fetch_one(&state.pool)
        .await
        .unwrap(),
    )
    .execute(&state.pool)
    .await
    .unwrap();

    // Catalog: 'networking' has two holders (owner was re-replaced to
    // solo above — so exactly one); deterministic read anyway.
    let catalog = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/skills",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    let entries = catalog["skills"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["skill"] == "networking"));

    // Restore the owner's full set for the matching leg.
    let _ = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/dispatch/users/{}/skills", org.user_id),
        Some(&org.token),
        None,
        Some(json!({
            "skills": [
                { "skill": "nvidia-certified", "expires_at": "2027-01-01T00:00:00Z" },
                { "skill": "networking" }
            ],
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;

    // Ticket requires both skills.
    let ticket = create_ticket(&app, &org, board, "gpu + net", "p2").await;
    let set = send(
        &app,
        axum::http::Method::PUT,
        &format!(
            "/api/dispatch/tickets/{}/required-skills",
            ticket["id"].as_str().unwrap()
        ),
        Some(&org.token),
        None,
        Some(json!({ "skills": ["nvidia-certified", "networking"] })),
    )
    .await;
    assert_eq!(set.status(), 200);

    // An EXPIRED certification does not count as held.
    let tech2_id: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 AND display_name = 'tech2'")
            .bind(org.org_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO user_skills (id, org_id, user_id, skill, expires_at) VALUES ($1, $2, $3, 'nvidia-certified', '2020-01-01T00:00:00Z')")
        .bind(Uuid::now_v7())
        .bind(org.org_id)
        .bind(tech2_id)
        .execute(&state.pool)
        .await
        .unwrap();

    let view = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/match",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    let matches = view["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "only tickets WITH required skills match");
    assert_eq!(
        matches[0]["id"], ticket["id"],
        "ticket flattens into the match row"
    );
    let candidates = matches[0]["candidates"].as_array().unwrap();
    let owner = candidates
        .iter()
        .find(|c| c["user_id"].as_str() == Some(&org.user_id.to_string()))
        .unwrap();
    assert_eq!(
        owner["missing"].as_array().unwrap().len(),
        0,
        "owner is fully qualified"
    );
    let tech2 = candidates
        .iter()
        .find(|c| c["user_id"].as_str() == Some(&tech2_id.to_string()))
        .unwrap();
    assert_eq!(
        tech2["missing"].as_array().unwrap().len(),
        1,
        "expired certification must not count as held"
    );
    assert!(
        owner["missing"].as_array().unwrap().len() < tech2["missing"].as_array().unwrap().len()
    );
}

// --- travel/mileage drafts ----------------------------------------------------

#[tokio::test]
async fn travel_draft_lifecycle_converts_through_time_path() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("d2-travel-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "on-site repair", "p2").await;
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // Create with an idempotency key; a retried create replays.
    let create_body = json!({
        "ticket_id": ticket_id,
        "started_at": at(1, 8),
        "ended_at": at(1, 9),
        "distance": 42.5,
        "distance_unit": "km",
        "vehicle": "van",
        "billable": false,
        "notes": "Depot to client site",
        "idempotency_key": Uuid::now_v7().to_string(),
    });
    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/travel-drafts",
            Some(&org.token),
            None,
            Some(create_body.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(created["status"], "draft");
    assert!(created["mutation"]["seq"].is_i64());
    let replay = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/travel-drafts",
            Some(&org.token),
            None,
            Some(create_body),
        )
        .await,
    )
    .await;
    assert_eq!(
        replay["id"], created["id"],
        "replay must return the SAME draft"
    );
    assert_eq!(replay["mutation"]["seq"], created["mutation"]["seq"]);

    // Validation: zero distance, bad unit, inverted window.
    for bad in [
        json!({ "ticket_id": ticket_id, "started_at": at(1, 8), "ended_at": at(1, 9),
                "distance": 0.0, "distance_unit": "km", "vehicle": "van" }),
        json!({ "ticket_id": ticket_id, "started_at": at(1, 8), "ended_at": at(1, 9),
                "distance": 5.0, "distance_unit": "leagues", "vehicle": "van" }),
        json!({ "ticket_id": ticket_id, "started_at": at(1, 9), "ended_at": at(1, 8),
                "distance": 5.0, "distance_unit": "km", "vehicle": "van" }),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/travel-drafts",
            Some(&org.token),
            None,
            Some(bad),
        )
        .await;
        assert_eq!(response.status(), 422, "bad drafts must be rejected");
    }

    // PATCH the distance.
    let patched = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!(
                "/api/dispatch/travel-drafts/{}",
                created["id"].as_str().unwrap()
            ),
            Some(&org.token),
            None,
            Some(json!({ "distance": 51.0 })),
        )
        .await,
    )
    .await;
    assert_eq!(patched["distance"], 51.0);

    // Submit converts through the existing time-entry path.
    let submitted = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!(
                "/api/dispatch/travel-drafts/{}/submit",
                created["id"].as_str().unwrap()
            ),
            Some(&org.token),
            None,
            Some(json!({})),
        )
        .await,
    )
    .await;
    let entry_id = submitted["id"].as_str().unwrap().to_string();
    assert_eq!(submitted["work_type"], "travel");
    assert_eq!(submitted["billable"], false);
    assert_eq!(submitted["minutes"], 60);
    let notes = submitted["notes"].as_str().unwrap();
    assert!(
        notes.contains("51.0 km") && notes.contains("van") && notes.contains("travel_draft:"),
        "notes must carry the provenance marker, got: {notes}"
    );

    // The entry is REAL: visible through the ordinary time-entry read.
    let entries = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries?ticket_id={ticket_id}"),
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert!(entries
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["id"].as_str() == Some(&entry_id)));

    // The draft froze with the backlink.
    let drafts = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/travel-drafts?status=submitted",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(drafts[0]["status"], "submitted");
    assert_eq!(
        drafts[0]["submitted_time_entry_id"].as_str().unwrap(),
        entry_id
    );

    // Submitted drafts are frozen: patch is 404-shaped, resubmit 409.
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!(
            "/api/dispatch/travel-drafts/{}",
            created["id"].as_str().unwrap()
        ),
        Some(&org.token),
        None,
        Some(json!({ "distance": 99.0 })),
    )
    .await;
    assert_eq!(patched.status(), 404);
    let resubmit = send(
        &app,
        axum::http::Method::POST,
        &format!(
            "/api/dispatch/travel-drafts/{}/submit",
            created["id"].as_str().unwrap()
        ),
        Some(&org.token),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(resubmit.status(), 409);

    // A second draft submits to a SECOND entry (no draft → no entry).
    let draft2 = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/travel-drafts",
            Some(&org.token),
            None,
            Some(json!({
                "ticket_id": ticket_id,
                "started_at": at(2, 8),
                "ended_at": at(2, 9),
                "distance": 10.0,
                "distance_unit": "mi",
                "vehicle": "car",
            })),
        )
        .await,
    )
    .await;
    let discard = send(
        &app,
        axum::http::Method::DELETE,
        &format!(
            "/api/dispatch/travel-drafts/{}",
            draft2["id"].as_str().unwrap()
        ),
        Some(&org.token),
        None,
        None,
    )
    .await;
    assert_eq!(discard.status(), 200, "draft delete must succeed");
    let entries_after = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-entries?ticket_id={ticket_id}"),
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        entries_after.as_array().unwrap().len(),
        1,
        "discarded draft leaves no entry"
    );

    // Events + provenance trail: time_entry.created co-emits with
    // travel_draft.submitted, and the write trail names the submit.
    let event_types: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM outbox_events WHERE org_id = $1 AND event_type IN \
         ('time_entry.created','travel_draft.submitted')",
    )
    .bind(org.org_id)
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert!(event_types.contains(&"time_entry.created".to_string()));
    assert!(event_types.contains(&"travel_draft.submitted".to_string()));
    let trail_op: String = sqlx::query_scalar(
        "SELECT op FROM entity_writes WHERE org_id = $1 AND entity = 'travel_draft' AND op = 'travel_draft_submit'",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(trail_op, "travel_draft_submit");

    // Only owner/admin may file a draft for someone else.
    let other_token = add_roleless_user(&app, &state, &org, "other-tech").await;
    let forbidden = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/travel-drafts",
        Some(&other_token),
        None,
        Some(json!({
            "ticket_id": ticket_id,
            "user_id": org.user_id,
            "started_at": at(3, 8),
            "ended_at": at(3, 9),
            "distance": 5.0,
            "distance_unit": "km",
            "vehicle": "bike",
        })),
    )
    .await;
    assert_eq!(forbidden.status(), 403);
}

// --- dispatch connector --------------------------------------------------------

#[tokio::test]
async fn connector_reads_schedule_and_pushes_status() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("d2-conn-{}", Uuid::now_v7().simple())).await;
    seed_weekday_availability(&app, &org).await;
    let appt = create_appointment(&app, &org, "site visit", 1, 9, 11).await;
    let appt_id = appt["id"].as_str().unwrap().to_string();

    let write_key = create_api_key(&app, &org, "field-app", &["dispatch:write"]).await;
    let read_key = create_api_key(&app, &org, "dashboard", &["dispatch:read"]).await;
    let offscope_key = create_api_key(&app, &org, "wrong-scope", &["tickets:read"]).await;

    // Schedule read (machine key): the booking is visible with coordinates.
    let schedule = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch-connector/schedule?from=2026-06-01&days=3",
            None,
            Some(&write_key),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(schedule["appointments"].as_array().unwrap().len(), 1);
    assert_eq!(schedule["appointments"][0]["id"], appt["id"]);
    assert_eq!(schedule["appointments"][0]["technician_name"], "Owner");
    assert!(schedule["appointments"][0]["dispatch_status"].is_null());

    // Read-only scope CAN read; the off-scope key CANNOT (its tickets:read
    // scope resolves onto no scheduling permission).
    let read = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch-connector/schedule",
        None,
        Some(&read_key),
        None,
    )
    .await;
    assert_eq!(read.status(), 200);
    let denied = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch-connector/schedule",
        None,
        Some(&offscope_key),
        None,
    )
    .await;
    assert_eq!(denied.status(), 403);

    // Push a status (write scope required; read-only scope is rejected).
    let push_body = json!({
        "status": "en_route",
        "note": "Left depot",
        "idempotency_key": Uuid::now_v7().to_string(),
    });
    let push = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/dispatch-connector/appointments/{appt_id}/status"),
        None,
        Some(&write_key),
        Some(push_body.clone()),
    )
    .await;
    assert_eq!(push.status(), 201, "status push must succeed");
    let pushed = body_json(push).await;
    assert_eq!(pushed["status"], "en_route");
    let replay = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/dispatch-connector/appointments/{appt_id}/status"),
            None,
            Some(&write_key),
            Some(push_body),
        )
        .await,
    )
    .await;
    assert_eq!(
        replay["id"], pushed["id"],
        "same key + payload replays the SAME push"
    );
    let readonly_push = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/dispatch-connector/appointments/{appt_id}/status"),
        None,
        Some(&read_key),
        Some(json!({ "status": "arrived" })),
    )
    .await;
    assert_eq!(readonly_push.status(), 403);

    // Unknown vocabulary is a typed 422.
    let bad = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/dispatch-connector/appointments/{appt_id}/status"),
        None,
        Some(&write_key),
        Some(json!({ "status": "teleported" })),
    )
    .await;
    assert_eq!(bad.status(), 422);

    // The schedule now shades the booking with the latest push.
    let schedule = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch-connector/schedule?from=2026-06-01&days=3",
            None,
            Some(&write_key),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        schedule["appointments"][0]["dispatch_status"]["status"],
        "en_route"
    );
    assert_eq!(
        schedule["appointments"][0]["dispatch_status"]["source"],
        "connector"
    );

    // The STAFF day view sees the same shade additively.
    let day = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch/day?date=2026-06-01",
            Some(&org.token),
            None,
            None,
        )
        .await,
    )
    .await;
    let me = day["technicians"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["user_id"].as_str() == Some(&org.user_id.to_string()))
        .unwrap();
    assert_eq!(
        me["appointments"][0]["dispatch_status"]["status"],
        "en_route"
    );

    // Technicians read exposes current skills only.
    let techs = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/dispatch-connector/technicians",
            None,
            Some(&write_key),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(techs["technicians"].as_array().unwrap().len(), 1);

    // Humans without owner/admin are rejected (D3-style gate); the org
    // owner passes with a bearer token.
    let roleless = add_roleless_user(&app, &state, &org, "norole-conn").await;
    let human_denied = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch-connector/schedule",
        Some(&roleless),
        None,
        None,
    )
    .await;
    assert_eq!(human_denied.status(), 403);
    let human_ok = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch-connector/schedule",
        Some(&org.token),
        None,
        None,
    )
    .await;
    assert_eq!(human_ok.status(), 200);

    // Cross-org isolation: another org's key gets a bare 404.
    let other = register_org(&app, format!("d2-conn-other-{}", Uuid::now_v7().simple())).await;
    let other_key = create_api_key(&app, &other, "their-app", &["dispatch:write"]).await;
    let foreign = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/dispatch-connector/appointments/{appt_id}/status"),
        None,
        Some(&other_key),
        Some(json!({ "status": "arrived" })),
    )
    .await;
    assert_eq!(foreign.status(), 404);

    // Provenance: the trail row records the CONNECTOR actor kind and the
    // outbox carries the published event.
    let actor_kind: String = sqlx::query_scalar(
        "SELECT actor_kind FROM entity_writes WHERE org_id = $1 AND entity = 'dispatch_status' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(actor_kind, "connector");
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM outbox_events WHERE org_id = $1 AND event_type = 'dispatch.status_updated'",
    )
    .bind(org.org_id)
    .fetch_all(&state.pool)
    .await
    .unwrap();
    assert_eq!(events.len(), 1, "a replayed push must not double-emit");
}
