//! Transcription metering end-to-end over the REAL router (Wave 54 lane S4b;
//! `qa-p54-security-decisions` record 6).
//!
//! The transcription edge used to write its usage row with
//! `prompt_tokens: 0, completion_tokens: 0`, which left `weighted_tokens` —
//! the generated `(prompt + completion) * cost_weight` column the caps sum —
//! at zero for every call: an org on a hard cap could transcribe without
//! limit while chat was refused. This suite pins the fix on both provider
//! shapes:
//!
//! * a response that carries a `usage` block (the gpt-4o transcription
//!   models report `input_tokens` / `output_tokens`): the ledger row records
//!   exactly those counts;
//! * a response that carries none (the whisper shape: text and duration
//!   only): the row records the derived counts, and never zero.
//!
//! Both rows must satisfy the generated-column formula, which is asserted
//! against the value the DATABASE computed rather than a re-derivation.
//!
//! Non-purpose: UI rendering, objective/per-org isolation (ai_policy_flow),
//! provider adapters beyond the transcription call.
//!
//! The provider is a loopback stub reached through `OPENAI_BASE_URL`, so this
//! suite must be its own test binary: the env var is process-global.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_domain::entities::AttachmentKind;
use core_store::memory::MemoryStore;
use core_store::ObjectStore;
use serde_json::{json, Value};
use sha2::Digest as _;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// The audio bytes the fake provider "transcribes". Content is irrelevant to
/// the edge beyond its size and content type (the route reads the object
/// store, it does not parse audio).
const AUDIO_BYTES: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt fake-audio-payload";

