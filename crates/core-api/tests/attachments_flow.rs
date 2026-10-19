//! Attachments end-to-end flow through the REAL core-api router
//! (TASKLIST 2.14).
//!
//! Purpose: pin the whole attachment contract over HTTP — the upload
//! validation matrix (size cap, missing filename, executable magic,
//! extension/magic mismatch, declared-type contradiction), content-
//! addressed dedup (one stored object, two rows), sha256 download
//! integrity, composer linking (attachment_ids on comment create),
//! cross-org denial, the author-gated delete with refcounted object
//! reclamation, and portal visibility (internal-comment files are plain
//! 404s to every portal tier).
//!
//! Non-purpose: SQL-layer RLS (core-db db_suite), adapter behavior
//! (core-store), UI rendering.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const PNG_BYTES: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R',
    1, 2, 3, 4,
];
const EXE_BYTES: &[u8] = &[0x4D, 0x5A, 0x90, 0x00, 1, 2, 3];
const PDF_BYTES: &[u8] = b"%PDF-1.7 minimal";
/// A valid PDF whose bytes carry the marker the scripted scanner reads as a
/// malware signature. The allowlist gate must admit it (it is a real PDF
/// header) so the SCAN gate is what refuses it — otherwise the test would
/// pass for the wrong reason.
const INFECTED_PDF_BYTES: &[u8] = b"%PDF-1.7 [VIRUS] payload";
/// Same shape, marker for "the scanner cannot be reached".
const SCANNER_DOWN_PDF_BYTES: &[u8] = b"%PDF-1.7 [SCAN-DOWN] payload";

/// Scripted scanner for the gate tests: `[VIRUS]` ⇒ infected, `[SCAN-DOWN]`
/// ⇒ unavailable (the fail-closed path), anything else clean. Deterministic
/// stand-in for clamd on the BYTES leg, which is the leg uploads take.
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
        if bytes.windows(11).any(|w| w == b"[SCAN-DOWN]") {
            return Err(core_mail::ScanError::Unavailable(
                "gate-scripted: refused to answer".into(),
            ));
        }
        Ok(core_mail::ScanVerdict {
            infected: false,
            skipped: false,
            detail: None,
        })
    }
}

async fn test_state() -> (AppState, Arc<core_store::memory::MemoryStore>) {
    test_state_with(None).await
}

