//! The messaging surface's storage contract (migration 0190).
//!
//! Purpose: prove the four properties the inbound callback path depends on,
//! each of which is a correctness or evidence property rather than a
//! formatting one:
//!
//!   * a vendor selector resolves **candidates**, only for its own vendor, and
//!     never a `disabled` connection — the signature decides which one is real;
//!   * a message is identified by connection + channel + message id, so a
//!     message id from another channel cannot attribute a press to the wrong
//!     card;
//!   * the ledger is one row per callback digest: the second insert returns the
//!     first row instead of writing a second;
//!   * the card's facts carry the client restriction list, and the ticket
//!     number resolves inside one org only.
//!
//! Non-purpose: signature verification (app-bridges' unit tests), the HTTP
//! shape (`tests/http_flows/connector_inbound_flow.rs` — a different binary),
//! and RLS isolation (`db_suite`).
//!
//! Slot discipline: fixed-UUID block 96000+, self-contained, so parallel suites
//! sharing one database cannot collide.

// The repository's core-db functions split between `&mut PgConnection` and a
// generic `Executor` bound. The reborrow (`&mut *tx`) is required by the
// generic half and merely explicit on the other, so the lint is off for the
// file rather than toggled per call.
#![allow(clippy::explicit_auto_deref)]

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use core_db::connector_messaging::{
    callback_candidates, identity_for_vendor, insert_action, insert_message,
    list_actions_for_ticket, lock_callback, message_by_vendor, ticket_facts, ticket_id_by_number,
    NewAction, NewChannelMessage,
};
use core_db::{boards, clients, orgs, plugins, tickets, users};
use core_domain::entities::{
    PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2, PluginRuntimeKind,
    PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_domain::status::{Priority, TicketStatusCategory};
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    org: Uuid,
    client: Uuid,
    ticket: Uuid,
    number: String,
    install: Uuid,
}

