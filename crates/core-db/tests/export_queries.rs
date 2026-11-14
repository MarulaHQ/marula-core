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
        None,
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
        "contracts",
        "invoice_runs",
        "invoices",
        "credit_notes",
        "assets",
        "kb_categories",
        "kb_articles",
        "board_operational",
        "audit_events",
        "plugin_installs",
        "connector_connections",
        "capability_defaults",
        "external_refs",
        "record_links",
        "documents",
        "document_links",
        "tags",
        "ticket_tags",
        "attachments",
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
    for empty in [
        "contracts",
        "invoice_runs",
        "invoices",
        "credit_notes",
        "assets",
        "kb_categories",
        "kb_articles",
        "audit_events",
        "plugin_installs",
        "connector_connections",
        "capability_defaults",
        "external_refs",
        "record_links",
        "documents",
        "document_links",
        "tags",
        "ticket_tags",
        "attachments",
    ] {
        assert_eq!(
            sections[empty].as_array().unwrap().len(),
            0,
            "empty org: {empty} is present and empty"
        );
    }
    // board_operational is never empty (defaults-first seeding plants a board),
    // so its empty state is "the extras are unset", not "no rows".
    let op = &sections["board_operational"];
    assert_eq!(op["boards"].as_array().unwrap().len(), 1);
    // Keys are present even when the value is unset — the section's shape does
    // not depend on the org having filled it in.
    assert!(op["boards"][0].get("description").is_some());
    assert!(op["boards"][0].get("inbound_address").is_some());
    assert!(op["boards"][0].get("board_group_id").is_some());
    assert_eq!(op["statuses"].as_array().unwrap().len(), 4);
    assert!(op["statuses"][0].get("wip_limit").is_some());
    assert!(op["statuses"][0].get("client_visible").is_some());
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

// --- G13 coverage: the sections the first cut of D1 did not carry ---------------
//
// Every test below seeds through raw SQL on purpose: it asserts what the LOADER
// reads, so it must not lean on the write path of the subsystem under test (a
// write path that silently drops a column would hide exactly the drift these
// tests exist to catch).

/// Distinctive marker material that must never appear in a serialized bundle.
const CONNECTOR_SEALED_SECRET: &[u8] = b"g13-sealed-connector-secret-DO-NOT-LEAK";
const ATTACHMENT_SHA: &str = "9f2c7b1e4d5a6038c9e1f2a3b4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7";
const ATTACHMENT_STORAGE_KEY: &str =
    "org/attachment/9f2c7b1e4d5a6038c9e1f2a3b4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7";

/// Ids a G13-seeded org hands back to its assertions.
#[allow(dead_code)]
struct G13Seed {
    org: Uuid,
    board: Uuid,
    user: Uuid,
    tickets: Vec<Uuid>,
    client: Uuid,
    contract: Uuid,
    invoice: Uuid,
    credit_note: Uuid,
    asset: Uuid,
    kb_article: Uuid,
    document: Uuid,
    tag: Uuid,
    attachment: Uuid,
    install: Uuid,
    connection: Uuid,
}

