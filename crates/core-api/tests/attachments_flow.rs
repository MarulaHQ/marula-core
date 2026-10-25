//! Attachments end-to-end flow through the REAL core-api router
//! (the private build tracker 2.14).
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
//! The scan gate's outcome is pinned here too (Wave 52 lanes P6a and P6b),
//! across all three doors that store uploaded bytes: the ticket attachment,
//! the document attachment, and the drawn sign-off mark. Each refusal must
//! carry the typed 422 fields, leave its own row unwritten, hold the bytes
//! under the quarantine key, and tell the uploader through the notification
//! center.
//!
//! Non-purpose: SQL-layer RLS (core-db db_suite), adapter behavior
//! (core-store), UI rendering.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::ObjectStore as _;
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
/// A drawn mark that is a real PNG by its magic bytes and carries the
/// signature marker — the sign-off door's version of the same trap.
const INFECTED_PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR[VIRUS]";

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
    assert_eq!(
        body["error"]["code"], 422,
        "the envelope shape is unchanged"
    );
    assert_eq!(
        body["error"]["sha256"], infected_sha,
        "the refusal names the object it held: {body}"
    );
    assert_eq!(
        body["error"]["filename"], "invoice.pdf",
        "the refusal names the file the uploader chose: {body}"
    );
    assert_eq!(
        body["error"]["scanner"], "gate-scripted",
        "the refusal names the adapter that decided: {body}"
    );
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Eicar"),
        "the scanner's detail rides the refusal: {body}"
    );
    assert_eq!(
        body["error"]["held_for_operator"], true,
        "a client must not offer `retry the same file` for held bytes: {body}"
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

    // The uploader was told, durably. The uploader here is also the org's
    // only owner, so the uploader leg and the owner/admin leg name the same
    // person: one row, not two.
    let notices = scan_notifications(&state, org_id).await;
    assert_eq!(
        notices.len(),
        1,
        "one person, one notification: {notices:?}"
    );
    let (title, notice_body, notice_ticket) = &notices[0];
    assert!(
        title.contains("held"),
        "the title names the outcome, not just the machinery: {title}"
    );
    let notice_text = notice_body.as_deref().unwrap_or_default();
    assert!(
        notice_text.contains("invoice.pdf"),
        "the body names the file that did not land: {notice_text}"
    );
    assert!(
        notice_text.contains("Eicar"),
        "the body names what the scanner found: {notice_text}"
    );
    assert_eq!(
        *notice_ticket,
        Some(Uuid::parse_str(&org.ticket_a).expect("ticket id")),
        "the notice points at the ticket the upload was aimed at"
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
    assert_eq!(
        scan_notifications(&state, org_id).await.len(),
        1,
        "a fail-closed refusal is not a quarantine: it holds nothing, so it notifies nobody new"
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

/// The scan-detection notifications one org holds, oldest first, as
/// (title, body, ticket anchor) — read under the org's RLS scope.
async fn scan_notifications(
    state: &AppState,
    org_id: Uuid,
) -> Vec<(String, Option<String>, Option<Uuid>)> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query_as(
        "SELECT title, body, ticket_id FROM notifications \
         WHERE org_id = $1 AND kind = 'scan_detection' ORDER BY created_at, id",
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await
    .expect("notification lookup")
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn sha_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// The org a ticket belongs to, read through the API: the test role may not
/// own the tables, so a bare SELECT would come back empty under RLS.
async fn org_of_ticket(app: &Router, token: &str, ticket_id: &str) -> Uuid {
    let view = body_json(
        send_json(
            app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(token),
            None,
        )
        .await,
    )
    .await;
    Uuid::parse_str(
        view["org_id"]
            .as_str()
            .unwrap_or_else(|| panic!("ticket view carries its org: {view}")),
    )
    .expect("org uuid")
}

/// The drawn mark is the third door that stores uploaded bytes, and the one
/// whose upload sits inside a consent flow (Wave 52 lane P6b): an infected
/// mark is held against the TICKET, the capture is refused with the typed
/// 422, nobody is told it was signed, and the consent path never runs. A
/// clean mark still captures exactly as it did before.
#[tokio::test]
async fn sign_off_gate_holds_an_infected_mark_and_captures_nothing() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(GateScan);
    let (state, store) = test_state_with(Some(scanner)).await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "signoff-gate").await;
    let sign_offs_uri = format!("/api/tickets/{}/sign-offs", org.ticket_a);
    let ticket_id = Uuid::parse_str(&org.ticket_a).expect("ticket id");
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;

    // --- 1. INFECTED: typed 422, held, no sign-off, one notice -----------
    let infected_sha = sha_of(INFECTED_PNG_BYTES);
    let response = send_json(
        &app,
        axum::http::Method::POST,
        &sign_offs_uri,
        Some(&org.token),
        Some(json!({
            "image_base64": b64(INFECTED_PNG_BYTES),
            "signer_name": "Casey Client",
            "captured_at": "2026-10-20T09:00:00Z",
            "consent_version": 1,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "an infected mark is refused, not captured"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "upload_quarantined");
    assert_eq!(body["error"]["sha256"], infected_sha);
    let minted = body["error"]["filename"]
        .as_str()
        .expect("the refusal names the payload file");
    assert!(
        minted.starts_with("signature-") && minted.ends_with(".png"),
        "the server minted the payload filename, and the refusal repeats it: {minted}"
    );
    assert_eq!(body["error"]["scanner"], "gate-scripted");
    assert!(body["error"]["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("Eicar"));
    assert_eq!(body["error"]["held_for_operator"], true);

    // No evidence row, because there was no capture.
    let listed = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &sign_offs_uri,
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        listed.as_array().map(Vec::len),
        Some(0),
        "a held mark is not a sign-off: {listed}"
    );

    // The verdict row names the ticket the mark was for, and the bytes live
    // under the quarantine namespace only.
    let (status, held_ticket) = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("org scope");
        sqlx::query_as::<_, (String, Option<Uuid>)>(
            "SELECT status, ticket_id FROM attachment_scans \
             WHERE org_id = $1 AND sha256 = $2",
        )
        .bind(org_id)
        .bind(&infected_sha)
        .fetch_one(&mut *tx)
        .await
        .expect("quarantine row")
    };
    assert_eq!(status, "infected");
    assert_eq!(
        held_ticket,
        Some(ticket_id),
        "the operator's list says which job the mark was for"
    );
    let puts: Vec<String> = store
        .ops()
        .iter()
        .filter_map(|op| match op {
            core_store::ObjectRecordedOp::Put { key } => Some(key.clone()),
            core_store::ObjectRecordedOp::Delete { .. } => None,
        })
        .collect();
    assert!(
        puts.iter().any(|k| k.contains("/quarantine/")),
        "the held mark kept its bytes: {puts:?}"
    );
    assert!(
        !puts
            .iter()
            .any(|k| k.contains("/attachments/") && k.ends_with(&infected_sha)),
        "the held mark must never reach the admitted key space: {puts:?}"
    );

    let notices = scan_notifications(&state, org_id).await;
    assert_eq!(
        notices.len(),
        1,
        "the tech who drew the mark was told once: {notices:?}"
    );
    let (_, notice_body, notice_ticket) = &notices[0];
    assert_eq!(*notice_ticket, Some(ticket_id));
    assert!(notice_body
        .as_deref()
        .unwrap_or_default()
        .contains("signature-"));

    // --- 2. CLEAN: the drawn path still captures, silently --------------
    let clean_sha = sha_of(PNG_BYTES);
    let response = send_json(
        &app,
        axum::http::Method::POST,
        &sign_offs_uri,
        Some(&org.token),
        Some(json!({
            "image_base64": b64(PNG_BYTES),
            "signer_name": "Casey Client",
            "captured_at": "2026-10-20T09:05:00Z",
            "consent_version": 1,
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    let status = response.status();
    let payload = body_json(response).await;
    assert_eq!(
        status, 200,
        "a clean mark captures exactly as before: {payload}"
    );
    let listed = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &sign_offs_uri,
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        listed.as_array().map(Vec::len),
        Some(1),
        "the clean capture landed: {listed}"
    );
    assert!(
        listed[0]["attachment_id"].is_string(),
        "a drawn mark still stores its payload row: {listed}"
    );
    assert_eq!(
        scan_row(&state, org_id, &clean_sha)
            .await
            .map(|(status, _, _)| status)
            .as_deref(),
        Some("clean"),
        "the admitted mark carries its verdict"
    );
    assert_eq!(
        scan_notifications(&state, org_id).await.len(),
        1,
        "a clean capture notifies nobody"
    );
}

/// The same held file, uploaded twice. The verdict row keeps its first
/// record (P6a's first-wins rule), and the first-wins rule must not turn the
/// second attempt into a different answer: it is the same typed refusal, and
/// the operator sees that somebody tried again.
#[tokio::test]
async fn repeat_upload_of_a_held_file_keeps_the_typed_refusal() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(GateScan);
    let (state, store) = test_state_with(Some(scanner)).await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "repeat-gate").await;
    let infected_sha = sha_of(INFECTED_PDF_BYTES);

    let mut answers = Vec::new();
    for _ in 0..2 {
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
        answers.push((response.status(), body_json(response).await));
    }
    for (status, body) in &answers {
        assert_eq!(*status, 422, "both attempts are refused: {body}");
        assert_eq!(body["error"]["reason"], "upload_quarantined");
        assert_eq!(body["error"]["sha256"], infected_sha);
        assert_eq!(body["error"]["held_for_operator"], true);
    }

    // One object, one quarantine copy, one notification per attempt.
    let puts: Vec<String> = store
        .ops()
        .iter()
        .filter_map(|op| match op {
            core_store::ObjectRecordedOp::Put { key } => Some(key.clone()),
            core_store::ObjectRecordedOp::Delete { .. } => None,
        })
        .collect();
    assert_eq!(
        puts.iter()
            .filter(|k| k.contains("/quarantine/") && k.ends_with(&infected_sha))
            .count(),
        1,
        "the bytes are held once, not once per attempt: {puts:?}"
    );
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    assert_eq!(
        scan_notifications(&state, org_id).await.len(),
        2,
        "each attempt told the uploader"
    );
}

/// The document door writes the same key space, so it gets the same outcome:
/// typed 422, held bytes, and a notice with no ticket to anchor it.
#[tokio::test]
async fn document_attachment_gate_refuses_and_notifies_without_a_ticket() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(GateScan);
    let (state, _store) = test_state_with(Some(scanner)).await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "docgate").await;

    let created = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            "/api/documents",
            Some(&org.token),
            Some(json!({
                "title": "Onboarding runbook",
                "body": "Step one.",
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    let document_id = created["id"].as_str().expect("document id").to_string();

    let response = send_raw(
        &app,
        axum::http::Method::POST,
        &format!("/api/documents/{document_id}/attachments"),
        Some(&org.token),
        &[
            ("x-filename", "runbook.pdf"),
            ("content-type", "application/pdf"),
        ],
        Some(INFECTED_PDF_BYTES.to_vec()),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "upload_quarantined");
    assert_eq!(body["error"]["filename"], "runbook.pdf");
    assert_eq!(body["error"]["held_for_operator"], true);
    assert_eq!(body["error"]["sha256"], sha_of(INFECTED_PDF_BYTES));

    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    assert!(
        scan_row(&state, org_id, &sha_of(INFECTED_PDF_BYTES))
            .await
            .is_some(),
        "the document door records the verdict it refused on"
    );
    let notices = scan_notifications(&state, org_id).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].2, None,
        "a document upload has no ticket to anchor the notice to"
    );
    assert!(notices[0]
        .1
        .as_deref()
        .unwrap_or_default()
        .contains("runbook.pdf"));

    let listed = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/documents/{document_id}/attachments"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        listed["attachments"].as_array().map(Vec::len),
        Some(0),
        "nothing was attached: {listed}"
    );
}

// ---------------------------------------------------------------------------
// Presigned downloads (Wave 52 lane P7)
// ---------------------------------------------------------------------------

/// One uploaded attachment as the tests need it: the row id, the digest, and
/// the object key the ticket must name.
struct Uploaded {
    id: String,
    sha256: String,
    storage_key: String,
}

/// Uploads one PNG through the real door.
async fn upload_png(app: &Router, org: &Org) -> Uploaded {
    let response = send_raw(
        app,
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
    let body = body_json(response).await;
    Uploaded {
        id: body["id"].as_str().unwrap().to_string(),
        sha256: body["sha256"].as_str().unwrap().to_string(),
        storage_key: body["storage_key"].as_str().unwrap().to_string(),
    }
}

/// Swaps the state's store for the presigning wrapper over the SAME bytes (a
/// `MemoryStore` clone shares its inner), so `presign_get` answers the way an
/// S3 bucket would while uploads and downloads still see one object space.
fn presigning_app(state: &mut AppState, store: &Arc<core_store::memory::MemoryStore>) -> Router {
    state.store = Arc::new(core_store::memory::PresigningStore((**store).clone()));
    core_api::routes::build_router(state.clone())
}

/// Flips an admitted object's verdict to `infected`, the way a scanner update
/// that finds a signature in bytes it once passed would.
async fn hold_object(state: &AppState, org_id: Uuid, sha: &str) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query(
        "UPDATE attachment_scans SET status = 'infected', \
         detail = 'test-hold: Eicar-Test-Signature FOUND', scanner = 'test-hold', \
         quarantined_at = now() WHERE org_id = $1 AND sha256 = $2",
    )
    .bind(org_id)
    .bind(sha)
    .execute(&mut *tx)
    .await
    .expect("verdict escalation");
    tx.commit().await.expect("commit");
}

async fn content_type_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// An S3-backed install hands the client a URL and keeps the bytes off the
/// API. The ticket names the object, carries the metadata the streamed
/// headers used to carry, and expires on the documented bound.
#[tokio::test]
async fn a_presigning_store_mints_a_short_lived_ticket() {
    let (mut state, store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "presign").await;
    let uploaded = upload_png(&app, &org).await;

    let app = presigning_app(&mut state, &store);
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download?presign=true", uploaded.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "an admitted object may be presigned"
    );
    let content_type = content_type_of(&response).await;
    assert!(
        content_type.starts_with("application/json"),
        "a ticket request must not receive bytes: {content_type}"
    );

    let body = body_json(response).await;
    assert_eq!(body["mode"], "presigned", "the store signed a URL: {body}");
    let expected = core_store::memory::PresigningStore::<core_store::memory::MemoryStore>::url_for(
        &uploaded.storage_key,
        core_api::routes::attachments::DOWNLOAD_URL_TTL,
    );
    assert_eq!(body["url"], expected.as_str(), "the store's URL, verbatim");
    assert_eq!(body["filename"], "screenshot.png");
    assert_eq!(body["content_type"], "image/png");
    assert_eq!(body["sha256"], uploaded.sha256.as_str());
    assert_eq!(body["size_bytes"], PNG_BYTES.len() as i64);
    assert!(
        body["expires_at"].as_str().is_some(),
        "a ticket without an expiry is a URL with no life: {body}"
    );
}

/// Local-fs is the documented fallback, chosen per request: the same
/// `Accept: application/json` request that yields a ticket on S3 yields the
/// bytes on a filesystem, with today's headers and status.
#[tokio::test]
async fn a_store_that_cannot_presign_still_streams() {
    let (state, _store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "presign-fallback").await;
    let uploaded = upload_png(&app, &org).await;

    // The ticket request says so in the body rather than failing: a local-fs
    // install is the documented deployment, not an error.
    let ticket = body_json(
        send_raw(
            &app,
            axum::http::Method::GET,
            &format!("/api/attachments/{}/download?presign=true", uploaded.id),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(ticket["mode"], "stream", "no URL on a filesystem: {ticket}");
    assert!(ticket["url"].is_null(), "no URL means no URL: {ticket}");
    assert_eq!(ticket["filename"], "screenshot.png");

    // And the plain request still streams, byte for byte, as it always did.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", uploaded.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    // The stored content type, as this door has always served it; the
    // `application/octet-stream` in the utoipa annotation describes the body
    // type for the spec, not the runtime header.
    assert_eq!(content_type_of(&response).await, "image/png");
    assert_eq!(
        response
            .headers()
            .get("x-marula-sha256")
            .and_then(|v| v.to_str().ok()),
        Some(uploaded.sha256.as_str()),
        "the streaming contract is unchanged"
    );
    assert_eq!(body_bytes(response).await, PNG_BYTES);
}

/// The hole P6 closed, tested at the door that hands out URLs: an object held
/// by the scan gate gets neither a URL nor the bytes, on either `Accept`.
#[tokio::test]
async fn a_held_object_issues_neither_a_url_nor_bytes() {
    let (mut state, store) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "presign-held").await;
    let uploaded = upload_png(&app, &org).await;
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    hold_object(&state, org_id, &uploaded.sha256).await;

    let app = presigning_app(&mut state, &store);
    for suffix in ["?presign=true", ""] {
        let response = send_raw(
            &app,
            axum::http::Method::GET,
            &format!("/api/attachments/{}/download{suffix}", uploaded.id),
            Some(&org.token),
            &[],
            None,
        )
        .await;
        assert_eq!(
            response.status(),
            409,
            "({suffix}) a held object must be refused, not signed and not streamed"
        );
        let body = body_json(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("held by the scan gate")),
            "the refusal says why: {body}"
        );
    }
}

/// No verdict on record means no URL, and in a deployment with NO scanner the
/// bytes keep the posture they had before the gate existed. Refusing here
/// would lock every pre-gate file out of an install that has no scanner to
/// re-scan it with — the no-scanner half of
/// `an_unscanned_object_is_refused_at_the_staff_and_portal_doors` below.
///
/// Wave 55 (F31) added the lists to this arm: they apply the same rule, and in
/// this mode the rule is a no-op, so a client is still offered the file it can
/// still download.
#[tokio::test]
async fn a_deployment_without_a_scanner_still_streams_an_unscanned_object() {
    let (mut state, store) = test_state().await;
    assert_eq!(
        state.mail.scanner.name(),
        "none",
        "this half of the rule is the none-mode deployment"
    );
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "presign-unscanned").await;
    let uploaded = upload_png(&app, &org).await;
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    // Surfaced through a client-visible note, so the portal list has a reason
    // to consider the object.
    let comment = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/comments", org.ticket_a),
        Some(&org.token),
        Some(json!({
            "body": "see attached",
            "kind": "client_visible",
            "attachment_ids": [uploaded.id],
        })),
    )
    .await;
    assert_eq!(comment.status(), 200, "linking the attachment to a note");
    let portal_token = portal_token_for(&app, &org, "presign-unscanned").await;
    delete_verdict(&state, org_id, &uploaded.sha256).await;

    let staff_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{}/attachments", org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&staff_list),
        vec![uploaded.id.clone()],
        "none mode lists as it always did: {staff_list}"
    );
    let portal_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/tickets/{}/attachments", org.ticket_a),
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&portal_list),
        vec![uploaded.id.clone()],
        "and so does the portal list: {portal_list}"
    );

    let app = presigning_app(&mut state, &store);
    let ticket = body_json(
        send_raw(
            &app,
            axum::http::Method::GET,
            &format!("/api/attachments/{}/download?presign=true", uploaded.id),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ticket["mode"], "stream",
        "an object with no verdict must not be handed out as a URL: {ticket}"
    );
    assert!(ticket["url"].is_null());

    // The bytes themselves keep the posture they had before this lane:
    // authorized per request, never signed.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", uploaded.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_bytes(response).await, PNG_BYTES);
}

