//! Org-data export loader coverage (the private build tracker D1, portability/DR).
//!
//! Purpose: prove the D1 export contract at the query layer —
//! 1. WIDER SCOPE: the operational data plane (org profile, client/site/
//!    contact hierarchy, tickets with comments + custom-field values, time
//!    entries) is present alongside the embedded U5 customization sections.
//! 2. Determinism golden stability: two consecutive dumps of an unchanged
//!    org serialize byte-identically apart from `generated_at`.
//! 3. Canonical ordering invariants (semantic keys, ids only tiebreaking).
//! 4. Cross-org isolation: another org's rows never appear (join-guarded
//!    custom-field values included).
//! 5. Secret + people discipline: sealed webhook ciphertext, API-key hash
//!    material, and users-table columns (password hashes, emails) are
//!    structurally absent — people appear only as opaque id references.
//!
//! Org rows use fixed UUIDs 195–199 (the import/export lane band of the
//! 170–199 convention) so parallel suites stay collision-free.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::custom_fields::FieldDefRow;
use core_domain::status::{NoteKind, Priority};
use serde_json::{json, Value};
use std::str::FromStr;
use uuid::Uuid;

/// Distinctive marker material that must NEVER leak into a bundle.
const SEALED_WEBHOOK_SECRET: &[u8] = b"d1-sealed-webhook-secret-DO-NOT-LEAK";
const API_KEY_HASH: &str = "d1-hmac-key-hash-DO-NOT-LEAK-0123456789abcdef";
const PASSWORD_HASH: &str = "d1-argon2-hash-DO-NOT-LEAK";

fn stamp(secs: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(2026, 1, 20, 8, 0, 0).unwrap() + chrono::Duration::seconds(secs)
}

/// Org + default roles + default board + default SLA policy + one user.
async fn seed_base(db: &common::TestDb, n: u64) -> (Uuid, Uuid, Uuid) {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Export Co", &format!("export-{n}"), "EXP")
        .await
        .expect("org");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    let board = core_db::orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board");
    core_db::orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla");
    let user = fixed_uuid(n + 100);
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("owner-{n}@export.test"),
        "Export Owner",
        PASSWORD_HASH,
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");
    (org, board, user)
}

/// Seeds the full operational plane with deliberately NON-canonical insert
/// order so the ordering invariants are exercised.
async fn seed_full_org(db: &common::TestDb, n: u64) -> (Uuid, Uuid, Uuid, Vec<Uuid>) {
    let (org, board, user) = seed_base(db, n).await;
    let mut tx = org_tx(&db.app, org).await;

    // Clients out of order (names force canonical ordering on load).
    let acme = Uuid::now_v7();
    let glob = Uuid::now_v7();
    core_db::clients::create_client(&mut *tx, org, glob, "Glob Inc", Some("second"))
        .await
        .expect("glob");
    core_db::clients::create_client(&mut *tx, org, acme, "Acme Corp", Some("first"))
        .await
        .expect("acme");

    // Sites + contacts under Acme.
    let site = Uuid::now_v7();
    core_db::clients::create_site(
        &mut *tx,
        org,
        site,
        acme,
        "HQ",
        Some("1 Main St"),
        Some("Springfield"),
        None,
        None,
        Some("US"),
        None,
        None,
        Some("America/Chicago"),
    )
    .await
    .expect("site");
    let contact = Uuid::now_v7();
    core_db::clients::create_contact(
        &mut *tx,
        org,
        contact,
        acme,
        Some(site),
        "Dana Contact",
        Some("dana@acme.test"),
        Some("555-0100"),
        Some("IT Lead"),
        true,
    )
    .await
    .expect("contact");

    // Custom field with a VALUE on the ticket (the D1-only data plane).
    let field = Uuid::now_v7();
    core_db::custom_fields::insert_def(
        &mut tx,
        &FieldDefRow {
            id: field,
            org_id: org,
            board_id: None,
            entity: "ticket".into(),
            name: "Impact".into(),
            kind: "select".into(),
            config: json!({"options": ["minor", "major"]}),
            required: false,
            sort_order: 1,
        },
    )
    .await
    .expect("field def");

    // Two tickets (inserted out of number order? numbers are sequential by
    // allocation; the canonical key IS the number, so they must come back
    // sorted by it) + comments + the field value.
    let mut ticket_ids = Vec::new();
    for title in ["B second", "A first"] {
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *tx, org)
            .await
            .expect("number");
        let number = format!("{prefix}-{seq}");
        let entry = core_db::boards::default_status_for_board(&mut *tx, board)
            .await
            .expect("entry");
        let ticket_id = Uuid::now_v7();
        core_db::tickets::insert_ticket(
            &mut tx,
            core_db::tickets::NewTicket {
                id: ticket_id,
                org_id: org,
                number: &number,
                board_id: board,
                status_id: entry.id,
                title,
                body: Some("body text"),
                priority: Priority::P3,
                client_id: Some(acme),
                site_id: None,
                contact_id: None,
                assignee_id: Some(user),
                requester_id: None,
                due_at: None,
            },
        )
        .await
        .expect("ticket");
        ticket_ids.push(ticket_id);
    }
    core_db::tickets::insert_comment(
        &mut *tx,
        core_db::tickets::NewTicketComment {
            id: Uuid::now_v7(),
            org_id: org,
            ticket_id: ticket_ids[0],
            author_id: Some(user),
            kind: NoteKind::Internal,
            body: "first comment",
            workflow_meta: None,
        },
    )
    .await
    .expect("comment");
    core_db::tickets::insert_comment(
        &mut *tx,
        core_db::tickets::NewTicketComment {
            id: Uuid::now_v7(),
            org_id: org,
            ticket_id: ticket_ids[0],
            author_id: None,
            kind: NoteKind::ClientVisible,
            body: "second comment",
            workflow_meta: None,
        },
    )
    .await
    .expect("comment 2");
    core_db::custom_fields::upsert_value(&mut tx, org, field, ticket_ids[0], &json!("major"))
        .await
        .expect("field value");

    // One time entry.
    let started = stamp(0);
    core_db::timers::insert_time_entry(
        &mut *tx,
        &core_domain::entities::TimeEntry {
            approval_status: None,
            approval_reason_code: None,
            unbillable_reason: None,
            id: Uuid::now_v7(),
            org_id: org,
            ticket_id: Some(ticket_ids[0]),
            phase_id: None,
            user_id: user,
            started_at: started,
            ended_at: started + chrono::Duration::minutes(45),
            minutes: 45,
            billable: true,
            notes: Some("work".to_string()),
            work_type: Some("remote".to_string()),
            timer_session_id: None,
            created_at: started,
        },
    )
    .await
    .expect("time entry");

    // Customization rows for the embedded manifest section (secret markers).
    core_db::webhooks::insert_webhook(
        &mut *tx,
        Uuid::now_v7(),
        org,
        "https://hooks.example.test/d1",
        SEALED_WEBHOOK_SECRET,
        &["ticket.created".to_string()],
        user,
        None,
    )
    .await
    .expect("webhook");
    let org_hash = format!("{API_KEY_HASH}-{n}");
    core_db::webhooks::insert_api_key(
        &mut *tx,
        Uuid::now_v7(),
        org,
        "Export Bot",
        &org_hash,
        &format!("mrl_d1key{n:02}"),
        &["tickets:read".to_string()],
        user,
    )
    .await
    .expect("api key");

    tx.commit().await.expect("commit");
    (org, board, user, ticket_ids)
}

