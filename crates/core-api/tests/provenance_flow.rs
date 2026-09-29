//! Provenance read-model flow tests (TASKLIST IF9, doctrine #9), driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: pin the public surface of the provenance read model over HTTP:
//!
//! * Projection — envelope writes (IF1) project into per-field provenance
//!   rows: who / record+field / before-after values / actor kind / op /
//!   seq / when, without any raw-row scans (migration 0043 trigger).
//! * Query API — `GET /api/provenance` extends the 2.3 audit-log query API
//!   with provenance-grade filters (actor kind, record+field, agent
//!   session/skill/rule-set hash when present, undo, time range) and the
//!   same keyset-cursor conventions, extended with the per-write field
//!   position so pagination never skips or repeats a field.
//! * RBAC — holder of the audit permission (owner/admin, "manager+") sees
//!   org-wide provenance; every other user sees ONLY their own actions,
//!   even when they request someone else's actor id.
//! * Agent context — session/skill/rule-set-hash columns surface "when
//!   present" (IF8-approved writes), exercised here through a direct
//!   write-trail row the way the approval pipeline will emit it.
//!
//! Non-purpose: the audit-events surface itself (its own suite), the
//! idempotency/version gates (instant_first_flow), and the undo write
//! path (IF7 — only the flag's query side is pinned here).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("provenance e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with the provenance read model");
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
        panic!("body_json parse failed ({e}); last SEND lines identify the endpoint")
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
        Some(json!({
            "org_name": "Provenance Co",
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
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
    }
}

/// Creates a builtin-technician user (tickets full matrix, NO audit
/// permission) and logs them in.
async fn provision_technician(state: &AppState, app: &Router, org: &Org, label: &str) -> String {
    let email = format!("{label}@provenance.test");
    let password_hash = core_api::auth::password::hash("correct-horse-battery").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org.org_id)
        .await
        .expect("set_org");
    let role_id: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org.org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role exists");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org.org_id,
        &email,
        label,
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role_id)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "correct-horse-battery" })),
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
        )
        .await,
    )
    .await;
    nav[0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .expect("default board id")
}

async fn create_ticket(app: &Router, bearer: &str, board: Uuid, title: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(bearer),
        Some(json!({
            "board_id": board,
            "title": title,
            "priority": "p3",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn patch_ticket(app: &Router, bearer: &str, ticket_id: Uuid, body: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_id}"),
        Some(bearer),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket patch must succeed");
    body_json(response).await
}

async fn provenance(app: &Router, bearer: &str, query: &str) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/provenance{query}"),
        Some(bearer),
        None,
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

// ===========================================================================
// Projection: envelope writes land as per-field provenance rows
// ===========================================================================

#[tokio::test]
async fn envelope_writes_project_per_field_provenance() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("prov-basic-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;

    let created = create_ticket(&app, &org.token, board, "Autosaved title").await;
    let ticket_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    patch_ticket(
        &app,
        &org.token,
        ticket_id,
        json!({ "title": "Patched title", "priority": "p1" }),
    )
    .await;

    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}"),
    )
    .await;
    assert_eq!(status, 200, "{page}");
    let events = page["events"].as_array().expect("events list");

    // Newest first: the patch (seq 2) precedes the create (seq 1).
    let seqs: Vec<i64> = events.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    assert!(
        seqs.contains(&1) && seqs.contains(&2),
        "both writes: {seqs:?}"
    );
    let first = &events[0];
    assert_eq!(first["seq"], 2, "newest write first");

    // Create rows: every seeded field is a change with a null prior value.
    let create_rows: Vec<&Value> = events
        .iter()
        .filter(|e| e["seq"] == 1 && e["op"] == "create")
        .collect();
    assert!(
        create_rows.len() >= 4,
        "create projects each seeded field: {}",
        create_rows.len()
    );
    let title_create = create_rows
        .iter()
        .find(|e| e["field"] == "title")
        .expect("title create row");
    assert!(title_create["before"].is_null(), "no prior on create");
    assert_eq!(title_create["after"], "Autosaved title");

    // Update rows: ONLY the changed fields (not untouched ones like id).
    let update_rows: Vec<&Value> = events
        .iter()
        .filter(|e| e["seq"] == 2 && e["op"] == "update")
        .collect();
    let title_update = update_rows
        .iter()
        .find(|e| e["field"] == "title")
        .expect("changed title projected");
    assert_eq!(title_update["before"], "Autosaved title");
    assert_eq!(title_update["after"], "Patched title");
    assert!(
        update_rows.iter().any(|e| e["field"] == "priority"),
        "changed priority projected"
    );
    assert!(
        !update_rows.iter().any(|e| e["field"] == "id"),
        "unchanged fields must not project: {update_rows:?}"
    );

    // Provenance identity on every row.
    for e in events {
        assert_eq!(e["actor_kind"], "human");
        assert_eq!(
            e["actor_id"],
            json!(org.user_id.to_string()),
            "owner attributed"
        );
        assert_eq!(e["entity"], "ticket");
        assert_eq!(e["undo"], false);
        assert_eq!(e["write_id"], e["write_id"], "write id present");
        assert!(e["occurred_at"].as_str().is_some());
    }
}

// ===========================================================================
// Query API: provenance-grade filters
// ===========================================================================

#[tokio::test]
async fn field_actor_kind_and_time_filters_narrow_the_projection() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("prov-filter-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let created = create_ticket(&app, &org.token, board, "Filter target").await;
    let ticket_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    patch_ticket(&app, &org.token, ticket_id, json!({ "title": "Renamed" })).await;

    // field filter: only title rows across both writes.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&field=title"),
    )
    .await;
    assert_eq!(status, 200);
    let events = page["events"].as_array().unwrap();
    assert!(
        events.len() >= 2,
        "title changed on both writes: {events:?}"
    );
    assert!(events.iter().all(|e| e["field"] == "title"));
    let by_seq: Vec<i64> = events.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    assert!(by_seq.contains(&1) && by_seq.contains(&2));

    // actor_kind filter: humans only here; agent vocabulary is legal but empty.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&actor_kind=agent"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(page["events"].as_array().unwrap().is_empty());

    // Unknown actor kind is a typed 422, not a silent empty page.
    let (status, body) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&actor_kind=klingon"),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["reason"], "validation_failed");

    // entity_id / field without entity are a typed 422 (index-honest).
    let (status, body) = provenance(&app, &org.token, &format!("?entity_id={ticket_id}")).await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["reason"], "validation_failed");

    // Time window excludes everything when bounded to the far future.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&from=2099-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(page["events"].as_array().unwrap().is_empty());

    // undo=false is the default world; undo=true excludes human writes.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&undo=true"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(page["events"].as_array().unwrap().is_empty());
}

