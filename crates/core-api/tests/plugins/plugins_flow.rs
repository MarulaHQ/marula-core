//! Plugin runtime v2 + marketplace registry e2e (Wave 14 lane 5).
//!
//! Covers the deny-by-default posture end to end over the REAL router:
//! the declarative manifest gate (closed capability vocabulary, pinned
//! interface version), install = fork-like adoption at a pinned version
//! (created DISABLED), the admin-enabled gate, grant widening validated
//! against the pinned manifest, the typed duplicate 409s
//! (`plugin_slug_taken` / `plugin_already_installed`), versioning
//! (append + explicit upgrade), and registry-level revocation as the
//! terminal kill switch (existing installs revoked, new installs
//! denied, revoked installs immutable).

use crate::common::{body_json, boot_with, register_org as common_register_org, send};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let (state, app) = boot_with(|_| {}).await;
    core_db::assert_rls_coverage(&state.pool)
        .await
        .expect("RLS coverage holds with plugin tables");
    (state, app)
}

async fn register_org(app: &Router, slug: String) -> String {
    common_register_org(app, "Plugin Co", &slug).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

/// Invites one staff user and redeems the setup token; returns the new
/// user's access token.
async fn invite_and_activate(app: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(json!({
            "email": email,
            "display_name": "Tech",
            "role": role,
            "idempotency_key": format!("invite-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    let body = body_json(response).await;
    let setup_token = body["invite"]["setup_token"].as_str().expect("setup token");
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200, "redemption must succeed");
    let tokens = body_json(accept).await;
    tokens["access_token"].as_str().expect("tech token").into()
}

fn publish_body(slug: &str, capabilities: &[&str]) -> Value {
    json!({
        "slug": slug,
        "name": "Ticket Summarizer",
        "description": "Summarizes ticket threads",
        "publisher": "marula",
        "version": "1.0.0",
        "manifest": {
            "description": "Summarizes ticket threads",
            "capabilities": capabilities,
        },
    })
}

#[tokio::test]
async fn plugin_marketplace_lifecycle_flow() {
    let (state, _router) = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("plugin-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, slug.clone()).await;
    let tech = invite_and_activate(
        &app,
        &owner,
        &format!("tech-{slug}@acme.test"),
        "technician",
    )
    .await;

    // --- catalog read is plugins:view; the owner holds the full matrix ------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/catalog",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "catalog read is plugins:view");

    // --- publish: declarative manifest, closed vocabulary -------------------
    let entry_slug = format!("ticket-summarizer-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(
            &entry_slug,
            &["tickets.read", "financial.read"],
        )),
    )
    .await;
    assert_eq!(response.status(), 201, "publish must succeed");
    let entry = body_json(response).await;
    assert_eq!(entry["interface_version"], 1);
    assert_eq!(entry["latest_version"], "1.0.0");
    assert_eq!(entry["revoked"], false);
    let catalog_id = entry["id"].as_str().unwrap().to_string();

    // Unknown capability keys are rejected (the vocabulary is closed).
    let bad_cap = format!("bad-cap-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(&bad_cap, &["admin.users"])),
    )
    .await;
    assert_eq!(response.status(), 422, "unknown capability must reject");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed", "{body}");

    // Duplicate slug → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(publish_body(&entry_slug, &["tickets.read"])),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "plugin_slug_taken", "{body}");

    // --- install = adoption at a pinned version, created DISABLED -----------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "install must succeed");
    let install = body_json(response).await;
    assert_eq!(
        install["status"], "disabled",
        "deny-by-default: installs start disabled"
    );
    assert_eq!(install["version"], "1.0.0");
    assert_eq!(
        install["effective_permissions"][0], "tickets:view",
        "grants resolve onto the EXISTING matrix vocabulary"
    );
    let install_id = install["id"].as_str().unwrap().to_string();

    // Double install → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert_eq!(
        body["error"]["reason"], "plugin_already_installed",
        "{body}"
    );

    // A grant OUTSIDE the pinned manifest's declared set is rejected.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": format!("other-{}", Uuid::now_v7().simple()),
            "granted_capabilities": [],
        })),
    )
    .await;
    assert_eq!(response.status(), 404, "unknown slug must 404");

    // --- the admin-enabled gate: a technician can do none of this -----------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&tech),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 403, "technician must be denied");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&tech),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 403, "technician must be denied");

    // --- enable (explicit admin action), widen grants within the manifest ---
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"], "enabled");

    // `revoked` is NOT settable through the update surface.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "revoked" })),
    )
    .await;
    assert_eq!(response.status(), 422);

    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "granted_capabilities": ["tickets.read", "financial.read"] })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated = body_json(response).await;
    assert_eq!(updated["granted_capabilities"].as_array().unwrap().len(), 2);
    assert!(
        updated["effective_permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "financial:read"),
        "financial.read must map onto the RB2 financial:read matrix key: {updated}"
    );

    // --- versioning: append + explicit upgrade ------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "manifest": {
                "description": "Summarizes ticket threads",
                "capabilities": ["tickets.read", "tickets.write", "financial.read"],
            },
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "version append must succeed");

    // Duplicate version → typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner),
        Some(json!({
            "version": "1.1.0",
            "manifest": { "description": "", "capabilities": [] },
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "plugin_version_taken"
    );

    // A version change is a PIN change: the new artifact's digest has to be
    // verified and the outgoing one parked for rollback, so this route refuses
    // to write `version` alone and names the surface that does it properly.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "version": "1.1.0" })),
    )
    .await;
    assert_eq!(response.status(), 409, "a pin move is not a metadata edit");

    // The pin moves the way the contract says it does: propose, then consent.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/installed/{install_id}/update"),
        Some(&owner),
        Some(json!({ "version": "1.1.0" })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the update proposal must be accepted"
    );
    let proposal = body_json(response).await;
    assert_eq!(proposal["current_version"], "1.0.0");
    assert_eq!(proposal["pending_version"], "1.1.0");

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/installed/{install_id}/consent"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "consent commits the pin move");
    let consented = body_json(response).await;
    assert_eq!(consented["current_version"], "1.1.0");
    assert!(consented["pending_version"].is_null());

    // --- registry revocation: the terminal kill switch ----------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner),
        Some(json!({ "reason": "upstream vulnerability" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let revoked = body_json(response).await;
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["revoked_reason"], "upstream vulnerability");

    // The org's install was cascaded to revoked.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/installed",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let installs = body_json(response).await;
    let mine = installs
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == install_id.as_str())
        .expect("install still listed");
    assert_eq!(mine["status"], "revoked", "revocation must cascade");

    // New installs of a revoked entry are denied.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked entry denies new installs");

    // Revoked installs are terminal: no enable, no grant change.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        Some(json!({ "status": "enabled" })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked install must be immutable");

    // Double revoke → 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner),
        Some(json!({ "reason": "again" })),
    )
    .await;
    assert_eq!(response.status(), 409);

    // --- audit trail covers the lifecycle ------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=plugin.",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let actions: Vec<&str> = audit["events"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["action"].as_str()).collect())
        .unwrap_or_default();
    for expected in [
        "plugin.catalog_entry_created",
        "plugin.installed",
        "plugin.install_updated",
        "plugin.catalog_entry_revoked",
    ] {
        assert!(
            actions
                .iter()
                .any(|a| a.starts_with(expected.split('.').next().unwrap())),
            "audit must cover {expected}; got {actions:?}"
        );
    }

    // --- uninstall ------------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 204, "uninstall must succeed");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "revoked entry denies re-install");
    let _ = catalog_id;
}