/// Seeds the pre-G13 plane (hierarchy, tickets, time entry, custom fields) and
/// then one row of every newly covered kind.
async fn seed_g13_org(db: &common::TestDb, n: u64) -> G13Seed {
    let (org, board, user, tickets) = seed_full_org(db, n).await;
    let mut tx = org_tx(&db.app, org).await;

    let (client,): (Uuid,) =
        sqlx::query_as("SELECT id FROM clients WHERE org_id = $1 AND name = 'Acme Corp'")
            .bind(org)
            .fetch_one(&mut *tx)
            .await
            .expect("acme client");
    let (time_entry,): (Uuid,) =
        sqlx::query_as("SELECT id FROM time_entries WHERE org_id = $1 LIMIT 1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await
            .expect("seeded time entry");

    // Money: contract → run → invoice (2 lines, 1 receipt, 1 consumed entry)
    // → credit note with one application. Inserted out of number order so the
    // section's ordering key is actually exercised.
    let contract = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type,
             status, currency, effective_from, billing_frequency, billing_anchor,
             amount_minor, hourly_rate_minor, included_minutes, overage_rate_minor,
             notes, created_at, updated_at)
         VALUES ($1,$2,$3,'CT-2','Managed Services','recurring','active','USD',
             DATE '2026-01-01','monthly',DATE '2026-01-01',500000,15000,600,18000,
             'auto-renew', $4, $4)",
    )
    .bind(contract)
    .bind(org)
    .bind(client)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("contract");

    let run = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoice_runs (id, org_id, number, period_from, period_to,
             invoice_count, total_minor, currency, due_in_days, created_at)
         VALUES ($1,$2,'RUN-2026-01',DATE '2026-01-01',DATE '2026-01-31',1,510000,'USD',30,$3)",
    )
    .bind(run)
    .bind(org)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("invoice run");

    let invoice = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number,
             status, currency, period_from, period_to, issued_at, due_on,
             total_minor, total_paid_minor, dunning_state, po_number, created_at, updated_at)
         VALUES ($1,$2,$3,$4,$5,'INV-0002','issued','USD',DATE '2026-01-01',
             DATE '2026-01-31',DATE '2026-02-01',DATE '2026-03-03',510000,150000,
             'overdue','PO-77',$6,$6)",
    )
    .bind(invoice)
    .bind(org)
    .bind(run)
    .bind(client)
    .bind(contract)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("invoice");

    for (position, kind, description, amount) in [
        (2_i32, "time_materials", "Remote support", 30000_i64),
        (1_i32, "fixed_fee", "Onsite visit", 480000_i64),
    ] {
        sqlx::query(
            "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind,
                 description, period_start, period_end, quantity_minutes,
                 unit_amount_minor, amount_minor, position)
             VALUES ($1,$2,$3,$4,$5,$6,DATE '2026-01-01',DATE '2026-01-31',60,$7,$8,$9)",
        )
        .bind(Uuid::now_v7())
        .bind(org)
        .bind(invoice)
        .bind(contract)
        .bind(kind)
        .bind(description)
        .bind(amount)
        .bind(amount)
        .bind(position)
        .execute(&mut *tx)
        .await
        .expect("invoice line");
    }

    let payment = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency,
             provider, provider_ref, notes, received_at, created_by)
         VALUES ($1,$2,$3,150000,'USD','stripe','pi_123','partial',$4,$5)",
    )
    .bind(payment)
    .bind(org)
    .bind(invoice)
    .bind(stamp(60))
    .bind(user)
    .execute(&mut *tx)
    .await
    .expect("payment");

    sqlx::query(
        "INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id)
         VALUES ($1,$2,$3,$4)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(invoice)
    .bind(time_entry)
    .execute(&mut *tx)
    .await
    .expect("consumed entry");

    let credit_note = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO credit_notes (id, org_id, number, client_id,
             issued_against_invoice_id, status, currency, total_minor, applied_minor,
             reason, created_by, created_at, updated_at)
         VALUES ($1,$2,'CN-0001',$3,$4,'issued','USD',40000,40000,'service credit',$5,$6,$6)",
    )
    .bind(credit_note)
    .bind(org)
    .bind(client)
    .bind(invoice)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("credit note");

    sqlx::query(
        "INSERT INTO credit_note_applications (id, org_id, credit_note_id, invoice_id,
             amount_minor, payment_id, created_by, created_at)
         VALUES ($1,$2,$3,$4,40000,$5,$6,$7)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(credit_note)
    .bind(invoice)
    .bind(payment)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("credit application");

    // An asset with the curated tier, identity keys and a rental lifecycle.
    let asset = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO device_assets (id, org_id, client_id, tag, name, kind, status,
             fields, curated_fields, identity_keys, ownership, rental_vendor,
             rental_start, rental_end, rental_cost_minor, rental_currency, rental_cycle,
             created_at, updated_at)
         VALUES ($1,$2,$3,'ACME-LT-01','Reception laptop','workstation','active',
             '{\"cpu\":\"i7\",\"ram_gb\":16}','{\"cpu\":\"i7\"}',
             '{\"serial\":\"SN-123\"}','leased','Acme Leasing',
             DATE '2026-01-01',DATE '2027-01-01',45000,'USD','monthly',$4,$4)",
    )
    .bind(asset)
    .bind(org)
    .bind(client)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("asset");

    // Knowledge base: a client-visible category and article.
    let category = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_categories (id, org_id, name, slug, description, position,
             client_visible, created_at, updated_at)
         VALUES ($1,$2,'Getting started','getting-started','Onboarding',1,true,$3,$3)",
    )
    .bind(category)
    .bind(org)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("kb category");

    let kb_article = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_articles (id, org_id, category_id, title, slug, body, status,
             client_visible, created_by, created_at, updated_at, content_jsonb)
         VALUES ($1,$2,$3,'Reset your password','reset-password','Click reset.','published',
             true,$4,$5,$5,'{\"schema_ver\":1}')",
    )
    .bind(kb_article)
    .bind(org)
    .bind(category)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("kb article");

    // Docs platform: a document with two versions and a link to its client.
    let document = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO documents (id, org_id, title, current_version, created_by,
             created_at, updated_at, client_id, fields)
         VALUES ($1,$2,'Network runbook',2,$3,$4,$4,$5,'{\"vlan\":\"10\"}')",
    )
    .bind(document)
    .bind(org)
    .bind(user)
    .bind(stamp(0))
    .bind(client)
    .execute(&mut *tx)
    .await
    .expect("document");

    for (version, body) in [(1_i32, "first draft"), (2_i32, "final")] {
        sqlx::query(
            "INSERT INTO document_versions (id, org_id, document_id, version, title,
                 body, change_summary, author_id, created_at)
             VALUES ($1,$2,$3,$4,'Network runbook',$5,'edit',$6,$7)",
        )
        .bind(Uuid::now_v7())
        .bind(org)
        .bind(document)
        .bind(version)
        .bind(body)
        .bind(user)
        .bind(stamp(version as i64))
        .execute(&mut *tx)
        .await
        .expect("document version");
    }

    sqlx::query(
        "INSERT INTO document_links (id, org_id, from_kind, from_id, to_kind, to_id,
             created_by, created_at)
         VALUES ($1,$2,'document',$3,'client',$4,$5,$6)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(document)
    .bind(client)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("document link");

    // A tag applied to the first ticket.
    let tag = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tags (id, org_id, name, color, created_by, created_at, updated_at)
         VALUES ($1,$2,'vip','#ff0000',$3,$4,$4)",
    )
    .bind(tag)
    .bind(org)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("tag");
    sqlx::query(
        "INSERT INTO ticket_tags (org_id, ticket_id, tag_id, applied_by, applied_at)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(org)
    .bind(tickets[0])
    .bind(tag)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("ticket tag");

    // Attachment METADATA (the bytes are never in the bundle).
    let attachment = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO attachments (id, org_id, kind, ticket_id, filename, content_type,
             size_bytes, sha256, storage_key, uploaded_by_user_id, created_at)
         VALUES ($1,$2,'ticket',$3,'screenshot.png','image/png',2048,$4,$5,$6,$7)",
    )
    .bind(attachment)
    .bind(org)
    .bind(tickets[0])
    .bind(ATTACHMENT_SHA)
    .bind(ATTACHMENT_STORAGE_KEY)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("attachment");

    // Plugin install (+ non-secret settings) and a connection carrying a sealed
    // credential, a binding, a capability default and a vendor identity.
    let catalog = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO plugin_catalog (id, slug, name, description, publisher,
             interface_version, created_at, updated_at)
         VALUES ($1,$3,'RMM Sync','Device sync','Acme Plugins','1',$2,$2)",
    )
    .bind(catalog)
    .bind(stamp(0))
    .bind(format!("rmm-sync-{n}"))
    .execute(&mut *tx)
    .await
    .expect("catalog entry");

    let install = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO plugin_installs (id, org_id, catalog_id, version, status,
             granted_capabilities, installed_by, installed_at, updated_at, module_digest,
             pending_version, pending_manifest)
         VALUES ($1,$2,$3,'1.4.0','enabled'::plugin_install_status,
             '[\"tickets:read\"]',$4,$5,$5,$6,'1.5.0','{}'::jsonb)",
    )
    .bind(install)
    .bind(org)
    .bind(catalog)
    .bind(user)
    .bind(stamp(0))
    .bind("a".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("install");

    sqlx::query(
        "INSERT INTO plugin_install_settings (install_id, config, updated_by, updated_at)
         VALUES ($1,'{\"sync_interval_minutes\":15}',NULL,$2)",
    )
    .bind(install)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("install settings");

    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO connector_connections (id, org_id, installation_id, label,
             settings, secrets_encrypted, status, created_at, updated_at)
         VALUES ($1,$2,$3,'Acme RMM','{\"base_url\":\"https://rmm.example.test\"}',
             $4,'connected',$5,$5)",
    )
    .bind(connection)
    .bind(org)
    .bind(install)
    .bind(CONNECTOR_SEALED_SECRET.to_vec())
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("connection");

    sqlx::query(
        "INSERT INTO connector_bindings (id, org_id, connection_id, binding_kind,
             label, external_id, vendor_ids, enabled, created_at, updated_at)
         VALUES ($1,$2,$3,'slack_channel','#helpdesk','C0123','{}',true,$4,$4)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(connection)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("binding");

    sqlx::query(
        "INSERT INTO capability_defaults (id, org_id, capability, connection_id)
         VALUES ($1,$2,'remote_access',$3)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(connection)
    .execute(&mut *tx)
    .await
    .expect("capability default");

    sqlx::query(
        "INSERT INTO external_refs (id, org_id, record_type, record_id, connection_id,
             source_system, external_kind, external_id, external_url, first_seen_at)
         VALUES ($1,$2,'configuration',$3,$4,'rmm','device','dev-42',
             'https://rmm.example.test/devices/42',$5)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(asset)
    .bind(connection)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("external ref");

    sqlx::query(
        "INSERT INTO record_links (id, org_id, from_kind, from_id, to_kind, to_id,
             created_by, created_at)
         VALUES ($1,$2,'ticket',$3,'configuration',$4,$5,$6)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(tickets[0])
    .bind(asset)
    .bind(user)
    .bind(stamp(0))
    .execute(&mut *tx)
    .await
    .expect("record link");

    // One audit row (the org's history stream).
    core_db::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id: org,
            actor: core_domain::entities::Actor::Human(user),
            action: "ticket.status_changed".into(),
            entity_type: Some("ticket".into()),
            entity_id: Some(tickets[0].to_string()),
            detail: json!({ "to": "resolved" }),
            ip: None,
            occurred_at: stamp(0),
        },
    )
    .await
    .expect("audit row");

    tx.commit().await.expect("commit");

    G13Seed {
        org,
        board,
        user,
        tickets,
        client,
        contract,
        invoice,
        credit_note,
        asset,
        kb_article,
        document,
        tag,
        attachment,
        install,
        connection,
    }
}