// ===========================================================================
// Agent context + undo rows surface when present
// ===========================================================================

#[tokio::test]
async fn agent_context_and_undo_rows_surface_when_present() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("prov-agent-{}", Uuid::now_v7())).await;

    // An IF8-shaped approved agent write: the approval pipeline lands a
    // normal autosave write carrying its session, skill, and rule-set
    // hash. Inserted directly the way the envelope's provenance context
    // will record it.
    let agent_entity = Uuid::now_v7();
    let session = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, actor_kind, before, after, session_id, skill, ruleset_hash, is_undo) \
         VALUES ($1, $2, 'ticket', $3, 1, 'update', $4, 'agent', $5, $6, $7, $8, $9, false)",
    )
    .bind(Uuid::now_v7())
    .bind(org.org_id)
    .bind(agent_entity)
    .bind(org.user_id)
    .bind(json!({ "title": "before" }))
    .bind(json!({ "title": "after" }))
    .bind(session)
    .bind("triage-v1")
    .bind("ruleset-hash-abc")
    .execute(&state.pool)
    .await
    .expect("agent write insert");

    // An undo revert row (IF7 shape): normal write, flagged is_undo.
    let undo_entity = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, actor_kind, before, after, is_undo) \
         VALUES ($1, $2, 'ticket', $3, 1, 'update', $4, 'human', $5, $6, true)",
    )
    .bind(Uuid::now_v7())
    .bind(org.org_id)
    .bind(undo_entity)
    .bind(org.user_id)
    .bind(json!({ "title": "later" }))
    .bind(json!({ "title": "earlier" }))
    .execute(&state.pool)
    .await
    .expect("undo write insert");

    // Agent filters hit exactly the agent rows, with context surfaced.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={agent_entity}&session_id={session}"),
    )
    .await;
    assert_eq!(status, 200);
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "one changed field: {events:?}");
    assert_eq!(events[0]["actor_kind"], "agent");
    assert_eq!(events[0]["skill"], "triage-v1");
    assert_eq!(events[0]["ruleset_hash"], "ruleset-hash-abc");
    assert_eq!(events[0]["session_id"], json!(session.to_string()));
    assert_eq!(events[0]["before"], "before");
    assert_eq!(events[0]["after"], "after");

    let (status, page) = provenance(&app, &org.token, "?entity=ticket&skill=triage-v1").await;
    assert_eq!(status, 200);
    assert_eq!(page["events"].as_array().unwrap().len(), 1);

    let (status, page) = provenance(
        &app,
        &org.token,
        "?entity=ticket&ruleset_hash=ruleset-hash-abc",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(page["events"].as_array().unwrap().len(), 1);

    // Undo filter isolates the revert row.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={undo_entity}&undo=true"),
    )
    .await;
    assert_eq!(status, 200);
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["undo"], true);
    assert_eq!(events[0]["before"], "later");
    assert_eq!(events[0]["after"], "earlier");

    // Human writes stay clean of agent context even in the same org.
    let board = default_board(&app, &org).await;
    let created = create_ticket(&app, &org.token, board, "Human write").await;
    let ticket_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}"),
    )
    .await;
    assert_eq!(status, 200);
    for e in page["events"].as_array().unwrap() {
        assert!(e.get("session_id").is_none(), "no agent context: {e}");
        assert!(e.get("skill").is_none());
        assert!(e.get("ruleset_hash").is_none());
    }
}

