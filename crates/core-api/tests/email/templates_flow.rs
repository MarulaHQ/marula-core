//! HTTP flow tests for the template registry surface (Wave 13 lane 5,
//! KB3/KB4 per ADR 0004) — over the real router + the shared lane DB.
//!
//! Purpose: prove the envelope wiring end to end — registration seeds the
//! shipped defaults (immutable system rows), the closed-variable gate
//! (out-of-vocabulary placeholders are typed 400s, never silent empties),
//! typed-variable render against a real ticket context, the fork/restore
//! lifecycle, the immutability rule (shipped defaults never edit/delete
//! in place — typed 409 `template_immutable`), the typed unique-constraint
//! reason code (`template_slug_taken`), and IF1 idempotent replay with
//! IF2 `mutation` metadata. Database selection follows the documented
//! contract in tests/common (MARULA_TEST_DATABASE_URL or DATABASE_URL;
//! no schema reset).
//!
//! Non-purpose: pure variable vocabulary/rendering rules (core-domain
//! templates unit tests) and SQL-layer semantics (core-db
//! templates_queries).

use crate::common::{auth, body_json, send_headers as send, state_with};
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| {
        config.secret_key = core_api::config::SecretKey([29; 32]);
        config.internal_token = Some("templates-flow-internal-token".to_string());
    })
    .await
}

