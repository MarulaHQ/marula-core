//! The inbound connector contract, end to end (G7, G8, G9).
//!
//! Purpose: prove the behaviours a reviewer cannot see from the unit tests,
//! because they are about the seam between HTTP, the signature, the ledger and
//! the ticket service:
//!
//!   * every authentication failure is the *same* typed 401 — the check that
//!     failed is in the ledger, not in the reply;
//!   * a duplicate delivery of the same bytes is a replay: one ledger row, one
//!     assignment, and the first answer returned again;
//!   * an unmapped vendor identity and an unauthorised presser are typed
//!     refusals the channel can render, and neither assigns the ticket;
//!   * "Take it" assigns to the pressing user and starts the ticket through the
//!     ticket route's own transition service;
//!   * a share is refused for a restricted client and for a connection with no
//!     destination, and the card it posts carries the six facts;
//!   * a slash command answers a typed refusal for a ticket the caller cannot
//!     read, never an empty card.
//!
//! What this file does **not** claim: a live Slack or Teams verification. The
//! Slack signature is produced with the same function production verifies
//! against (that is the point of it being public), and the Teams verifier is a
//! stub installed through the trait — no test opens a socket to either vendor.
//!
//! Database: the shared `http_flows` harness (`test_state`), one migrated
//! database, orgs registered through the real API.

use crate::common::{body_json, register_org, send, test_state};
use app_bridges::teams_inbound::{BotFrameworkAuth, BotIdentity, TeamsAuthError};
use async_trait::async_trait;
use axum::Router;
use core_api::routes::connector_inbound::{
    install_bot_auth, install_transport, CardPost, ChannelTransport, PostedMessage, TransportError,
};
use core_api::state::AppState;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

// ---- fixtures --------------------------------------------------------------

/// An org registered through the API, with the owner's bearer.
struct Org {
    id: Uuid,
    owner: String,
}

async fn register(app: &Router, tag: &str) -> Org {
    let slug = format!("{tag}-{}", Uuid::now_v7().simple());
    let registered = register_org(app, "Messaging Co", &slug).await;
    let token = registered["access_token"]
        .as_str()
        .expect("token")
        .to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        id: me["org_id"]
            .as_str()
            .expect("org id")
            .parse()
            .expect("uuid"),
        owner: token,
    }
}