async fn test_state() -> (AppState, Arc<MemoryStore>, PgPool) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("transcription metering e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let store = Arc::new(MemoryStore::new());
    let state = AppState {
        plugin_pool: Default::default(),
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: store.clone(),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    (state, store, pool)
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({e}); status={status} body={:?}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

/// Serves one canned transcription response on a loopback port and returns
/// the base URL a provider call should target.
async fn stub_transcription_provider(body: Value) -> String {
    let app = axum::Router::new().route(
        "/v1/audio/transcriptions",
        axum::routing::post(move || {
            let body = body.clone();
            async move { axum::Json(body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let addr = listener.local_addr().expect("loopback addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

struct Seeded {
    bearer: String,
    org_id: Uuid,
    user_id: Uuid,
    attachment_id: Uuid,
}

/// One org with the AI switch on, an allowlisted model, a sealed provider
/// credential, a `transcription` grant for its owner, and an audio
/// attachment on a ticket — everything the transcribe edge admits on.
async fn seed_org(app: &Router, pool: &PgPool, store: &MemoryStore, tag: &str) -> Seeded {
    let slug = format!("transcribe-{tag}-{}", Uuid::now_v7().simple());
    let hex = Uuid::now_v7().simple().to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Transcribe Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let bearer = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();

    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&bearer), None).await).await;
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();

    // Org master switch: OFF by default, so every AI edge refuses until set.
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/org",
        Some(&bearer),
        Some(json!({ "key": "ai.features_enabled", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 200, "org AI switch");

    // Allowlist the model the edge will resolve (weight 2, so a weighted
    // total that ignores the weight cannot pass by accident).
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/catalog",
        Some(&bearer),
        Some(json!({
            "provider": "openai",
            "model": "gpt-4o-transcribe",
            "label": "GPT-4o transcribe",
            "capabilities": ["completion"],
            "cost_weight": 2,
            "idempotency_key": format!("cat-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "catalog curation");

    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/credentials",
        Some(&bearer),
        Some(json!({
            "provider": "openai",
            "kind": "api_key",
            "secret": "sk-transcription-never-surfaces",
            "label": "org openai",
            "idempotency_key": format!("cred-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "credential seal");

    // The `transcription` surface is seeded globally (0081) and is a grant
    // target; deny-by-default until this row lands.
    let response = send(
        app,
        axum::http::Method::PUT,
        "/api/settings/ai/grants",
        Some(&bearer),
        Some(json!({
            "surface_key": "transcription",
            "grantee_kind": "user",
            "user_id": user_id.to_string(),
            "idempotency_key": format!("grant-{hex}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "transcription grant");

    // A board ticket to hang the audio on (the upload door only accepts the
    // allowlisted document families, so the audio row is written directly —
    // the edge under test reads the row and the object store, not the
    // upload path).
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
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let ticket = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            Some(json!({
                "board_id": board_id,
                "title": "Voice note",
                "priority": "p3",
            })),
        )
        .await,
    )
    .await;
    let ticket_id: Uuid = ticket["id"].as_str().unwrap().parse().unwrap();

    let attachment_id = Uuid::now_v7();
    let sha256 = sha2::Sha256::digest(AUDIO_BYTES)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let storage_key = core_store::attachment_key(org_id, &sha256);
    store
        .put(
            &storage_key,
            "audio/wav",
            axum::body::Bytes::from_static(AUDIO_BYTES),
        )
        .await
        .expect("store the audio object");

    // The row is written on the same pool the handler reads it from.
    let mut tx = pool.begin().await.expect("row tx");
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .expect("org GUC");
    core_db::attachments::insert(
        &mut tx,
        core_db::attachments::NewAttachment {
            id: attachment_id,
            org_id,
            kind: AttachmentKind::Ticket,
            ticket_id: Some(ticket_id),
            ticket_comment_id: None,
            email_message_id: None,
            filename: "voice-note.wav",
            content_type: "audio/wav",
            size_bytes: AUDIO_BYTES.len() as i64,
            sha256: &sha256,
            storage_key: &storage_key,
            uploaded_by_user_id: Some(user_id),
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("attachment row");
    tx.commit().await.expect("row commit");

    Seeded {
        bearer,
        org_id,
        user_id,
        attachment_id,
    }
}

/// The ledger row the edge just wrote, plus the audit detail beside it.
async fn ledger_row(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> (i32, i32, i32, i64, String) {
    let mut tx = pool.begin().await.expect("ledger tx");
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .expect("org GUC");
    let row: (i32, i32, i32, i64) = sqlx::query_as(
        r#"SELECT prompt_tokens, completion_tokens, cost_weight, weighted_tokens
             FROM ai_usage_ledger
            WHERE org_id = $1 AND surface = 'transcription'
            ORDER BY created_at DESC, id DESC
            LIMIT 1"#,
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("the transcription usage row");
    let source: Option<String> = sqlx::query_scalar(
        r#"SELECT detail->>'token_source'
             FROM audit_events
            WHERE org_id = $1 AND action = 'ai.transcription_performed'
            ORDER BY occurred_at DESC
            LIMIT 1"#,
    )
    .bind(org_id)
    .fetch_optional(&mut *tx)
    .await
    .expect("the transcription audit row")
    .flatten();
    tx.commit().await.expect("ledger commit");
    (
        row.0,
        row.1,
        row.2,
        row.3,
        source.unwrap_or_else(|| "missing".into()),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn transcription_records_provider_tokens_and_derived_tokens() {
    let (state, store, pool) = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = seed_org(&app, &pool, &store, "meter").await;

    // --- 1. A provider that reports its own usage -----------------------
    std::env::set_var(
        "OPENAI_BASE_URL",
        stub_transcription_provider(json!({
            "text": "The client called about the printer again.",
            "usage": {
                "type": "tokens",
                "input_tokens": 14,
                "input_token_details": { "text_tokens": 0, "audio_tokens": 14 },
                "output_tokens": 45,
                "total_tokens": 59,
            },
        }))
        .await,
    );
    let uri = format!("/api/attachments/{}/transcribe", org.attachment_id);
    let response = send(
        &app,
        axum::http::Method::POST,
        &uri,
        Some(&org.bearer),
        None,
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "transcription admitted: {body}");
    assert_eq!(body["surface"], "transcription");
    assert_eq!(body["model"], "gpt-4o-transcribe");

    let (prompt, completion, weight, weighted, source) =
        ledger_row(&pool, org.org_id, org.user_id).await;
    assert_eq!(
        prompt, 14,
        "the provider's input token count lands verbatim"
    );
    assert_eq!(
        completion, 45,
        "the provider's output token count lands verbatim"
    );
    assert_eq!(weight, 2, "the catalog weight is snapshotted on the row");
    assert_eq!(
        weighted,
        i64::from(prompt + completion) * i64::from(weight),
        "weighted_tokens is the generated column's (prompt + completion) x weight"
    );
    assert_eq!(
        source, "provider_usage",
        "the audit row names the provenance"
    );

    // --- 2. A provider that reports no usage (the whisper shape) --------
    std::env::set_var(
        "OPENAI_BASE_URL",
        stub_transcription_provider(json!({
            "text": "0123456789",
            "duration": 30.0,
        }))
        .await,
    );
    let response = send(
        &app,
        axum::http::Method::POST,
        &uri,
        Some(&org.bearer),
        None,
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "second transcription admitted: {body}");

    let (prompt, completion, weight, weighted, source) =
        ledger_row(&pool, org.org_id, org.user_id).await;
    assert!(
        prompt > 0 && completion > 0,
        "a metered transcription never records zero tokens (prompt={prompt}, completion={completion})"
    );
    assert_eq!(prompt, 300, "30 s of audio at the documented audio basis");
    assert_eq!(completion, 3, "ten transcript characters at four per token");
    assert_eq!(
        weighted,
        i64::from(prompt + completion) * i64::from(weight),
        "derived counts price through the same generated column"
    );
    assert_eq!(
        source, "derived",
        "the audit row says the counts were derived"
    );

    std::env::remove_var("OPENAI_BASE_URL");
}
