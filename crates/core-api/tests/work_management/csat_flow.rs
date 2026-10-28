//! CSAT surveys on resolution (Wave 34 lane 2) — portal response flow over
//! the REAL core-api router.
//!
//! Purpose: pin the single-use token contract end to end — view never
//! burns the token, submit consumes exactly once (replay is a 404), the
//! link is scoped to the survey's CLIENT (a portal user of client B gets
//! the same 404 as an expired link), scores are 1-5, and the response
//! records score + comment against the ticket.
//!
//! Non-purpose: worker policy (interval/quiet hours — worker suite), mail
//! rendering (internal enqueue handler), scorecard rollups (db_suite).

use crate::common::{body_json, send, test_state as common_test_state};
use core_api::state::AppState;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

async fn test_state() -> AppState {
    common_test_state().await.0
}

fn token_hash(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

#[tokio::test]
async fn csat_single_use_client_scoped_flow() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let tag = Uuid::now_v7().simple().to_string()[..8].to_string();
    let slug = format!("csat-flow-{tag}");
    let tokens = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/auth/register",
            None,
            Some(json!({
                "org_name": "Csat Flow Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": "correct-horse-battery",
                "display_name": "Owner",
            })),
        )
        .await,
    )
    .await;
    let staff = tokens["access_token"].as_str().unwrap().to_string();
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");

    let client_a: Uuid = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&staff),
            Some(json!({"name": "Client A"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let client_b: Uuid = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&staff),
            Some(json!({"name": "Client B"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let board_id: Uuid = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&staff),
            None,
        )
        .await,
    )
    .await[0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let ticket_a: Uuid = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&staff),
            Some(json!({"board_id": board_id, "title": "Client A issue", "priority": "p2", "client_id": client_a})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Expose the default board to client A so the portal can read the
    // ticket behind the survey.
    let exposure = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_a}/board-exposures"),
        Some(&staff),
        Some(json!({"board_id": board_id, "exposed": true, "visible_status_ids": []})),
    )
    .await;
    assert_eq!(exposure.status(), 200);

    // Portal users: contact for A (the survey recipient) and contact for B
    // (the cross-client probe).
    for (client, email) in [
        (client_a, format!("csat-a-{tag}@a.test")),
        (client_b, format!("csat-b-{tag}@b.test")),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/clients/{client}/portal-users"),
            Some(&staff),
            Some(json!({
                "email": email,
                "display_name": "Portal Person",
                "password": "portal-passphrase-1",
                "role": "contact",
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "invite {email}");
    }
    let email_a = format!("csat-a-{tag}@a.test");
    let email_b = format!("csat-b-{tag}@b.test");
    let login_a = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/login",
            None,
            Some(json!({"email": &email_a, "password": "portal-passphrase-1"})),
        )
        .await,
    )
    .await;
    let bearer_a = login_a["access_token"].as_str().unwrap().to_string();
    let login_b = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/login",
            None,
            Some(json!({"email": &email_b, "password": "portal-passphrase-1"})),
        )
        .await,
    )
    .await;
    let bearer_b = login_b["access_token"].as_str().unwrap().to_string();

    // Seed ONE sent survey for ticket_a with a known token.
    let raw_token = format!("tok-{}", Uuid::now_v7().simple());
    let survey_id = Uuid::now_v7();
    {
        let mut tx = state.pool.begin().await.unwrap();
        core_db::set_org(&mut tx, org_id).await.unwrap();
        sqlx::query(
            "INSERT INTO csat_surveys (id, org_id, ticket_id, client_id, contact_email, token_hash, token_expires_at, status, scheduled_for)
             VALUES ($1, $2, $3, $4, $6, $5, now() + interval '14 days', 'sent', now())",
        )
        .bind(survey_id)
        .bind(org_id)
        .bind(ticket_a)
        .bind(client_a)
        .bind(token_hash(&raw_token))
        .bind(&email_a)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let path = format!("/api/portal/csat/{raw_token}");

    // 1. View: valid, read-only (the token still works after).
    let view = send(&app, axum::http::Method::GET, &path, Some(&bearer_a), None).await;
    assert_eq!(view.status(), 200, "view must render for the owning client");
    let view = body_json(view).await;
    assert_eq!(view["status"], "valid");
    assert_eq!(view["ticket_id"], ticket_a.to_string());

    // 2. Cross-client scoping: client B's portal user gets the SAME 404 an
    // expired link gets — no existence leak.
    let foreign = send(&app, axum::http::Method::GET, &path, Some(&bearer_b), None).await;
    assert_eq!(foreign.status(), 404, "cross-client view must 404");
    let foreign_post = send(
        &app,
        axum::http::Method::POST,
        &path,
        Some(&bearer_b),
        Some(json!({"score": 5})),
    )
    .await;
    assert_eq!(foreign_post.status(), 404, "cross-client submit must 404");

    // 3. Score validation: out of range is rejected BEFORE any consume.
    let bad = send(
        &app,
        axum::http::Method::POST,
        &path,
        Some(&bearer_a),
        Some(json!({"score": 6})),
    )
    .await;
    assert_eq!(bad.status(), 422, "score must be 1-5");

    // 4. Submit: consume + record in one step.
    let submit = send(
        &app,
        axum::http::Method::POST,
        &path,
        Some(&bearer_a),
        Some(json!({"score": 2, "comment": "Slow resolution, kind tech"})),
    )
    .await;
    assert_eq!(submit.status(), 200, "submit must succeed");
    let submit = body_json(submit).await;
    assert_eq!(submit["status"], "responded");

    let recorded = sqlx::query_as::<_, (Option<i16>, Option<String>)>(
        "SELECT score, comment FROM csat_surveys WHERE id = $1",
    )
    .bind(survey_id)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(recorded.0, Some(2));
    assert_eq!(recorded.1.as_deref(), Some("Slow resolution, kind tech"));

    // 5. Single use: view now reports `responded`, replay POST is a 404.
    let after =
        body_json(send(&app, axum::http::Method::GET, &path, Some(&bearer_a), None).await).await;
    assert_eq!(after["status"], "responded");
    let replay = send(
        &app,
        axum::http::Method::POST,
        &path,
        Some(&bearer_a),
        Some(json!({"score": 5})),
    )
    .await;
    assert_eq!(replay.status(), 404, "replayed token must 404");
}