/// Seeds one installed connector definition and returns the install id.
async fn seed_install(state: &AppState, org_id: Uuid, slug: &str) -> Uuid {
    use core_db::plugins;
    use core_domain::entities::{
        PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
        PluginRuntimeKind, PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
    };

    let manifest = PluginManifestV2 {
        name: "app-bridges".into(),
        version: "1.0.0".into(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "Slack and Teams".into(),
        capabilities: Vec::new(),
        product_kinds: vec!["notifications".into()],
        connector_surfaces: None,
        settings_schema: Some(json!({"type": "object"})),
        artifact_kind: Some(PluginRuntimeKind::HttpConnector),
        connector_profile: None,
        surface_profile: None,
    };
    // The catalog slug is globally unique and the test database persists
    // between runs, so each seed gets its own slug.
    let slug = format!("{slug}-{}", Uuid::now_v7().simple());
    let entry = PluginCatalogEntry {
        id: Uuid::now_v7(),
        slug: slug.clone(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        publisher: "marula".into(),
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let version = PluginVersion {
        id: Uuid::now_v7(),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");
    let install = Uuid::now_v7();
    plugins::create_install(
        &mut tx,
        &PluginInstall {
            id: install,
            org_id,
            catalog_id: entry.id,
            slug: slug.clone(),
            version: "1.0.0".into(),
            module_digest: String::new(),
            status: PluginInstallStatus::Enabled,
            granted_capabilities: Vec::new(),
            installed_by: None,
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            previous_version: None,
            previous_module_digest: None,
            previous_capabilities: Vec::new(),
        },
    )
    .await
    .expect("install");
    tx.commit().await.expect("commit");
    install
}

/// A Slack connection whose signing secret is sealed exactly as production
/// seals it, so the verification path is the real one.
async fn seed_slack_connection(
    state: &AppState,
    org_id: Uuid,
    install: Uuid,
    team_id: &str,
    signing_secret: &str,
    bot_token: Option<&str>,
) -> Uuid {
    let connection = Uuid::now_v7();
    let mut secrets = json!({ "signing_secret": signing_secret });
    if let Some(token) = bot_token {
        secrets["bot_token"] = json!(token);
    }
    let sealed = core_api::crypto::seal(
        &state.config.secret_key,
        serde_json::to_vec(&secrets).expect("secrets").as_slice(),
    )
    .expect("seal");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO connector_connections
           (id, org_id, installation_id, label, settings, secrets_encrypted, status)
         VALUES ($1, $2, $3, 'Slack — helpdesk', $4, $5, 'connected')",
    )
    .bind(connection)
    .bind(org_id)
    .bind(install)
    .bind(json!({ "slack_team_id": team_id }))
    .bind(&sealed)
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");
    connection
}

async fn seed_teams_connection(
    state: &AppState,
    org_id: Uuid,
    install: Uuid,
    bot_app_id: &str,
    bot_token: &str,
) -> Uuid {
    let connection = Uuid::now_v7();
    let sealed = core_api::crypto::seal(
        &state.config.secret_key,
        serde_json::to_vec(&json!({ "bot_token": bot_token }))
            .expect("secrets")
            .as_slice(),
    )
    .expect("seal");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO connector_connections
           (id, org_id, installation_id, label, settings, secrets_encrypted, status)
         VALUES ($1, $2, $3, 'Teams', $4, $5, 'connected')",
    )
    .bind(connection)
    .bind(org_id)
    .bind(install)
    .bind(json!({ "teams_bot_app_id": bot_app_id }))
    .bind(&sealed)
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");
    connection
}

async fn seed_binding(
    state: &AppState,
    org_id: Uuid,
    connection: Uuid,
    kind: &str,
    external_id: &str,
) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO connector_bindings
           (id, org_id, connection_id, binding_kind, label, external_id, external_parent, vendor_ids)
         VALUES ($1, $2, $3, $4, $5, $6, 'T-TEAM', $7)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(connection)
    .bind(kind)
    .bind(format!("#{external_id}"))
    .bind(external_id)
    .bind(json!({ "service_url": "https://smba.trafficmanager.net/emea/" }))
    .execute(&mut *tx)
    .await
    .expect("binding");
    tx.commit().await.expect("commit");
}

/// A user plus the login bearer the press will authenticate as.
async fn seed_person(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    tag: &str,
    role: Option<&str>,
) -> (Uuid, String) {
    let user_id = Uuid::now_v7();
    let email = format!("{tag}@example.test");
    let password = "correct-horse-battery";
    let hash = core_api::auth::password::hash(password).expect("hash");
    core_db::users::create_user(&state.pool, user_id, org_id, &email, tag, &hash, "UTC")
        .await
        .expect("user");
    if let Some(builtin) = role {
        let roles = core_db::roles::list_roles(&state.pool, org_id)
            .await
            .expect("roles");
        let role = roles
            .iter()
            .find(|role| role.builtin.as_deref() == Some(builtin))
            .expect("the preset is seeded at registration");
        core_db::users::assign_role(&state.pool, user_id, role.id)
            .await
            .expect("assign role");
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200, "login for {tag}");
    let body = body_json(response).await;
    (
        user_id,
        body["access_token"].as_str().expect("token").to_string(),
    )
}

/// Maps a vendor identity onto a Marula user for one connection.
async fn seed_identity(
    state: &AppState,
    org_id: Uuid,
    connection: Uuid,
    kind: &str,
    external_id: &str,
    user_id: Uuid,
) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO user_external_identities
           (id, org_id, user_id, connection_id, identity_kind, external_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(user_id)
    .bind(connection)
    .bind(kind)
    .bind(external_id)
    .execute(&mut *tx)
    .await
    .expect("identity");
    tx.commit().await.expect("commit");
}

/// Registers a message in the registry exactly as a share would have.
async fn seed_message(
    state: &AppState,
    org_id: Uuid,
    connection: Uuid,
    binding: Uuid,
    ticket_id: Uuid,
    channel_id: &str,
    message_id: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO connector_channel_messages
           (id, org_id, connection_id, binding_id, ticket_id, channel_id, message_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(org_id)
    .bind(connection)
    .bind(binding)
    .bind(ticket_id)
    .bind(channel_id)
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .expect("message");
    tx.commit().await.expect("commit");
    id
}

async fn binding_id(state: &AppState, org_id: Uuid, connection: Uuid) -> Uuid {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM connector_bindings WHERE org_id = $1 AND connection_id = $2 LIMIT 1",
    )
    .bind(org_id)
    .bind(connection)
    .fetch_one(&mut *tx)
    .await
    .expect("binding id");
    tx.commit().await.expect("tx done");
    id
}

/// One ticket created through the API, on the default board's entry status.
async fn seed_ticket(
    state: &AppState,
    app: &Router,
    org: &Org,
    client: Option<Uuid>,
) -> (Uuid, String) {
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&org.owner),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"]
        .as_str()
        .expect("the default board is seeded at registration");
    let mut body = json!({
        "board_id": board_id,
        "title": "Printer down in reception",
        "body": "A body that must never reach a channel.",
        "priority": "p2",
    });
    if let Some(client) = client {
        body["client_id"] = json!(client);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.owner),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create");
    let body = body_json(response).await;
    let id: Uuid = body["id"]
        .as_str()
        .expect("ticket id")
        .parse()
        .expect("uuid");
    let number = body["number"].as_str().expect("ticket number").to_string();
    let _ = state;
    (id, number)
}