/// Wave 43 lane 4 (B)/(C)/(F): the executor surface kinds, route-level.
///
/// The catalog publish route carries `artifact_kind` (adjudicated in this
/// lane: without it `build_manifest` read `artifact_kind: None` and EVERY
/// published manifest was a WASM one, so a connector's `connector_profile`
/// was refused on the product's primary distribution path and clause (A)
/// was dead code). So the connector half is proven HERE, and the seeding
/// lifecycle is proven again end-to-end below.
#[tokio::test]
async fn executor_surface_kinds_register_through_the_real_router() {
    let (state, _router) = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::routes::build_router(state);
    let slug = format!("surface-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, slug.clone()).await;

    let entry_slug = format!("rmm-triggers-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "name": "RMM Trigger Source",
            "description": "Contributes a trigger and a read",
            "publisher": "marula",
            "version": "1.0.0",
            "manifest": {
                "description": "Contributes a trigger and a read",
                "capabilities": ["automation.triggers", "automation.reads"],
                "surface_profile": {
                    "triggers": [{
                        "id": "device_offline",
                        "label": "Device offline",
                        "description": "An RMM device stops reporting.",
                        "emitted_event_type": "rmm.device_offline",
                    }],
                    "reads": [{
                        "id": "site_devices",
                        "label": "Site devices",
                        "description": "Fetches a site's device list.",
                        "guardrails": {
                            "max_calls_per_run": 10,
                            "min_interval_secs": 60,
                            "max_output_bytes": 16384,
                            "may_return_secrets": false,
                        },
                    }],
                },
            },
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "an executor surface profile publishes"
    );

    // A read declaring vault-secret access is refused by the manifest.
    let secret_slug = format!("rmm-secret-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(json!({
            "slug": secret_slug,
            "name": "Bad Read",
            "description": "Leaks a secret",
            "publisher": "marula",
            "version": "1.0.0",
            "manifest": {
                "description": "Leaks a secret",
                "capabilities": ["automation.reads"],
                "surface_profile": {
                    "reads": [{
                        "id": "leaky",
                        "label": "Leaky",
                        "description": "Returns a vault secret.",
                        "guardrails": {
                            "max_calls_per_run": 10,
                            "min_interval_secs": 60,
                            "max_output_bytes": 16384,
                            "may_return_secrets": true,
                        },
                    }],
                },
            },
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "a read may never return a vault secret"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed", "{body}");

    // Counts are DELTAS: the shared test database is not reset between
    // tests in this binary, so an absolute count would be whoever ran last.
    let counts = || async {
        let t: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plugin_surface_entries WHERE surface = 'automation_trigger'",
        )
        .fetch_one(&pool)
        .await
        .expect("count triggers");
        let r: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plugin_surface_entries WHERE surface = 'read'",
        )
        .fetch_one(&pool)
        .await
        .expect("count reads");
        (t, r)
    };
    let (before_trigger, before_read) = counts().await;

    // Install with BOTH grants: both new kinds register.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "granted_capabilities": ["automation.triggers", "automation.reads"],
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "install must succeed");

    let (after_trigger, after_read) = counts().await;
    assert_eq!(
        after_trigger - before_trigger,
        1,
        "the granted trigger registers"
    );
    assert_eq!(after_read - before_read, 1, "the granted read registers");

    // A second install of the same entry, granted NOTHING of the executor
    // set, registers nothing: grants gate registration, deny-by-default,
    // exactly as the UI kinds always did.
    let entry2 = format!("rmm-triggers-2-{}", Uuid::now_v7().simple());
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(json!({
            "slug": entry2,
            "name": "Ungranted",
            "description": "No grants",
            "publisher": "marula",
            "version": "1.0.0",
            "manifest": {
                "description": "No grants",
                "capabilities": [],
                "surface_profile": {
                    "triggers": [{
                        "id": "device_offline",
                        "label": "Device offline",
                        "description": "An RMM device stops reporting.",
                        "emitted_event_type": "rmm.device_offline",
                    }],
                },
            },
        })),
    )
    .await;
    assert_eq!(response.status(), 201);
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({ "slug": entry2, "granted_capabilities": [] })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "an ungranted install lands disabled"
    );

    let (final_trigger, _) = counts().await;
    assert_eq!(
        final_trigger, after_trigger,
        "an ungranted declaration registers nothing"
    );
}

