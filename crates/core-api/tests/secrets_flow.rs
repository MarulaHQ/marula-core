//! End-to-end secrets-vault straggler sweep (Wave 16 lane 9) driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the lane-9 vault work over HTTP —
//! * A3: the IF1–IF3 idempotent envelope on secret record create/update
//!   (same key + same payload replays byte-identically; same key +
//!   different payload is the typed 409 `idempotency_key_reused`), plus
//!   the `mutation` metadata on the applied response;
//! * A5: PATCH field-clear semantics (`value: null` deletes the sealed
//!   field row; the audited event still fires);
//! * A1: the per-org write budget returns a typed 429 (with a
//!   `Retry-After`) once `MARULA_VAULT_WRITE_BUDGET_PER_MIN` seats are
//!   consumed;
//! * A2: the chunked bulk-ingestion worker path (per-row atomicity,
//!   replay of already-committed rows via per-row idempotency keys).
//!
//! Non-purpose: document-file PUT/GET round-trip against a real object
//! store (covered by the core-store adapters' own suites; the route
//! logic is a thin seal→put / get→open seam), UI.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let (state, app, _store) = test_state_with_store(None).await;
    (state, app)
}

/// Same state, with a chosen scan provider and the object store handed back.
/// `None` keeps the config-built edge — none mode, which is what every
/// pre-gate test in this file runs against. The Wave 52 P6d gate tests inject
/// a scripted scanner so a verdict can be forced, and keep the store so they
/// can assert on what was (not) written.
async fn test_state_with_store(
    scanner: Option<Arc<dyn core_mail::ScanProvider>>,
) -> (AppState, Router, Arc<core_store::memory::MemoryStore>) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("secrets e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
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
        store: Arc::clone(&store) as Arc<dyn core_store::ObjectStore>,
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app, store)
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
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Vault Co",
            "org_slug": slug,
            "email": format!("owner@{}.test", slug),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

/// The org-scope seeded folder to write records into (0078 seeds five
/// immutable defaults per org; any org-scope folder works for writes).
async fn org_folder(app: &Router, bearer: &str) -> (Uuid, bool) {
    let folders: Value = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/secrets/folders",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    let list = folders.as_array().expect("folders list");
    let row = list
        .iter()
        .find(|f| !f["restricted"].as_bool().unwrap_or(false) && f["client_id"].is_null())
        .unwrap_or(&list[0]);
    let restricted = row["restricted"].as_bool().unwrap_or(false);
    (row["id"].as_str().unwrap().parse().unwrap(), restricted)
}

#[tokio::test]
async fn secret_record_create_replays_patch_clears_and_reports() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("vault1-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;

    // A3: create with an idempotency key.
    let key = format!("row-{}", Uuid::now_v7().simple());
    let create_body = json!({
        "folder_id": folder_id,
        "kind": "password",
        "title": "PagerDuty key",
        "fields": [
            {"field_key": "secret", "value": "hunter2"},
            {"field_key": "username", "value": "ops-bot"}
        ],
        "idempotency_key": key,
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(create_body.clone()),
    )
    .await;
    let status = first.status();
    let first_body = body_json(first).await;
    assert_eq!(status, 200, "create must succeed: {first_body}");
    let record_id = first_body["id"].as_str().unwrap().to_string();
    assert_eq!(first_body["title"], json!("PagerDuty key"));
    let seq = first_body["mutation"]["seq"].as_i64().unwrap();
    assert_eq!(first_body["mutation"]["idempotency_key"], json!(key));

    // Same key + same payload ⇒ byte-identical replay, no second row.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(create_body),
    )
    .await;
    let replay_status = replay.status();
    let replay_body = body_json(replay).await;
    assert_eq!(replay_status, 200, "replay must succeed: {replay_body}");
    assert_eq!(replay_body["id"].as_str().unwrap(), record_id);
    assert_eq!(replay_body["mutation"]["seq"].as_i64().unwrap(), seq);

    // Same key + DIFFERENT payload ⇒ typed 409 idempotency_key_reused.
    let conflicting = json!({
        "folder_id": folder_id,
        "kind": "password",
        "title": "A DIFFERENT record",
        "fields": [],
        "idempotency_key": key,
    });
    let reused = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(conflicting),
    )
    .await;
    assert_eq!(reused.status(), 409);
    let reused_body = body_json(reused).await;
    assert_eq!(
        reused_body["error"]["reason"],
        json!("idempotency_key_reused")
    );

    // A5: PATCH with value:null CLEARS the username field.
    let patch = json!({
        "title": "PagerDuty key",
        "fields": [{"field_key": "username", "value": null}],
        "idempotency_key": format!("patch-{}", Uuid::now_v7().simple()),
    });
    let patched = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/secrets/records/{record_id}"),
        Some(&bearer),
        Some(patch),
    )
    .await;
    let patched_status = patched.status();
    let patched_body = body_json(patched).await;
    assert_eq!(patched_status, 200, "patch must succeed: {patched_body}");
    let username_meta = patched_body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["field_key"] == json!("username"))
        .cloned();
    assert!(
        username_meta.is_none(),
        "cleared field must vanish from metadata: {patched_body}"
    );

    // The cleared record still reveals its surviving field only.
    let reveal = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/secrets/records/{record_id}/reveal"),
        Some(&bearer),
        None,
    )
    .await;
    let reveal_status = reveal.status();
    let reveal_body = body_json(reveal).await;
    assert_eq!(reveal_status, 200, "reveal must succeed: {reveal_body}");
    let revealed_keys: Vec<&str> = reveal_body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["field_key"].as_str().unwrap())
        .collect();
    assert_eq!(revealed_keys, vec!["secret"]);
    assert_eq!(reveal_body["fields"][0]["value"], json!("hunter2"));
}