async fn register_org(app: &axum::Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Template Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

#[tokio::test]
async fn templates_flow_seeds_fork_immutability_render_and_reason_codes() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("tmpl-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- registration seeded the shipped defaults -----------------------
    let list = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/templates",
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let templates = list["templates"].as_array().expect("templates array");
    assert!(templates.len() >= 7, "one shipped default per kind seeds");
    assert!(templates
        .iter()
        .any(|t| t["builtin"] == "ticket_first_response"
            && t["kind"] == "email"
            && t["is_system"] == true));
    assert!(
        templates
            .iter()
            .all(|t| t["builtin"] == serde_json::Value::Null || t["is_system"] == true),
        "every builtin row is a system row"
    );
    let org_id = templates[0]["org_id"].as_str().unwrap().to_string();
    let email_default_id = templates
        .iter()
        .find(|t| t["builtin"] == "ticket_first_response")
        .and_then(|t| t["id"].as_str())
        .unwrap()
        .to_string();
    // The closed per-kind vocabulary ships for editors.
    let vocab = list["vocabulary"].as_array().expect("vocabulary");
    assert!(vocab.iter().any(|v| v["kind"] == "quote_invoice"
        && v["variables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "quote.total")));

    // --- create: org template + typed validation gates ------------------
    let key = format!("create-{}", Uuid::now_v7());
    let created = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/templates",
            &bearer,
            Some(json!({
                "kind": "email",
                "slug": "handover_email",
                "name": "Handover email",
                "description": "End-of-engagement handover",
                "subject": "Handover for {{client.name}}",
                "body": "Hi {{contact.name}},\n\nHandover for {{ticket.number}} from {{org.name}}.\n\n{{user.name}}",
                "variables": [{ "key": "client.name", "label": "Client name" }],
                "idempotency_key": key,
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "create must succeed");
        body_json(response).await
    };
    let created_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["is_system"], false);
    assert_eq!(created["forked_from"], serde_json::Value::Null);
    assert!(
        created["mutation"]["seq"].is_i64(),
        "IF2 read-your-write metadata"
    );
    assert_eq!(created["mutation"]["idempotency_key"], key);

    // IF1: the same key replays the same response.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/templates",
        &bearer,
        Some(json!({
            "kind": "email",
            "slug": "handover_email",
            "name": "Handover email",
            "description": "End-of-engagement handover",
            "subject": "Handover for {{client.name}}",
            "body": "Hi {{contact.name}},\n\nHandover for {{ticket.number}} from {{org.name}}.\n\n{{user.name}}",
            "variables": [{ "key": "client.name", "label": "Client name" }],
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replay_body = body_json(replay).await;
    assert_eq!(
        replay_body["id"].as_str().unwrap(),
        created_id,
        "replay returns the original row"
    );
    assert_eq!(
        replay_body["body"], created["body"],
        "replay returns the original content"
    );

    // Duplicate slug in the same kind: typed 409 `template_slug_taken`.
    let dup = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/templates",
        &bearer,
        Some(json!({
            "kind": "email",
            "slug": "handover_email",
            "name": "Second",
            "body": "x",
        })),
    )
    .await;
    assert_eq!(dup.status(), 409);
    let dup_body = body_json(dup).await;
    assert_eq!(
        dup_body["error"]["reason"], "template_slug_taken",
        "typed reason code, not generic conflict"
    );

    // Same slug in a DIFFERENT kind is fine (uniqueness is org+kind+slug).
    let other_kind = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/templates",
        &bearer,
        Some(json!({
            "kind": "note",
            "slug": "handover_email",
            "name": "Handover note",
            "body": "note body",
        })),
    )
    .await;
    assert_eq!(
        other_kind.status(),
        200,
        "slug uniqueness is scoped to kind"
    );

    // Out-of-vocabulary variable: typed 400, never a silent slot.
    let bad_var = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/templates",
        &bearer,
        Some(json!({
            "kind": "kb",
            "slug": "kb_with_ticket_var",
            "name": "Bad",
            "body": "{{ticket.number}}",
        })),
    )
    .await;
    assert_eq!(
        bad_var.status(),
        422,
        "kb kind cannot declare ticket variables"
    );
    assert!(
        body_json(bad_var).await["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ticket.number"),
        "error names the offending variable"
    );

    // --- immutability: shipped defaults never edit or delete in place ---
    let patch_system = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/templates/{email_default_id}"),
        &bearer,
        Some(json!({ "body": "hijacked" })),
    )
    .await;
    assert_eq!(patch_system.status(), 409);
    assert_eq!(
        body_json(patch_system).await["error"]["reason"],
        "template_immutable"
    );
    let delete_system = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/templates/{email_default_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(delete_system.status(), 409);
    assert_eq!(
        body_json(delete_system).await["error"]["reason"],
        "template_immutable"
    );

    // Org rows edit fine.
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/templates/{created_id}"),
        &bearer,
        Some(json!({ "body": "Hi {{contact.name}}, updated body." })),
    )
    .await;
    assert_eq!(patch.status(), 200);
    assert_eq!(
        body_json(patch).await["body"],
        "Hi {{contact.name}}, updated body."
    );

    // --- fork + restore lifecycle ---------------------------------------
    let forked = {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/templates/{email_default_id}/fork"),
            &bearer,
            Some(json!({ "slug": "first_response_custom", "name": "First response (custom)" })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let forked_id = forked["id"].as_str().unwrap().to_string();
    assert_eq!(forked["is_system"], false);
    assert_eq!(forked["forked_from"], "ticket_first_response");
    assert_eq!(forked["body"], forked["body"]);
    assert!(
        forked["body"]
            .as_str()
            .unwrap()
            .contains("{{ticket.number}}"),
        "the fork inherits the shipped content"
    );

    // The fork is editable org data.
    let edited_fork = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/templates/{forked_id}"),
        &bearer,
        Some(json!({ "body": "Hi {{contact.name}}, our custom first response." })),
    )
    .await;
    assert_eq!(edited_fork.status(), 200);

    // A fork is org data — never restorable (only shipped presets are).
    let restore_fork = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/templates/{forked_id}/restore"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        restore_fork.status(),
        422,
        "org-owned forks are not restorable"
    );

    // The shipped default restores to the canonical copy.
    let restored = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/templates/{email_default_id}/restore"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(restored.status(), 200);
    let restored_body = body_json(restored).await;
    assert!(
        restored_body["body"]
            .as_str()
            .unwrap()
            .contains("Thanks for reaching out"),
        "restore returns the canonical shipped copy"
    );

    // --- render: typed variables from a real ticket context -------------
    // Seed a ticket directly (RLS-scoped) to render against.
    let (board_id, status_id, ticket_id) = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org_and_user(
            &mut tx,
            Uuid::parse_str(&org_id).unwrap(),
            authed_user_id(&state, &org_id).await,
        )
        .await
        .expect("set org");
        let board_id: Uuid = sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1 LIMIT 1")
            .bind(Uuid::parse_str(&org_id).unwrap())
            .fetch_one(&mut *tx)
            .await
            .expect("board");
        let status_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM board_statuses WHERE board_id = $1 ORDER BY sort_order LIMIT 1",
        )
        .bind(board_id)
        .fetch_one(&mut *tx)
        .await
        .expect("status");
        let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
            .bind(Uuid::parse_str(&org_id).unwrap())
            .fetch_one(&mut *tx)
            .await
            .expect("user");
        let _ = user_id;
        let ticket_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category) \
             VALUES ($1, $2, 'T-9001', $3, $4, 'Laptop will not boot', 'p1', 'open')",
        )
        .bind(ticket_id)
        .bind(Uuid::parse_str(&org_id).unwrap())
        .bind(board_id)
        .bind(status_id)
        .execute(&mut *tx)
        .await
        .expect("ticket insert");
        tx.commit().await.expect("commit");
        (board_id, status_id, ticket_id)
    };
    let _ = (board_id, status_id);

    let rendered = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/templates/{email_default_id}/render"),
        &bearer,
        Some(json!({ "context_id": ticket_id })),
    )
    .await;
    assert_eq!(rendered.status(), 200);
    let rendered_body = body_json(rendered).await;
    let subject = rendered_body["subject"].as_str().unwrap();
    let body = rendered_body["body"].as_str().unwrap();
    assert!(
        subject.contains("T-9001") && subject.contains("Laptop will not boot"),
        "subject resolves ticket.number + ticket.summary: {subject}"
    );
    assert!(
        !body.contains("{{"),
        "no unresolved placeholders leak: {body}"
    );
    assert!(body.contains("T-9001"), "body resolves the ticket context");

    // A template whose text carries an out-of-vocabulary placeholder is a
    // typed 400 at render time too (defence in depth behind the write gate).
    let rogue = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/templates",
        &bearer,
        Some(json!({
            "kind": "kb",
            "slug": "kb_org_only",
            "name": "KB org template",
            "body": "Maintained by {{user.name}}.",
        })),
    )
    .await;
    assert_eq!(rogue.status(), 200);
    let rogue_id = body_json(rogue).await["id"].as_str().unwrap().to_string();
    let kb_render = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/templates/{rogue_id}/render"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(kb_render.status(), 200);
    assert!(
        body_json(kb_render).await["body"]
            .as_str()
            .unwrap()
            .contains("Owner"),
        "kb kind renders user.name from the org context"
    );

    // --- delete: org rows only ------------------------------------------
    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/templates/{created_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200);
    let gone = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/templates/{created_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(gone.status(), 404, "deleted row is gone");
}

async fn authed_user_id(state: &AppState, org_id: &str) -> Uuid {
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(Uuid::parse_str(org_id).unwrap())
        .fetch_one(&state.pool)
        .await
        .expect("user");
    user_id
}