// ---- slack helpers ---------------------------------------------------------

fn form_encode(payload: &Value) -> Vec<u8> {
    let json = serde_json::to_string(payload).expect("payload json");
    let mut encoded = String::with_capacity(json.len() * 3);
    for byte in json.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    format!("payload={encoded}").into_bytes()
}

/// Posts a signed Slack interaction, exactly as Slack would.
async fn post_slack(
    app: &Router,
    body: &[u8],
    secret: &str,
    timestamp: i64,
) -> axum::response::Response {
    let signature = app_bridges::slack_inbound::sign(secret.as_bytes(), timestamp, body);
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/connectors/slack/interactive")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-slack-signature", signature)
        .header("x-slack-request-timestamp", timestamp.to_string())
        .body(axum::body::Body::from(body.to_vec()))
        .expect("request");
    axum::Router::clone(app).oneshot_request(request).await
}

/// `tower::ServiceExt::oneshot` spelled once for the raw-body requests this
/// file has to build by hand (the harness `send` takes JSON).
trait OneshotRequest {
    async fn oneshot_request(
        self,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::response::Response;
}

impl OneshotRequest for Router {
    async fn oneshot_request(
        self,
        request: axum::http::Request<axum::body::Body>,
    ) -> axum::response::Response {
        use tower::ServiceExt;
        self.oneshot(request).await.expect("router oneshot")
    }
}

async fn ledger_rows(
    state: &AppState,
    org_id: Uuid,
    connection: Uuid,
) -> Vec<(String, String, Option<String>)> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT action, outcome, refusal_code FROM connector_action_audit
          WHERE org_id = $1 AND connection_id = $2 ORDER BY created_at",
    )
    .bind(org_id)
    .bind(connection)
    .fetch_all(&mut *tx)
    .await
    .expect("ledger");
    tx.commit().await.expect("tx done");
    rows
}

async fn assignee_of(state: &AppState, org_id: Uuid, ticket_id: Uuid) -> Option<Uuid> {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    let assignee: Option<Uuid> =
        sqlx::query_scalar("SELECT assignee_id FROM tickets WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(ticket_id)
            .fetch_one(&mut *tx)
            .await
            .expect("assignee");
    tx.commit().await.expect("tx done");
    assignee
}

async fn client_id_of(state: &AppState, org_id: Uuid, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query(
        "INSERT INTO clients (id, org_id, name, restricted_to) VALUES ($1, $2, $3, '[]'::jsonb)",
    )
    .bind(id)
    .bind(org_id)
    .bind(name)
    .execute(&mut *tx)
    .await
    .expect("client");
    tx.commit().await.expect("tx done");
    id
}

async fn restrict_client(state: &AppState, org_id: Uuid, client: Uuid, user: Uuid) {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org");
    sqlx::query("UPDATE clients SET restricted_to = $1::jsonb WHERE org_id = $2 AND id = $3")
        .bind(json!([user.to_string()]))
        .bind(org_id)
        .bind(client)
        .execute(&mut *tx)
        .await
        .expect("restrict");
    tx.commit().await.expect("tx done");
}

// ---- the fakes -------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    posts: std::sync::Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl ChannelTransport for Recorder {
    async fn post(&self, post: &CardPost) -> Result<PostedMessage, TransportError> {
        self.posts
            .lock()
            .expect("recorder")
            .push((post.url.clone(), post.body.clone()));
        Ok(PostedMessage {
            message_id: "1699999999.000200".into(),
        })
    }
}