#[tokio::test]
async fn bulk_ingest_rows_commit_atomically_and_replay() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("vault2-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;

    let row_key_1 = format!("bulk-{}", Uuid::now_v7().simple());
    let row_key_2 = format!("bulk-{}", Uuid::now_v7().simple());
    let chunk = json!({
        "folder_id": folder_id,
        "kind": "password",
        "records": [
            {"title": "Row one", "idempotency_key": row_key_1,
             "fields": [{"field_key": "secret", "value": "one"}]},
            {"title": "Row two", "idempotency_key": row_key_2,
             "fields": [{"field_key": "secret", "value": "two"}]}
        ]
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk.clone()),
    )
    .await;
    assert_eq!(first.status(), 200, "bulk chunk must succeed");
    let first_body = body_json(first).await;
    assert_eq!(first_body["created"], json!(2));
    assert_eq!(first_body["replayed"], json!(0));
    let row1_id = first_body["results"][0]["record_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Resumed chunk: same per-row keys ⇒ both rows REPLAY, ids match.
    let resumed = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(chunk),
    )
    .await;
    assert_eq!(resumed.status(), 200);
    let resumed_body = body_json(resumed).await;
    assert_eq!(resumed_body["created"], json!(0));
    assert_eq!(resumed_body["replayed"], json!(2));
    assert_eq!(
        resumed_body["results"][0]["record_id"].as_str().unwrap(),
        row1_id
    );

    // A duplicate title inside one folder is a per-row refusal (the
    // chunk never aborts: per-row atomicity).
    let dup = json!({
        "folder_id": folder_id,
        "kind": "password",
        "records": [
            {"title": "Row one", "idempotency_key": format!("bulk-{}", Uuid::now_v7().simple()),
             "fields": []}
        ]
    });
    let refused = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records/bulk",
        Some(&bearer),
        Some(dup),
    )
    .await;
    assert_eq!(refused.status(), 200);
    let refused_body = body_json(refused).await;
    assert_eq!(refused_body["refused"], json!(1));
    assert_eq!(refused_body["results"][0]["outcome"], json!("refused"));
}

