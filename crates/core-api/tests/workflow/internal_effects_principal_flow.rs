//! The internal effects seam runs as the rule's author (Wave 58 lane
//! `p58-effects-seam`, ruling A, audit record
//! `core-api/routes/internal.rs/internal-effects-seam-has-no-principal`).
//!
//! Purpose: `POST /api/internal/effects` used to open its transaction with the
//! request's org id standing in for the acting user, so the holder of the
//! deployment-wide internal token could name any org and write there with no
//! permission check. The seam now resolves the author recorded on the rule and
//! applies the grant a manual write would need before dispatch. These cases pin
//! that from the outside:
//!
//! 1. a rule whose author holds the grant applies its effect, and the applied
//!    audit row names that author;
//! 2. a rule whose author lacks the grant writes nothing, answers the typed
//!    refusal, and leaves a refusal audit row naming the missing grant;
//! 3. a rule with no author is refused rather than attributed;
//! 4. a request that names no rule is refused;
//! 5. an inactive author is refused.
//!
//! Non-purpose: the action-to-authority mapping (core-api unit tests), the
//! worker's side of the call (worker unit tests), and plugin install grants
//! (the plugin-host suite).
//!
//! Database selection follows the documented contract in
//! `crates/core-db/tests/common/mod.rs` (MARULA_TEST_DATABASE_URL or
//! DATABASE_URL; no schema reset).

use crate::common::{body_json, send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::Row as _;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "effects-seam-internal-token";
const PASSWORD: &str = "correct-horse-battery";

async fn test_state() -> AppState {
    state_with(|config| {
        config.secret_key = core_api::config::SecretKey([57; 32]);
        config.internal_token = Some(INTERNAL_TOKEN.to_string());
    })
    .await
}

struct Org {
    token: String,
    org_id: Uuid,
    user_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Effects Seam Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
    }
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"].as_str().unwrap().parse().expect("board id")
}

async fn create_ticket(app: &Router, org: &Org, board: Uuid, title: &str) -> Uuid {
    let created = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&org.token),
            Some(json!({ "board_id": board, "title": title, "priority": "p3" })),
        )
        .await,
    )
    .await;
    created["id"].as_str().unwrap().parse().expect("ticket id")
}