/// Drops the verdict row for one object, leaving bytes the store holds and no
/// scan record of them: the pre-0139 shape, and the shape the inbound-email
/// path used to produce.
async fn delete_verdict(state: &AppState, org_id: Uuid, sha: &str) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query("DELETE FROM attachment_scans WHERE org_id = $1 AND sha256 = $2")
        .bind(org_id)
        .bind(sha)
        .execute(&mut *tx)
        .await
        .expect("drop the verdict");
    tx.commit().await.expect("commit");
}

/// Writes an admitted verdict back for an object that already has bytes, so a
/// test can then escalate it the way a scanner update would.
async fn admit_object(state: &AppState, org_id: Uuid, sha: &str, storage_key: &str, size: i64) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query(
        "INSERT INTO attachment_scans (org_id, sha256, status, scanner, size_bytes, \
         content_type, filename, storage_key) \
         VALUES ($1, $2, 'clean', 'test', $3, 'image/png', 'screenshot.png', $4)",
    )
    .bind(org_id)
    .bind(sha)
    .bind(size)
    .bind(storage_key)
    .execute(&mut *tx)
    .await
    .expect("admitted verdict");
    tx.commit().await.expect("commit");
}

/// Exposes the default board to client A and signs a fresh portal contact in.
/// The tag becomes the contact's email, so two runs never collide on the
/// login `LIMIT 1`.
async fn portal_token_for(app: &Router, org: &Org, tag: &str) -> String {
    let boards = body_json(
        send_json(
            app,
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
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{}/board-exposures", org.client_a),
        Some(&org.token),
        Some(json!({"board_id": board_id, "exposed": true, "visible_status_ids": []})),
    )
    .await;
    assert_eq!(exposure.status(), 200);
    let email = format!("contact-{tag}@portal.test");
    let invite = send_json(
        app,
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
    body_json(
        send_json(
            app,
            axum::http::Method::POST,
            "/api/portal/login",
            None,
            Some(json!({"email": email, "password": "portal-passphrase-1"})),
        )
        .await,
    )
    .await["access_token"]
        .as_str()
        .expect("portal token")
        .to_string()
}

/// The record this lane closes (Wave 54 lane S7). In a deployment WITH a
/// scanner, an object with no verdict on record is refused by every door that
/// hands bytes out — the staff ticket door on both `Accept` shapes and the
/// portal door — because the bytes got into the store without passing the
/// gate, which is precisely what a download must not reward. The portal door
/// also refuses a HELD object now; it used to stream whatever the visibility
/// JOIN returned.
#[tokio::test]
async fn an_unscanned_object_is_refused_at_the_staff_and_portal_doors() {
    let (mut state, store) = test_state_with(Some(Arc::new(GateScan))).await;
    assert_eq!(
        state.mail.scanner.name(),
        "gate-scripted",
        "a scanner is configured, so an absent verdict is a hole, not a none-mode skip"
    );
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string();
    let org = seed_org(&app, &tag).await;
    let portal_token = portal_token_for(&app, &org, &tag).await;

    // A file the gate ADMITTED, surfaced to the client through a
    // client-visible note so the portal door has something to refuse.
    let uploaded = upload_png(&app, &org).await;
    let comment = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/comments", org.ticket_a),
        Some(&org.token),
        Some(json!({
            "body": "see attached",
            "kind": "client_visible",
            "attachment_ids": [uploaded.id],
        })),
    )
    .await;
    assert_eq!(comment.status(), 200, "linking the attachment to a note");
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;

    let app = presigning_app(&mut state, &store);

    // --- case 1: no verdict at all ---
    delete_verdict(&state, org_id, &uploaded.sha256).await;

    for suffix in ["?presign=true", ""] {
        let response = send_raw(
            &app,
            axum::http::Method::GET,
            &format!("/api/attachments/{}/download{suffix}", uploaded.id),
            Some(&org.token),
            &[],
            None,
        )
        .await;
        assert_eq!(
            response.status(),
            409,
            "(staff{suffix}) an unscanned object must be refused"
        );
        let body = body_json(response).await;
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("no malware-scan verdict")),
            "the refusal names the real condition: {body}"
        );
    }

    let response = portal_download(&app, &org.ticket_a, &uploaded.id, &portal_token).await;
    assert_eq!(
        response.status(),
        409,
        "the portal door must refuse an unscanned object too"
    );
    let body = body_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("no malware-scan verdict")),
        "a client reads the same typed answer a staff downloader does: {body}"
    );

    // --- case 2: held for an operator ---
    admit_object(
        &state,
        org_id,
        &uploaded.sha256,
        &uploaded.storage_key,
        PNG_BYTES.len() as i64,
    )
    .await;
    hold_object(&state, org_id, &uploaded.sha256).await;

    let response = portal_download(&app, &org.ticket_a, &uploaded.id, &portal_token).await;
    assert_eq!(
        response.status(),
        409,
        "a held object must not reach a client, whatever the visibility JOIN says"
    );
    let body = body_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("held by the scan gate")),
        "the held refusal says why: {body}"
    );
}

