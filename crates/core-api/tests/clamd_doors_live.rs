//! The scan gate's doors against a REAL clamd (Wave 52 lane P6f).
//!
//! Every other test of these doors scripts the scanner (`attachments_flow.rs`,
//! `secrets_flow.rs`). That proves the gate's plumbing and says nothing about
//! the daemon: whether the bytes leave the router in a frame clamd accepts,
//! whether a real `FOUND` becomes the typed 422 the client reads, and what a
//! real daemon does when the stream is bigger than its `StreamMaxLength`.
//!
//! This file answers those through the same router the product serves, with
//! the scanner wired the way production wires it: `EMAIL_SCAN=clamd` and
//! `CLAMD_ADDR` through [`Config::from_env_allow_dev_fallback`], not a test
//! double. Run it with `scripts/p6f-clamd-live.sh --doors`.
//!
//! Why the payload is a ZIP: the doors validate magic bytes before the scan,
//! so raw EICAR octets are refused as an unknown file type and never reach
//! clamd. The obvious shortcut for a PDF door — `%PDF-1.7` with the EICAR
//! string after it — is reported CLEAN by ClamAV 1.5.4, because the EICAR
//! signature is a whole-file match and a PDF is scanned as parsed object
//! streams. A ZIP holding the test file is both an allowlisted attachment and
//! something the daemon actually detects.
//!
//! Every test is `#[ignore]`d: this leg needs a clamd with a signature DB and
//! a database, which is a nightly cost, not a per-push one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use core_api::config::Config;
use core_api::email::MailStack;
use core_api::state::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use tower::ServiceExt;
use uuid::Uuid;

// --- harness ---------------------------------------------------------------

/// The EICAR test string, whole-file form. ClamAV only matches it as a
/// complete file, which is why it is wrapped in a ZIP below rather than
/// padded into a PDF.
const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

struct Bootstrap {
    admin_url: String,
    _container: Option<testcontainers::ContainerAsync<testcontainers::GenericImage>>,
}

static BOOTSTRAP: tokio::sync::OnceCell<Bootstrap> = tokio::sync::OnceCell::const_new();

async fn bootstrap() -> &'static Bootstrap {
    BOOTSTRAP
        .get_or_init(|| async {
            if let Ok(url) =
                std::env::var("MARULA_TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))
            {
                let admin = connect_with_retry(&url).await;
                reset_schema(&admin).await;
                core_db::run_migrations(&admin).await.expect("migrate");
                return Bootstrap {
                    admin_url: url,
                    _container: None,
                };
            }
            let request = testcontainers::ContainerRequest::from(
                testcontainers::GenericImage::new("pgvector/pgvector", "pg18")
                    .with_wait_for(testcontainers::core::WaitFor::message_on_stderr(
                        "database system is ready to accept connections",
                    ))
                    .with_exposed_port(testcontainers::core::ContainerPort::Tcp(5432)),
            )
            // Named apart from the suite's harness so a concurrent run of
            // `two_weeks` cannot reset this schema out from under the leg.
            .with_container_name("marula-p6f-clamd-pg18")
            .with_reuse(testcontainers::core::ReuseDirective::Always)
            .with_env_var("POSTGRES_USER", "marula")
            .with_env_var("POSTGRES_PASSWORD", "marula")
            .with_env_var("POSTGRES_DB", "marula_p6f");
            let container = request
                .start()
                .await
                .expect("docker required for the door leg");
            let host = container.get_host().await.expect("host");
            let port = container.get_host_port_ipv4(5432_u16).await.expect("port");
            let admin_url = format!("postgres://marula:marula@{host}:{port}/marula_p6f");
            let admin = connect_with_retry(&admin_url).await;
            reset_schema(&admin).await;
            core_db::run_migrations(&admin).await.expect("migrate");
            Bootstrap {
                admin_url,
                _container: Some(container),
            }
        })
        .await
}

async fn connect_with_retry(url: &str) -> sqlx::PgPool {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(pool) = core_db::connect(url).await {
            if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                return pool;
            }
        }
        assert!(Instant::now() < deadline, "postgres unreachable at {url}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn reset_schema(admin: &sqlx::PgPool) {
    sqlx::query(
            "DO $$ BEGIN DROP SCHEMA IF EXISTS ops CASCADE; DROP SCHEMA IF EXISTS public CASCADE; END $$",
        )
        .execute(admin)
        .await
        .expect("drop");
    sqlx::query("CREATE SCHEMA public")
        .execute(admin)
        .await
        .expect("create");
}