async fn ticket_priority(app: &Router, org: &Org, ticket_id: Uuid) -> String {
    let ticket = body_json(
        send(
            app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    ticket["priority"].as_str().unwrap_or_default().to_string()
}

/// Inserts a same-org user with one builtin role and returns its id.
async fn add_user_with_role(state: &AppState, org_id: Uuid, builtin: &str, label: &str) -> Uuid {
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = $2")
        .bind(org_id)
        .bind(builtin)
        .fetch_one(&mut *tx)
        .await
        .expect("builtin role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        &format!("{label}-{user_id}@seam.test"),
        label,
        &core_api::auth::password::hash(PASSWORD).expect("hash"),
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");
    user_id
}

/// Creates a rule through the authoring route as the org's owner, and returns
/// its id. The route is what the ruling expects to start recording the author.
async fn create_rule(app: &Router, org: &Org, actions: Value) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/automation-rules",
        Some(&org.token),
        Some(json!({
            "name": format!("seam rule {}", Uuid::now_v7()),
            "trigger_event": "ticket.updated",
            "conditions": [],
            "actions": actions,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "rule creation must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("rule id")
        .parse()
        .expect("rule uuid")
}

async fn rewrite_author(state: &AppState, rule_id: Uuid, author: Option<Uuid>) {
    sqlx::query("UPDATE automation_rules SET created_by = $2 WHERE id = $1")
        .bind(rule_id)
        .bind(author)
        .execute(&state.pool)
        .await
        .expect("author rewrite");
}

async fn deactivate(state: &AppState, user_id: Uuid) {
    sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await
        .expect("deactivate");
}

/// Plays the worker: one automation batch for `rule_id` (None = the request
/// names no rule at all) carrying the actions the rule matched.
async fn seam_effects(
    app: &Router,
    org_id: Uuid,
    rule_id: Option<Uuid>,
    ticket_id: Uuid,
) -> axum::response::Response {
    seam_effects_with(
        app,
        org_id,
        rule_id,
        ticket_id,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await
}

/// The same call with an explicit action document, for the case where the
/// batch claims actions the rule does not carry.
async fn seam_effects_with(
    app: &Router,
    org_id: Uuid,
    rule_id: Option<Uuid>,
    ticket_id: Uuid,
    actions: Value,
) -> axum::response::Response {
    let mut body = json!({
        "org_id": org_id,
        "ticket_id": ticket_id,
        "actions": actions,
    });
    if let Some(rule_id) = rule_id {
        body["rule_id"] = json!(rule_id);
    }
    send(
        app,
        axum::http::Method::POST,
        "/api/internal/effects",
        Some(INTERNAL_TOKEN),
        Some(body),
    )
    .await
}

/// Refusal rows this seam writes, newest first.
async fn refusals(pool: &PgPool, org_id: Uuid) -> Vec<(String, Option<String>, Value)> {
    let rows = sqlx::query(
        "SELECT actor_kind, actor_id, detail FROM audit_events \
         WHERE org_id = $1 AND action = 'automation.effects_denied' \
         ORDER BY occurred_at DESC",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .expect("audit read");
    rows.into_iter()
        .map(|row| {
            let kind: String = row.get("actor_kind");
            let id: Option<String> = row.get("actor_id");
            let detail: Value = row.get("detail");
            (kind, id, detail)
        })
        .collect()
}

/// The applied row's actor, which is the principal the batch ran as.
async fn applied_actor(pool: &PgPool, org_id: Uuid) -> Option<(String, Option<String>)> {
    let row = sqlx::query(
        "SELECT actor_kind, actor_id FROM audit_events \
         WHERE org_id = $1 AND action = 'automation.applied' \
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .expect("audit read");
    row.map(|row| (row.get("actor_kind"), row.get("actor_id")))
}

// ===========================================================================
// 1. An authored rule with the grant applies, and the audit names the author
// ===========================================================================

#[tokio::test]
async fn an_authored_rule_applies_and_the_audit_names_the_author() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-allow-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Prioritise me").await;

    let rule = create_rule(
        &app,
        &org,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await;

    // The authoring route records the author, which is what makes the rule
    // runnable at all.
    let author: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by FROM automation_rules WHERE id = $1")
            .bind(rule)
            .fetch_one(&state.pool)
            .await
            .expect("rule row");
    assert_eq!(author, Some(org.user_id), "create records the author");

    let response = seam_effects(&app, org.org_id, Some(rule), ticket).await;
    assert_eq!(response.status(), 200, "the effect applies");
    assert_eq!(ticket_priority(&app, &org, ticket).await, "p1");
    assert_eq!(
        applied_actor(&state.pool, org.org_id).await,
        Some(("human".to_string(), Some(org.user_id.to_string()))),
        "the applied row names the rule's author, not the system"
    );
    assert!(
        refusals(&state.pool, org.org_id).await.is_empty(),
        "an applied batch records no denial"
    );
}

// ===========================================================================
// 2. An author without the grant writes nothing, and the refusal is audited
// ===========================================================================

#[tokio::test]
async fn an_author_without_the_grant_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-denied-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Not for a viewer").await;
    let viewer = add_user_with_role(&state, org.org_id, "read_only", "viewer").await;
    let rule = create_rule(
        &app,
        &org,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await;
    rewrite_author(&state, rule, Some(viewer)).await;

    let response = seam_effects(&app, org.org_id, Some(rule), ticket).await;
    assert_eq!(
        response.status(),
        403,
        "a rule authored by a read-only user cannot prioritise a ticket"
    );
    let body = body_json(response).await;
    assert_eq!(
        body["error"]["reason"], "forbidden",
        "the refusal is typed, not a 500"
    );

    assert_eq!(
        ticket_priority(&app, &org, ticket).await,
        "p3",
        "the refused batch wrote nothing"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "human");
    assert_eq!(
        actor_id.as_deref(),
        Some(viewer.to_string().as_str()),
        "the row names the principal the batch would have run as"
    );
    assert_eq!(detail["rule_id"], json!(rule.to_string()));
    assert_eq!(detail["required"], json!("tickets:edit"));
    assert_eq!(detail["reason_code"], json!("missing_grant"));
}

// ===========================================================================
// 3. A rule with no author is refused, not attributed
// ===========================================================================

#[tokio::test]
async fn an_unauthored_rule_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-noauthor-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Legacy rule subject").await;
    let rule = create_rule(
        &app,
        &org,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await;
    // A rule that predates the author column: the column is nullable, and the
    // ruling is to refuse rather than invent an owner.
    rewrite_author(&state, rule, None).await;

    let response = seam_effects(&app, org.org_id, Some(rule), ticket).await;
    assert_eq!(response.status(), 403, "an unauthored rule is refused");
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    assert_eq!(
        ticket_priority(&app, &org, ticket).await,
        "p3",
        "the refused batch wrote nothing"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "system", "there is nobody to attribute it to");
    assert!(actor_id.is_none());
    assert_eq!(detail["reason_code"], json!("no_author"));
    assert_eq!(detail["rule_id"], json!(rule.to_string()));
}

// ===========================================================================
// 3b. A rule the org does not have is refused, not applied
// ===========================================================================

#[tokio::test]
async fn a_rule_the_org_does_not_have_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-norule-row-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Nobody's rule").await;

    let response = seam_effects(&app, org.org_id, Some(Uuid::now_v7()), ticket).await;
    assert_eq!(
        response.status(),
        404,
        "a rule id that resolves to nothing has no author to run as"
    );
    assert_eq!(ticket_priority(&app, &org, ticket).await, "p3");

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "system");
    assert!(actor_id.is_none());
    assert_eq!(detail["reason_code"], json!("rule_not_found"));
}

// ===========================================================================
// 3c. A batch whose actions are not the rule's is refused
// ===========================================================================

#[tokio::test]
async fn an_action_list_that_is_not_the_rules_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-mismatch-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Not that rule's job").await;
    // The rule's stored action is a note. The batch below asks for a priority
    // change instead, so the rule id is being used as a capability handle for
    // something its author did not author.
    let rule = create_rule(
        &app,
        &org,
        json!([{ "type": "add_internal_comment", "body": "the rule's own note" }]),
    )
    .await;

    let response = seam_effects_with(
        &app,
        org.org_id,
        Some(rule),
        ticket,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "a rule id authorises its own effects, not any effect"
    );
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    assert_eq!(
        ticket_priority(&app, &org, ticket).await,
        "p3",
        "the refused batch wrote nothing"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "human");
    assert_eq!(actor_id.as_deref(), Some(org.user_id.to_string().as_str()));
    assert_eq!(detail["reason_code"], json!("actions_mismatch"));
    assert_eq!(detail["rule_id"], json!(rule.to_string()));
}

// ===========================================================================
// 4. A request that names no rule is refused
// ===========================================================================

#[tokio::test]
async fn a_request_that_names_no_rule_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-norule-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "No rule names me").await;

    let response = seam_effects(&app, org.org_id, None, ticket).await;
    assert_eq!(
        response.status(),
        403,
        "a batch that names no rule has no author to run as"
    );
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    assert_eq!(ticket_priority(&app, &org, ticket).await, "p3");

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "system");
    assert!(actor_id.is_none());
    assert_eq!(detail["reason_code"], json!("no_rule"));
}

// ===========================================================================
// 5. An inactive author is refused
// ===========================================================================

#[tokio::test]
async fn an_inactive_author_is_refused_and_audited() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let org = register_org(&app, format!("effects-inactive-{}", Uuid::now_v7())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "Author walked").await;
    let leaver = add_user_with_role(&state, org.org_id, "technician", "leaver").await;
    let rule = create_rule(
        &app,
        &org,
        json!([{ "type": "set_priority", "priority": "p1" }]),
    )
    .await;
    rewrite_author(&state, rule, Some(leaver)).await;
    deactivate(&state, leaver).await;

    let response = seam_effects(&app, org.org_id, Some(rule), ticket).await;
    assert_eq!(response.status(), 403, "an inactive author is refused");
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    assert_eq!(
        ticket_priority(&app, &org, ticket).await,
        "p3",
        "the refused batch wrote nothing"
    );

    let rows = refusals(&state.pool, org.org_id).await;
    assert_eq!(rows.len(), 1, "the refusal is on the audit trail");
    let (kind, actor_id, detail) = &rows[0];
    assert_eq!(kind, "human");
    assert_eq!(actor_id.as_deref(), Some(leaver.to_string().as_str()));
    assert_eq!(detail["reason_code"], json!("author_inactive"));
}