/// One portal download request, for the tests that need that door.
async fn portal_download(
    app: &Router,
    ticket_id: &str,
    attachment_id: &str,
    portal_token: &str,
) -> axum::response::Response {
    send_raw(
        app,
        axum::http::Method::GET,
        &format!("/api/portal/tickets/{ticket_id}/attachments/{attachment_id}/download"),
        Some(portal_token),
        &[],
        None,
    )
    .await
}

// ---------------------------------------------------------------------------
// The verdict backfill and the list/door parity (Wave 55, F30 + F31)
// ---------------------------------------------------------------------------

/// A scanner that counts the BYTES legs it is asked to take, so a test can
/// prove a second backfill run paid for nothing. The verdicts are
/// [`GateScan`]'s, so the two tests agree on what a marker means.
struct CountingScan {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl core_mail::ScanProvider for CountingScan {
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
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        core_mail::ScanProvider::scan_bytes(&GateScan, bytes).await
    }
}

/// A row and its bytes exactly as a deployment WITHOUT the gate used to write
/// them: the object at its canonical key, an `attachments` row pointing at it,
/// and no verdict row at all. Every door now refuses to store unscanned bytes,
/// so this is the only way to build the shape the backfill exists for.
#[allow(clippy::too_many_arguments)]
async fn store_legacy_object(
    state: &AppState,
    store: &Arc<core_store::memory::MemoryStore>,
    org_id: Uuid,
    ticket_id: Uuid,
    comment_id: Option<Uuid>,
    bytes: &[u8],
    filename: &str,
    content_type: &str,
) -> Uploaded {
    use sha2::Digest as _;
    let sha = hex::encode(sha2::Sha256::digest(bytes));
    let key = core_store::attachment_key(org_id, &sha);
    store
        .put(&key, content_type, bytes::Bytes::copy_from_slice(bytes))
        .await
        .expect("legacy bytes");
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    core_db::attachments::insert(
        &mut tx,
        core_db::attachments::NewAttachment {
            id,
            org_id,
            kind: core_domain::entities::AttachmentKind::TicketComment,
            ticket_id: Some(ticket_id),
            ticket_comment_id: comment_id,
            email_message_id: None,
            filename,
            content_type,
            size_bytes: bytes.len() as i64,
            sha256: &sha,
            storage_key: &key,
            uploaded_by_user_id: None,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("legacy row");
    tx.commit().await.expect("commit");
    Uploaded {
        id: id.to_string(),
        sha256: sha,
        storage_key: key,
    }
}

/// An operator release, the way the P6c admin surface writes it: the row keeps
/// its `infected` status as evidence and gains the override that admits the
/// object again.
async fn release_object(state: &AppState, org_id: Uuid, sha: &str) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let owner: Uuid = sqlx::query_scalar(
        "SELECT id FROM users WHERE org_id = $1 ORDER BY created_at, id LIMIT 1",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("the org has an owner");
    sqlx::query(
        "UPDATE attachment_scans SET released_at = now(), released_by_user_id = $3, \
         release_reason = 'wave 55 list parity' WHERE org_id = $1 AND sha256 = $2",
    )
    .bind(org_id)
    .bind(sha)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .expect("release");
    tx.commit().await.expect("commit");
}

fn ids_of(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap_or_else(|| panic!("expected a list of rows: {rows}"))
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .unwrap_or_else(|| panic!("every row carries an id: {row}"))
                .to_string()
        })
        .collect()
}