/// State whose scanner came from `EMAIL_SCAN=clamd` + `CLAMD_ADDR`, exactly
/// as the server builds it. The assertion on the adapter name is the point:
/// a leg that silently fell back to none mode would pass every "clean file is
/// admitted" check and prove nothing.
async fn live_state() -> (AppState, Arc<core_store::memory::MemoryStore>) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    assert_eq!(
        std::env::var("EMAIL_SCAN").as_deref(),
        Ok("clamd"),
        "run this leg through scripts/p6f-clamd-live.sh --doors (EMAIL_SCAN=clamd)"
    );
    assert!(
        std::env::var("CLAMD_ADDR").is_ok(),
        "CLAMD_ADDR must point at the live daemon"
    );
    let bootstrap = bootstrap().await;
    let pool = connect_with_retry(&bootstrap.admin_url).await;
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = bootstrap.admin_url.clone();
    let mail = Arc::new(MailStack::from_config(&config));
    assert_eq!(
        mail.scanner.name(),
        "clamd",
        "the config-built scanner is the daemon, not a double"
    );
    make_state(pool, config, mail)
}

/// The same doors with a scanner whose daemon is not running. Built here
/// rather than by changing env vars so the test cannot race the rest of the
/// process.
async fn dead_daemon_state() -> (AppState, Arc<core_store::memory::MemoryStore>) {
    let bootstrap = bootstrap().await;
    let pool = connect_with_retry(&bootstrap.admin_url).await;
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = bootstrap.admin_url.clone();
    let mail = Arc::new(MailStack {
        transport: Arc::new(core_mail::NoopTransport),
        // Nothing listens here. A refused connection is the outage the gate
        // has to turn into a refusal with nothing written.
        scanner: Arc::new(core_mail::ClamdScan::new("127.0.0.1:1")),
        from_address: config.email_from_address.clone(),
        from_name: config.email_from_name.clone(),
    });
    make_state(pool, config, mail)
}

fn make_state(
    pool: sqlx::PgPool,
    config: Config,
    mail: Arc<MailStack>,
) -> (AppState, Arc<core_store::memory::MemoryStore>) {
    let store = Arc::new(core_store::memory::MemoryStore::new());
    let reporting_url = config.database_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail,
        store: Arc::clone(&store) as Arc<dyn core_store::ObjectStore>,
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: AppState::reporting_pool_lazy(&reporting_url).expect("lazy reporting pool"),
    };
    (state, store)
}

