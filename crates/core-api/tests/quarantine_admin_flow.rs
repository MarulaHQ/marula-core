//! Quarantine admin flow through the REAL core-api router (Wave 52 lane P6c).
//!
//! Purpose: pin the operator surface over `attachment_scans` end to end —
//! the list (pending and released together), inspection by digest, the
//! release override with its required reason, the byte move out of the
//! quarantine namespace, the audit row that records actor/reason/timestamp,
//! release idempotency, the delete path and its tombstone rule, permission
//! denial for a technician, and org isolation.
//!
//! Non-purpose: scan-gate behavior on the upload path (P6a's
//! `attachments_flow`), SQL-layer RLS (core-db `db_suite`), UI rendering.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::ObjectStore;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Two valid PDFs whose bytes carry the marker the scripted scanner reads as
/// a malware signature. The allowlist gate must admit them (real PDF header)
/// so the SCAN gate is what refuses them — otherwise the fixture would pass
/// for the wrong reason.
const INFECTED_ONE: &[u8] = b"%PDF-1.7 [VIRUS] alpha payload";
const INFECTED_TWO: &[u8] = b"%PDF-1.7 [VIRUS] beta payload";

struct GateScan;

#[async_trait::async_trait]
impl core_mail::ScanProvider for GateScan {
    fn name(&self) -> &'static str {
        "gate-scripted"
    }

    async fn scan(&self, body_text: &str) -> Result<core_mail::ScanVerdict, core_mail::ScanError> {
        self.scan_bytes(body_text.as_bytes()).await
    }

    async fn scan_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<core_mail::ScanVerdict, core_mail::ScanError> {
        if bytes.windows(7).any(|w| w == b"[VIRUS]") {
            return Ok(core_mail::ScanVerdict {
                infected: true,
                skipped: false,
                detail: Some("stream: Eicar-Test-Signature FOUND".into()),
            });
        }
        Ok(core_mail::ScanVerdict {
            infected: false,
            skipped: false,
            detail: None,
        })
    }
}