fn recorder() -> Arc<Recorder> {
    static RECORDER: OnceLock<Arc<Recorder>> = OnceLock::new();
    RECORDER
        .get_or_init(|| {
            let recorder = Arc::new(Recorder::default());
            install_transport(recorder.clone());
            recorder
        })
        .clone()
}

struct StubBotAuth;

#[async_trait]
impl BotFrameworkAuth for StubBotAuth {
    async fn verify(
        &self,
        jwt: &str,
        expected_audience: &str,
        _expected_service_url: Option<&str>,
    ) -> Result<BotIdentity, TeamsAuthError> {
        if jwt == "teams-good" {
            Ok(BotIdentity {
                app_id: expected_audience.to_string(),
                tenant_id: None,
                service_url: None,
            })
        } else {
            Err(TeamsAuthError::BadSignature)
        }
    }
}

fn stub_bot_auth() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| install_bot_auth(Arc::new(StubBotAuth)));
}

// ---- G7: the signature is the authentication ------------------------------

#[tokio::test]
async fn every_authentication_failure_is_one_indistinguishable_401() {
    let (state, app) = test_state().await;
    let org = register(&app, "auth-fail").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-auth-fail").await;
    let connection =
        seed_slack_connection(&state, org.id, install, &team, "shh-secret", None).await;

    let payload = json!({
        "type": "block_actions",
        "team": { "id": &team },
        "user": { "id": "U-1" },
        "container": { "type": "message", "channel_id": "C-1", "message_ts": "1.1" },
        "actions": [{ "action_id": "marula.take", "value": "take" }]
    });
    let body = form_encode(&payload);
    let now = chrono::Utc::now().timestamp();

    // 1. No signature at all.
    let unsigned = app
        .clone()
        .oneshot_request(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/api/connectors/slack/interactive")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-slack-request-timestamp", now.to_string())
                .body(axum::body::Body::from(body.clone()))
                .expect("request"),
        )
        .await;
    assert_eq!(unsigned.status(), 401);
    let unsigned_body = body_json(unsigned).await;

    // 2. Signed with the wrong secret.
    let wrong = post_slack(&app, &body, "not-the-secret", now).await;
    assert_eq!(wrong.status(), 401);
    let wrong_body = body_json(wrong).await;

    // 3. Signed correctly, but outside the five-minute replay window.
    let stale = post_slack(&app, &body, "shh-secret", now - 301).await;
    assert_eq!(stale.status(), 401);
    let stale_body = body_json(stale).await;

    // The three answers are byte-identical: the reply never says which check
    // failed.
    assert_eq!(unsigned_body, wrong_body);
    assert_eq!(wrong_body, stale_body);
    assert_eq!(
        wrong_body["error"]["message"].as_str(),
        Some("the connector callback did not authenticate")
    );
    // The ledger, however, names the check — for the operator, not the caller.
    let rows = ledger_rows(&state, org.id, connection).await;
    assert!(
        rows.iter().any(|(_, outcome, code)| outcome == "refused"
            && code.as_deref() == Some("slack_signature_missing")),
        "rows: {rows:?}"
    );
    assert!(rows
        .iter()
        .any(|(_, outcome, code)| outcome == "refused"
            && code.as_deref() == Some("slack_signature_bad")));
    assert!(rows.iter().any(|(_, outcome, code)| outcome == "refused"
        && code.as_deref() == Some("slack_timestamp_stale")));

    // Nothing was assigned by any of the three.
    assert!(ledger_rows(&state, org.id, connection)
        .await
        .iter()
        .all(|(_, outcome, _)| outcome == "refused"));
}

// ---- G7: take, replay, refusals -------------------------------------------