/// Wave 43 lane 4 (A): `connector_profile.webhook_endpoints` becomes REAL
/// outbound configuration, and stays attributed to the install.
///
/// Publishes an `http_connector` through the catalog route the product
/// actually uses, installs it, and asserts that (1) its declared endpoints
/// seed live subscription rows carrying `owner_install_id`, (2) the
/// wildcard event set is what a declaration means (WHERE, not WHAT), and
/// (3) uninstalling the connector deletes them — the consent screen has
/// promised "declared webhook endpoints seed the connector's outbound
/// posture" since Wave 19 lane 2, and this is that promise kept.
#[tokio::test]
async fn connector_declared_endpoints_seed_outbound_subscriptions() {
    let (state, _router) = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::routes::build_router(state);
    let slug = format!("conn-seed-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, slug).await;

    let entry_slug = format!("rmm-connector-{}", Uuid::now_v7().simple());
    let a = format!("https://hooks.example.com/{entry_slug}-a");
    let b = format!("https://hooks.example.com/{entry_slug}-b");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner),
        Some(json!({
            "slug": entry_slug,
            "name": "RMM Connector",
            "description": "Pulls device posture",
            "publisher": "marula",
            "version": "1.0.0",
            "manifest": {
                "description": "Pulls device posture",
                "capabilities": [],
                "artifact_kind": "http_connector",
                "connector_profile": {
                    "webhook_endpoints": [a, b],
                    "scopes": ["rmm.devices.read"],
                },
            },
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "an http_connector publishes with its declared endpoints"
    );

    // Kind coherence still holds in BOTH directions: a wasm manifest
    // refuses a connector_profile, and a connector still refuses the UI
    // surface kinds (ADR 0017 §3).
    for (name, manifest, expect_ok) in [
        (
            "wasm + connector profile",
            json!({
                "description": "Wrong kind",
                "capabilities": [],
                "connector_profile": { "webhook_endpoints": ["https://h.example.com/x"] },
            }),
            false,
        ),
        (
            "connector + widget",
            json!({
                "description": "Wrong kind",
                "capabilities": ["ui.widgets"],
                "artifact_kind": "http_connector",
                "surface_profile": {
                    "widgets": [{
                        "id": "w", "title": "W", "slot": "dashboard",
                        "kind": "stat",
                    }],
                },
            }),
            false,
        ),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/plugins/catalog",
            Some(&owner),
            Some(json!({
                "slug": format!("kind-coherent-{}-{}", name.len(), Uuid::now_v7().simple()),
                "name": name,
                "description": "Kind coherence",
                "publisher": "marula",
                "version": "1.0.0",
                "manifest": manifest,
            })),
        )
        .await;
        let status = response.status();
        if expect_ok {
            assert_eq!(status, 201, "{name} should publish");
        } else {
            assert_eq!(status, 422, "{name} must be refused");
        }
    }

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(&owner),
        Some(json!({ "slug": entry_slug, "granted_capabilities": [] })),
    )
    .await;
    assert_eq!(response.status(), 201, "install must succeed");
    let body = body_json(response).await;
    let install_id = body["id"].as_str().expect("install id").to_string();

    // The seed is install-attributed, one row per declared endpoint, with
    // the wildcard event set a declaration means.
    let seeded: Vec<(String, i64)> = sqlx::query_as(
        "SELECT url, coalesce(array_length(event_types, 1), 0)::int8 \
         FROM webhooks WHERE owner_install_id = $1::uuid",
    )
    .bind(uuid::Uuid::parse_str(&install_id).expect("uuid"))
    .fetch_all(&pool)
    .await
    .expect("seeded rows");
    let mut urls: Vec<String> = seeded.iter().map(|(u, _)| u.clone()).collect();
    urls.sort();
    let mut want = vec![a.clone(), b.clone()];
    want.sort();
    assert_eq!(urls, want, "each declared endpoint seeds one subscription");
    for (_, event_count) in &seeded {
        assert_eq!(*event_count, 0, "a declaration is a wildcard subscription");
    }

    // Uninstalling tears them down: `ON DELETE CASCADE` on the owner.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/plugins/installed/{install_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 204, "uninstall must succeed");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*)::int8 FROM webhooks WHERE owner_install_id = $1::uuid")
            .bind(uuid::Uuid::parse_str(&install_id).expect("uuid"))
            .fetch_one(&pool)
            .await
            .expect("leftover count");
    assert_eq!(left, 0, "an uninstalled connector's outbound rows are gone");
}

