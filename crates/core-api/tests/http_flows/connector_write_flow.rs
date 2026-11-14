//! The connector write path's HTTP contract (ADR 0032; migration 0186):
//! create a connection, add a destination, set the capability default, resolve
//! it, and refuse to delete what something still points at.
//!
//! What this target adds over `core-db/tests/connector_actions_resolution.rs`
//! is the part a client branches on and the part a careless route gets wrong:
//!
//!   * a settings document the pinned manifest does not declare is a 422 that
//!     names the field — never a saved row the vendor receives nothing for;
//!   * a credential crosses one way only: `secrets_set` names it, no response
//!     body contains its value, and `settings` never carries it;
//!   * deleting a connection with destinations or external references is a 409
//!     naming both counts, because a cascade here would erase which vendor
//!     object a configuration is;
//!   * an audience outside the caller's roles yields no provider, and the same
//!     caller still sees a connection with an empty audience.
//!
//! Non-purpose: the resolver's grouping rules (the core-db target) and the
//! settings-schema subset itself (its unit tests in `core-domain`).

use crate::common::{body_json, register_org, send, test_state};
use axum::Router;
use core_api::state::AppState;
use core_domain::entities::{
    ConnectorSurfaces, PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
    PluginRuntimeKind, PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// The manifest a connector author would ship for a per-client RMM: one
/// non-secret setting (`base_url`), one credential (`api_key`), one product
/// kind, and one record block so the resolver has something to draw.
fn manifest() -> PluginManifestV2 {
    PluginManifestV2 {
        name: "ninjaone".into(),
        version: "1.0.0".into(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "Pulls device posture".into(),
        capabilities: Vec::new(),
        product_kinds: vec!["rmm".into()],
        connector_surfaces: Some(ConnectorSurfaces {
            base_url_key: None,
            external_refs: vec![core_domain::entities::ExternalRefBlock {
                record: "configuration".into(),
                external_kind: "device".into(),
                open_url: Some("{base_url}/device/{external_id}".into()),
                launch: Some(core_domain::entities::LaunchBlock {
                    url_template: "{base_url}/device/{external_id}/join".into(),
                    capability: None,
                }),
            }],
        }),
        settings_schema: Some(json!({
            "type": "object",
            "properties": {
                "base_url": { "type": "string", "pattern": "^https://" },
                "api_key": { "type": "string", "writeOnly": true }
            },
            "required": ["base_url", "api_key"]
        })),
        artifact_kind: Some(PluginRuntimeKind::HttpConnector),
        connector_profile: None,
        surface_profile: None,
    }
}

/// Publishes and installs the connector, returning the installation id.
async fn seed_install(state: &AppState, org_id: Uuid, slug: &str) -> Uuid {
    let manifest = manifest();
    let catalog_id = Uuid::now_v7();
    let install_id = Uuid::now_v7();
    let entry = PluginCatalogEntry {
        id: catalog_id,
        slug: slug.into(),
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
        catalog_id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };

    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id)
        .await
        .expect("org binding");
    core_db::plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");
    core_db::plugins::create_install(
        &mut tx,
        &PluginInstall {
            id: install_id,
            org_id,
            catalog_id,
            slug: slug.into(),
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
    install_id
}

/// One client plus one configuration on it.
async fn seed_client_asset(state: &AppState, org_id: Uuid, tag: &str) -> (Uuid, Uuid) {
    let client_id = Uuid::now_v7();
    let asset_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name, restricted_to) VALUES ($1, $2, $3, '[]')")
        .bind(client_id)
        .bind(org_id)
        .bind(format!("Client {tag}"))
        .execute(&state.pool)
        .await
        .expect("client");
    core_db::assets::create_asset(
        &state.pool,
        org_id,
        asset_id,
        Some(client_id),
        "client",
        None,
        tag,
        "Dell Latitude",
        "laptop",
        "active",
        None,
        &json!({}),
    )
    .await
    .expect("asset");
    (client_id, asset_id)
}

fn reason(body: &Value) -> &str {
    body["error"]["reason"].as_str().unwrap_or_default()
}

fn message(body: &Value) -> &str {
    body["error"]["message"].as_str().unwrap_or_default()
}

/// Registers an org and returns (owner token, org id, owner user id).
async fn owner(app: &Router, slug: &str) -> (String, Uuid, Uuid) {
    let registered = register_org(app, "Connector Co", slug).await;
    let token = registered["access_token"]
        .as_str()
        .expect("token")
        .to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    let org_id: Uuid = me["org_id"].as_str().expect("org").parse().expect("uuid");
    let user_id: Uuid = me["user_id"].as_str().expect("user").parse().expect("uuid");
    (token, org_id, user_id)
}

/// A technician's bearer, plus the ids of the org's roles.
async fn technician(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    slug: &str,
) -> (String, Vec<(Uuid, Option<String>)>) {
    let user_id = Uuid::now_v7();
    let email = format!("tech@{slug}.test");
    let password = "correct-horse-battery";
    let hash = core_api::auth::password::hash(password).expect("hash");
    core_db::users::create_user(&state.pool, user_id, org_id, &email, "Tech", &hash, "UTC")
        .await
        .expect("technician");
    let roles = core_db::roles::list_roles(&state.pool, org_id)
        .await
        .expect("roles");
    let role_list: Vec<(Uuid, Option<String>)> = roles
        .iter()
        .map(|role| (role.id, role.builtin.clone()))
        .collect();
    let tech_role = roles
        .iter()
        .find(|role| role.builtin.as_deref() == Some("technician"))
        .expect("the technician preset is seeded at registration")
        .id;
    core_db::users::assign_role(&state.pool, user_id, tech_role)
        .await
        .expect("assign role");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login");
    let body = body_json(response).await;
    (
        body["access_token"]
            .as_str()
            .expect("access token")
            .to_string(),
        role_list,
    )
}

/// The settings document the schema accepts, with the credential kept out of
/// `settings` on purpose.
fn good_body(secret: &str) -> Value {
    json!({
        "label": "NinjaOne — Acme",
        "settings": { "base_url": "https://rmm.example.test" },
        "secrets": { "api_key": secret }
    })
}

/// A settings document the connector never declared is refused, and nothing is
/// stored; the accepted one stores the credential without returning it.
#[tokio::test]
async fn settings_are_validated_and_secrets_never_come_back() {
    let (state, app) = test_state().await;
    let slug = format!("connector-write-{}", Uuid::now_v7().simple());
    let (token, org_id, _owner_id) = owner(&app, &slug).await;
    let install = seed_install(&state, org_id, &format!("ninjaone-{slug}")).await;
    let create_uri = format!("/api/connector-installations/{install}/connections");

    // 1. An unknown key is a typo, not data the vendor will receive.
    let response = send(
        &app,
        axum::http::Method::POST,
        &create_uri,
        Some(&token),
        Some(json!({
            "label": "Typo",
            "settings": { "base_url": "https://rmm.example.test", "base_uri": "https://wrong" },
            "secrets": { "api_key": "k" }
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "an undeclared key is refused");
    let body = body_json(response).await;
    assert_eq!(reason(&body), "validation_failed");
    assert!(
        message(&body).contains("base_uri"),
        "the refusal names the field: {}",
        message(&body)
    );

    // 2. A missing required credential is refused, because a connection that
    //    cannot authenticate is not a connection.
    let response = send(
        &app,
        axum::http::Method::POST,
        &create_uri,
        Some(&token),
        Some(json!({
            "label": "No key",
            "settings": { "base_url": "https://rmm.example.test" }
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "a missing credential is refused");

    // 3. A credential sent through `settings` is refused rather than sealed:
    //    the caller believes it was stored in the clear, and is right.
    let response = send(
        &app,
        axum::http::Method::POST,
        &create_uri,
        Some(&token),
        Some(json!({
            "label": "In the wrong field",
            "settings": { "base_url": "https://rmm.example.test", "api_key": "k" }
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "a credential belongs in `secrets`");

    // 4. The accepted body.
    let secret = format!("tier-two-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        &create_uri,
        Some(&token),
        Some(good_body(&secret)),
    )
    .await;
    assert_eq!(response.status(), 201, "a valid connection lands");
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8(raw.to_vec()).expect("utf8");
    assert!(
        !text.contains(&secret),
        "the sealed credential never crosses the API boundary"
    );
    let created: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        created["secrets_set"],
        json!(["api_key"]),
        "the client learns which credential is configured, by name"
    );
    assert_eq!(
        created["settings"]["base_url"], "https://rmm.example.test",
        "non-secret settings are returned"
    );
    assert!(
        created["settings"]
            .get("api_key")
            .is_none_or(Value::is_null),
        "a credential is never part of settings"
    );
    let connection_id: Uuid = created["id"]
        .as_str()
        .expect("connection id")
        .parse()
        .expect("uuid");

    // 5. The roster says the same thing, and still holds no secret.
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/connectors/roster",
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let roster_text = roster.to_string();
    assert!(!roster_text.contains(&secret));
    let row = &roster["connectors"][0]["connections"][0];
    assert_eq!(row["id"], json!(connection_id));
    assert_eq!(row["secrets_set"], json!(["api_key"]));
    assert_eq!(
        row["audience_roles"],
        json!([]),
        "empty audience is the shipped default"
    );
}

/// A PATCH replaces only what it names: an absent `secrets` keeps the stored
/// credential, a present one merges over it.
#[tokio::test]
async fn patching_settings_and_secrets_merges_rather_than_wipes() {
    let (state, app) = test_state().await;
    let slug = format!("connector-patch-{}", Uuid::now_v7().simple());
    let (token, org_id, _owner_id) = owner(&app, &slug).await;
    let install = seed_install(&state, org_id, &format!("ninjaone-{slug}")).await;

    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/connector-installations/{install}/connections"),
            Some(&token),
            Some(good_body("first-key")),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().expect("id");
    let base = format!("/api/connector-connections/{id}");

    // A label-only patch must not need the credential retyped.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &base,
        Some(&token),
        Some(json!({ "label": "NinjaOne — Acme (renamed)" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let patched = body_json(response).await;
    assert_eq!(patched["label"], "NinjaOne — Acme (renamed)");
    assert_eq!(
        patched["secrets_set"],
        json!(["api_key"]),
        "an absent secrets field preserves what is stored"
    );

    // A replacement credential keeps the rest of the document.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &base,
        Some(&token),
        Some(json!({ "secrets": { "api_key": "second-key" } })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["secrets_set"], json!(["api_key"]));

    // Settings are merged, so a partial document does not drop the origin the
    // resolver needs to build a link.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &base,
        Some(&token),
        Some(json!({ "settings": { "base_url": "https://rmm2.example.test" } })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["settings"]["base_url"], "https://rmm2.example.test");
}

/// Create → add destination → set the default → resolve → refuse to delete →
/// remove what points at it → delete.
#[tokio::test]
async fn a_connection_round_trips_create_bind_default_resolve_delete() {
    let (state, app) = test_state().await;
    let slug = format!("connector-round-{}", Uuid::now_v7().simple());
    let (token, org_id, _owner_id) = owner(&app, &slug).await;
    let install = seed_install(&state, org_id, &format!("ninjaone-{slug}")).await;
    let (client_id, asset_id) = seed_client_asset(&state, org_id, "LT-ROUND").await;

    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/connector-installations/{install}/connections"),
            Some(&token),
            Some(json!({
                "label": "NinjaOne — Acme",
                "scope_client_id": client_id,
                "settings": { "base_url": "https://rmm.example.test" },
                "secrets": { "api_key": "k" }
            })),
        )
        .await,
    )
    .await;
    let connection_id: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");
    assert_eq!(created["scope_client_id"], json!(client_id));

    // A destination, so the connection can be a share target.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/connector-connections/{connection_id}/bindings"),
        Some(&token),
        Some(json!({
            "binding_kind": "slack_channel",
            "label": "#helpdesk-alerts",
            "external_id": "C0123456"
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "a destination lands");
    let binding = body_json(response).await;
    assert_eq!(binding["binding_kind"], "slack_channel");
    let binding_id = binding["id"].as_str().expect("binding id").to_string();

    // The destination list is what the share dialog reads.
    let body = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/connector-connections/{connection_id}/bindings"),
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(body["bindings"].as_array().expect("array").len(), 1);

    // The capability default decides the chip's press target.
    let response = send(
        &app,
        axum::http::Method::PUT,
        "/api/capability-defaults/rmm",
        Some(&token),
        Some(json!({ "connection_id": connection_id })),
    )
    .await;
    assert_eq!(response.status(), 200, "the default is set");

    // The record now resolves through this connection. The reference is the
    // sync's row, written by the same function a sync uses.
    {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("org");
        core_db::connector_connections::upsert_live_ref(
            &mut tx,
            org_id,
            Uuid::now_v7(),
            core_domain::connectors::RecordType::Configuration,
            asset_id,
            connection_id,
            "ninjaone",
            "device",
            "NIN-4412",
            None,
            None,
            None,
        )
        .await
        .expect("ref");
        tx.commit().await.expect("commit");
    }

    let resolved = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/records/configuration/{asset_id}/external-actions"),
            Some(&token),
            None,
        )
        .await,
    )
    .await;
    let provider = &resolved["capabilities"][0]["providers"][0];
    assert_eq!(provider["connection_id"], json!(connection_id));
    assert_eq!(
        provider["is_primary"],
        json!(true),
        "the default marks this provider primary"
    );
    assert_eq!(provider["actions"][0]["kind"], "open");
    assert_eq!(
        provider["actions"][0]["url"],
        "https://rmm.example.test/device/NIN-4412"
    );

    // Deleting with a destination and a reference is refused, naming both.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/connector-connections/{connection_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 409, "a referenced connection is kept");
    let body = body_json(response).await;
    assert_eq!(reason(&body), "conflict");
    assert!(
        message(&body).contains("1 destination(s)")
            && message(&body).contains("1 external reference(s)"),
        "the refusal names what still points at it: {}",
        message(&body)
    );

    // Remove the destination. The reference still blocks.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/connector-bindings/{binding_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 204);

    // Clearing the reference is the sync's job; do it as one would.
    {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("org");
        sqlx::query("DELETE FROM external_refs WHERE org_id = $1 AND connection_id = $2")
            .bind(org_id)
            .bind(connection_id)
            .execute(&mut *tx)
            .await
            .expect("unref");
        tx.commit().await.expect("commit");
    }

    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/connector-connections/{connection_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 204, "an unreferenced connection deletes");
}

/// An audience outside the caller's roles hides the provider; an empty audience
/// shows it. The technician keeps `clients:view`, so the only difference is the
/// audience itself.
#[tokio::test]
async fn an_audience_hides_the_provider_from_a_caller_without_the_role() {
    let (state, app) = test_state().await;
    let slug = format!("connector-audience-{}", Uuid::now_v7().simple());
    let (token, org_id, _owner_id) = owner(&app, &slug).await;
    let install = seed_install(&state, org_id, &format!("ninjaone-{slug}")).await;
    let (client_id, asset_id) = seed_client_asset(&state, org_id, "LT-AUDIENCE").await;
    let (tech_token, roles) = technician(&state, &app, org_id, &slug).await;

    // The owner's own role is the audience: the technician holds no such role.
    let owner_role = roles
        .iter()
        .find(|(_, builtin)| builtin.as_deref() == Some("owner"))
        .map(|(id, _)| *id)
        .expect("owner preset");

    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/connector-installations/{install}/connections"),
            Some(&token),
            Some(json!({
                "label": "NinjaOne — leads only",
                "scope_client_id": client_id,
                "settings": { "base_url": "https://rmm.example.test" },
                "secrets": { "api_key": "k" },
                "audience_roles": [owner_role.to_string()]
            })),
        )
        .await,
    )
    .await;
    assert_eq!(created["audience_roles"], json!([owner_role.to_string()]));
    let connection_id: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");

    {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("org");
        core_db::connector_connections::upsert_live_ref(
            &mut tx,
            org_id,
            Uuid::now_v7(),
            core_domain::connectors::RecordType::Configuration,
            asset_id,
            connection_id,
            "ninjaone",
            "device",
            "NIN-1",
            None,
            None,
            None,
        )
        .await
        .expect("ref");
        tx.commit().await.expect("commit");
    }

    let uri = format!("/api/records/configuration/{asset_id}/external-actions");

    let tech_view =
        body_json(send(&app, axum::http::Method::GET, &uri, Some(&tech_token), None).await).await;
    assert_eq!(
        tech_view["capabilities"],
        json!([]),
        "a caller outside the audience sees no provider at all"
    );

    let owner_view =
        body_json(send(&app, axum::http::Method::GET, &uri, Some(&token), None).await).await;
    assert_eq!(
        owner_view["capabilities"][0]["providers"][0]["connection_id"],
        json!(connection_id),
        "the audience member sees the provider"
    );

    // The roster agrees: the technician's client-scoped connection is absent.
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/connectors/roster",
            Some(&tech_token),
            None,
        )
        .await,
    )
    .await;
    let visible = roster["connectors"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .flat_map(|row| row["connections"].as_array().cloned().unwrap_or_default())
                .count()
        })
        .unwrap_or_default();
    assert_eq!(visible, 0, "the roster hides it from the same caller");
}

/// A caller without the plugin admin gate cannot write, and a machine or
/// technician token gets 403 rather than a 404 that would hide the surface.
#[tokio::test]
async fn writes_need_the_plugin_admin_gate() {
    let (state, app) = test_state().await;
    let slug = format!("connector-gate-{}", Uuid::now_v7().simple());
    let (_token, org_id, _owner_id) = owner(&app, &slug).await;
    let install = seed_install(&state, org_id, &format!("ninjaone-{slug}")).await;
    let (tech_token, _roles) = technician(&state, &app, org_id, &slug).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/connector-installations/{install}/connections"),
        Some(&tech_token),
        Some(good_body("k")),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a technician cannot configure a connector"
    );
}