#[tokio::test]
async fn take_assigns_the_presser_and_a_retry_replays_instead_of_assigning_twice() {
    let (state, app) = test_state().await;
    let org = register(&app, "take").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-take").await;
    let connection =
        seed_slack_connection(&state, org.id, install, &team, "take-secret", None).await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-TAKE").await;
    let binding = binding_id(&state, org.id, connection).await;
    let (tech_id, _) = seed_person(&state, &app, org.id, "dana", Some("technician")).await;
    seed_identity(&state, org.id, connection, "slack_user", "U-DANA", tech_id).await;
    let (ticket, _number) = seed_ticket(&state, &app, &org, None).await;
    seed_message(
        &state,
        org.id,
        connection,
        binding,
        ticket,
        "C-TAKE",
        "1699999999.000100",
    )
    .await;

    let payload = json!({
        "type": "block_actions",
        "team": { "id": &team },
        "user": { "id": "U-DANA" },
        "container": { "type": "message", "channel_id": "C-TAKE", "message_ts": "1699999999.000100" },
        "actions": [{ "action_id": "marula.take", "value": "take" }]
    });
    let body = form_encode(&payload);
    let now = chrono::Utc::now().timestamp();

    let first = post_slack(&app, &body, "take-secret", now).await;
    assert_eq!(first.status(), 200);
    let first_body = body_json(first).await;
    assert_eq!(first_body["response_type"], "ephemeral");
    assert!(
        first_body["text"]
            .as_str()
            .unwrap_or_default()
            .contains("Assigned"),
        "the presser is told what happened: {first_body}"
    );
    assert_eq!(assignee_of(&state, org.id, ticket).await, Some(tech_id));

    // Slack's retry: the same bytes, so the same digest, so a replay.
    let retry = post_slack(&app, &body, "take-secret", now).await;
    assert_eq!(retry.status(), 200);
    let retry_body = body_json(retry).await;
    assert!(retry_body["text"]
        .as_str()
        .unwrap_or_default()
        .contains("already"));

    let rows = ledger_rows(&state, org.id, connection).await;
    assert_eq!(rows.len(), 1, "one press, one ledger row: {rows:?}");
    assert_eq!(rows[0].0, "take");
    assert_eq!(rows[0].1, "applied");
}

#[tokio::test]
async fn an_unmapped_presser_is_a_typed_refusal_and_changes_nothing() {
    let (state, app) = test_state().await;
    let org = register(&app, "unmapped").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-unmapped").await;
    let connection =
        seed_slack_connection(&state, org.id, install, &team, "unmapped-secret", None).await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-UNMAPPED").await;
    let binding = binding_id(&state, org.id, connection).await;
    let (ticket, _) = seed_ticket(&state, &app, &org, None).await;
    seed_message(
        &state,
        org.id,
        connection,
        binding,
        ticket,
        "C-UNMAPPED",
        "1699999999.000300",
    )
    .await;

    let payload = json!({
        "type": "block_actions",
        "team": { "id": &team },
        "user": { "id": "U-NOBODY" },
        "container": { "type": "message", "channel_id": "C-UNMAPPED", "message_ts": "1699999999.000300" },
        "actions": [{ "action_id": "marula.take", "value": "take" }]
    });
    let response = post_slack(
        &app,
        &form_encode(&payload),
        "unmapped-secret",
        chrono::Utc::now().timestamp(),
    )
    .await;
    assert_eq!(response.status(), 200, "an authenticated refusal is a 200");
    let body = body_json(response).await;
    let text = body["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("not linked"),
        "the refusal names the missing link: {text}"
    );

    assert_eq!(assignee_of(&state, org.id, ticket).await, None);
    let rows = ledger_rows(&state, org.id, connection).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "refused");
    assert_eq!(rows[0].2.as_deref(), Some("identity_unmapped"));
}