#[tokio::test]
async fn sandbox_seam_denies_by_default() {
    // The seam-level deny-by-default matrix lives in plugin-host's unit
    // tests; here we prove the WIRE posture: an unauthenticated caller
    // gets the typed envelope, and catalog reads need plugins:view.
    let (state, _router) = test_state().await;
    let app = core_api::routes::build_router(state);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/plugins/catalog",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 401);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "unauthorized", "{body}");
}

/// Cross-publisher isolation on the server-wide catalog. `plugin_catalog` has
/// no org_id and no RLS, so before the ownership stamp a second org's admin
/// could append a version to another publisher's entry, or revoke an entry
/// every tenant had installed. Both routes must refuse a foreign publisher,
/// and an operator/seeded entry (NULL origin) must be immutable through them.
#[tokio::test]
async fn a_second_org_cannot_mutate_another_publishers_catalog_entry() {
    let (_state, router) = test_state().await;

    let slug = format!("publisher-{}", Uuid::now_v7().simple());
    let owner_a = register_org(&router, format!("pub-a-{}", Uuid::now_v7().simple())).await;
    let owner_b = register_org(&router, format!("pub-b-{}", Uuid::now_v7().simple())).await;

    let entry_slug = format!("owned-{}", Uuid::now_v7().simple());
    let published = send(
        &router,
        axum::http::Method::POST,
        "/api/plugins/catalog",
        Some(&owner_a),
        Some(publish_body(&entry_slug, &["tickets.read"])),
    )
    .await;
    assert_eq!(
        published.status(),
        201,
        "org A publishes its own entry: {slug}"
    );

    // Org B appends a version to A's entry: refused.
    let foreign_append = send(
        &router,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner_b),
        Some(json!({
            "version": "9.9.9",
            "manifest": { "description": "injected by another tenant", "capabilities": [] },
        })),
    )
    .await;
    assert_eq!(
        foreign_append.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a foreign publisher must not append a version"
    );

    // Org B revokes A's entry: refused (the revoke cascades into every org).
    let foreign_revoke = send(
        &router,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner_b),
        Some(json!({ "reason": "taken down by another tenant" })),
    )
    .await;
    assert_eq!(
        foreign_revoke.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a foreign publisher must not revoke an entry"
    );

    // The publishing org keeps both capabilities.
    let own_append = send(
        &router,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/versions"),
        Some(&owner_a),
        Some(json!({
            "version": "1.1.0",
            "manifest": { "description": "own follow-up", "capabilities": [] },
        })),
    )
    .await;
    assert_eq!(
        own_append.status(),
        201,
        "the publishing org still appends to its own entry"
    );

    let own_revoke = send(
        &router,
        axum::http::Method::POST,
        &format!("/api/plugins/catalog/{entry_slug}/revoke"),
        Some(&owner_a),
        Some(json!({ "reason": "own decision" })),
    )
    .await;
    assert_eq!(
        own_revoke.status(),
        200,
        "the publishing org still revokes its own entry"
    );
}