/// Row F30: files stored before a scanner existed carry no verdict, and since
/// the Wave 54 admission rule a deployment WITH a scanner refuses them at every
/// door. The backfill is the pass that gives them one — and it must judge
/// exactly those files, once.
///
/// Three objects: one the upload door already admitted (its verdict is the
/// record of what happened and must not be re-examined), one legacy object a
/// scanner will pass, one legacy object it will hold. Each leg is a claim the
/// operator command depends on: only unverdict objects are scanned, one verdict
/// per object, an infection goes through the SAME quarantine writer the doors
/// use, and a second run finds nothing to do.
#[tokio::test]
async fn the_verdict_backfill_judges_only_objects_without_a_verdict_once() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(CountingScan {
        calls: Arc::clone(&calls),
    });
    let (state, store) = test_state_with(Some(scanner)).await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string();
    let org = seed_org(&app, &tag).await;
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    let ticket_id = Uuid::parse_str(&org.ticket_a).expect("ticket uuid");

    // Admitted through the real door: its verdict already exists.
    let admitted = upload_png(&app, &org).await;
    let before = scan_row(&state, org_id, &admitted.sha256)
        .await
        .expect("the upload door recorded a verdict");

    // The two pre-gate objects: bytes on disk, a row, no verdict.
    let clean = store_legacy_object(
        &state,
        &store,
        org_id,
        ticket_id,
        None,
        PDF_BYTES,
        "policy.pdf",
        "application/pdf",
    )
    .await;
    let held = store_legacy_object(
        &state,
        &store,
        org_id,
        ticket_id,
        None,
        INFECTED_PDF_BYTES,
        "invoice.pdf",
        "application/pdf",
    )
    .await;

    let plan = core_api::scan_backfill::plan(&state.pool, Some(org_id))
        .await
        .expect("plan");
    assert_eq!(plan.total, 2, "the admitted object is not part of the plan");

    let calls_before = calls.load(std::sync::atomic::Ordering::SeqCst);
    let report = core_api::scan_backfill::run(
        &state.pool,
        state.store.as_ref(),
        state.mail.scanner.as_ref(),
        Some(org_id),
        50,
        Some(2),
    )
    .await
    .expect("the backfill runs");
    assert_eq!(
        report.found, 2,
        "one object per missing verdict: {report:?}"
    );
    assert_eq!(report.clean, 1, "{report:?}");
    assert_eq!(report.quarantined, 1, "{report:?}");
    assert_eq!(report.skipped, 0, "a scanner was configured: {report:?}");
    assert_eq!(report.unreadable, 0, "{report:?}");
    assert_eq!(report.already, 0, "{report:?}");
    assert!(report.aborted.is_none(), "{report:?}");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst) - calls_before,
        2,
        "one bytes leg per object with no verdict, and none for the admitted one"
    );

    // The clean legacy object reads exactly like a door-admitted one.
    let (status, storage_key, quarantined) = scan_row(&state, org_id, &clean.sha256)
        .await
        .expect("the clean object now has a verdict");
    assert_eq!(status, "clean");
    assert_eq!(
        storage_key, clean.storage_key,
        "the verdict points at its bytes"
    );
    assert!(!quarantined);
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", clean.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "a backfilled clean object downloads again"
    );
    assert_eq!(body_bytes(response).await, PDF_BYTES);

    // The infected one went through the shared quarantine writer: held row,
    // quarantine copy, and the notification the doors send.
    let (status, storage_key, quarantined) = scan_row(&state, org_id, &held.sha256)
        .await
        .expect("the held object now has a verdict");
    assert_eq!(status, "infected");
    assert!(quarantined, "the row names when it was held");
    assert_eq!(
        storage_key,
        core_store::quarantine_key(org_id, &held.sha256),
        "the verdict points at the quarantine copy, not the admitted key"
    );
    assert!(
        store
            .get(&core_store::quarantine_key(org_id, &held.sha256))
            .await
            .is_ok(),
        "the held bytes were written where the row says they are"
    );
    let notices = scan_notifications(&state, org_id).await;
    assert_eq!(
        notices.len(),
        1,
        "the backfill sends the same quarantine notice an upload does: {notices:?}"
    );
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", held.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "an object the backfill held must not download"
    );

    // The admitted object's verdict is the record of what happened: untouched.
    let after = scan_row(&state, org_id, &admitted.sha256)
        .await
        .expect("still on record");
    assert_eq!(after, before, "the backfill rewrote an existing verdict");

    // Second run: nothing to scan, nothing to write, no scanner call paid for.
    let calls_before = calls.load(std::sync::atomic::Ordering::SeqCst);
    let plan = core_api::scan_backfill::plan(&state.pool, Some(org_id))
        .await
        .expect("second plan");
    assert_eq!(plan.total, 0, "a re-run must find nothing");
    let second = core_api::scan_backfill::run(
        &state.pool,
        state.store.as_ref(),
        state.mail.scanner.as_ref(),
        Some(org_id),
        50,
        None,
    )
    .await
    .expect("the second run");
    assert_eq!(second.found, 0);
    assert_eq!(
        second.clean + second.skipped + second.quarantined + second.already + second.unreadable,
        0,
        "a second run records nothing: {second:?}"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        calls_before,
        "a second run pays for no scan"
    );
}