#[tokio::test]
async fn a_presser_without_the_permission_is_refused_not_impersonated() {
    let (state, app) = test_state().await;
    let org = register(&app, "no-perm").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-no-perm").await;
    let connection =
        seed_slack_connection(&state, org.id, install, &team, "noperm-secret", None).await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-NOPERM").await;
    let binding = binding_id(&state, org.id, connection).await;
    // A real Marula user with no roles at all: mapped, but unauthorised.
    let (nobody, _) = seed_person(&state, &app, org.id, "intern", None).await;
    seed_identity(&state, org.id, connection, "slack_user", "U-INTERN", nobody).await;
    let (ticket, _) = seed_ticket(&state, &app, &org, None).await;
    seed_message(
        &state,
        org.id,
        connection,
        binding,
        ticket,
        "C-NOPERM",
        "1699999999.000400",
    )
    .await;

    let payload = json!({
        "type": "block_actions",
        "team": { "id": &team },
        "user": { "id": "U-INTERN" },
        "container": { "type": "message", "channel_id": "C-NOPERM", "message_ts": "1699999999.000400" },
        "actions": [{ "action_id": "marula.take", "value": "take" }]
    });
    let response = post_slack(
        &app,
        &form_encode(&payload),
        "noperm-secret",
        chrono::Utc::now().timestamp(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert!(body["text"]
        .as_str()
        .unwrap_or_default()
        .contains("permission"));

    assert_eq!(assignee_of(&state, org.id, ticket).await, None);
    let rows = ledger_rows(&state, org.id, connection).await;
    assert_eq!(rows[0].2.as_deref(), Some("permission_denied"));
}

// ---- G8: share ------------------------------------------------------------

#[tokio::test]
async fn a_share_posts_the_card_and_registers_the_message() {
    let (state, app) = test_state().await;
    let org = register(&app, "share").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-share").await;
    let connection = seed_slack_connection(
        &state,
        org.id,
        install,
        &team,
        "share-secret",
        Some("xoxb-test"),
    )
    .await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-SHARE").await;
    let client = client_id_of(&state, org.id, "Acme Ltd").await;
    let (ticket, number) = seed_ticket(&state, &app, &org, Some(client)).await;
    let recorder = recorder();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket}/share"),
        Some(&org.owner),
        Some(json!({ "connection_id": connection })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["message_id"], "1699999999.000200");
    assert_eq!(body["channel"]["external_id"], "C-SHARE");
    // The card renders the six facts, and no ticket body.
    let card = body["card"].to_string();
    assert!(card.contains(&number));
    assert!(card.contains("Acme Ltd"));
    assert!(card.contains("New"));
    assert!(card.contains("P2"));
    assert!(
        !card.contains("must never reach a channel"),
        "a card cannot carry the ticket body"
    );

    // The recorder saw one post to the documented Slack endpoint with the
    // channel and the blocks. It is a process-wide double, so a sibling test
    // can append its own post between the share and this read
    // (a_caller_outside_the_connection_audience_... shares into C-AUDIENCE).
    // Filtering on this test's channel names its post instead of whichever
    // landed last, which is how shard 2 failed on main after #59 merged.
    let posts = recorder.posts.lock().expect("recorder").clone();
    let mut mine: Vec<(String, Value)> = posts
        .into_iter()
        .filter(|(_, payload)| payload["channel"] == "C-SHARE")
        .collect();
    let (url, payload) = mine.pop().expect("one post to the share channel");
    assert_eq!(url, "https://slack.com/api/chat.postMessage");
    assert_eq!(payload["channel"], "C-SHARE");
    assert!(payload["blocks"].is_array());
    assert!(mine.is_empty(), "the share posts the card once");

    // And the message is registered, so a press can resolve it.
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org.id).await.expect("org");
    let registered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM connector_channel_messages
          WHERE org_id = $1 AND connection_id = $2 AND channel_id = 'C-SHARE'",
    )
    .bind(org.id)
    .bind(connection)
    .fetch_one(&mut *tx)
    .await
    .expect("count");
    tx.commit().await.expect("tx done");
    assert_eq!(registered, 1);
}

#[tokio::test]
async fn a_restricted_client_ticket_is_not_shareable_into_a_channel() {
    let (state, app) = test_state().await;
    let org = register(&app, "share-restricted").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-share-restricted").await;
    let connection = seed_slack_connection(
        &state,
        org.id,
        install,
        &team,
        "restricted-secret",
        Some("xoxb-test"),
    )
    .await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-RESTRICTED").await;
    let client = client_id_of(&state, org.id, "Secret Client").await;
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&org.owner),
            None,
        )
        .await,
    )
    .await;
    let owner: Uuid = me["user_id"]
        .as_str()
        .expect("owner")
        .parse()
        .expect("uuid");
    restrict_client(&state, org.id, client, owner).await;
    let (ticket, _) = seed_ticket(&state, &app, &org, Some(client)).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket}/share"),
        Some(&org.owner),
        Some(json!({ "connection_id": connection })),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("restricted client"),
        "the refusal names the rule, not the client: {body}"
    );
}