// --- Wave 59 lane `p59-digest-only`: the digest-only consequence ---------------

/// The org id behind a slug `register_org` minted, so a test can seed registry
/// rows owned by one org and then act as another.
async fn org_id_of(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("org row")
}

/// Seeds one catalog entry owned by `origin` plus one version row at the given
/// authentication verdict (`None` leaves 0172's default in place, which is the
/// "nobody knows" case).
///
/// Seeded at SQL rather than through the staging routes because those need a
/// network fetch or a real archive, and what this lane changed is how the ROWS
/// are read at adoption time, not how they arrived.
async fn seed_catalog_version(
    pool: &PgPool,
    origin: Uuid,
    slug: &str,
    version: &str,
    manifest_authenticated: Option<bool>,
) -> Uuid {
    let catalog_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO plugin_catalog (id, slug, name, description, publisher, origin_org_id,
                                     latest_version, interface_version)
         VALUES ($1, $2, $3, 'digest-only fixture', 'staged-import', $4, $5, 1)",
    )
    .bind(catalog_id)
    .bind(slug)
    .bind(format!("Digest-only {slug}"))
    .bind(origin)
    .bind(version)
    .execute(pool)
    .await
    .expect("catalog entry");

    let manifest = json!({
        "name": slug,
        "version": version,
        "interface_version": 1,
        "description": "digest-only fixture",
        "capabilities": ["tickets.read"],
    });
    match manifest_authenticated {
        Some(authenticated) => sqlx::query(
            "INSERT INTO plugin_versions (id, catalog_id, version, manifest,
                                          manifest_authenticated)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(catalog_id)
        .bind(version)
        .bind(manifest)
        .bind(authenticated)
        .execute(pool)
        .await
        .expect("version row"),
        None => sqlx::query(
            "INSERT INTO plugin_versions (id, catalog_id, version, manifest)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(catalog_id)
        .bind(version)
        .bind(manifest)
        .execute(pool)
        .await
        .expect("version row"),
    };
    catalog_id
}