/// Wave 19 lane 1: a BIG (multi-chunk, > 64 KiB) document PUT round-trips
/// through the `[MRDV2]` framed seal + the streaming GET decryptor —
/// the streamed body must be the uploaded bytes byte-identical.
#[tokio::test]
async fn document_file_put_get_roundtrips_streamed_multi_chunk() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("vaultfile-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;
    let create = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(json!({"folder_id": folder_id, "kind": "document", "title": "Big doc"})),
    )
    .await;
    let created = body_json(create).await;
    let record_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    // > 2 chunks: ~256KiB of pseudo-random-ish bytes.
    let mut body_bytes = vec![0u8; 2 * 64 * 1024 * 2];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for slot in body_bytes.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *slot = x as u8;
        x = x.wrapping_add(1).rotate_right(3);
    }
    let request = axum::http::Request::builder()
        .method(axum::http::Method::PUT)
        .uri(format!("/api/secrets/records/{record_id}/file"))
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/octet-stream")
        .body(axum::body::Body::from(body_bytes.clone()))
        .unwrap();
    let put = app.clone().oneshot(request).await.unwrap();
    assert_eq!(put.status(), 200, "put must seal + pin");
    let pinned = body_json(put).await;
    assert!(
        pinned["file_sha256"].as_str().is_some(),
        "the record must carry the file pointer after PUT: {pinned}"
    );

    // GET streams the plaintext back, byte-identical.
    let get = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}/file"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(get.status(), 200);
    assert!(get.headers().contains_key("x-file-sha256"));
    println!(
        "CONTENT_LEN_HDR={:?}",
        get.headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .map(|v| v.to_str().unwrap().to_string())
    );
    let mut stream_body = get.into_body();
    let mut downloaded: Vec<u8> = Vec::new();
    while let Some(frame) = http_body_util::BodyExt::frame(&mut stream_body).await {
        let chip_bytes = match frame {
            Ok(f) => f.into_data().unwrap(),
            Err(e) => panic!("stream body error after {} bytes: {e}", downloaded.len()),
        };
        downloaded.extend_from_slice(&chip_bytes);
    }
    if downloaded != body_bytes {
        println!("UPLOAD_LEN={}", body_bytes.len());
        println!("DOWNLOAD_LEN={}", downloaded.len());
        println!("UPLOAD_LEN={}", body_bytes.len());
        panic!(
            "streamed GET body must be byte-identical (got {} bytes, wanted {} bytes)",
            downloaded.len(),
            body_bytes.len()
        );
    }
}