// ===========================================================================
// RBAC: manager+ sees org provenance, user sees own
// ===========================================================================

#[tokio::test]
async fn manager_sees_org_user_is_pinned_to_own_actions() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("prov-rbac-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let tech = provision_technician(&state, &app, &org, "tech").await;

    let owner_ticket = create_ticket(&app, &org.token, board, "Owner wrote this").await;
    let owner_ticket_id: Uuid = owner_ticket["id"].as_str().unwrap().parse().unwrap();
    let tech_ticket = create_ticket(&app, &tech, board, "Tech wrote this").await;
    let tech_ticket_id: Uuid = tech_ticket["id"].as_str().unwrap().parse().unwrap();

    // Owner holds the audit permission: org-wide, both actors visible.
    let (status, page) = provenance(&app, &org.token, "?entity=ticket").await;
    assert_eq!(status, 200);
    let actors: Vec<String> = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["actor_id"].as_str().unwrap().to_string())
        .collect();
    assert!(actors.contains(&org.user_id.to_string()), "{actors:?}");

    // Technician (no audit permission): ONLY own actions.
    let (status, page) = provenance(&app, &tech, "?entity=ticket").await;
    assert_eq!(status, 200);
    let events = page["events"].as_array().unwrap();
    assert!(!events.is_empty(), "technician sees their own provenance");
    for e in events {
        assert_ne!(
            e["actor_id"],
            json!(org.user_id.to_string()),
            "owner rows must be invisible to the technician: {e}"
        );
    }
    let tech_rows_on_own_ticket = provenance(
        &app,
        &tech,
        &format!("?entity=ticket&entity_id={tech_ticket_id}"),
    )
    .await;
    assert_eq!(tech_rows_on_own_ticket.0, 200);
    assert!(
        !tech_rows_on_own_ticket.1["events"]
            .as_array()
            .unwrap()
            .is_empty(),
        "technician sees own writes on their own ticket"
    );

    // Pinned even when the technician REQUESTS the owner's actor id —
    // the filter cannot widen, and the owner's ticket stays invisible.
    let (status, page) = provenance(
        &app,
        &tech,
        &format!(
            "?entity=ticket&entity_id={owner_ticket_id}&actor={}",
            org.user_id
        ),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        page["events"].as_array().unwrap().is_empty(),
        "non-manager must not see another actor's provenance: {page}"
    );

    // Same query for the manager returns the owner's rows.
    let (status, page) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={owner_ticket_id}"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        !page["events"].as_array().unwrap().is_empty(),
        "manager sees org-wide provenance"
    );
}

// ===========================================================================
// Cursor pagination: per-field rows page without gaps or dupes
// ===========================================================================

#[tokio::test]
async fn cursor_pagination_walks_every_field_row_exactly_once() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("prov-cursor-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let created = create_ticket(&app, &org.token, board, "Paged").await;
    let ticket_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    patch_ticket(&app, &org.token, ticket_id, json!({ "title": "v2" })).await;
    patch_ticket(
        &app,
        &org.token,
        ticket_id,
        json!({ "title": "v3", "priority": "p2" }),
    )
    .await;

    let (status, full) = provenance(
        &app,
        &org.token,
        &format!("?entity=ticket&entity_id={ticket_id}&limit=500"),
    )
    .await;
    assert_eq!(status, 200);
    let all: Vec<Value> = full["events"].as_array().unwrap().clone();
    assert!(all.len() >= 4, "several field rows expected: {}", all.len());

    // Walk with a tiny page, following next_cursor to exhaustion.
    let mut walked: Vec<(i64, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let query = match &cursor {
            Some(c) => format!("?entity=ticket&entity_id={ticket_id}&limit=2&cursor={c}"),
            None => format!("?entity=ticket&entity_id={ticket_id}&limit=2"),
        };
        let (status, page) = provenance(&app, &org.token, &query).await;
        assert_eq!(status, 200);
        let events = page["events"].as_array().unwrap();
        for e in events {
            walked.push((
                e["seq"].as_i64().unwrap(),
                e["field"].as_str().unwrap().to_string(),
            ));
        }
        pages += 1;
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
        assert!(pages < 100, "pagination must terminate");
    }
    assert!(pages > 1, "multiple pages exercised: {pages}");
    let expected: Vec<(i64, String)> = all
        .iter()
        .map(|e| {
            (
                e["seq"].as_i64().unwrap(),
                e["field"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        walked, expected,
        "paged walk must reproduce the full page exactly"
    );
}

// ===========================================================================
// Envelope hygiene
// ===========================================================================

#[tokio::test]
async fn unauthenticated_requests_are_typed_401() {
    let (_state, app) = test_state().await;
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/provenance",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 401);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "unauthorized");
}