async fn test_state() -> (AppState, Arc<core_store::memory::MemoryStore>) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("quarantine admin e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let store = Arc::new(core_store::memory::MemoryStore::new());
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(GateScan);
    let mail = Arc::new(core_api::email::MailStack {
        transport: Arc::new(core_mail::NoopTransport),
        scanner,
        from_address: config.email_from_address.clone(),
        from_name: config.email_from_name.clone(),
    });
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail,
        store: std::sync::Arc::clone(&store) as Arc<dyn core_store::ObjectStore>,
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    (state, store)
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send_raw(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let request = match body {
        Some(bytes) => builder
            .header("content-type", "application/octet-stream")
            .body(axum::body::Body::from(bytes))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn send_json(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

struct Org {
    token: String,
    org_id: Uuid,
    ticket: String,
}

/// Registers one org and one ticket on its default board.
async fn seed_org(app: &Router, tag: &str) -> Org {
    let slug = format!("quarantine-{tag}-{}", Uuid::now_v7().simple());
    let response = send_json(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Quarantine Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let me =
        body_json(send_json(app, axum::http::Method::GET, "/api/me", Some(&token), None).await)
            .await;
    let org_id = Uuid::parse_str(me["org_id"].as_str().unwrap()).expect("org id");

    let boards = body_json(
        send_json(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let ticket = body_json(
        send_json(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&token),
            Some(json!({
                "board_id": board_id,
                "title": "Quarantine target",
                "priority": "p3",
            })),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    Org {
        token,
        org_id,
        ticket,
    }
}

/// Creates a second user with the builtin technician role and logs in over
/// HTTP. Technicians hold no Admin matrix grants, so every route here denies.
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid =
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
    core_db::users::assign_role(&mut *tx, user_id, role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send_json(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": "tech-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Uploads bytes that the scripted scanner will refuse, and returns the
/// digest the quarantine row is keyed by.
async fn upload_infected(app: &Router, org: &Org, filename: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let response = send_raw(
        app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/attachments", org.ticket),
        Some(&org.token),
        &[
            ("x-filename", filename),
            ("content-type", "application/pdf"),
        ],
        Some(bytes.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422, "an infected upload is refused");
    hex::encode(Sha256::digest(bytes))
}

/// One audit row for one object: action, actor id, detail, and when.
struct AuditRow {
    action: String,
    actor: String,
    detail: Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

/// Every audit row for one object, newest first, read under the org scope so
/// the assertion does not depend on the test role owning the table.
async fn audit_rows(state: &AppState, org_id: Uuid, digest: &str) -> Vec<AuditRow> {
    use sqlx::Row;
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let rows = sqlx::query(
        "SELECT action, actor_id, detail, occurred_at FROM audit_events \
         WHERE org_id = $1 AND entity_id = $2 ORDER BY occurred_at DESC",
    )
    .bind(org_id)
    .bind(digest)
    .fetch_all(&mut *tx)
    .await
    .expect("audit lookup");
    rows.into_iter()
        .map(|r| AuditRow {
            action: r.get("action"),
            actor: r.get::<Option<String>, _>("actor_id").unwrap_or_default(),
            detail: r.get("detail"),
            occurred_at: r.get("occurred_at"),
        })
        .collect()
}

async fn scan_storage_key(state: &AppState, org_id: Uuid, digest: &str) -> Option<String> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query_scalar("SELECT storage_key FROM attachment_scans WHERE org_id = $1 AND sha256 = $2")
        .bind(org_id)
        .bind(digest)
        .fetch_optional(&mut *tx)
        .await
        .expect("scan lookup")
}

#[tokio::test]
async fn quarantine_admin_list_inspect_release_and_delete() {
    let (state, store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "a").await;
    let other = seed_org(&app, "b").await;

    let digest = upload_infected(&app, &org, "invoice.pdf", INFECTED_ONE).await;
    let digest_two = upload_infected(&app, &org, "statement.pdf", INFECTED_TWO).await;

    // --- list -------------------------------------------------------------
    let page = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/admin/quarantine",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(page["pending"], 2, "two held objects: {page}");
    assert_eq!(page["released"], 0);
    let items = page["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2, "newest first, both listed: {page}");
    let first = items
        .iter()
        .find(|i| i["sha256"] == digest.as_str())
        .expect("the first object is listed");
    assert_eq!(first["filename"], "invoice.pdf");
    assert_eq!(first["content_type"], "application/pdf");
    assert_eq!(first["scanner"], "gate-scripted");
    assert_eq!(first["state"], "pending");
    assert!(first["quarantined_at"].is_string());
    assert_eq!(
        first["ticket_id"].as_str(),
        Some(org.ticket.as_str()),
        "the ticket the upload was aimed at rides the row"
    );
    assert!(first["uploaded_by_user_id"].is_string());
    assert_eq!(first["released_at"], Value::Null);

    // --- inspect ----------------------------------------------------------
    let one = send_json(
        &app,
        axum::http::Method::GET,
        &format!("/api/admin/quarantine/{digest}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(one.status(), 200);
    let one = body_json(one).await;
    assert_eq!(one["sha256"], digest.as_str());
    assert!(
        one["detail"].as_str().unwrap_or_default().contains("Eicar"),
        "the scanner's own words are on the record: {one}"
    );
    assert!(
        one["storage_key"]
            .as_str()
            .unwrap_or_default()
            .contains("/quarantine/"),
        "held bytes point at the quarantine key: {one}"
    );

    // A digest nobody quarantined is a plain 404, and a malformed one is a
    // typed validation refusal rather than a query.
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/admin/quarantine/0000000000000000000000000000000000000000000000000000000000000000",
            Some(&org.token),
            None,
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/admin/quarantine/not-a-digest",
            Some(&org.token),
            None,
        )
        .await
        .status(),
        422
    );

    // --- release requires a reason ---------------------------------------
    let no_reason = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/admin/quarantine/{digest}/release"),
        Some(&org.token),
        Some(json!({ "reason": "   " })),
    )
    .await;
    assert_eq!(no_reason.status(), 422, "a blank reason is refused");
    assert_eq!(
        body_json(no_reason).await["error"]["reason"],
        "validation_failed"
    );
    assert!(
        scan_storage_key(&state, org.org_id, &digest)
            .await
            .expect("row still there")
            .contains("/quarantine/"),
        "a refused release moved nothing"
    );

    // --- release ----------------------------------------------------------
    let released = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/admin/quarantine/{digest}/release"),
        Some(&org.token),
        Some(json!({ "reason": "Vendor confirmed a false positive on ticket MAR-1" })),
    )
    .await;
    assert_eq!(released.status(), 200);
    let released = body_json(released).await;
    assert_eq!(released["state"], "released");
    assert_eq!(
        released["release_reason"],
        "Vendor confirmed a false positive on ticket MAR-1"
    );
    assert!(released["released_at"].is_string());
    assert!(released["released_by_user_id"].is_string());
    assert_eq!(
        released["storage_key"],
        format!("org/{}/attachments/{digest}", org.org_id),
        "the row points at the admitted key after release"
    );

    // The bytes moved: admitted key written, quarantine key deleted, and the
    // held copy is gone.
    let ops = store.ops();
    assert!(
        ops.iter().any(|op| matches!(
            op,
            core_store::ObjectRecordedOp::Put { key }
                if key.ends_with(&digest) && key.contains("/attachments/")
        )),
        "release writes the canonical copy: {ops:?}"
    );
    assert!(
        ops.iter().any(|op| matches!(
            op,
            core_store::ObjectRecordedOp::Delete { key }
                if key.ends_with(&digest) && key.contains("/quarantine/")
        )),
        "release removes the held copy: {ops:?}"
    );
    assert!(
        store
            .get(&core_store::quarantine_key(org.org_id, &digest))
            .await
            .is_err(),
        "the quarantined bytes are no longer readable"
    );

    // --- the audit row ----------------------------------------------------
    let rows = audit_rows(&state, org.org_id, &digest).await;
    assert_eq!(rows.len(), 1, "one release, one audit row");
    let row = &rows[0];
    assert_eq!(row.action, "attachment_scan.released");
    let me = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        row.actor,
        me["user_id"].as_str().unwrap(),
        "actor is the operator"
    );
    assert_eq!(
        row.detail["reason"],
        "Vendor confirmed a false positive on ticket MAR-1"
    );
    assert_eq!(row.detail["sha256"], digest.as_str());
    assert_eq!(row.detail["filename"], "invoice.pdf");
    assert!(row.detail["bytes_moved"].as_bool().unwrap_or(false));
    let age = chrono::Utc::now() - row.occurred_at;
    assert!(
        age.num_seconds().abs() < 300,
        "the audit row carries the moment it happened: {age}"
    );

    // --- release is idempotent -------------------------------------------
    let again = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/admin/quarantine/{digest}/release"),
        Some(&org.token),
        Some(json!({ "reason": "a second operator typing a different story" })),
    )
    .await;
    assert_eq!(again.status(), 200, "a repeat release is not an error");
    let again = body_json(again).await;
    assert_eq!(
        again["release_reason"], "Vendor confirmed a false positive on ticket MAR-1",
        "the first operator's reason stands"
    );
    assert_eq!(
        audit_rows(&state, org.org_id, &digest).await.len(),
        1,
        "a repeat release writes no second audit row"
    );

    // --- a released object is not deletable -------------------------------
    let denied_delete = send_json(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/admin/quarantine/{digest}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(denied_delete.status(), 409);
    assert_eq!(
        body_json(denied_delete).await["error"]["reason"],
        "conflict"
    );
    assert!(scan_storage_key(&state, org.org_id, &digest)
        .await
        .is_some());

    // --- delete removes the held bytes and the row ------------------------
    let deleted = send_json(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/admin/quarantine/{digest_two}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(deleted.status(), 204);
    assert!(
        store
            .get(&core_store::quarantine_key(org.org_id, &digest_two))
            .await
            .is_err(),
        "the held bytes are gone"
    );
    assert!(
        scan_storage_key(&state, org.org_id, &digest_two)
            .await
            .is_none(),
        "the row is a tombstone in the audit trail, not in this table"
    );
    let delete_rows = audit_rows(&state, org.org_id, &digest_two).await;
    assert_eq!(delete_rows.len(), 1, "the delete is audited");
    assert_eq!(delete_rows[0].action, "attachment_scan.deleted");
    assert_eq!(delete_rows[0].detail["filename"], "statement.pdf");
    assert_eq!(
        delete_rows[0].detail["scanner_detail"], "stream: Eicar-Test-Signature FOUND",
        "the signature stays in the trail after the bytes go"
    );
    assert!(
        store
            .get(&core_store::attachment_key(org.org_id, &digest_two))
            .await
            .is_err(),
        "deleting a held object never admits it"
    );

    // A second delete finds nothing and says so.
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::DELETE,
            &format!("/api/admin/quarantine/{digest_two}"),
            Some(&org.token),
            None,
        )
        .await
        .status(),
        404
    );

    // The list still shows the release and drops the deleted object.
    let after = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/admin/quarantine",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(after["pending"], 0);
    assert_eq!(after["released"], 1);
    assert_eq!(after["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(after["items"][0]["sha256"], digest.as_str());
    assert_eq!(after["items"][0]["state"], "released");

    // --- org isolation ----------------------------------------------------
    let other_page = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/admin/quarantine",
            Some(&other.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        other_page["items"].as_array().map(Vec::len),
        Some(0),
        "another org sees none of it: {other_page}"
    );
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/admin/quarantine/{digest}"),
            Some(&other.token),
            None,
        )
        .await
        .status(),
        404,
        "inspection is org-scoped"
    );
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::POST,
            &format!("/api/admin/quarantine/{digest}/release"),
            Some(&other.token),
            Some(json!({ "reason": "cross-org attempt" })),
        )
        .await
        .status(),
        404,
        "release is org-scoped"
    );

    // --- permission denial ------------------------------------------------
    let tech = provision_technician(
        &state,
        &app,
        org.org_id,
        &format!("tech-{}@quarantine.test", Uuid::now_v7().simple()),
    )
    .await;
    for (method, uri) in [
        (axum::http::Method::GET, "/api/admin/quarantine".to_string()),
        (
            axum::http::Method::GET,
            format!("/api/admin/quarantine/{digest}"),
        ),
    ] {
        let response = send_json(&app, method, &uri, Some(&tech), None).await;
        assert_eq!(response.status(), 403, "reads need Admin:View: {uri}");
        assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    }
    assert_eq!(
        send_json(
            &app,
            axum::http::Method::POST,
            &format!("/api/admin/quarantine/{digest}/release"),
            Some(&tech),
            Some(json!({ "reason": "technician override" })),
        )
        .await
        .status(),
        403,
        "release needs Admin:Edit"
    );
}