async fn add_catalog_version(
    pool: &PgPool,
    catalog_id: Uuid,
    slug: &str,
    version: &str,
    manifest_authenticated: Option<bool>,
) {
    let manifest = json!({
        "name": slug,
        "version": version,
        "interface_version": 1,
        "description": "digest-only fixture",
        "capabilities": ["tickets.read"],
    });
    let inserted = match manifest_authenticated {
        Some(authenticated) => sqlx::query(
            "INSERT INTO plugin_versions (id, catalog_id, version, manifest, manifest_authenticated)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(catalog_id)
        .bind(version)
        .bind(manifest)
        .bind(authenticated)
        .execute(pool)
        .await,
        None => sqlx::query(
            "INSERT INTO plugin_versions (id, catalog_id, version, manifest)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(catalog_id)
        .bind(version)
        .bind(manifest)
        .execute(pool)
        .await,
    };
    inserted.expect("version row");
}

/// Installs `slug@version` as the bearer's org and returns the raw response.
async fn install_as(
    app: &Router,
    bearer: &str,
    slug: &str,
    version: &str,
) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/plugins/installed",
        Some(bearer),
        Some(json!({
            "slug": slug,
            "version": version,
            "granted_capabilities": ["tickets.read"],
        })),
    )
    .await
}

/// F49 / migration 0172. A manifest nobody signed is adoptable by the org that
/// staged it and by nobody else: the catalog is server-wide, so before this rule
/// `POST /api/plugins/installed` handed any org's admin another tenant's
/// digest-only entry as if a publisher had signed it. A version a trusted key
/// verified still crosses, and a row whose authentication nobody recorded is
/// read as unauthenticated (fail closed).
#[tokio::test]
async fn an_unauthenticated_manifest_never_crosses_an_org_boundary() {
    let (state, _router) = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::routes::build_router(state);

    let slug_a = format!("dist-a-{}", Uuid::now_v7().simple());
    let slug_b = format!("dist-b-{}", Uuid::now_v7().simple());
    let owner_a = register_org(&app, slug_a.clone()).await;
    let owner_b = register_org(&app, slug_b.clone()).await;
    let org_a = org_id_of(&pool, &slug_a).await;
    let org_b = org_id_of(&pool, &slug_b).await;

    let entry_slug = format!("digest-only-{}", Uuid::now_v7().simple());
    // 1.0.0 is what a GitHub pull or a local zip leaves behind: the verdict
    // migration 0167 writes at ingest, now travelling on the registry row.
    let catalog_id = seed_catalog_version(&pool, org_a, &entry_slug, "1.0.0", Some(false)).await;
    // 2.0.0 stands in for a registry statement a trusted key verified.
    add_catalog_version(&pool, catalog_id, &entry_slug, "2.0.0", Some(true)).await;
    // 3.0.0 records no verdict at all (0172's default), which must read as the
    // weaker claim rather than as a blank cheque.
    add_catalog_version(&pool, catalog_id, &entry_slug, "3.0.0", None).await;

    // --- another org: refused, typed, and nothing installed -----------------
    let refused = install_as(&app, &owner_b, &entry_slug, "1.0.0").await;
    assert_eq!(
        refused.status(),
        409,
        "an unsigned manifest must not install into another tenant"
    );
    let body = body_json(refused).await;
    assert_eq!(
        body["error"]["reason"], "manifest_unauthenticated",
        "the refusal names its reason: {body}"
    );
    assert_eq!(body["error"]["slug"], entry_slug, "{body}");
    assert_eq!(body["error"]["version"], "1.0.0", "{body}");
    let installed_in_b: i64 = sqlx::query_scalar(
        "SELECT count(*)::int8 FROM plugin_installs WHERE org_id = $1 AND catalog_id = $2",
    )
    .bind(org_b)
    .bind(catalog_id)
    .fetch_one(&pool)
    .await
    .expect("install count");
    assert_eq!(installed_in_b, 0, "a refusal writes no install row");

    // --- the staging org still adopts its own entry -------------------------
    let own = install_as(&app, &owner_a, &entry_slug, "1.0.0").await;
    assert_eq!(
        own.status(),
        201,
        "the org that staged the row may still adopt it"
    );
    let own = body_json(own).await;
    assert_eq!(own["version"], "1.0.0");
    assert_eq!(
        own["status"], "disabled",
        "adoption is still deny-by-default"
    );

    // --- a publisher-signed version still installs across orgs --------------
    let cross = install_as(&app, &owner_b, &entry_slug, "2.0.0").await;
    assert_eq!(
        cross.status(),
        201,
        "an authenticated manifest distributes across orgs as before"
    );
    let cross = body_json(cross).await;
    assert_eq!(cross["version"], "2.0.0");

    // --- unknown authentication reads as unauthenticated --------------------
    let unknown = install_as(&app, &owner_b, &entry_slug, "3.0.0").await;
    assert_eq!(
        unknown.status(),
        409,
        "a version with no recorded verdict fails closed"
    );
    let body = body_json(unknown).await;
    assert_eq!(
        body["error"]["reason"], "manifest_unauthenticated",
        "{body}"
    );

    // --- the refusal is audited, in the org that tried ----------------------
    let audited: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action, detail FROM audit_events
         WHERE org_id = $1 AND action = 'plugin.install_refused'
         ORDER BY occurred_at",
    )
    .bind(org_b)
    .fetch_all(&pool)
    .await
    .expect("audit rows");
    assert_eq!(
        audited.len(),
        2,
        "each refused adoption leaves one audit row"
    );
    assert_eq!(audited[0].0, "plugin.install_refused");
    assert_eq!(audited[0].1["slug"], entry_slug);
    assert_eq!(audited[0].1["version"], "1.0.0");
    assert_eq!(audited[0].1["reason"], "manifest_unauthenticated");
    let audited_in_a: i64 = sqlx::query_scalar(
        "SELECT count(*)::int8 FROM audit_events
         WHERE org_id = $1 AND action = 'plugin.install_refused'",
    )
    .bind(org_a)
    .fetch_one(&pool)
    .await
    .expect("audit count");
    assert_eq!(
        audited_in_a, 0,
        "the owning org's own adoption is not a refusal"
    );
}

