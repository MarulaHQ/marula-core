//! Customization-manifest loader coverage (the private build tracker U5, policy §5).
//!
//! Purpose: prove the U5 export contract at the query layer —
//! 1. FULL-coverage org: every section populated and round-tripped
//!    faithfully (categories, configs, filters, ladders, attachments).
//! 2. Determinism golden stability: two consecutive dumps of an unchanged
//!    org are byte-identical apart from `generated_at`.
//! 3. Canonical ordering invariants (semantic keys, ids only tiebreaking).
//! 4. EMPTY org: sections present and empty; the defaults-first baseline
//!    (default board + category-covered statuses, default SLA policy)
//!    still documented.
//! 5. PARTIAL org: populated sections round-trip, the rest stay empty.
//! 6. Secret discipline: sealed webhook ciphertext and API-key hash
//!    material NEVER appear in the manifest, by construction.
//!
//! Org rows use fixed UUIDs 170–179 (lane convention) so parallel suites
//! sharing one database stay collision-free.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use core_db::custom_fields::FieldDefRow;
use core_db::sla::LadderStepInput;
use core_domain::status::TicketStatusCategory;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use std::str::FromStr;
use uuid::Uuid;

/// Org+user scoped transaction: user-scope settings rows are RLS-gated on
/// BOTH `app.org_id` and `app.user_id`, so seeding them needs the full GUC
/// pair (the shared `org_tx` helper only binds the org).
async fn org_user_tx(db: &TestDb, org: Uuid, user: Uuid) -> Transaction<'_, Postgres> {
    let mut tx = db.app.begin().await.expect("begin");
    core_db::set_org_and_user(&mut tx, org, user)
        .await
        .expect("set org+user scope");
    tx
}

/// Distinctive marker material that must NEVER leak into a manifest.
const SEALED_WEBHOOK_SECRET: &[u8] = b"u5-sealed-webhook-secret-DO-NOT-LEAK";
const API_KEY_HASH: &str = "u5-hmac-key-hash-DO-NOT-LEAK-0123456789abcdef";

/// Deterministic wall-clock stamps for the two consecutive dumps.
fn stamp(secs: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap() + chrono::Duration::seconds(secs)
}

struct Base {
    org: Uuid,
    board: Uuid,
    user: Uuid,
    policy_id: Uuid,
}

/// Org + default roles + default board + default SLA policy, mirroring
/// org-creation seeding (defaults-first doctrine).
async fn seed_base(db: &TestDb, n: u64) -> Base {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    core_db::orgs::insert_org(
        &mut *tx,
        org,
        "Manifest Co",
        &format!("manifest-{n}"),
        "MFT",
    )
    .await
    .expect("org");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    let board = core_db::orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board");
    let policy_id = core_db::orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla policy");
    // Standard taxonomy baseline (Wave 11 lane 3): mirrors the shared
    // org-creation seeding so the manifest section documents itself.
    core_db::orgs::seed_default_taxonomy(&mut tx, org)
        .await
        .expect("taxonomy");
    let user = fixed_uuid(n + 10);
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("owner-{n}@manifest.test"),
        "Manifest Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");
    Base {
        org,
        board,
        user,
        policy_id,
    }
}

async fn load_manifest(db: &TestDb, org: Uuid, at: chrono::DateTime<chrono::Utc>) -> Value {
    let mut tx = org_tx(&db.app, org).await;
    let manifest = core_db::manifest::load_customization_manifest(&mut tx, org, at)
        .await
        .expect("manifest loads");
    tx.commit().await.expect("commit");
    serde_json::to_value(&manifest).expect("serialize")
}

/// Serializes a manifest JSON with `generated_at` pinned, so two dumps
/// compare byte-for-byte on everything the determinism contract covers.
fn pinned(m: &Value) -> String {
    let mut copy = m.clone();
    copy["generated_at"] = json!("2026-01-15T12:00:00Z");
    serde_json::to_string(&copy).expect("re-serialize")
}

// --- the full-coverage org ------------------------------------------------------