#[tokio::test]
async fn money_documents_travel_whole() {
    let db = test_db().await;
    let s = seed_g13_org(&db, 190).await;
    let bundle = load_bundle(&db, s.org, stamp(0)).await;
    let sections = &bundle["sections"];

    let contracts = sections["contracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0]["id"], json!(s.contract.to_string()));
    assert_eq!(contracts[0]["contract_type"], "recurring");
    assert_eq!(contracts[0]["included_minutes"], 600);

    let runs = sections["invoice_runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["number"], "RUN-2026-01");

    let invoices = sections["invoices"].as_array().unwrap();
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice["id"], json!(s.invoice.to_string()));
    assert_eq!(invoice["number"], "INV-0002");
    assert_eq!(invoice["status"], "issued");
    assert_eq!(invoice["dunning_state"], "overdue");
    assert_eq!(invoice["total_minor"], 510000);
    assert_eq!(invoice["total_paid_minor"], 150000);
    assert_eq!(invoice["contract_id"], json!(s.contract.to_string()));

    // Lines arrive in `position` order although they were inserted 2, 1.
    let lines = invoice["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["position"], 1);
    assert_eq!(lines[0]["description"], "Onsite visit");
    assert_eq!(lines[1]["position"], 2);
    assert_eq!(lines[1]["kind"], "time_materials");

    let payments = invoice["payments"].as_array().unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0]["provider"], "stripe");
    assert_eq!(payments[0]["amount_minor"], 150000);

    // The consumed time entry is referenced by id, never copied.
    let consumed = invoice["billed_time_entry_ids"].as_array().unwrap();
    assert_eq!(consumed.len(), 1);
    let entries = sections["time_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(consumed[0], entries[0]["id"]);

    let notes = sections["credit_notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["id"], json!(s.credit_note.to_string()));
    assert_eq!(notes[0]["total_minor"], 40000);
    assert_eq!(notes[0]["applied_minor"], 40000);
    let applications = notes[0]["applications"].as_array().unwrap();
    assert_eq!(applications.len(), 1);
    assert_eq!(applications[0]["invoice_id"], json!(s.invoice.to_string()));
    assert_eq!(applications[0]["amount_minor"], 40000);
}

#[tokio::test]
async fn assets_kb_documents_tags_and_attachments_are_carried() {
    let db = test_db().await;
    let s = seed_g13_org(&db, 191).await;
    let bundle = load_bundle(&db, s.org, stamp(0)).await;
    let sections = &bundle["sections"];

    let assets = sections["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 1);
    let asset = &assets[0];
    assert_eq!(asset["id"], json!(s.asset.to_string()));
    assert_eq!(asset["tag"], "ACME-LT-01");
    assert_eq!(asset["ownership"], "leased");
    assert_eq!(asset["fields"]["ram_gb"], 16);
    assert_eq!(asset["curated_fields"]["cpu"], "i7");
    assert_eq!(asset["identity_keys"]["serial"], "SN-123");
    // Feed-written churn is not carried: it names source links that do not
    // exist in a target org.
    assert!(asset.get("observed_fields").is_none());
    assert!(asset.get("field_sources").is_none());

    let categories = sections["kb_categories"].as_array().unwrap();
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0]["client_visible"], true);
    let articles = sections["kb_articles"].as_array().unwrap();
    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0]["id"], json!(s.kb_article.to_string()));
    assert_eq!(
        articles[0]["client_visible"], true,
        "portal visibility moves"
    );
    assert_eq!(articles[0]["status"], "published");
    assert_eq!(articles[0]["content_jsonb"]["schema_ver"], 1);

    let documents = sections["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document["id"], json!(s.document.to_string()));
    assert_eq!(document["current_version"], 2);
    let versions = document["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["version"], 1);
    assert_eq!(versions[1]["body"], "final");
    let links = sections["document_links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["to_kind"], "client");
    assert_eq!(links[0]["to_id"], json!(s.client.to_string()));

    let tags = sections["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0]["name"], "vip");
    let ticket_tags = sections["ticket_tags"].as_array().unwrap();
    assert_eq!(ticket_tags.len(), 1);
    assert_eq!(ticket_tags[0]["tag_id"], json!(s.tag.to_string()));
    assert_eq!(ticket_tags[0]["ticket_id"], json!(s.tickets[0].to_string()));

    let attachments = sections["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1);
    let attachment = &attachments[0];
    assert_eq!(attachment["id"], json!(s.attachment.to_string()));
    assert_eq!(attachment["filename"], "screenshot.png");
    assert_eq!(attachment["sha256"], ATTACHMENT_SHA);
    // Metadata only: the object key (and therefore the bytes' location) is not
    // in the bundle; an import re-derives it from the content address.
    assert!(attachment.get("storage_key").is_none());

    // Assets' custom-field values ride the flat section (entity = asset).
    let values = sections["custom_field_values"].as_array().unwrap();
    assert!(values.iter().all(|v| v["entity"] == "ticket"));
}

#[tokio::test]
async fn board_operational_section_adds_only_the_manifest_remainder() {
    let db = test_db().await;
    let s = seed_g13_org(&db, 192).await;

    // Give the default board/status the extra state the manifest cannot hold.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        sqlx::query(
            "UPDATE boards SET description = 'Front door', inbound_address = 'help@acme.test'
             WHERE org_id = $1 AND id = $2",
        )
        .bind(s.org)
        .bind(s.board)
        .execute(&mut *tx)
        .await
        .expect("board extras");
        sqlx::query(
            "UPDATE board_statuses SET wip_limit = 5, client_visible = true
             WHERE org_id = $1 AND board_id = $2 AND is_entry",
        )
        .bind(s.org)
        .bind(s.board)
        .execute(&mut *tx)
        .await
        .expect("status extras");
        tx.commit().await.expect("commit");
    }

    let bundle = load_bundle(&db, s.org, stamp(0)).await;
    let sections = &bundle["sections"];
    let op = &sections["board_operational"];

    let manifest_boards = sections["customization"]["boards"].as_array().unwrap();
    let detail_boards = op["boards"].as_array().unwrap();
    assert_eq!(
        detail_boards.len(),
        manifest_boards.len(),
        "one remainder row per board, never a second board"
    );
    assert_eq!(detail_boards[0]["board_id"], manifest_boards[0]["id"]);
    assert_eq!(detail_boards[0]["description"], "Front door");
    assert_eq!(detail_boards[0]["inbound_address"], "help@acme.test");
    // The remainder carries no field the manifest already owns.
    assert!(detail_boards[0].get("name").is_none());

    let manifest_statuses = manifest_boards[0]["statuses"].as_array().unwrap();
    let detail_statuses = op["statuses"].as_array().unwrap();
    assert_eq!(detail_statuses.len(), manifest_statuses.len());
    assert_eq!(detail_statuses[0]["status_id"], manifest_statuses[0]["id"]);
    assert_eq!(detail_statuses[0]["wip_limit"], 5);
    assert_eq!(detail_statuses[0]["client_visible"], true);
    assert!(detail_statuses[0].get("category").is_none());

    // Priorities are a closed enum, not org data: the only place a priority
    // travels is the ticket's own value.
    assert_eq!(sections["tickets"][0]["priority"], "p3");
    assert!(sections.get("priorities").is_none());
}

#[tokio::test]
async fn audit_stream_and_plugin_connector_state_carry_no_secrets() {
    let db = test_db().await;
    let s = seed_g13_org(&db, 193).await;
    let bundle = load_bundle(&db, s.org, stamp(0)).await;
    let sections = &bundle["sections"];

    let audits = sections["audit_events"].as_array().unwrap();
    assert!(audits
        .iter()
        .any(|a| a["action"] == "ticket.status_changed"));
    let ticket_history = audits
        .iter()
        .find(|a| a["action"] == "ticket.status_changed")
        .unwrap();
    assert_eq!(ticket_history["entity_type"], "ticket");
    assert_eq!(ticket_history["actor_kind"], "human");
    assert_eq!(ticket_history["detail"]["to"], "resolved");

    let installs = sections["plugin_installs"].as_array().unwrap();
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0]["id"], json!(s.install.to_string()));
    assert_eq!(installs[0]["status"], "enabled");
    assert_eq!(installs[0]["settings"]["sync_interval_minutes"], 15);
    assert_eq!(installs[0]["pending_version"], "1.5.0");

    let connections = sections["connector_connections"].as_array().unwrap();
    assert_eq!(connections.len(), 1);
    let connection = &connections[0];
    assert_eq!(connection["id"], json!(s.connection.to_string()));
    assert_eq!(connection["label"], "Acme RMM");
    assert_eq!(
        connection["settings"]["base_url"],
        "https://rmm.example.test"
    );
    assert_eq!(
        connection["has_sealed_credential"], true,
        "the bundle says a credential EXISTS so an import demands re-entry"
    );
    let bindings = connection["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["external_id"], "C0123");

    let defaults = sections["capability_defaults"].as_array().unwrap();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0]["capability"], "remote_access");
    assert_eq!(
        defaults[0]["connection_id"],
        json!(s.connection.to_string())
    );

    let refs = sections["external_refs"].as_array().unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["record_id"], json!(s.asset.to_string()));
    assert_eq!(refs[0]["external_id"], "dev-42");

    let links = sections["record_links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["from_kind"], "ticket");
    assert_eq!(links[0]["to_kind"], "configuration");

    // Secret discipline across the WHOLE serialized bundle.
    let serialized = serde_json::to_string(&bundle).unwrap();
    assert!(
        !serialized.contains(std::str::from_utf8(CONNECTOR_SEALED_SECRET).unwrap()),
        "sealed connector credential leaked"
    );
    assert!(
        !serialized.contains("secrets_encrypted"),
        "the sealed column's name is a claim the bundle must not make"
    );
    assert!(
        !serialized.contains(ATTACHMENT_STORAGE_KEY),
        "an object key points at a machine the target cannot reach"
    );
    assert!(!serialized.contains("secret_record_id"));
}