async fn send_bytes(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"));
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let request = builder
        .header("content-type", "application/octet-stream")
        .body(axum::body::Body::from(body))
        .unwrap();
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
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

struct Org {
    token: String,
    ticket: String,
    id: Uuid,
}

async fn seed_org(app: &Router, tag: &str) -> Org {
    let slug = format!("p6f-{tag}-{}", Uuid::now_v7().simple());
    let response = send_json(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "P6f Live Co",
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
    let ticket = body_json(
        send_json(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&token),
            Some(json!({
                "board_id": board_id,
                "title": "Live scan target",
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
    let view = body_json(
        send_json(
            app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket}"),
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let id = Uuid::parse_str(
        view["org_id"]
            .as_str()
            .expect("ticket view carries its org"),
    )
    .expect("org uuid");
    Org { token, ticket, id }
}

/// Row counts read with the org scope set, the way the rest of the suite
/// reads tables the request role does not own.
async fn count_rows(state: &AppState, org: Uuid, sql: &str) -> i64 {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("org scope");
    sqlx::query_scalar::<_, i64>(sql)
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .expect("count")
}

async fn scan_row(state: &AppState, org: Uuid, sha256: &str) -> Option<(String, String, String)> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("org scope");
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT status, scanner, storage_key FROM attachment_scans \
         WHERE org_id = $1 AND sha256 = $2",
    )
    .bind(org)
    .bind(sha256)
    .fetch_optional(&mut *tx)
    .await
    .expect("scan row")
}

fn sha_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn puts(store: &core_store::memory::MemoryStore) -> Vec<String> {
    store
        .ops()
        .iter()
        .filter_map(|op| match op {
            core_store::ObjectRecordedOp::Put { key } => Some(key.clone()),
            core_store::ObjectRecordedOp::Delete { .. } => None,
        })
        .collect()
}

/// CRC-32 (reflected, poly 0xEDB88320) so the ZIP below is a real archive and
/// not a blob that happens to start with `PK\x03\x04`.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A stored (uncompressed) single-entry ZIP holding the EICAR test file.
/// The door admits it as an allowlisted archive and clamd finds the signature
/// inside, which is the shape a real infected attachment takes.
fn eicar_zip() -> Vec<u8> {
    let name = b"eicar.txt";
    let crc = crc32(EICAR);
    let size = EICAR.len() as u32;
    let mut out = Vec::new();
    let push = |out: &mut Vec<u8>, bytes: &[u8]| out.extend_from_slice(bytes);

    // Local file header.
    push(&mut out, b"PK\x03\x04");
    push(&mut out, &20u16.to_le_bytes()); // version needed
    push(&mut out, &0u16.to_le_bytes()); // flags
    push(&mut out, &0u16.to_le_bytes()); // stored
    push(&mut out, &0u16.to_le_bytes()); // mod time
    push(&mut out, &0u16.to_le_bytes()); // mod date
    push(&mut out, &crc.to_le_bytes());
    push(&mut out, &size.to_le_bytes());
    push(&mut out, &size.to_le_bytes());
    push(&mut out, &(name.len() as u16).to_le_bytes());
    push(&mut out, &0u16.to_le_bytes()); // extra len
    push(&mut out, name);
    push(&mut out, EICAR);
    let local_len = out.len() as u32;

    // Central directory.
    push(&mut out, b"PK\x01\x02");
    push(&mut out, &20u16.to_le_bytes()); // version made by
    push(&mut out, &20u16.to_le_bytes()); // version needed
    push(&mut out, &0u16.to_le_bytes());
    push(&mut out, &0u16.to_le_bytes());
    push(&mut out, &0u16.to_le_bytes());
    push(&mut out, &0u16.to_le_bytes());
    push(&mut out, &crc.to_le_bytes());
    push(&mut out, &size.to_le_bytes());
    push(&mut out, &size.to_le_bytes());
    push(&mut out, &(name.len() as u16).to_le_bytes());
    push(&mut out, &0u16.to_le_bytes()); // extra
    push(&mut out, &0u16.to_le_bytes()); // comment
    push(&mut out, &0u16.to_le_bytes()); // disk
    push(&mut out, &0u16.to_le_bytes()); // internal attrs
    push(&mut out, &0u32.to_le_bytes()); // external attrs
    push(&mut out, &0u32.to_le_bytes()); // local header offset
    push(&mut out, name);
    let cd_len = out.len() as u32 - local_len;

    // End of central directory.
    push(&mut out, b"PK\x05\x06");
    push(&mut out, &0u16.to_le_bytes()); // disk
    push(&mut out, &0u16.to_le_bytes()); // cd disk
    push(&mut out, &1u16.to_le_bytes()); // entries on disk
    push(&mut out, &1u16.to_le_bytes()); // entries total
    push(&mut out, &cd_len.to_le_bytes());
    push(&mut out, &local_len.to_le_bytes());
    push(&mut out, &0u16.to_le_bytes()); // comment len
    out
}

/// The daemon's `StreamMaxLength`, as the boot script configures it. The test
/// sends past it deliberately.
fn stream_limit() -> usize {
    std::env::var("CLAMD_LIVE_STREAM_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024 * 1024)
}

// --- the tests -------------------------------------------------------------

/// The ticket door, end to end: a real daemon finds the signature inside a
/// real attachment, and the result is the typed 422 that says the bytes are
/// held. Nothing is admitted; the only stored object is the quarantine copy;
/// the verdict row names the live daemon.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn ticket_door_refuses_eicar_and_holds_only_the_quarantine_copy() {
    let (state, store) = live_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "ticket-infected").await;
    let bytes = eicar_zip();
    let sha = sha_of(&bytes);

    let response = send_bytes(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/attachments", org.ticket),
        &org.token,
        &[
            ("x-filename", "invoice.zip"),
            ("content-type", "application/zip"),
        ],
        bytes.clone(),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;

    assert_eq!(status, 422, "an infection is a refusal: {body}");
    assert_eq!(body["error"]["reason"], "upload_quarantined");
    assert_eq!(body["error"]["scanner"], "clamd", "{body}");
    let detail = body["error"]["detail"]
        .as_str()
        .unwrap_or_else(|| panic!("the daemon's signature name rides the refusal: {body}"));
    assert!(
        detail.contains("Eicar"),
        "the detail is the daemon's finding, not a fallback: {detail}"
    );
    assert_eq!(body["error"]["sha256"], json!(sha));

    // Nothing admitted: no attachment row, and no object at the admitted key.
    let admitted = count_rows(
        &state,
        org.id,
        "SELECT count(*) FROM attachments WHERE org_id = $1",
    )
    .await;
    assert_eq!(admitted, 0, "the refused file attached to nothing");
    let keys = puts(&store);
    let admitted_key = core_store::attachment_key(org.id, &sha);
    assert!(
        !keys.contains(&admitted_key),
        "no bytes at the admitted key: {keys:?}"
    );
    let quarantine_key = core_store::quarantine_key(org.id, &sha);
    assert!(
        keys.iter().filter(|k| **k == quarantine_key).count() == 1,
        "the bytes are held once, under the quarantine key: {keys:?}"
    );

    let (status, scanner, storage_key) = scan_row(&state, org.id, &sha)
        .await
        .expect("a held object has a verdict row");
    assert_eq!(status, "infected");
    assert_eq!(scanner, "clamd");
    assert_eq!(storage_key, quarantine_key);
}

/// The same door with clean octets: admitted, stored at the content key, and
/// recorded clean by the daemon — not skipped, which is the none-mode answer.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn ticket_door_admits_clean_octets_and_records_clamd_clean() {
    let (state, store) = live_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "ticket-clean").await;
    let bytes =
        b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";
    let sha = sha_of(bytes);

    let response = send_bytes(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/attachments", org.ticket),
        &org.token,
        &[
            ("x-filename", "quote.pdf"),
            ("content-type", "application/pdf"),
        ],
        bytes.to_vec(),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "a clean file is admitted: {body}");

    assert_eq!(
        count_rows(
            &state,
            org.id,
            "SELECT count(*) FROM attachments WHERE org_id = $1"
        )
        .await,
        1
    );
    assert!(
        puts(&store).contains(&core_store::attachment_key(org.id, &sha)),
        "the admitted bytes are at the content key"
    );
    let (status, scanner, _) = scan_row(&state, org.id, &sha)
        .await
        .expect("an admitted object has a verdict row");
    assert_eq!(status, "clean", "a real scan, not a skip");
    assert_eq!(scanner, "clamd");
}

/// The document door shares the key space and the gate, so it must reach the
/// same verdict through the daemon. Same payload, different route.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn document_door_refuses_eicar_the_same_way() {
    let (state, store) = live_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "document-infected").await;
    let bytes = eicar_zip();
    let sha = sha_of(&bytes);

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

    let response = send_bytes(
        &app,
        axum::http::Method::POST,
        &format!("/api/documents/{document_id}/attachments"),
        &org.token,
        &[
            ("x-filename", "runbook.zip"),
            ("content-type", "application/zip"),
        ],
        bytes,
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["reason"], "upload_quarantined");
    assert_eq!(body["error"]["scanner"], "clamd");
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Eicar"),
        "{body}"
    );

    let admitted_key = core_store::attachment_key(org.id, &sha);
    assert!(
        !puts(&store).contains(&admitted_key),
        "a document attachment refused by the scan is not admitted"
    );
}