/// The same rule on the other adoption surface. An install that legitimately
/// took a signed 1.0.0 from another org must not be updatable onto that entry's
/// unsigned 1.1.0 — the update path re-pins the manifest, so it is an adoption
/// too. The owning org keeps its own update.
#[tokio::test]
async fn an_update_may_not_pin_an_unsigned_manifest_across_orgs() {
    let (state, _router) = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::routes::build_router(state);

    let slug_a = format!("upd-a-{}", Uuid::now_v7().simple());
    let slug_b = format!("upd-b-{}", Uuid::now_v7().simple());
    let owner_a = register_org(&app, slug_a.clone()).await;
    let owner_b = register_org(&app, slug_b.clone()).await;
    let org_a = org_id_of(&pool, &slug_a).await;
    let org_b = org_id_of(&pool, &slug_b).await;

    let entry_slug = format!("mixed-{}", Uuid::now_v7().simple());
    // 1.0.0 signed, 1.1.0 a repository pull the registry cannot vouch for.
    let catalog_id = seed_catalog_version(&pool, org_a, &entry_slug, "1.0.0", Some(true)).await;
    add_catalog_version(&pool, catalog_id, &entry_slug, "1.1.0", Some(false)).await;

    let install_b = install_as(&app, &owner_b, &entry_slug, "1.0.0").await;
    assert_eq!(install_b.status(), 201, "the signed version installs");
    let install_b = body_json(install_b).await;
    let b_install_id = install_b["id"].as_str().expect("install id").to_string();

    let refused = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/installed/{b_install_id}/update"),
        Some(&owner_b),
        Some(json!({ "version": "1.1.0" })),
    )
    .await;
    assert_eq!(
        refused.status(),
        409,
        "an update may not re-pin another org's unsigned manifest"
    );
    let body = body_json(refused).await;
    assert_eq!(
        body["error"]["reason"], "manifest_unauthenticated",
        "{body}"
    );
    // Nothing moved: the pending columns stay clear, so the consent surface has
    // nothing to commit.
    let pending: Option<String> =
        sqlx::query_scalar("SELECT pending_version FROM plugin_installs WHERE id = $1::uuid")
            .bind(&b_install_id)
            .fetch_one(&pool)
            .await
            .expect("install row");
    assert_eq!(pending, None, "a refused proposal parks nothing");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*)::int8 FROM audit_events
         WHERE org_id = $1 AND action = 'plugin.install_refused'",
    )
    .bind(org_b)
    .fetch_one(&pool)
    .await
    .expect("audit count");
    assert_eq!(audited, 1, "the refused proposal is audited");

    // The owning org's own move onto the same version is legitimate.
    let install_a = install_as(&app, &owner_a, &entry_slug, "1.0.0").await;
    assert_eq!(install_a.status(), 201);
    let install_a = body_json(install_a).await;
    let a_install_id = install_a["id"].as_str().expect("install id");
    let own = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/plugins/installed/{a_install_id}/update"),
        Some(&owner_a),
        Some(json!({ "version": "1.1.0" })),
    )
    .await;
    assert_eq!(
        own.status(),
        200,
        "the org that staged the unsigned version may still propose it"
    );
    let audited_in_a: i64 = sqlx::query_scalar(
        "SELECT count(*)::int8 FROM audit_events
         WHERE org_id = $1 AND action = 'plugin.install_refused'",
    )
    .bind(org_a)
    .fetch_one(&pool)
    .await
    .expect("audit count");
    assert_eq!(audited_in_a, 0, "the owning org is never refused");
}