/// Seeds EVERY section with deliberately NON-canonical insert order so the
/// ordering invariants are actually exercised. `n` is the org slot — each
/// test owns its own org id (170–179 lane convention) because tests in one
/// binary run in parallel against the shared database.
async fn seed_full_org(db: &TestDb, n: u64) -> Base {
    let base = seed_base(db, n).await;
    let mut tx = org_user_tx(db, base.org, base.user).await;

    // custom fields: ticket entity names out of order, plus one client
    // field and one ARCHIVED ticket field.
    for (name, entity, kind, config, sort) in [
        (
            "Severity",
            "ticket",
            "select",
            json!({"options": ["S1", "S2"]}),
            5,
        ),
        (
            "Escalation Tier",
            "ticket",
            "number",
            json!({"min": 1, "max": 5}),
            2,
        ),
        ("Account Code", "client", "text", json!({}), 0),
    ] {
        core_db::custom_fields::insert_def(
            &mut tx,
            &FieldDefRow {
                id: Uuid::now_v7(),
                org_id: base.org,
                board_id: None,
                entity: entity.into(),
                name: name.into(),
                kind: kind.into(),
                config,
                required: false,
                sort_order: sort,
            },
        )
        .await
        .expect("field def");
    }
    let archived_id = Uuid::now_v7();
    core_db::custom_fields::insert_def(
        &mut tx,
        &FieldDefRow {
            id: archived_id,
            org_id: base.org,
            board_id: None,
            entity: "ticket".into(),
            name: "Legacy Board Ref".into(),
            kind: "text".into(),
            config: json!({}),
            required: false,
            sort_order: 9,
        },
    )
    .await
    .expect("archived-able def");
    sqlx::query("UPDATE custom_field_defs SET archived_at = now() WHERE id = $1")
        .bind(archived_id)
        .execute(&mut *tx)
        .await
        .expect("archive");

    // boards: one extra board with explicit transitions; one custom status
    // on the default board whose sort_order slots BETWEEN seeded defaults.
    let net_board = Uuid::now_v7();
    core_db::boards::create_board(
        &mut *tx,
        base.org,
        net_board,
        "Network Ops",
        Some("network queue"),
        false,
    )
    .await
    .expect("extra board");
    let escalated = core_db::boards::add_status(
        &mut tx,
        base.org,
        Uuid::now_v7(),
        base.board,
        "Escalated",
        TicketStatusCategory::Waiting,
        15,
        false,
        None,
    )
    .await
    .expect("custom status");
    let _ = escalated.id;
    // Two custom statuses on the extra board, then an explicit adjacency
    // (the seeded default board keeps EMPTY transitions = category floor).
    let net_new = core_db::boards::add_status(
        &mut tx,
        base.org,
        Uuid::now_v7(),
        net_board,
        "Triage",
        TicketStatusCategory::Open,
        0,
        true,
        None,
    )
    .await
    .expect("net status");
    let net_done = core_db::boards::add_status(
        &mut tx,
        base.org,
        Uuid::now_v7(),
        net_board,
        "Done",
        TicketStatusCategory::Closed,
        10,
        false,
        None,
    )
    .await
    .expect("net status");
    core_db::boards::replace_transitions(
        &mut tx,
        base.org,
        net_board,
        &[
            core_domain::workflow::TransitionEdge {
                from_status_id: net_new.id,
                to_status_id: net_done.id,
            },
            core_domain::workflow::TransitionEdge {
                from_status_id: net_done.id,
                to_status_id: net_new.id,
            },
        ],
    )
    .await
    .expect("transitions");

    // automations: names out of order; one org-wide (board NULL), one
    // board-bound and disabled.
    core_db::automations::insert(
        &mut *tx,
        Uuid::now_v7(),
        base.org,
        None,
        core_domain::automations::AutomationSubjectKind::Ticket,
        "Auto Acknowledge",
        "ticket.created",
        json!([{"field": "priority", "equals": "p1"}]),
        json!([{"type": "assign_round_robin"}]),
    )
    .await
    .expect("rule 1");
    core_db::automations::insert(
        &mut *tx,
        Uuid::now_v7(),
        base.org,
        Some(net_board),
        core_domain::automations::AutomationSubjectKind::Ticket,
        "Close Stale",
        "ticket.updated",
        json!([{"field": "idle_days", "gt": 30}]),
        json!([{"type": "set_status", "status": "Done"}]),
    )
    .await
    .expect("rule 2");

    // webhook with a SEALED secret that must never surface.
    core_db::webhooks::insert_webhook(
        &mut *tx,
        Uuid::now_v7(),
        base.org,
        "https://hooks.example.test/marula",
        SEALED_WEBHOOK_SECRET,
        &["ticket.created".to_string(), "ticket.closed".to_string()],
        base.user,
        None,
    )
    .await
    .expect("webhook");

    // api key: hash + scopes; the hash must never surface. key_hash is
    // GLOBALLY unique, so each org slot carries its own marker.
    let org_hash = format!("{API_KEY_HASH}-{n}");
    core_db::webhooks::insert_api_key(
        &mut *tx,
        Uuid::now_v7(),
        base.org,
        "Provisioning Bot",
        &org_hash,
        &format!("mrl_u5key{n:02}"),
        &["tickets:read".to_string(), "tickets:write".to_string()],
        base.user,
    )
    .await
    .expect("api key");

    // org-scope settings overrides, keys out of order; plus a USER-scope
    // row that must NOT be exported (per-user preference, not org data).
    core_db::settings::upsert_org_setting(
        &mut tx,
        base.org,
        "portal.branding_color",
        &json!("#0a7ea4"),
        base.user,
    )
    .await
    .expect("setting 1");
    core_db::settings::upsert_org_setting(
        &mut tx,
        base.org,
        "tickets.default_priority",
        &json!("p3"),
        base.user,
    )
    .await
    .expect("setting 2");
    core_db::settings::upsert_user_setting(
        &mut tx,
        base.org,
        base.user,
        "ui.theme",
        &json!("dark"),
    )
    .await
    .expect("user setting");

    // SLA: a named calendar attached to the default policy + a ladder with
    // steps inserted OUT of index order, one of them disabled.
    let cal = core_db::sla::insert_calendar(
        &mut *tx,
        &core_domain::entities::SlaCalendar {
            id: Uuid::now_v7(),
            org_id: base.org,
            name: "APAC Support".into(),
            calendar: core_domain::entities::BusinessCalendar {
                name: "APAC Support".into(),
                timezone: "Asia/Singapore".into(),
                windows: [
                    Some(("09:00".into(), "18:00".into())),
                    Some(("09:00".into(), "18:00".into())),
                    Some(("09:00".into(), "18:00".into())),
                    Some(("09:00".into(), "18:00".into())),
                    Some(("09:00".into(), "18:00".into())),
                    None,
                    None,
                ],
                holidays: vec!["2026-01-01".into()],
                maintenance: vec![],
            },
        },
    )
    .await
    .expect("calendar");
    core_db::sla::set_policy_calendar(&mut *tx, base.org, base.policy_id, Some(cal.id))
        .await
        .expect("attach calendar");
    core_db::sla::replace_ladder(
        &mut tx,
        base.org,
        base.policy_id,
        &[
            LadderStepInput {
                step_index: 1,
                trigger_kind: "breach".into(),
                at_pct: 100,
                notify_user_ids: vec![base.user],
                enabled: false,
            },
            LadderStepInput {
                step_index: 0,
                trigger_kind: "risk".into(),
                at_pct: 80,
                notify_user_ids: vec![],
                enabled: true,
            },
        ],
    )
    .await
    .expect("ladder");

    // report definitions (the private build tracker R3): two rows inserted OUT of name
    // order, one org-published and one private, so the section sort +
    // scope round-trip are exercised. Spec bodies are registry-valid.
    for (name, kind, body, viz, scope) in [
        (
            "Weekly Tickets by Status",
            "tickets",
            json!({
                "filters": [{"column": "priority", "op": "not_eq", "value": {"text": "p4"}}],
                "group_by": ["status_name"],
                "measures": [
                    {"agg": "count"},
                    {"agg": "min", "column": "created_at"}
                ]
            }),
            "bar",
            "org",
        ),
        (
            "Billable Minutes by Client",
            "time_entries",
            json!({
                "filters": [{"column": "billable", "op": "eq", "value": {"bool": true}}],
                "group_by": ["client_id"],
                "measures": [{"agg": "sum", "column": "minutes"}]
            }),
            "pie",
            "self",
        ),
    ] {
        core_db::report_defs::insert(
            &mut tx,
            Uuid::now_v7(),
            base.org,
            base.user,
            &core_db::report_defs::DefinitionFields {
                name: name.into(),
                description: None,
                kind: kind.into(),
                spec: body,
                viz: viz.into(),
                scope: scope.into(),
            },
        )
        .await
        .expect("report definition");
    }

    // ticket taxonomy (Wave 11 lane 3): custom rows in deliberately
    // NON-canonical insert order; one standard type RETIRED (flag, not
    // delete) so retired inclusion is exercised.
    let custom_type = Uuid::now_v7();
    core_db::taxonomy::insert_node(
        &mut tx,
        base.org,
        custom_type,
        core_db::taxonomy::TaxonomyTable::Type,
        None,
        "Custom Project",
        None,
        99,
    )
    .await
    .expect("custom type");
    let euc = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM ticket_categories WHERE org_id = $1 AND name = 'End-User Computing' AND parent_id IS NULL",
    )
    .bind(base.org)
    .fetch_one(&mut *tx)
    .await
    .expect("euc root");
    let routers = Uuid::now_v7();
    core_db::taxonomy::insert_node(
        &mut tx,
        base.org,
        routers,
        core_db::taxonomy::TaxonomyTable::Category,
        Some(euc),
        "Routers & Switches",
        None,
        50,
    )
    .await
    .expect("custom subcategory");
    core_db::taxonomy::retire_node(
        &mut tx,
        base.org,
        core_db::taxonomy::TaxonomyTable::Category,
        routers,
    )
    .await
    .expect("retire subcategory");
    let change_type = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM ticket_types WHERE org_id = $1 AND name = 'Change'",
    )
    .bind(base.org)
    .fetch_one(&mut *tx)
    .await
    .expect("change type");
    core_db::taxonomy::retire_node(
        &mut tx,
        base.org,
        core_db::taxonomy::TaxonomyTable::Type,
        change_type,
    )
    .await
    .expect("retire");

    tx.commit().await.expect("commit full org");
    base
}