/// An org, a client, a board with two columns, and one ticket.
async fn seed(db: &TestDb, base: u64, slug: &str) -> Fixture {
    let org = fixed_uuid(base);
    let client = fixed_uuid(base + 1);
    let admin: Uuid = fixed_uuid(base + 2);
    let board = fixed_uuid(base + 3);
    let open_status = fixed_uuid(base + 4);
    let progress_status = fixed_uuid(base + 5);
    let ticket = fixed_uuid(base + 6);

    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Messaging Co", slug, "MSG")
        .await
        .expect("org");
    clients::create_client(&mut *tx, org, client, "Acme", None)
        .await
        .expect("client");
    // One active user so the assignee name has something to resolve.
    let hash = "x".repeat(32);
    users::create_user(
        &mut *tx,
        admin,
        org,
        "tech@example.test",
        "Dana Scott",
        &hash,
        "UTC",
    )
    .await
    .expect("user");
    boards::create_board(&mut *tx, org, board, "Service desk", None, true)
        .await
        .expect("board");
    boards::add_status(
        &mut *tx,
        org,
        open_status,
        board,
        "New",
        TicketStatusCategory::Open,
        0,
        true,
        None,
    )
    .await
    .expect("open status");
    boards::add_status(
        &mut *tx,
        org,
        progress_status,
        board,
        "Working",
        TicketStatusCategory::InProgress,
        1,
        false,
        None,
    )
    .await
    .expect("progress status");
    let number = "T-1042".to_string();
    tickets::insert_ticket(
        &mut *tx,
        tickets::NewTicket {
            id: ticket,
            org_id: org,
            number: &number,
            board_id: board,
            status_id: open_status,
            title: "Printer down in reception",
            body: Some("A long body that must never reach a channel."),
            priority: Priority::P2,
            client_id: Some(client),
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket");

    let install = seed_install(&mut tx, org, base + 7, slug).await;
    tx.commit().await.expect("commit");

    Fixture {
        org,
        client,
        ticket,
        number,
        install,
    }
}

/// One installed connector definition, so a connection has a parent row.
async fn seed_install(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: Uuid,
    base: u64,
    slug: &str,
) -> Uuid {
    let manifest = PluginManifestV2 {
        name: "app-bridges".into(),
        version: "1.0.0".into(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "Slack and Teams".into(),
        capabilities: Vec::new(),
        product_kinds: vec!["notifications".into()],
        connector_surfaces: None,
        settings_schema: Some(json!({
            "type": "object",
            "properties": {
                "slack_team_id": { "type": "string" },
                "teams_bot_app_id": { "type": "string" }
            }
        })),
        artifact_kind: Some(PluginRuntimeKind::HttpConnector),
        connector_profile: None,
        surface_profile: None,
    };
    let entry = PluginCatalogEntry {
        id: fixed_uuid(base),
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
        id: fixed_uuid(base + 1),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(&mut **tx, &entry, &version)
        .await
        .expect("catalog entry");
    let install = fixed_uuid(base + 2);
    plugins::create_install(
        &mut **tx,
        &PluginInstall {
            id: install,
            org_id: org,
            catalog_id: entry.id,
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
    install
}

/// One connection, stated directly (the write route is a later lane).
async fn seed_connection(
    db: &TestDb,
    fix: &Fixture,
    id: Uuid,
    label: &str,
    settings: serde_json::Value,
    status: &str,
) {
    let mut tx = org_tx(&db.app, fix.org).await;
    sqlx::query(
        "INSERT INTO connector_connections (id, org_id, installation_id, label, settings, status)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(fix.org)
    .bind(fix.install)
    .bind(label)
    .bind(settings)
    .bind(status)
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");
}

async fn seed_binding(db: &TestDb, fix: &Fixture, id: Uuid, connection: Uuid, channel: &str) {
    let mut tx = org_tx(&db.app, fix.org).await;
    sqlx::query(
        "INSERT INTO connector_bindings
           (id, org_id, connection_id, binding_kind, label, external_id, external_parent)
         VALUES ($1, $2, $3, 'slack_channel', $4, $5, 'T-TEAM')",
    )
    .bind(id)
    .bind(fix.org)
    .bind(connection)
    .bind(format!("#{channel}"))
    .bind(channel)
    .execute(&mut *tx)
    .await
    .expect("binding");
    tx.commit().await.expect("commit");
}

/// The selector resolves candidates for its own vendor only, and a `disabled`
/// connection is never one of them.
#[tokio::test]
async fn candidates_are_per_vendor_and_skip_a_disabled_connection() {
    let db = test_db().await;
    let fix = seed(&db, 96000, "msg-candidates").await;
    let slack_live = fixed_uuid(96020);
    let slack_off = fixed_uuid(96021);
    let teams = fixed_uuid(96022);
    seed_connection(
        &db,
        &fix,
        slack_live,
        "Slack — helpdesk",
        json!({ "slack_team_id": "T-AAA" }),
        "connected",
    )
    .await;
    seed_connection(
        &db,
        &fix,
        slack_off,
        "Slack — retired",
        json!({ "slack_team_id": "T-OFF" }),
        "disabled",
    )
    .await;
    seed_connection(
        &db,
        &fix,
        teams,
        "Teams",
        json!({ "teams_bot_app_id": "bot-app-1" }),
        "connected",
    )
    .await;

    let slack = callback_candidates(&db.admin, "slack", "T-AAA")
        .await
        .expect("candidates");
    assert_eq!(slack.len(), 1, "only the live Slack connection matches");
    assert_eq!(slack[0].r_id, slack_live);
    assert_eq!(slack[0].r_org_id, fix.org);

    // A disabled connection is not a candidate: a destination an operator
    // switched off must not accept presses either.
    assert!(callback_candidates(&db.admin, "slack", "T-OFF")
        .await
        .expect("candidates")
        .is_empty());

    // The same string under the other vendor resolves through the other key.
    let teams = callback_candidates(&db.admin, "teams", "bot-app-1")
        .await
        .expect("candidates");
    assert_eq!(teams.len(), 1);
    assert!(callback_candidates(&db.admin, "teams", "T-AAA")
        .await
        .expect("candidates")
        .is_empty());
    assert!(callback_candidates(&db.admin, "slack", "")
        .await
        .expect("candidates")
        .is_empty());
}

/// A vendor identity maps to exactly one Marula user, and an unmapped one is
/// `None` rather than a guess.
#[tokio::test]
async fn a_vendor_identity_resolves_to_one_user() {
    let db = test_db().await;
    let fix = seed(&db, 96100, "msg-identity").await;
    let connection = fixed_uuid(96120);
    seed_connection(
        &db,
        &fix,
        connection,
        "Slack",
        json!({ "slack_team_id": "T-ID" }),
        "connected",
    )
    .await;
    let mut tx = org_tx(&db.app, fix.org).await;
    let user: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 LIMIT 1")
        .bind(fix.org)
        .fetch_one(&mut *tx)
        .await
        .expect("user");
    sqlx::query(
        "INSERT INTO user_external_identities
           (id, org_id, user_id, connection_id, identity_kind, external_id)
         VALUES ($1, $2, $3, $4, 'slack_user', 'U-1')",
    )
    .bind(Uuid::now_v7())
    .bind(fix.org)
    .bind(user)
    .bind(connection)
    .execute(&mut *tx)
    .await
    .expect("identity");
    let mapped = identity_for_vendor(&mut *tx, fix.org, connection, "slack_user", "U-1")
        .await
        .expect("lookup");
    let unmapped = identity_for_vendor(&mut *tx, fix.org, connection, "slack_user", "U-2")
        .await
        .expect("lookup");
    let other_kind = identity_for_vendor(&mut *tx, fix.org, connection, "teams_user", "U-1")
        .await
        .expect("lookup");
    tx.commit().await.expect("commit");

    assert_eq!(mapped, Some(user));
    assert_eq!(unmapped, None, "an unmapped press is not somebody else");
    assert_eq!(
        other_kind, None,
        "identity kinds do not bleed into each other"
    );
}

/// One vendor message is one registry row, and the channel is part of its
/// identity.
#[tokio::test]
async fn a_message_is_identified_by_connection_channel_and_id() {
    let db = test_db().await;
    let fix = seed(&db, 96200, "msg-registry").await;
    let connection = fixed_uuid(96220);
    let binding = fixed_uuid(96221);
    seed_connection(
        &db,
        &fix,
        connection,
        "Slack",
        json!({ "slack_team_id": "T-MSG" }),
        "connected",
    )
    .await;
    seed_binding(&db, &fix, binding, connection, "C-1").await;

    let mut tx = org_tx(&db.app, fix.org).await;
    let row = insert_message(
        &mut *tx,
        NewChannelMessage {
            id: Uuid::now_v7(),
            org_id: fix.org,
            connection_id: connection,
            binding_id: binding,
            ticket_id: fix.ticket,
            channel_id: "C-1",
            message_id: "1699999999.000100",
            shared_by: None,
            shared_client_id: Some(fix.client),
        },
    )
    .await
    .expect("message");
    assert_eq!(row.ticket_id, fix.ticket);

    let found = message_by_vendor(&mut *tx, fix.org, connection, "C-1", "1699999999.000100")
        .await
        .expect("lookup")
        .expect("a registered message resolves");
    assert_eq!(found.id, row.id);

    // The same ts in a different channel is a different message; resolving it
    // must not return this card.
    let other_channel =
        message_by_vendor(&mut *tx, fix.org, connection, "C-2", "1699999999.000100")
            .await
            .expect("lookup");
    assert!(other_channel.is_none());

    // A second row for the same vendor message is refused by the unique index.
    let duplicate = insert_message(
        &mut *tx,
        NewChannelMessage {
            id: Uuid::now_v7(),
            org_id: fix.org,
            connection_id: connection,
            binding_id: binding,
            ticket_id: fix.ticket,
            channel_id: "C-1",
            message_id: "1699999999.000100",
            shared_by: None,
            shared_client_id: None,
        },
    )
    .await;
    assert!(duplicate.is_err(), "one vendor message, one registry row");
    tx.rollback().await.expect("rollback");
}

/// The ledger is one row per callback digest: a retry reads the first answer.
#[tokio::test]
async fn a_callback_digest_has_exactly_one_ledger_row() {
    let db = test_db().await;
    let fix = seed(&db, 96300, "msg-ledger").await;
    let connection = fixed_uuid(96320);
    seed_connection(
        &db,
        &fix,
        connection,
        "Slack",
        json!({ "slack_team_id": "T-LEDGER" }),
        "connected",
    )
    .await;
    let detail = json!({ "assignee_id": null });
    let key = "a".repeat(64);

    let mut tx = org_tx(&db.app, fix.org).await;
    lock_callback(&mut *tx, fix.org, connection, &key)
        .await
        .expect("lock");
    let first = insert_action(
        &mut *tx,
        NewAction {
            id: Uuid::now_v7(),
            org_id: fix.org,
            connection_id: connection,
            callback_key: &key,
            action: "take",
            outcome: "applied",
            refusal_code: None,
            ticket_id: Some(fix.ticket),
            ticket_number: Some(&fix.number),
            message_id: None,
            channel_id: Some("C-1"),
            vendor_user_id: Some("U-1"),
            actor_user_id: None,
            detail: &detail,
        },
    )
    .await
    .expect("first insert");

    // The retry: same digest, different candidate row id, and the stored row
    // comes back instead of a second write.
    let second = insert_action(
        &mut *tx,
        NewAction {
            id: Uuid::now_v7(),
            org_id: fix.org,
            connection_id: connection,
            callback_key: &key,
            action: "take",
            outcome: "applied",
            refusal_code: None,
            ticket_id: Some(fix.ticket),
            ticket_number: Some(&fix.number),
            message_id: None,
            channel_id: Some("C-1"),
            vendor_user_id: Some("U-1"),
            actor_user_id: None,
            detail: &detail,
        },
    )
    .await
    .expect("replay");
    assert_eq!(second.id, first.id, "a retry is the same ledger row");

    let rows = list_actions_for_ticket(&mut *tx, fix.org, fix.ticket, 10)
        .await
        .expect("history");
    assert_eq!(rows.len(), 1, "one press, one row");
    assert_eq!(rows[0].ticket_number.as_deref(), Some("T-1042"));
    assert_eq!(rows[0].vendor_user_id.as_deref(), Some("U-1"));
    tx.commit().await.expect("commit");
}

/// The card's facts are the six the renderers show, plus the client's
/// restriction list — and the ticket number resolves inside one org only.
#[tokio::test]
async fn ticket_facts_carry_the_six_facts_and_the_restriction_list() {
    let db = test_db().await;
    let fix = seed(&db, 96400, "msg-facts").await;

    // A restricted client list on the ticket's client.
    let mut tx = org_tx(&db.app, fix.org).await;
    sqlx::query("UPDATE clients SET restricted_to = $1::jsonb WHERE id = $2")
        .bind(json!([fixed_uuid(96450).to_string()]))
        .bind(fix.client)
        .execute(&mut *tx)
        .await
        .expect("restrict");

    let facts = ticket_facts(&mut *tx, fix.org, fix.ticket)
        .await
        .expect("facts")
        .expect("the ticket exists");
    assert_eq!(facts.number, "T-1042");
    assert_eq!(facts.title, "Printer down in reception");
    assert_eq!(facts.client_name.as_deref(), Some("Acme"));
    assert_eq!(facts.status_name, "New");
    assert_eq!(facts.status_category, "open");
    assert_eq!(facts.priority, "p2");
    assert_eq!(facts.restricted_to, vec![fixed_uuid(96450)]);

    // The number resolves in its own org, and nowhere else.
    let found = ticket_id_by_number(&mut *tx, fix.org, "T-1042")
        .await
        .expect("lookup");
    assert_eq!(found, Some(fix.ticket));
    let elsewhere = ticket_id_by_number(&mut *tx, fixed_uuid(96499), "T-1042")
        .await
        .expect("lookup");
    assert_eq!(elsewhere, None, "a number is unique per org, not globally");
    tx.commit().await.expect("commit");
}