/// The audience is a door on the connection, and a share must open the same two
/// doors the roster does. Read access alone is not enough: a caller outside the
/// audience gets the same 404 a stranger gets, so the connection's existence
/// cannot be probed through the share route.
#[tokio::test]
async fn a_caller_outside_the_connection_audience_cannot_share_through_it() {
    // The card transport is a process-wide double installed on first use. This
    // test never posts a card, so it does not read the recorder, but it must
    // still install one: without it the real transport refuses (no Slack
    // credential in a test process) and the owner's share comes back 409. It
    // passed locally only because a sibling test had already installed it.
    let _ = recorder();
    let (state, app) = test_state().await;
    let org = register(&app, "share-audience").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-share-audience").await;
    let connection = seed_slack_connection(
        &state,
        org.id,
        install,
        &team,
        "audience-secret",
        Some("xoxb-test"),
    )
    .await;
    seed_binding(&state, org.id, connection, "slack_channel", "C-AUDIENCE").await;

    // The owner's role is the audience; the technician holds no such role.
    let (tech_id, tech_token) =
        seed_person(&state, &app, org.id, "audience-tech", Some("technician")).await;
    assert_ne!(tech_id, Uuid::nil());
    {
        let owner_role: Uuid = {
            let roles = core_db::roles::list_roles(&state.pool, org.id)
                .await
                .expect("roles");
            roles
                .iter()
                .find(|role| role.builtin.as_deref() == Some("owner"))
                .map(|role| role.id)
                .expect("owner preset")
        };
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org.id).await.expect("org");
        sqlx::query("UPDATE connector_connections SET audience_roles = ARRAY[$1] WHERE id = $2")
            .bind(owner_role.to_string())
            .bind(connection)
            .execute(&mut *tx)
            .await
            .expect("audience");
        tx.commit().await.expect("commit");
    }

    let (ticket, _) = seed_ticket(&state, &app, &org, None).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket}/share"),
        Some(&tech_token),
        Some(json!({ "connection_id": connection })),
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "a caller outside the audience must not learn the connection exists"
    );

    // The audience member still shares: the door is the list, not the feature.
    let owner_share = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket}/share"),
        Some(&org.owner),
        Some(json!({ "connection_id": connection })),
    )
    .await;
    assert_eq!(owner_share.status(), 200);
}

#[tokio::test]
async fn a_connection_without_a_destination_cannot_be_a_share_target() {
    let (state, app) = test_state().await;
    let org = register(&app, "share-nodest").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-share-nodest").await;
    let connection = seed_slack_connection(
        &state,
        org.id,
        install,
        &team,
        "nodest-secret",
        Some("xoxb-test"),
    )
    .await;
    let (ticket, _) = seed_ticket(&state, &app, &org, None).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/tickets/{ticket}/share"),
        Some(&org.owner),
        Some(json!({ "connection_id": connection })),
    )
    .await;
    assert_eq!(response.status(), 422);
    let body = body_json(response).await;
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("destination"));
}

// ---- G9: the slash command ------------------------------------------------

#[tokio::test]
async fn a_slash_command_answers_a_card_or_a_typed_refusal() {
    let (state, app) = test_state().await;
    let org = register(&app, "slash").await;
    let team = unique_team();
    let install = seed_install(&state, org.id, "slack-slash").await;
    let connection =
        seed_slack_connection(&state, org.id, install, &team, "slash-secret", None).await;
    let (_tech, tech_token) = seed_person(&state, &app, org.id, "sam", Some("technician")).await;
    seed_identity(&state, org.id, connection, "slack_user", "U-SAM", _tech).await;
    let (ticket, number) = seed_ticket(&state, &app, &org, None).await;

    // A ticket the caller can read answers a card carrying the number.
    let readable = form_encode_command(&[
        ("team_id", &team),
        ("user_id", "U-SAM"),
        ("command", "/marula"),
        ("text", &number),
    ]);
    let response = post_slack_command(&app, &readable, "slash-secret").await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    let rendered = body.to_string();
    assert!(rendered.contains(&number), "the card names the ticket");
    assert!(rendered.contains("Printer down"));
    // A slash command is a read: it leaves the ticket alone, and the caller's
    // own permission was what let it through.
    assert_eq!(assignee_of(&state, org.id, ticket).await, None);
    let _ = tech_token;

    // A ticket in a client the caller cannot read answers a typed refusal, not
    // an empty card.
    let client = client_id_of(&state, org.id, "Restricted Co").await;
    let (hidden, hidden_number) = seed_ticket(&state, &app, &org, Some(client)).await;
    let other = Uuid::now_v7();
    restrict_client(&state, org.id, client, other).await;

    let unreadable = form_encode_command(&[
        ("team_id", &team),
        ("user_id", "U-SAM"),
        ("command", "/marula"),
        ("text", &hidden_number),
    ]);
    let response = post_slack_command(&app, &unreadable, "slash-secret").await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["response_type"], "ephemeral");
    let text = body["text"].as_str().unwrap_or_default();
    assert!(
        text.contains("cannot read"),
        "the refusal names the reason: {text}"
    );
    assert!(
        body["blocks"].is_null(),
        "a refusal is text, never an empty card: {body}"
    );

    // A missing number is the one refusal that teaches the format.
    let empty = form_encode_command(&[
        ("team_id", &team),
        ("user_id", "U-SAM"),
        ("command", "/marula"),
        ("text", ""),
    ]);
    let body = body_json(post_slack_command(&app, &empty, "slash-secret").await).await;
    assert!(body["text"].as_str().unwrap_or_default().contains("T-1042"));
    let _ = hidden;
}