// --- tests ------------------------------------------------------------------------

#[tokio::test]
async fn full_org_manifest_covers_every_section() {
    let db = test_db().await;
    let base = seed_full_org(&db, 170).await;
    let m = load_manifest(&db, base.org, stamp(0)).await;

    assert_eq!(m["manifest_version"], 1, "envelope version is 1");
    assert_eq!(m["org_id"], json!(base.org), "envelope names the org");

    // custom fields: 4 defs (one archived), ordered by (entity, name).
    let fields = m["sections"]["custom_fields"].as_array().unwrap();
    assert_eq!(fields.len(), 4, "3 active + 1 archived definition");
    assert_eq!(fields[0]["entity"], "client", "client entity sorts first");
    let ticket_names: Vec<&str> = fields
        .iter()
        .filter(|f| f["entity"] == "ticket")
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        ticket_names,
        vec!["Escalation Tier", "Legacy Board Ref", "Severity"],
        "ticket fields ordered by name"
    );
    assert_eq!(
        fields
            .iter()
            .find(|f| f["name"] == "Escalation Tier")
            .unwrap()["config"],
        json!({"min": 1, "max": 5}),
        "config document passes through verbatim"
    );
    assert!(
        !fields
            .iter()
            .find(|f| f["name"] == "Legacy Board Ref")
            .unwrap()["archived_at"]
            .is_null(),
        "archived definitions export with their archived_at"
    );

    // boards: two, name-ordered; default board has the custom status slotted
    // by sort_order and EMPTY transitions; Network Ops has explicit edges.
    let boards = m["sections"]["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 2);
    assert_eq!(
        boards[0]["name"], "Network Ops",
        "boards ordered by name (ids never influence order here)"
    );
    assert_eq!(boards[1]["name"], "Service Desk");
    assert_eq!(
        boards[1]["is_default"], true,
        "the default flag rides on the Service Desk row"
    );
    let default_statuses: Vec<&str> = boards[1]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        default_statuses,
        vec!["New", "In Progress", "Escalated", "Waiting", "Resolved"],
        "statuses ordered by (sort_order, name) with the custom one slotted in"
    );
    let escalated = boards[1]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Escalated")
        .unwrap();
    assert_eq!(
        escalated["category"], "waiting",
        "semantic-category binding survives the export"
    );
    assert!(
        boards[1]["transitions"].as_array().unwrap().is_empty(),
        "empty adjacency means the semantic-category floor applies"
    );
    let net_transitions = boards[0]["transitions"].as_array().unwrap();
    assert_eq!(net_transitions.len(), 2, "explicit adjacency exported");
    let status_ids: Vec<Uuid> = boards[0]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Uuid::from_str(s["id"].as_str().unwrap()).unwrap())
        .collect();
    for edge in net_transitions {
        let from = Uuid::from_str(edge["from_status_id"].as_str().unwrap()).unwrap();
        let to = Uuid::from_str(edge["to_status_id"].as_str().unwrap()).unwrap();
        assert!(
            status_ids.contains(&from) && status_ids.contains(&to),
            "transition endpoints resolve within the SAME board"
        );
    }

    // automations: name-ordered; org-wide rule keeps board_id null.
    let rules = m["sections"]["automation_rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["name"], "Auto Acknowledge");
    assert_eq!(rules[1]["name"], "Close Stale");
    assert_eq!(rules[0]["board_id"], Value::Null);
    assert_eq!(
        rules[1]["board_id"],
        json!(m["sections"]["boards"][0]["id"])
    );
    assert_eq!(
        rules[0]["conditions"],
        json!([{"field": "priority", "equals": "p1"}]),
        "condition documents pass through verbatim"
    );

    // webhooks: filter array exported, no secret fields anywhere.
    let hooks = m["sections"]["webhooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["url"], "https://hooks.example.test/marula");
    assert_eq!(
        hooks[0]["event_types"],
        json!(["ticket.created", "ticket.closed"]),
        "event-type filters export verbatim"
    );

    // api keys: metadata + scopes only.
    let keys = m["sections"]["api_keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["name"], "Provisioning Bot");
    assert_eq!(
        keys[0]["prefix"], "mrl_u5key170",
        "per-slot display prefix round-trips"
    );
    assert_eq!(keys[0]["scopes"], json!(["tickets:read", "tickets:write"]));

    // settings: org-scope rows only, key-ordered.
    let settings = m["sections"]["settings"].as_array().unwrap();
    assert_eq!(
        settings.len(),
        2,
        "user-scope rows are NOT org customization"
    );
    assert_eq!(settings[0]["key"], "portal.branding_color");
    assert_eq!(settings[1]["key"], "tickets.default_priority");
    assert_eq!(settings[0]["value"], json!("#0a7ea4"));

    // sla: calendar attached to the default policy; ladder step_index-ordered
    // (insertion order was 1 then 0) and the DISABLED step still exported.
    let sla = &m["sections"]["sla"];
    let calendars = sla["calendars"].as_array().unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0]["name"], "APAC Support");
    assert_eq!(calendars[0]["calendar"]["timezone"], "Asia/Singapore");
    assert_eq!(
        calendars[0]["calendar"]["windows"][0],
        json!(["09:00", "18:00"]),
        "business windows round-trip through the domain type"
    );
    let policies = sla["policies"].as_array().unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0]["is_default"], true);
    assert_eq!(
        policies[0]["calendar_id"], calendars[0]["id"],
        "policy→calendar attachment exported"
    );
    assert_eq!(
        policies[0]["targets"]["p1"]["first_response_mins"], 15,
        "targets document passes through verbatim"
    );
    let steps = policies[0]["escalation_steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["step_index"], 0, "ladder ordered by step_index");
    assert_eq!(steps[1]["step_index"], 1);
    assert_eq!(steps[0]["enabled"], true);
    assert_eq!(steps[1]["enabled"], false, "disabled steps still export");

    // report definitions: name-ordered, slugs and spec body verbatim.
    let defs = m["sections"]["report_definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 2);
    assert_eq!(
        defs[0]["name"], "Billable Minutes by Client",
        "definitions ordered by name regardless of insert order"
    );
    assert_eq!(defs[0]["kind"], "time_entries");
    assert_eq!(defs[0]["viz"], "pie");
    assert_eq!(defs[0]["scope"], "self");
    assert_eq!(
        defs[0]["spec"]["measures"][0],
        json!({"agg": "sum", "column": "minutes"}),
        "spec body passes through verbatim"
    );
    assert_eq!(defs[1]["name"], "Weekly Tickets by Status");
    assert_eq!(defs[1]["scope"], "org");
    assert_eq!(defs[1]["spec"]["group_by"], json!(["status_name"]));

    // ticket taxonomy (Wave 11 lane 3): seeded standard set + custom rows,
    // retired rows INCLUDED, deterministic order (types by name).
    let taxonomy = &m["sections"]["taxonomy"];
    let types = taxonomy["types"].as_array().unwrap();
    assert_eq!(
        types.len(),
        5,
        "4 standard types + 1 custom, retired rows included"
    );
    let type_names: Vec<&str> = types.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        type_names,
        vec![
            "Change",
            "Custom Project",
            "Incident",
            "Problem",
            "Service Request"
        ],
        "types ordered by name; the RETIRED standard type still exports"
    );
    assert!(
        types
            .iter()
            .any(|t| t["name"] == "Change" && t["is_retired"] == true),
        "retirement is a flag, not a deletion"
    );
    let categories = taxonomy["categories"].as_array().unwrap();
    assert_eq!(
        categories.len(),
        38,
        "8 standard roots + 29 standard subcategories + 1 custom subcategory"
    );
    assert!(
        categories
            .iter()
            .any(|c| c["name"] == "Routers & Switches" && c["is_retired"] == true),
        "the retired subcategory exports with its reference intact"
    );
}