async fn load_bundle(db: &common::TestDb, org: Uuid, at: chrono::DateTime<chrono::Utc>) -> Value {
    let mut tx = org_tx(&db.app, org).await;
    let bundle = core_db::export::load_export_bundle(&mut tx, org, at)
        .await
        .expect("bundle loads");
    tx.commit().await.expect("commit");
    serde_json::to_value(&bundle).expect("serialize")
}

/// Serializes with `generated_at` pinned so two dumps compare byte-for-byte.
fn pinned(bundle: &Value) -> String {
    let mut copy = bundle.clone();
    copy["generated_at"] = json!("2026-01-20T08:00:00Z");
    serde_json::to_string(&copy).expect("re-serialize")
}

#[tokio::test]
async fn full_org_bundle_carries_every_section_in_canonical_order() {
    let db = test_db().await;
    let (org, _board, user, tickets) = seed_full_org(&db, 195).await;

    let bundle = load_bundle(&db, org, stamp(0)).await;
    assert_eq!(bundle["bundle_version"], 1);
    assert_eq!(bundle["org_id"], json!(org.to_string()));

    let sections = &bundle["sections"];
    for section in [
        "customization",
        "org",
        "clients",
        "sites",
        "contacts",
        "tickets",
        "time_entries",
        "custom_field_values",
    ] {
        assert!(
            sections.get(section).is_some(),
            "section {section} must always be present"
        );
    }

    // Org profile round-trips identity, never volatile counters.
    assert_eq!(sections["org"]["name"], "Export Co");
    assert_eq!(sections["org"]["slug"], json!("export-195"));
    assert_eq!(sections["org"]["ticket_prefix"], "EXP");
    assert!(sections["org"].get("ticket_sequence").is_none());

    // Clients sorted by NAME (inserted Glob first).
    let clients = sections["clients"].as_array().unwrap();
    let names: Vec<&str> = clients
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Acme Corp", "Glob Inc"]);

    // Tickets sorted by human number, with comments nested in order and
    // the custom-field value in the flat org-wide section.
    let exported = sections["tickets"].as_array().unwrap();
    let numbers: Vec<&str> = exported
        .iter()
        .map(|t| t["number"].as_str().unwrap())
        .collect();
    let mut sorted = numbers.clone();
    sorted.sort();
    assert_eq!(numbers, sorted, "tickets sort by number");
    assert_eq!(exported.len(), 2);
    let first = exported
        .iter()
        .find(|t| t["id"] == json!(tickets[0].to_string()))
        .expect("first ticket present");
    let comments = first["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["body"], "first comment");
    assert_eq!(comments[1]["body"], "second comment");
    assert_eq!(comments[0]["kind"], "internal");
    assert_eq!(first["priority"], "p3");
    assert_eq!(first["category"], "open");
    assert_eq!(first["assignee_id"], json!(user.to_string()));

    let values = sections["custom_field_values"].as_array().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["entity"], "ticket");
    assert_eq!(values[0]["entity_id"], json!(tickets[0].to_string()));
    assert_eq!(values[0]["value"], "major");

    let entries = sections["time_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["minutes"], 45);
    assert_eq!(entries[0]["ticket_id"], json!(tickets[0].to_string()));

    // The embedded customization section is the manifest's shape verbatim.
    let customization = &sections["customization"];
    for section in [
        "custom_fields",
        "boards",
        "automation_rules",
        "webhooks",
        "api_keys",
        "settings",
        "sla",
    ] {
        assert!(
            customization.get(section).is_some(),
            "customization.{section}"
        );
    }
    assert_eq!(customization["webhooks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unchanged_org_dumps_are_byte_identical_apart_from_generated_at() {
    let db = test_db().await;
    let (org, _board, _user, _tickets) = seed_full_org(&db, 196).await;

    let first = load_bundle(&db, org, stamp(0)).await;
    let second = load_bundle(&db, org, stamp(30)).await;
    assert_eq!(pinned(&first), pinned(&second));
    assert_ne!(
        first["generated_at"], second["generated_at"],
        "each dump carries its own stamp"
    );
    assert_eq!(
        first["generated_at"],
        json!(stamp(0).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    );
    assert_eq!(
        second["generated_at"],
        json!(stamp(30).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    );
}

#[tokio::test]
async fn cross_org_rows_never_appear() {
    let db = test_db().await;
    let (org, board, _user, _tickets) = seed_full_org(&db, 197).await;
    // A second org with its own client + ticket.
    let (other, other_board, other_user, _) = seed_full_org(&db, 198).await;
    assert_ne!(org, other);

    let bundle = load_bundle(&db, org, stamp(0)).await;
    let serialized = serde_json::to_string(&bundle).unwrap();
    assert!(
        !serialized.contains(&other.to_string()),
        "foreign org id leaked"
    );
    assert!(
        !serialized.contains("owner-197@export.test"),
        "foreign user identity leaked"
    );

    // And the other way: the second org's bundle carries its own rows and
    // none of the first org's — its default board is genuinely its own.
    let other_bundle = load_bundle(&db, other, stamp(0)).await;
    let other_serialized = serde_json::to_string(&other_bundle).unwrap();
    assert!(!other_serialized.contains(&org.to_string()));
    assert_eq!(
        other_bundle["sections"]["customization"]["boards"][0]["id"],
        json!(other_board.to_string()),
        "the second org's default board is present"
    );
    assert!(
        !other_serialized.contains(&board.to_string()),
        "the first org's board must not leak into the second org's bundle"
    );
    let _ = other_user;
}

#[tokio::test]
async fn secrets_and_user_material_are_structurally_absent() {
    let db = test_db().await;
    let (org, _board, _user, _tickets) = seed_full_org(&db, 199).await;

    let bundle = load_bundle(&db, org, stamp(0)).await;
    let serialized = serde_json::to_string(&bundle).unwrap();
    // Sealed webhook secret / raw hmac hash never enter the path.
    assert!(
        !serialized.contains(std::str::from_utf8(SEALED_WEBHOOK_SECRET).unwrap()),
        "webhook secret leaked"
    );
    assert!(!serialized.contains(API_KEY_HASH), "api-key hash leaked");
    assert!(!serialized.contains("secret_encrypted"));
    assert!(!serialized.contains("key_hash"));
    assert!(
        !serialized.contains("password_hash"),
        "password column name leaked"
    );
    assert!(!serialized.contains(PASSWORD_HASH), "password hash leaked");
    // People appear as opaque ids: no user email surfaces.
    assert!(!serialized.contains("@export.test"), "user email leaked");
}

#[tokio::test]
async fn empty_org_yields_present_but_empty_sections() {
    let db = test_db().await;
    let (org, _board, _user) = seed_base(&db, 194).await;
    let bundle = load_bundle(&db, org, stamp(0)).await;
    let sections = &bundle["sections"];
    assert_eq!(sections["clients"].as_array().unwrap().len(), 0);
    assert_eq!(sections["sites"].as_array().unwrap().len(), 0);
    assert_eq!(sections["contacts"].as_array().unwrap().len(), 0);
    assert_eq!(sections["tickets"].as_array().unwrap().len(), 0);
    assert_eq!(sections["time_entries"].as_array().unwrap().len(), 0);
    assert_eq!(sections["custom_field_values"].as_array().unwrap().len(), 0);
    // Defaults-first baseline still documented in the customization part.
    assert_eq!(
        sections["customization"]["boards"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The Priority/NoteKind/FromStr imports stay exercised even when empty.
    assert_eq!(Priority::from_str("p2").unwrap().as_str(), "p2");
}