/// Row F31: a list must not offer what its own download refuses. The rule is
/// one predicate in the shared query layer, so it is asserted on every list
/// that names the same object — the ticket list, the comment list, the portal
/// list and the document list — against the door's answer for the same object.
#[tokio::test]
async fn the_lists_omit_what_their_door_refuses() {
    let (state, store) = test_state_with(Some(Arc::new(GateScan))).await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string();
    let org = seed_org(&app, &tag).await;
    let portal_token = portal_token_for(&app, &org, &tag).await;
    let org_id = org_of_ticket(&app, &org.token, &org.ticket_a).await;
    let ticket_id = Uuid::parse_str(&org.ticket_a).expect("ticket uuid");

    // One admitted object, linked to a client-visible note so the portal list
    // has a reason to consider it at all.
    let admitted = upload_png(&app, &org).await;
    let comment = send_json(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/comments", org.ticket_a),
        Some(&org.token),
        Some(json!({
            "body": "see attached",
            "kind": "client_visible",
            "attachment_ids": [admitted.id],
        })),
    )
    .await;
    assert_eq!(comment.status(), 200, "linking the attachment to a note");
    let comment_id = Uuid::parse_str(
        body_json(comment).await["id"]
            .as_str()
            .expect("the created comment carries its id"),
    )
    .expect("comment uuid");

    // And one object no scanner ever looked at, on the same ticket and note.
    let legacy = store_legacy_object(
        &state,
        &store,
        org_id,
        ticket_id,
        Some(comment_id),
        PDF_BYTES,
        "legacy.pdf",
        "application/pdf",
    )
    .await;

    let ticket_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{}/attachments", org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&ticket_list),
        vec![admitted.id.clone()],
        "the ticket list offers exactly what its door would hand out: {ticket_list}"
    );
    let comment_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/tickets/{}/comments/{comment_id}/attachments",
                org.ticket_a
            ),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&comment_list),
        vec![admitted.id.clone()],
        "the comment list applies the same rule: {comment_list}"
    );
    let portal_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/tickets/{}/attachments", org.ticket_a),
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&portal_list),
        vec![admitted.id.clone()],
        "a client is not offered a file the portal download refuses: {portal_list}"
    );

    // The door agrees with all three lists about the object they left out.
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", legacy.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 409, "the omitted object is refused");
    let response = portal_download(&app, &org.ticket_a, &legacy.id, &portal_token).await;
    assert_eq!(response.status(), 409, "and refused at the portal door too");

    // --- a held object: hidden while held, back after a release -----------
    hold_object(&state, org_id, &admitted.sha256).await;
    let held_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{}/attachments", org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        ids_of(&held_list).is_empty(),
        "a held object is not offered: {held_list}"
    );
    let held_portal = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/tickets/{}/attachments", org.ticket_a),
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        ids_of(&held_portal).is_empty(),
        "and the portal list hides it too: {held_portal}"
    );
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", admitted.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 409, "the door refuses the held object");

    release_object(&state, org_id, &admitted.sha256).await;
    let released_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{}/attachments", org.ticket_a),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&released_list),
        vec![admitted.id.clone()],
        "a release puts the object back in the list, because the door admits it again: {released_list}"
    );
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/attachments/{}/download", admitted.id),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "and the door hands it out again");

    // --- the document list is the same rule on the same key space ---------
    let document_id = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            "/api/documents",
            Some(&org.token),
            Some(json!({
                "title": "Attachment policy",
                "body": "Files go here.",
                "idempotency_key": format!("doc-{tag}"),
            })),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .expect("the created document carries its id")
        .to_string();
    let uploaded_doc = body_json(
        send_raw(
            &app,
            axum::http::Method::POST,
            &format!("/api/documents/{document_id}/attachments"),
            Some(&org.token),
            &[
                ("x-filename", "rfc.pdf"),
                ("content-type", "application/pdf"),
            ],
            Some(b"%PDF-1.7 rfc 1918".to_vec()),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .expect("the document attachment carries its id")
        .to_string();
    let document_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/documents/{document_id}/attachments"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        ids_of(&document_list["attachments"]),
        vec![uploaded_doc.clone()],
        "an admitted document attachment lists: {document_list}"
    );
    let doc_sha = sha_of(b"%PDF-1.7 rfc 1918");
    delete_verdict(&state, org_id, &doc_sha).await;
    let document_list = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/documents/{document_id}/attachments"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        ids_of(&document_list["attachments"]).is_empty(),
        "an unverdict document attachment is not offered: {document_list}"
    );
    let response = send_raw(
        &app,
        axum::http::Method::GET,
        &format!("/api/documents/{document_id}/attachments/{uploaded_doc}/download"),
        Some(&org.token),
        &[],
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "which is what the document door answers for it"
    );
}