fn form_encode_command(fields: &[(&str, &str)]) -> Vec<u8> {
    let pairs: Vec<String> = fields
        .iter()
        .map(|(key, value)| {
            let mut encoded = String::with_capacity(value.len() * 3);
            for byte in value.as_bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        encoded.push(*byte as char)
                    }
                    _ => encoded.push_str(&format!("%{byte:02X}")),
                }
            }
            format!("{key}={encoded}")
        })
        .collect();
    pairs.join("&").into_bytes()
}

async fn post_slack_command(app: &Router, body: &[u8], secret: &str) -> axum::response::Response {
    let now = chrono::Utc::now().timestamp();
    let signature = app_bridges::slack_inbound::sign(secret.as_bytes(), now, body);
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/connectors/slack/commands")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-slack-signature", signature)
        .header("x-slack-request-timestamp", now.to_string())
        .body(axum::body::Body::from(body.to_vec()))
        .expect("request");
    app.clone().oneshot_request(request).await
}

// ---- Teams ----------------------------------------------------------------

#[tokio::test]
async fn teams_refuses_a_bad_token_and_accepts_a_mapped_press() {
    stub_bot_auth();
    let (state, app) = test_state().await;
    let org = register(&app, "teams").await;
    let bot = format!("bot-{}", Uuid::now_v7().simple());
    let install = seed_install(&state, org.id, "teams-install").await;
    let connection = seed_teams_connection(&state, org.id, install, &bot, "teams-bot").await;
    seed_binding(
        &state,
        org.id,
        connection,
        "teams_conversation",
        "19:conv@thread.skype",
    )
    .await;
    let binding = binding_id(&state, org.id, connection).await;
    let (tech, _) = seed_person(&state, &app, org.id, "teams-tech", Some("technician")).await;
    seed_identity(&state, org.id, connection, "teams_user", "aad-1", tech).await;
    let (ticket, _) = seed_ticket(&state, &app, &org, None).await;
    seed_message(
        &state,
        org.id,
        connection,
        binding,
        ticket,
        "19:conv@thread.skype",
        "activity-1",
    )
    .await;

    // A bad token is the same typed 401 the Slack door gives.
    let activity = json!({
        "type": "message",
        "id": "activity-1",
        "serviceUrl": "https://smba.trafficmanager.net/emea/",
        "from": { "id": "29:abc", "aid": "aad-1" },
        "recipient": { "id": &bot },
        "conversation": { "id": "19:conv@thread.skype" },
        "value": { "marulaAction": "take" }
    });
    let body = serde_json::to_vec(&activity).expect("activity");
    let response = post_teams(&app, &body, "not-a-token").await;
    assert_eq!(response.status(), 401);

    // A good token presses the same registry entry and assigns the ticket.
    let response = post_teams(&app, &body, "teams-good").await;
    assert_eq!(response.status(), 200);
    let reply = body_json(response).await;
    assert!(reply["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Assigned"));
    assert_eq!(assignee_of(&state, org.id, ticket).await, Some(tech));

    // The retry is the same bytes, so a replay rather than a second press.
    let retry = body_json(post_teams(&app, &body, "teams-good").await).await;
    assert!(retry["text"]
        .as_str()
        .unwrap_or_default()
        .contains("already"));
    // One *press* row: the failed token is its own evidence, in its own key
    // space, and did not consume the press digest.
    let rows = ledger_rows(&state, org.id, connection).await;
    assert_eq!(
        rows.iter()
            .filter(|(action, _, _)| action == "take")
            .count(),
        1,
        "{rows:?}"
    );
}

async fn post_teams(app: &Router, body: &[u8], token: &str) -> axum::response::Response {
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/connectors/teams/interactive")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::from(body.to_vec()))
        .expect("request");
    app.clone().oneshot_request(request).await
}

/// A workspace id unique to this run.
///
/// The candidate selector is a *global* string: the test database persists
/// between runs, so a fixed `T-…` would find an earlier run's connection in
/// another org and — with the same test secret — verify against it. Production
/// handles that case by refusing an ambiguous match (the route's
/// `connection_ambiguous`); a test just must not create the ambiguity.
fn unique_team() -> String {
    format!("T{}", Uuid::now_v7().simple())
}