#[tokio::test]
async fn manifest_dump_is_byte_stable_across_consecutive_dumps() {
    let db = test_db().await;
    let base = seed_full_org(&db, 171).await;

    // Two consecutive dumps at DIFFERENT stamps of the same unchanged org.
    let first = load_manifest(&db, base.org, stamp(0)).await;
    let second = load_manifest(&db, base.org, stamp(3600)).await;
    assert_ne!(
        first["generated_at"], second["generated_at"],
        "the dumps genuinely carry different stamps"
    );

    // Byte-identical modulo generated_at: the golden stability contract.
    assert_eq!(
        pinned(&first),
        pinned(&second),
        "two consecutive dumps must be byte-identical apart from generated_at"
    );
}

#[tokio::test]
async fn manifest_ordering_invariants_hold() {
    let db = test_db().await;
    let base = seed_full_org(&db, 175).await;
    let m = load_manifest(&db, base.org, stamp(0)).await;
    let sections = &m["sections"];

    let sorted_by = |arr: &[Value], key: &str| -> bool {
        arr.windows(2)
            .all(|w| w[0][key].as_str() <= w[1][key].as_str())
    };
    assert!(sorted_by(
        &sections["custom_fields"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["boards"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["automation_rules"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["webhooks"].as_array().unwrap().clone(),
        "url"
    ));
    assert!(sorted_by(
        &sections["api_keys"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["settings"].as_array().unwrap().clone(),
        "key"
    ));
    assert!(sorted_by(
        &sections["sla"]["calendars"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["sla"]["policies"].as_array().unwrap().clone(),
        "name"
    ));
    assert!(sorted_by(
        &sections["report_definitions"].as_array().unwrap().clone(),
        "name"
    ));
}

#[tokio::test]
async fn empty_org_manifest_sections_are_empty() {
    let db = test_db().await;
    let base = seed_base(&db, 173).await;
    let m = load_manifest(&db, base.org, stamp(0)).await;

    let sections = &m["sections"];
    for empty_section in [
        "custom_fields",
        "automation_rules",
        "webhooks",
        "api_keys",
        "settings",
        "report_definitions",
    ] {
        assert_eq!(
            sections[empty_section],
            json!([]),
            "{empty_section} must be an empty ARRAY, never absent"
        );
    }

    // Defaults-first: the baseline still documents itself.
    let boards = sections["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 1, "the default board is part of the org");
    assert_eq!(boards[0]["is_default"], true);
    assert_eq!(
        boards[0]["transitions"],
        json!([]),
        "category floor applies"
    );
    let categories: Vec<&str> = boards[0]["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["category"].as_str().unwrap())
        .collect();
    assert_eq!(
        categories,
        vec!["open", "in_progress", "waiting", "closed"],
        "one status per semantic category is guaranteed"
    );
    assert!(
        boards[0]["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["is_entry"] == true),
        "the entry status is exported"
    );

    // The seeded default policy (embedded calendar, no ladder, no
    // attached named calendar) round-trips.
    assert_eq!(sections["sla"]["calendars"], json!([]));
    let policies = sections["sla"]["policies"].as_array().unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0]["is_default"], true);
    assert_eq!(policies[0]["calendar_id"], Value::Null);
    assert_eq!(
        policies[0]["calendar"]["name"], "9to5 M-F",
        "the embedded fallback calendar exports"
    );
    assert_eq!(policies[0]["escalation_steps"], json!([]));

    // Taxonomy (Wave 11 lane 3): the shared standard seed exports for a
    // fresh org — the defaults-first classification baseline.
    let taxonomy = &sections["taxonomy"];
    assert_eq!(taxonomy["types"].as_array().unwrap().len(), 4);
    assert_eq!(
        taxonomy["categories"].as_array().unwrap().len(),
        37,
        "8 roots + 29 subcategories, the standard accepted set"
    );
}

#[tokio::test]
async fn partial_org_manifest_rounds_populated_sections() {
    let db = test_db().await;
    let base = seed_base(&db, 174).await;
    let mut tx = org_tx(&db.app, base.org).await;
    core_db::webhooks::insert_webhook(
        &mut *tx,
        Uuid::now_v7(),
        base.org,
        "https://hooks.partial.test/only",
        b"partial-org-sealed-secret",
        &[],
        base.user,
        None,
    )
    .await
    .expect("webhook");
    core_db::settings::upsert_org_setting(
        &mut tx,
        base.org,
        "portal.enabled",
        &json!(true),
        base.user,
    )
    .await
    .expect("setting");
    tx.commit().await.expect("commit");

    let m = load_manifest(&db, base.org, stamp(0)).await;
    let sections = &m["sections"];

    // Exactly the two populated sections carry rows.
    assert_eq!(sections["webhooks"].as_array().unwrap().len(), 1);
    assert_eq!(sections["settings"].as_array().unwrap().len(), 1);
    assert_eq!(
        sections["webhooks"][0]["event_types"],
        json!([]),
        "wildcard filter exports as empty array"
    );
    for untouched in [
        "custom_fields",
        "automation_rules",
        "api_keys",
        "report_definitions",
    ] {
        assert_eq!(sections[untouched], json!([]));
    }
    // Boards/sla stay at the defaults-first baseline, not empty.
    assert_eq!(sections["boards"].as_array().unwrap().len(), 1);
    assert_eq!(sections["sla"]["policies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn manifest_never_contains_secret_material() {
    let db = test_db().await;
    let base = seed_full_org(&db, 172).await;
    let m = load_manifest(&db, base.org, stamp(0)).await;
    let serialized = serde_json::to_string(&m).unwrap();

    assert!(
        !serialized.contains(std::str::from_utf8(SEALED_WEBHOOK_SECRET).unwrap()),
        "sealed webhook ciphertext must never appear in the manifest"
    );
    assert!(
        !serialized.contains(API_KEY_HASH),
        "api-key hash material must never appear in the manifest"
    );
    // The COLUMNS themselves are structurally absent — no field anywhere in
    // the document could carry them even if a future edit reintroduced one.
    assert!(!serialized.contains("secret_encrypted"));
    assert!(!serialized.contains("key_hash"));

    // Exact key sets: webhook and api-key section objects expose exactly the
    // approved metadata keys and nothing else.
    let hook = &m["sections"]["webhooks"][0];
    let hook_keys: Vec<&str> = hook
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        hook_keys,
        vec!["created_by", "enabled", "event_types", "id", "url"],
        "webhook manifest objects expose exactly the non-secret metadata"
    );
    let key = &m["sections"]["api_keys"][0];
    let key_keys: Vec<&str> = key
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        key_keys,
        vec![
            "created_at",
            "created_by",
            "id",
            "last_used_at",
            "name",
            "prefix",
            "revoked_at",
            "scopes"
        ],
        "api-key manifest objects expose exactly the non-secret metadata"
    );
}

/// Cross-org isolation sanity: the loader exports exactly the requested
/// org's rows even when another org shares the database.
#[tokio::test]
async fn manifest_exports_only_the_requested_org() {
    let db = test_db().await;
    let full = seed_full_org(&db, 176).await;
    let other = seed_base(&db, 177).await;

    let m = load_manifest(&db, other.org, stamp(0)).await;
    assert_eq!(m["org_id"], json!(other.org));
    assert_eq!(m["sections"]["custom_fields"], json!([]));
    assert_eq!(m["sections"]["webhooks"], json!([]));
    // The populated neighbor exists alongside; nothing bled over.
    assert_eq!(full.org, fixed_uuid(176));
}