#[tokio::test]
async fn g13_sections_are_deterministic_and_round_trip() {
    let db = test_db().await;
    let s = seed_g13_org(&db, 189).await;

    let first = load_bundle(&db, s.org, stamp(0)).await;
    let second = load_bundle(&db, s.org, stamp(45)).await;
    assert_eq!(pinned(&first), pinned(&second));

    // Every section is present, and the set is stable (serde_json's map is
    // key-sorted, so the comparison is over the sorted set, not the struct
    // field order — the STRUCT's order is what fixes the bytes on disk).
    let mut keys: Vec<&str> = first["sections"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = [
        "customization",
        "org",
        "clients",
        "sites",
        "contacts",
        "tickets",
        "time_entries",
        "custom_field_values",
        "contracts",
        "invoice_runs",
        "invoices",
        "credit_notes",
        "assets",
        "kb_categories",
        "kb_articles",
        "board_operational",
        "audit_events",
        "plugin_installs",
        "connector_connections",
        "capability_defaults",
        "external_refs",
        "record_links",
        "documents",
        "document_links",
        "tags",
        "ticket_tags",
        "attachments",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected.to_vec());

    // A typed round trip: what a consumer deserializes is what we wrote.
    let typed: core_domain::entities::export::ExportBundle =
        serde_json::from_value(first.clone()).expect("bundle deserializes");
    assert_eq!(typed.bundle_version, 1);
    assert_eq!(
        serde_json::to_value(&typed).unwrap(),
        first,
        "serialization is lossless in both directions"
    );
}