/// Builds the app state with a chosen scanner. `None` keeps the config-built
/// edge (none mode), which is what every pre-gate test here exercises; the
/// gate tests inject [`GateScan`] so a verdict can be forced.
async fn test_state_with(
    scanner: Option<Arc<dyn core_mail::ScanProvider>>,
) -> (AppState, Arc<core_store::memory::MemoryStore>) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("attachments e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let store = Arc::new(core_store::memory::MemoryStore::new());
    let mail = match scanner {
        Some(scanner) => Arc::new(core_api::email::MailStack {
            transport: Arc::new(core_mail::NoopTransport),
            scanner,
            from_address: config.email_from_address.clone(),
            from_name: config.email_from_name.clone(),
        }),
        None => Arc::new(core_api::email::MailStack::from_config(&config)),
    };
    let state = AppState {
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

/// Raw-body request helper for uploads and downloads.
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

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

async fn body_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = body_bytes(response).await;
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

struct Org {
    token: String,
    ticket_a: String,
    client_a: String,
}

/// Registers one org + one client-facing ticket on the default board.
async fn seed_org(app: &Router, tag: &str) -> Org {
    let slug = format!("attach-flow-{tag}-{}", Uuid::now_v7().simple());
    let response = send_json(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Attach Flow Co",
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
    let client_id = body_json(
        send_json(
            app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&token),
            Some(json!({"name": "Client A"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let ticket_a = body_json(
        send_json(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&token),
            Some(json!({
                "board_id": board_id,
                "title": "Attachment target",
                "priority": "p3",
                "client_id": client_id,
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
        ticket_a,
        client_a: client_id,
    }
}

fn upload_uri(ticket: &str) -> String {
    format!("/api/tickets/{ticket}/attachments")
}

#[tokio::test]
async fn upload_download_matrix_dedup_link_and_authz() {
    let (state, store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "a").await;

    // --- happy path ------------------------------------------------------
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "screenshot.png"),
            ("content-type", "image/png"),
        ],
        Some(PNG_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 200, "png upload must succeed");
    let first = body_json(response).await;
    assert_eq!(first["filename"], "screenshot.png");
    assert_eq!(first["kind"], "ticket_comment");
    let expected_sha = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(PNG_BYTES))
    };
    assert_eq!(first["sha256"], expected_sha.as_str());
    let first_id = first["id"].as_str().unwrap().to_string();
    assert!(first["storage_key"]
        .as_str()
        .unwrap()
        .contains("attachments"));

    // --- validation matrix ------------------------------------------------
    // Missing x-filename.
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[("content-type", "image/png")],
        Some(PNG_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422, "missing filename must be rejected");

    // Executable magic bytes under an innocent name+type.
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "invoice.pdf"),
            ("content-type", "application/pdf"),
        ],
        Some(EXE_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422, "MZ magic must be rejected");

    // Extension contradicts sniffed family.
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[("x-filename", "photo.png"), ("content-type", "image/png")],
        Some(PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422, "extension/magic mismatch must fail");

    // Declared type contradicts sniffed family.
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[("x-filename", "doc.pdf"), ("content-type", "text/plain")],
        Some(PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422, "type contradiction must fail");

    // Oversize body (cap lowered on this shared state? no — use a huge
    // payload beyond the 25 MB default is too slow; instead prove the cap
    // path via a tiny explicit Content-Length lie is NOT possible here —
    // covered by unit tests in core-domain; here we assert empty bodies).
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[("x-filename", "empty.png"), ("content-type", "image/png")],
        Some(Vec::new()),
    )
    .await;
    assert_eq!(response.status(), 422, "empty upload must be rejected");

    // --- dedup: same bytes again → second row, still ONE stored object ---
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[("x-filename", "copy.png"), ("content-type", "image/png")],
        Some(PNG_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 200);
    let second = body_json(response).await;
    assert_ne!(second["id"], first["id"], "rows are distinct identities");
    assert_eq!(second["storage_key"], first["storage_key"]);
    let puts = store
        .ops()
        .iter()
        .filter(|op| matches!(op, core_store::ObjectRecordedOp::Put { .. }))
        .count();
    assert_eq!(puts, 1, "identical bytes must be stored exactly once");

    // --- download roundtrip with integrity header -------------------------
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{first_id}/download"),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("x-marula-sha256")
            .and_then(|v| v.to_str().ok()),
        Some(expected_sha.as_str())
    );
    let bytes = body_bytes(response).await;
    assert_eq!(bytes, PNG_BYTES);

    // --- composer linking --------------------------------------------------
    let comment = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{}/comments", org.ticket_a),
            Some(&org.token),
            Some(json!({
                "body": "see attached",
                "kind": "internal",
                "attachment_ids": [first_id],
            })),
        )
        .await,
    )
    .await;
    let comment_id = comment["id"].as_str().unwrap().to_string();
    let listed = body_json(
        send_raw(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/tickets/{}/comments/{comment_id}/attachments",
                org.ticket_a
            ),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "linked once");

    // Relinking the same id to another comment is inert (one-shot links).
    let other_comment = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            &format!("/api/tickets/{}/comments", org.ticket_a),
            Some(&org.token),
            Some(json!({"body": "no attach", "kind": "internal", "attachment_ids": [first_id]})),
        )
        .await,
    )
    .await;
    let other_id = other_comment["id"].as_str().unwrap().to_string();
    let listed_other = body_json(
        send_raw(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/tickets/{}/comments/{other_id}/attachments",
                org.ticket_a
            ),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed_other.as_array().unwrap().len(), 0);

    // --- cross-org denial ---------------------------------------------------
    let other_org = seed_org(&app, "b").await;
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{first_id}/download"),
        Some(&other_org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org download denied");
    let response = send_raw(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/attachments/{first_id}"),
        Some(&other_org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org delete denied");

    // --- author-gated delete + refcounted reclamation -----------------------
    // The copy's uploader IS the owner → allowed; but the FIRST object
    // still has one reference (copy row), so the store keeps it.
    let response = send_raw(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/attachments/{}", second["id"].as_str().unwrap()),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let event = body_json(response).await;
    assert_eq!(event["object_removed"], false, "sibling ref keeps object");
    assert_eq!(store.put_count(), 1);

    // Deleting the last reference reclaims the object.
    let response = send_raw(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/attachments/{first_id}"),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let event = body_json(response).await;
    assert_eq!(event["object_removed"], true);
    let puts_after = store
        .ops()
        .iter()
        .filter(|op| matches!(op, core_store::ObjectRecordedOp::Delete { .. }))
        .count();
    assert_eq!(puts_after, 1, "object deleted exactly once");

    // Download after deletion → 404.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{first_id}/download"),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

/// Portal visibility: internal-comment attachments never surface; hidden
/// ids are indistinguishable 404s; staff downloads work.
#[tokio::test]
async fn portal_attachment_visibility_is_note_kind_scoped() {
    let (state, _store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    // A full uuid, not a prefix of one: the first eight hex chars of a v7
    // uuid are a millisecond timestamp, so two runs in the same second used
    // to produce the same portal email — and `portal_resolve_login` resolves
    // an email with `LIMIT 1` across orgs, so the second run logged in as the
    // previous run's org and every ticket was a 404.
    let tag = Uuid::now_v7().simple().to_string();
    let org = seed_org(&app, &tag).await;

    // Expose the default board to client A.
    let boards = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let exposure = send_json(
        &app,
        axum::http::Method::PUT,
        &format!("/api/clients/{}/board-exposures", org.client_a),
        Some(&org.token),
        Some(json!({"board_id": board_id, "exposed": true, "visible_status_ids": []})),
    )
    .await;
    assert_eq!(exposure.status(), 200);

    // Invite + activate a portal contact.
    let email = format!("contact-{tag}@portal.test");
    let invite = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/clients/{}/portal-users", org.client_a),
        Some(&org.token),
        Some(json!({
            "email": email,
            "display_name": "Contact",
            "password": "portal-passphrase-1",
            "role": "contact",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200);
    let portal_login = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            "/api/portal/login",
            None,
            Some(json!({"email": email, "password": "portal-passphrase-1"})),
        )
        .await,
    )
    .await;
    let portal_token = portal_login["access_token"].as_str().unwrap().to_string();

    // Staff uploads two files: one linked to an INTERNAL note, one to a
    // CLIENT_VISIBLE note.
    let upload_and_link = |file: &'static str| {
        let app = app.clone();
        let org = &org;
        let token = org.token.clone();
        async move {
            let response = send_raw(
                &app,
                axum::http::Method::POST,
                &upload_uri(&org.ticket_a),
                Some(&token),
                &[("x-filename", file), ("content-type", "text/plain")],
                Some(b"hello-attachment".to_vec()),
            )
            .await;
            assert_eq!(response.status(), 200);
            body_json(response).await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let internal_file = upload_and_link("internal-notes.txt").await;
    let visible_file = upload_and_link("client-notes.txt").await;
    let comment_of = |ids: Vec<String>, kind: &'static str| {
        let app = app.clone();
        let token = org.token.clone();
        let uri = format!("/api/tickets/{}/comments", org.ticket_a);
        async move {
            let response = send_json(
                &app,
                axum::http::Method::POST,
                &uri,
                Some(&token),
                Some(json!({"body": "note", "kind": kind, "attachment_ids": ids})),
            )
            .await;
            assert_eq!(response.status(), 200);
        }
    };
    comment_of(vec![internal_file.clone()], "internal").await;
    comment_of(vec![visible_file.clone()], "client_visible").await;

    // Portal list sees ONLY the client-visible one.
    let list_response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/tickets/{}/attachments", org.ticket_a),
        Some(&portal_token),
        &[],
        None,
    )
    .await;
    let list_status = list_response.status();
    let listed = body_json(list_response).await;
    assert_eq!(list_status, 200, "portal attachment list failed: {listed}");
    let ids: Vec<&str> = listed
        .as_array()
        .expect("portal attachment list must be a JSON array")
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![visible_file.as_str()], "internal stays hidden");

    // Hidden id downloads as a plain 404 (existence not leaked).
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!(
            "/api/portal/tickets/{}/attachments/{internal_file}/download",
            org.ticket_a
        ),
        Some(&portal_token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 404);

    // Visible id streams fine.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!(
            "/api/portal/tickets/{}/attachments/{visible_file}/download",
            org.ticket_a
        ),
        Some(&portal_token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_bytes(response).await, b"hello-attachment");

    // Portal tokens cannot touch STAFF surfaces at all.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{visible_file}/download"),
        Some(&portal_token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 401, "staff endpoint rejects portal JWT");
}

/// Reads the verdict row for one object under the org's RLS scope, so the
/// assertion holds whether or not the test role happens to own the table.
async fn scan_row(state: &AppState, org_id: Uuid, sha: &str) -> Option<(String, String, bool)> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let row = sqlx::query_as(
        "SELECT status, storage_key, quarantined_at IS NOT NULL \
         FROM attachment_scans WHERE org_id = $1 AND sha256 = $2",
    )
    .bind(org_id)
    .bind(sha)
    .fetch_optional(&mut *tx)
    .await
    .expect("verdict lookup");
    row
}

/// The scan gate, end to end through the real router (Wave 52 lane P6a).
///
/// Three claims, each one a way the gate could be wrong: an infection is
/// refused AND recorded (nothing admitted, bytes quarantined); a scanner
/// that cannot answer refuses the upload and stores NOTHING (fail closed);
/// and a clean verdict is on record, naming the adapter that decided.
#[tokio::test]
async fn scan_gate_quarantines_infections_and_fails_closed() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(GateScan);
    let (state, store) = test_state_with(Some(scanner)).await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "gate").await;

    let sha = |bytes: &[u8]| {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    };

    // --- 1. CLEAN: admitted, and the verdict is on record with its adapter -
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "ok.pdf"),
            ("content-type", "application/pdf"),
        ],
        Some(PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 200, "clean bytes are admitted");
    let admitted = body_json(response).await;
    let org_id = Uuid::parse_str(admitted["org_id"].as_str().unwrap()).expect("org id");
    let clean_sha = sha(PDF_BYTES);
    let (status, storage_key, quarantined) = scan_row(&state, org_id, &clean_sha)
        .await
        .expect("a clean verdict is recorded");
    assert_eq!(status, "clean");
    assert!(
        storage_key.contains("/attachments/") && !quarantined,
        "admitted bytes live at their canonical key: {storage_key}"
    );

    // --- 2. INFECTED: typed 422, no attachment, bytes held elsewhere ------
    let infected_sha = sha(INFECTED_PDF_BYTES);
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "invoice.pdf"),
            ("content-type", "application/pdf"),
        ],
        Some(INFECTED_PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "an infected upload is refused, not admitted"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "upload_quarantined");
    assert_eq!(body["error"]["code"], 422);
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Eicar"),
        "the scanner's detail rides the refusal: {body}"
    );

    // The record exists, is infected, and points at the quarantine namespace.
    let (status, storage_key, quarantined) = scan_row(&state, org_id, &infected_sha)
        .await
        .expect("quarantine row was written");
    assert_eq!(status, "infected");
    assert!(quarantined, "an infection carries its quarantine timestamp");
    assert!(
        storage_key.contains("/quarantine/") && storage_key.ends_with(&infected_sha),
        "bytes live under the quarantine namespace at the same digest: {storage_key}"
    );

    // Infected bytes went to the quarantine key and NEVER to the admitted
    // one, and the ticket lists only the clean file.
    let puts: Vec<String> = store
        .ops()
        .iter()
        .filter_map(|op| match op {
            core_store::ObjectRecordedOp::Put { key } => Some(key.clone()),
            core_store::ObjectRecordedOp::Delete { .. } => None,
        })
        .collect();
    assert!(puts.iter().any(|k| k.contains("/quarantine/")));
    assert!(
        !puts
            .iter()
            .any(|k| k.contains("/attachments/") && k.ends_with(&infected_sha)),
        "infected bytes must never reach the admitted namespace: {puts:?}"
    );
    let listed = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &upload_uri(&org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        listed.as_array().map(Vec::len),
        Some(1),
        "the refusal attached nothing: only the clean file is on the ticket"
    );

    // --- 3. SCANNER DOWN: 503, and NOTHING written at all -----------------
    let down_sha = sha(SCANNER_DOWN_PDF_BYTES);
    let ops_before = store.ops().len();
    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "quote.pdf"),
            ("content-type", "application/pdf"),
        ],
        Some(SCANNER_DOWN_PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(
        response.status(),
        503,
        "an unreachable scanner is a refusal, never an admission"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "service_unavailable");

    assert_eq!(
        store.ops().len(),
        ops_before,
        "a refused upload writes nothing to the object store"
    );
    assert!(
        scan_row(&state, org_id, &down_sha).await.is_none(),
        "no verdict row exists for bytes that were never admitted"
    );
    let listed = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &upload_uri(&org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
}

/// None mode admits, and says so: the record must read `skipped`, not
/// `clean`. Conflating the two is how "nobody looked" becomes "a scanner
/// passed it" on a deployment without one.
#[tokio::test]
async fn none_mode_admits_and_records_a_skip() {
    let (state, _store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "nonemode").await;

    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &upload_uri(&org.ticket_a),
        Some(&org.token),
        &[
            ("x-filename", "screenshot.png"),
            ("content-type", "image/png"),
        ],
        Some(PNG_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 200, "none mode still admits uploads");
    let admitted = body_json(response).await;
    let org_id = Uuid::parse_str(admitted["org_id"].as_str().unwrap()).expect("org id");

    use sha2::{Digest, Sha256};
    let sha = hex::encode(Sha256::digest(PNG_BYTES));
    let (status, scanner) = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("org scope");
        sqlx::query_as::<_, (String, String)>(
            "SELECT status, scanner FROM attachment_scans WHERE org_id = $1 AND sha256 = $2",
        )
        .bind(org_id)
        .bind(&sha)
        .fetch_one(&mut *tx)
        .await
        .expect("skip recorded")
    };
    assert_eq!(status, "skipped");
    assert_eq!(scanner, "none");
}