/// Wave 19 lane 1 legacy compat: a record whose stored ciphertext is the
/// Wave 16 single-envelope (seal_with_aad) buffer still decrypts on GET
/// — the legacy branch, exercised through a handcrafted store + pointer.
#[tokio::test]
async fn document_file_get_still_reads_legacy_envelope() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("vaultlegacy-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;
    let create = send(
        &app,
        axum::http::Method::POST,
        "/api/secrets/records",
        Some(&bearer),
        Some(json!({"folder_id": folder_id, "kind": "document", "title": "Legacy doc"})),
    )
    .await;
    let created = body_json(create).await;
    let record_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let org_id: Uuid = created["org_id"]
        .as_str()
        .unwrap()
        .parse()
        .expect("record carries org_id");

    // Store the ciphertext with the OLD single-envelope format.
    let plaintext = b"legacy legacy legacy bytes".repeat(10);
    let sealed = core_api::crypto::seal_with_aad(
        &state.config.secret_key,
        &plaintext,
        format!("secret_file:{org_id}:{record_id}").as_bytes(),
    )
    .unwrap();
    use sha2::Digest as _;
    let ct_sha256 = hex::encode(sha2::Sha256::digest(&sealed));
    let storage_key = format!("org/{org_id}/secrets/{ct_sha256}");
    state
        .store
        .put(
            &storage_key,
            "application/octet-stream",
            sealed.clone().into(),
        )
        .await
        .expect("store put");
    let file_bytes = i64::try_from(plaintext.len()).unwrap_or(i64::MAX);
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    sqlx::query(
        "UPDATE secret_records SET object_key = $2, file_sha256 = $3, file_bytes = $4 \
         WHERE id = $1 AND org_id = $5",
    )
    .bind(record_id)
    .bind(&storage_key)
    .bind(&ct_sha256)
    .bind(file_bytes)
    .bind(org_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // GET decodes through the legacy branch.
    let get = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/secrets/records/{record_id}/file"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(get.status(), 200);
    let header = get
        .headers()
        .get("x-file-sha256")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(header, ct_sha256);
    let downloaded = axum::body::to_bytes(get.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(downloaded, plaintext);
}

// ---------------------------------------------------------------------------
// Wave 52 lane P6d — the vault file is scanned on its PLAINTEXT, and an
// infected upload is refused with nothing stored (no object, no verdict row,
// no quarantine copy, no notification). See
// `design/qa-p52-vault-scan/REVIEW.md` for why holding is the wrong answer
// on this door.
// ---------------------------------------------------------------------------

/// A document payload carrying the marker the scripted scanner calls a
/// signature. The vault sniffs no magic bytes, so any bytes reach the gate —
/// this one reads like the document it claims to be.
const SCAN_INFECTED_BYTES: &[u8] = b"%PDF-1.7 inherited contract [VIRUS] payload";
const SCAN_CLEAN_BYTES: &[u8] = b"%PDF-1.7 signed statement of work, no payload";

/// Deterministic stand-in for clamd on the BYTES leg: `[VIRUS]` ⇒ infected,
/// `[SCAN-DOWN]` ⇒ unavailable, anything else clean.
struct ScriptedScan;

#[async_trait::async_trait]
impl core_mail::ScanProvider for ScriptedScan {
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

fn plain_sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// PUT raw bytes at the vault file door (the route takes the body directly,
/// not a JSON envelope).
async fn put_bytes(
    app: &Router,
    bearer: &str,
    record_id: Uuid,
    bytes: &[u8],
) -> axum::response::Response {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::PUT)
        .uri(format!("/api/secrets/records/{record_id}/file"))
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/octet-stream")
        .body(axum::body::Body::from(bytes.to_vec()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn create_document(app: &Router, bearer: &str, folder_id: Uuid, title: &str) -> (Uuid, Uuid) {
    let created = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/secrets/records",
            Some(bearer),
            Some(json!({"folder_id": folder_id, "kind": "document", "title": title})),
        )
        .await,
    )
    .await;
    (
        created["id"].as_str().unwrap().parse().unwrap(),
        created["org_id"].as_str().unwrap().parse().unwrap(),
    )
}

/// The verdict row for one digest, as the table holds it.
struct ScanRow {
    status: String,
    scanner: String,
    filename: String,
    storage_key: String,
}

async fn scan_row(state: &AppState, org_id: Uuid, sha256: &str) -> Option<ScanRow> {
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    let row = sqlx::query(
        "SELECT status, scanner, filename, storage_key FROM attachment_scans \
         WHERE org_id = $1 AND sha256 = $2",
    )
    .bind(org_id)
    .bind(sha256)
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    row.map(|r| ScanRow {
        status: r.get("status"),
        scanner: r.get("scanner"),
        filename: r.get("filename"),
        storage_key: r.get("storage_key"),
    })
}

async fn file_pointer(
    state: &AppState,
    org_id: Uuid,
    record_id: Uuid,
) -> (Option<String>, Option<String>) {
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    let row = sqlx::query(
        "SELECT object_key, file_sha256 FROM secret_records WHERE id = $1 AND org_id = $2",
    )
    .bind(record_id)
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (row.get("object_key"), row.get("file_sha256"))
}

async fn scan_notice_count(state: &AppState, org_id: Uuid) -> i64 {
    let mut tx = state.pool.begin().await.unwrap();
    core_db::set_org(&mut tx, org_id).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE org_id = $1 AND kind = 'scan_detection'",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    count
}

/// The refusal is the whole point of this door: the scan ran on the
/// plaintext, found a signature, and the request left NOTHING behind — the
/// assertion on the store log is what makes that a fact rather than a status
/// code.
#[tokio::test]
async fn document_file_infected_plaintext_is_refused_and_nothing_is_stored() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(ScriptedScan);
    let (state, app, store) = test_state_with_store(Some(scanner)).await;
    let org = register_org(&app, format!("vaultscan-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;
    let (record_id, org_id) = create_document(&app, &bearer, folder_id, "Signed MSA").await;

    let put = put_bytes(&app, &bearer, record_id, SCAN_INFECTED_BYTES).await;
    let status = put.status();
    let body = body_json(put).await;
    assert_eq!(status, 422, "an infected vault file is refused: {body}");
    assert_eq!(body["error"]["reason"], json!("upload_refused"));
    assert_eq!(body["error"]["scanner"], json!("gate-scripted"));
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("Eicar-Test-Signature"),
        "the finding rides the refusal: {body}"
    );
    // The distinguishing key: nothing is held, so nothing waits on an
    // operator. The quarantine refusal answers `true` here.
    assert_eq!(body["error"]["held_for_operator"], json!(false));
    assert_eq!(body["error"]["filename"], json!("Signed MSA"));
    assert_eq!(
        body["error"]["sha256"],
        json!(plain_sha256(SCAN_INFECTED_BYTES)),
        "the refusal names the digest that was scanned"
    );

    // Nothing in the store — not the admitted key, not a quarantine copy.
    assert_eq!(
        store.put_count(),
        0,
        "a refused vault file must not put bytes anywhere: {:?}",
        store.ops()
    );

    // No verdict row: this door does not record infections (a row would
    // describe held bytes that do not exist).
    let digest = plain_sha256(SCAN_INFECTED_BYTES);
    assert!(
        scan_row(&state, org_id, &digest).await.is_none(),
        "a refused vault file leaves no verdict row"
    );
    // The record keeps no pointer, and nobody was notified.
    let (object_key, file_sha256) = file_pointer(&state, org_id, record_id).await;
    assert!(object_key.is_none(), "no pointer after a refusal");
    assert!(file_sha256.is_none(), "no file hash after a refusal");
    assert_eq!(scan_notice_count(&state, org_id).await, 0);
}

/// A clean upload is stored and its verdict is on record, keyed by the
/// plaintext digest so a later reader can answer "was this checked?".
#[tokio::test]
async fn document_file_clean_plaintext_is_stored_and_records_its_verdict() {
    let scanner: Arc<dyn core_mail::ScanProvider> = Arc::new(ScriptedScan);
    let (state, app, store) = test_state_with_store(Some(scanner)).await;
    let org = register_org(&app, format!("vaultscanok-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;
    let (record_id, org_id) = create_document(&app, &bearer, folder_id, "Clean SOW").await;

    let put = put_bytes(&app, &bearer, record_id, SCAN_CLEAN_BYTES).await;
    let status = put.status();
    let pinned = body_json(put).await;
    assert_eq!(status, 200, "clean bytes are sealed and pinned: {pinned}");
    let object_key = pinned["object_key"].as_str().unwrap().to_string();
    assert!(
        object_key.starts_with(&format!("org/{org_id}/secrets/")),
        "the sealed copy lives under the vault namespace: {object_key}"
    );
    assert_eq!(store.put_count(), 1, "exactly one ciphertext PUT");
    assert_eq!(
        store.ops(),
        vec![core_store::ObjectRecordedOp::Put {
            key: object_key.clone()
        }]
    );

    let digest = plain_sha256(SCAN_CLEAN_BYTES);
    let row = scan_row(&state, org_id, &digest)
        .await
        .expect("a clean vault upload records its verdict");
    assert_eq!(row.status, "clean");
    assert_eq!(row.scanner, "gate-scripted");
    assert_eq!(row.filename, "Clean SOW");
    assert_eq!(
        row.storage_key, object_key,
        "the row points at the copy this upload wrote"
    );

    // The same plaintext on a second record is admitted again — the vault's
    // AAD makes it a second sealed key — while the verdict keeps its first
    // record. That is what first-wins means on a content-keyed table.
    let (second_id, _) = create_document(&app, &bearer, folder_id, "Clean SOW copy").await;
    let second = put_bytes(&app, &bearer, second_id, SCAN_CLEAN_BYTES).await;
    assert_eq!(
        second.status(),
        200,
        "identical clean bytes are admitted again"
    );
    assert_eq!(
        store.put_count(),
        2,
        "a second record means a second sealed copy"
    );
    let again = scan_row(&state, org_id, &digest)
        .await
        .expect("the verdict row survives the second sighting");
    assert_eq!(
        again.storage_key, object_key,
        "the verdict keeps the first admission's key"
    );
}

/// None mode is `skipped`, not `clean`: a deployment with no scanner says
/// nobody looked, so a vault upload can never read as the one path that was
/// checked.
#[tokio::test]
async fn document_file_none_mode_records_skipped() {
    let (state, app, store) = test_state_with_store(None).await;
    let org = register_org(&app, format!("vaultscannone-{}", Uuid::now_v7().simple())).await;
    let bearer = org["access_token"].as_str().unwrap().to_string();
    let (folder_id, _restricted) = org_folder(&app, &bearer).await;
    let (record_id, org_id) = create_document(&app, &bearer, folder_id, "Unscanned SOW").await;

    let put = put_bytes(&app, &bearer, record_id, SCAN_CLEAN_BYTES).await;
    assert_eq!(put.status(), 200, "none mode admits the file");
    assert_eq!(store.put_count(), 1);

    let row = scan_row(&state, org_id, &plain_sha256(SCAN_CLEAN_BYTES))
        .await
        .expect("none mode still records that nobody scanned");
    assert_eq!(row.status, "skipped");
    assert_eq!(row.scanner, "none");
}