/// The vault door keeps NOTHING on an infection (lane P6d): the payload would
/// otherwise sit unsealed in the one unencrypted namespace. A real daemon has
/// to produce the finding, the route has to refuse, and the record must be
/// byte-less afterwards.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn vault_door_refuses_eicar_and_quarantines_nothing() {
    let (state, store) = live_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "vault-infected").await;
    let sha = sha_of(EICAR);

    let folders = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            "/api/secrets/folders",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    let folder_id = folders
        .as_array()
        .and_then(|list| list.iter().find(|f| f["client_id"].is_null()))
        .and_then(|f| f["id"].as_str())
        .expect("a default vault folder")
        .to_string();
    let record = body_json(
        send_json(
            &app,
            axum::http::Method::POST,
            "/api/secrets/records",
            Some(&org.token),
            Some(json!({"folder_id": folder_id, "kind": "document", "title": "Live doc"})),
        )
        .await,
    )
    .await;
    let record_id = record["id"].as_str().expect("record id").to_string();

    // Raw EICAR octets: this door validates no magic bytes, so the test file
    // can arrive as itself.
    let response = send_bytes(
        &app,
        axum::http::Method::PUT,
        &format!("/api/secrets/records/{record_id}/file"),
        &org.token,
        &[],
        EICAR.to_vec(),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["reason"], "upload_refused",
        "the vault refuses instead of holding: {body}"
    );
    assert_eq!(body["error"]["scanner"], "clamd");
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Eicar"),
        "{body}"
    );

    // Nothing quarantined, nothing sealed, no verdict row: the vault keeps an
    // absence, not a held object.
    let keys = puts(&store);
    assert!(
        keys.iter().all(|k| !k.contains("/quarantine/")),
        "the vault never writes a quarantine copy: {keys:?}"
    );
    assert_eq!(
        scan_row(&state, org.id, &sha).await,
        None,
        "no verdict row survives a vault refusal"
    );
    let after = body_json(
        send_json(
            &app,
            axum::http::Method::GET,
            &format!("/api/secrets/records/{record_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        after["object_key"].is_null(),
        "the record never pinned a pointer: {after}"
    );
}

/// A stream past the daemon's `StreamMaxLength` is answered
/// `INSTREAM size limit exceeded. ERROR`, which is not a verdict. The gate
/// fails closed: 503, nothing stored, no "clean" verdict anywhere. The bytes
/// are refused rather than scanned, and this is the test that says so.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn oversized_stream_is_refused_and_never_recorded_clean() {
    let (state, store) = live_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "oversize").await;

    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.resize(stream_limit() + 64 * 1024, 0u8);
    let sha = sha_of(&bytes);

    let response = send_bytes(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/attachments", org.ticket),
        &org.token,
        &[
            ("x-filename", "big.pdf"),
            ("content-type", "application/pdf"),
        ],
        bytes,
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;

    assert_eq!(
        status, 503,
        "past the daemon's limit is a refusal, not a clean scan: {body}"
    );
    assert_eq!(body["error"]["reason"], "service_unavailable");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("scanner"),
        "the refusal names the scanner: {body}"
    );
    assert_eq!(
        count_rows(
            &state,
            org.id,
            "SELECT count(*) FROM attachments WHERE org_id = $1"
        )
        .await,
        0,
        "nothing was attached"
    );
    assert_eq!(
        scan_row(&state, org.id, &sha).await,
        None,
        "nothing was recorded about bytes that were never scanned"
    );
    assert!(
        puts(&store).is_empty(),
        "nothing was stored: {:?}",
        puts(&store)
    );
}

/// A stopped daemon is an outage, not a verdict: 503 and nothing written.
#[tokio::test]
#[ignore = "live leg: needs a clamd container with a signature DB; run scripts/p6f-clamd-live.sh --doors"]
async fn stopped_daemon_refuses_with_503_and_writes_nothing() {
    let (state, store) = dead_daemon_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, "daemon-down").await;
    let bytes = eicar_zip();

    let response = send_bytes(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{}/attachments", org.ticket),
        &org.token,
        &[
            ("x-filename", "invoice.zip"),
            ("content-type", "application/zip"),
        ],
        bytes,
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;

    assert_eq!(
        status, 503,
        "an unreachable scanner refuses the upload: {body}"
    );
    assert_eq!(body["error"]["reason"], "service_unavailable");
    assert_eq!(
        count_rows(
            &state,
            org.id,
            "SELECT count(*) FROM attachments WHERE org_id = $1"
        )
        .await,
        0
    );
    assert!(
        puts(&store).is_empty(),
        "a refusal stores nothing, not even a quarantine copy: {:?}",
        puts(&store)
    );
}
