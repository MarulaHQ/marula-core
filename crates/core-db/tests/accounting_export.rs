//! The accounting export ledger (claims-gaps G1–G3, migration 0187).
//!
//! Purpose: prove the four rules that make an export safe to retry, because
//! each one is a money-correctness rule rather than formatting:
//!
//!   * a claim is exclusive inside its window and re-claimable outside it, so
//!     two attempts never push the same invoice at once;
//!   * a successful run is never re-claimed, and its receipt is readable by the
//!     document that needs the vendor id (a credit note needs its invoice's);
//!   * a held run is re-claimable and clears its reason, which is what "add the
//!     mapping and export it without a re-issue" means in the schema;
//!   * a second receipt carrying a DIFFERENT vendor id is refused, because two
//!     vendor documents for one Marula document is the double-count this table
//!     exists to prevent.
//!
//! Non-purpose: the HTTP shape (route tests), the vendor wire shape (each
//! adapter's own suite), and the tax resolution rules (the route's).
//!
//! Slot discipline: fixed-UUID slot 96000+ for the first fixture, 96100+ for the
//! second, so a parallel suite sharing one database cannot collide.

mod common;

use common::{db_now, fixed_uuid, org_tx, test_db, TestDb};
use core_db::accounting_export::{
    claim_run, delete_tax_mapping, find_tax_mapping, finish_succeeded, get_receipt, get_run,
    list_runs, list_tax_mappings, mark_held, upsert_tax_mapping, ClaimOutcome, ClaimSpec,
    ExportState, ExportVendor, NewReceipt, RecordKind, RunFilters,
};
use core_db::{clients, orgs, plugins, users};
use core_domain::connectors::ConnectionStatus;
use core_domain::entities::{
    PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2, PluginRuntimeKind,
    PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    org: Uuid,
    user: Uuid,
    connection: Uuid,
    credit_note: Uuid,
    other_invoice: Uuid,
    reclaim_invoice: Uuid,
}

/// An org, a user, a client, one installed QuickBooks connector and one
/// connection made from it.
async fn seed_fixture(db: &TestDb, base: u64, slug: &str) -> Fixture {
    let org = fixed_uuid(base);
    let user = fixed_uuid(base + 1);
    let client = fixed_uuid(base + 2);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Accounting Co", slug, "ACC")
        .await
        .expect("org");
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("{slug}@example.test"),
        "Bookkeeper",
        "not-a-real-hash",
        "UTC",
    )
    .await
    .expect("user");
    clients::create_client(&mut *tx, org, client, "Acme", None)
        .await
        .expect("client");
    tx.commit().await.expect("commit");

    let manifest = PluginManifestV2 {
        name: "QuickBooks Online".into(),
        version: "1.0.0".into(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "accounting connector".into(),
        capabilities: Vec::new(),
        product_kinds: vec!["accounting".into()],
        connector_surfaces: None,
        settings_schema: None,
        artifact_kind: Some(PluginRuntimeKind::HttpConnector),
        connector_profile: None,
        surface_profile: None,
    };
    // The catalog slug is unique across the whole database (it names a
    // marketplace entry, not an install), so each fixture's connector gets its
    // own slug rather than colliding with a sibling test.
    let catalog_slug = format!("qbo-{slug}");
    let mut tx = org_tx(&db.app, org).await;
    let entry = PluginCatalogEntry {
        id: fixed_uuid(base + 3),
        slug: catalog_slug.clone(),
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
        id: fixed_uuid(base + 4),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog");
    let install = fixed_uuid(base + 5);
    plugins::create_install(
        &mut tx,
        &PluginInstall {
            id: install,
            org_id: org,
            catalog_id: entry.id,
            slug: catalog_slug.clone(),
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
    let connection = fixed_uuid(base + 6);
    sqlx::query(
        "INSERT INTO connector_connections
           (id, org_id, installation_id, scope_client_id, label, settings, status)
         VALUES ($1, $2, $3, NULL, $4, $5, $6)",
    )
    .bind(connection)
    .bind(org)
    .bind(install)
    .bind("QuickBooks — Acme")
    .bind(json!({ "realm_id": "987654321" }))
    .bind(ConnectionStatus::Connected.as_str())
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");

    Fixture {
        org,
        user,
        connection,
        credit_note: fixed_uuid(base + 7),
        other_invoice: fixed_uuid(base + 8),
        reclaim_invoice: fixed_uuid(base + 9),
    }
}

fn spec<'a>(fx: &Fixture, kind: RecordKind, id: Uuid, tax_code: Option<&'a str>) -> ClaimSpec<'a> {
    ClaimSpec {
        connection_id: fx.connection,
        vendor: ExportVendor::Qbo,
        record_kind: kind,
        record_id: id,
        tax_code,
        requested_by: fx.user,
    }
}

async fn claim(db: &TestDb, fx: &Fixture, spec: &ClaimSpec<'_>, window: i64) -> ClaimOutcome {
    let mut tx = org_tx(&db.app, fx.org).await;
    let outcome = claim_run(&mut tx, fx.org, spec, window)
        .await
        .expect("claim");
    tx.commit().await.expect("commit");
    outcome
}

/// Claim → succeed → idempotent re-claim, and the receipt the credit-note path
/// reads.
#[tokio::test]
async fn a_succeeded_export_is_never_sent_twice() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 96_000, "accounting-a").await;
    let invoice = fixed_uuid(96_100);

    let claimed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, invoice, None),
        300,
    )
    .await;
    let run = match claimed {
        ClaimOutcome::Claimed(run) => *run,
        other => panic!("first claim must own the run, got {other:?}"),
    };
    assert_eq!(run.state, ExportState::InFlight);
    assert_eq!(run.attempts, 1);
    assert_eq!(
        run.connection_label, "QuickBooks — Acme",
        "the report names the company, not the uuid"
    );

    // A second attempt inside the window must not race the first.
    let blocked = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, invoice, None),
        300,
    )
    .await;
    assert!(
        matches!(blocked, ClaimOutcome::InFlight(_)),
        "a live claim must not be re-claimed, got {blocked:?}"
    );

    let mut tx = org_tx(&db.app, fx.org).await;
    let (settled, receipt) = finish_succeeded(
        &mut tx,
        fx.org,
        run.id,
        &NewReceipt {
            connection_id: fx.connection,
            record_kind: RecordKind::Invoice,
            record_id: invoice,
            external_id: "184",
            external_number: Some("INV-000042"),
            created_by: Some(fx.user),
        },
    )
    .await
    .expect("settle");
    tx.commit().await.expect("commit");
    assert_eq!(settled.state, ExportState::Succeeded);
    assert_eq!(receipt.external_id, "184");

    let again = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, invoice, None),
        300,
    )
    .await;
    match again {
        ClaimOutcome::AlreadySucceeded(run, receipt) => {
            assert_eq!(run.state, ExportState::Succeeded);
            assert_eq!(
                receipt.external_id, "184",
                "the retry answers with the stored vendor id, not a new push"
            );
        }
        other => panic!("a succeeded run must never be re-claimed, got {other:?}"),
    }

    // The question a credit note asks about its parent invoice.
    let mut tx = org_tx(&db.app, fx.org).await;
    let parent = get_receipt(&mut tx, fx.org, fx.connection, RecordKind::Invoice, invoice)
        .await
        .expect("receipt");
    assert_eq!(
        parent.map(|r| r.external_id).as_deref(),
        Some("184"),
        "the vendor id a credit note allocates against"
    );
    let orphan = get_receipt(
        &mut tx,
        fx.org,
        fx.connection,
        RecordKind::Invoice,
        fx.other_invoice,
    )
    .await
    .expect("receipt");
    assert!(
        orphan.is_none(),
        "an invoice that was never exported has no receipt — the held state"
    );
    tx.commit().await.expect("commit");
}

/// Held is re-claimable and clears its reason: that is "add the mapping and the
/// same run exports, without a re-issue".
#[tokio::test]
async fn a_held_run_resumes_without_a_second_document() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 96_200, "accounting-b").await;

    let claimed = claim(
        &db,
        &fx,
        &spec(
            &fx,
            RecordKind::CreditNote,
            fx.credit_note,
            Some("US-CA-8.25"),
        ),
        300,
    )
    .await;
    let run = match claimed {
        ClaimOutcome::Claimed(run) => *run,
        other => panic!("expected a claim, got {other:?}"),
    };
    assert_eq!(
        run.tax_code.as_deref(),
        Some("US-CA-8.25"),
        "the Marula tax code is remembered on the run, so a resume needs no new body"
    );

    let mut tx = org_tx(&db.app, fx.org).await;
    let held = mark_held(
        &mut tx,
        fx.org,
        run.id,
        "parent_invoice_not_exported",
        Some("export the invoice first"),
        None,
    )
    .await
    .expect("hold");
    tx.commit().await.expect("commit");
    assert_eq!(held.state, ExportState::Held);
    assert_eq!(held.reason.as_deref(), Some("parent_invoice_not_exported"));
    assert_eq!(
        held.attempts, 1,
        "holding is not a second attempt: nothing was sent"
    );

    // A held run is a different queue from a failed one, and the report can say
    // so without parsing prose.
    let mut tx = org_tx(&db.app, fx.org).await;
    let held_only = list_runs(
        &mut tx,
        fx.org,
        &RunFilters {
            state: Some(ExportState::Held),
            ..Default::default()
        },
        50,
    )
    .await
    .expect("list");
    let failed_only = list_runs(
        &mut tx,
        fx.org,
        &RunFilters {
            state: Some(ExportState::Failed),
            ..Default::default()
        },
        50,
    )
    .await
    .expect("list");
    tx.commit().await.expect("commit");
    assert_eq!(held_only.len(), 1);
    assert!(failed_only.is_empty(), "held is not failed");

    // The resume: same run, attempts advanced, reason cleared.
    let resumed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::CreditNote, fx.credit_note, None),
        300,
    )
    .await;
    match resumed {
        ClaimOutcome::Claimed(run) => {
            assert_eq!(run.id, held.id, "the same run advanced, not a second row");
            assert_eq!(run.attempts, 2);
            assert!(
                run.reason.is_none(),
                "a resumed run must not keep the stale reason"
            );
            assert_eq!(
                run.tax_code.as_deref(),
                Some("US-CA-8.25"),
                "the remembered tax code survives a resume whose body named none"
            );
        }
        other => panic!("a held run must be re-claimable, got {other:?}"),
    }
}

/// The crash window: a claim older than the window is the only proof a process
/// died, and it is what lets a retry proceed.
#[tokio::test]
async fn a_stale_claim_is_reclaimable_and_a_fresh_one_is_not() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 96_400, "accounting-c").await;

    let claimed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, fx.reclaim_invoice, None),
        300,
    )
    .await;
    let run = match claimed {
        ClaimOutcome::Claimed(run) => *run,
        other => panic!("expected a claim, got {other:?}"),
    };

    // Age the claim past the window the way a dead process would.
    let mut tx = org_tx(&db.app, fx.org).await;
    sqlx::query(
        "UPDATE accounting_export_runs SET claimed_at = now() - interval '10 minutes'
          WHERE id = $1",
    )
    .bind(run.id)
    .execute(&mut *tx)
    .await
    .expect("age");
    tx.commit().await.expect("commit");

    let reclaimed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, fx.reclaim_invoice, None),
        300,
    )
    .await;
    match reclaimed {
        ClaimOutcome::Claimed(run) => {
            assert_eq!(run.attempts, 2);
            assert_eq!(run.state, ExportState::InFlight);
        }
        other => panic!("a stale claim must be re-claimable, got {other:?}"),
    }
}

/// Two vendor documents for one Marula document is the double-count the receipt
/// table exists to refuse, so the store refuses it by name rather than keeping
/// whichever id arrived second.
#[tokio::test]
async fn a_second_receipt_with_a_different_vendor_id_is_refused() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 96_600, "accounting-d").await;
    let invoice = fixed_uuid(96_700);

    let claimed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, invoice, None),
        300,
    )
    .await;
    let run = match claimed {
        ClaimOutcome::Claimed(run) => *run,
        other => panic!("expected a claim, got {other:?}"),
    };

    let mut tx = org_tx(&db.app, fx.org).await;
    finish_succeeded(
        &mut tx,
        fx.org,
        run.id,
        &NewReceipt {
            connection_id: fx.connection,
            record_kind: RecordKind::Invoice,
            record_id: invoice,
            external_id: "184",
            external_number: None,
            created_by: Some(fx.user),
        },
    )
    .await
    .expect("first receipt");
    let second = finish_succeeded(
        &mut tx,
        fx.org,
        run.id,
        &NewReceipt {
            connection_id: fx.connection,
            record_kind: RecordKind::Invoice,
            record_id: invoice,
            external_id: "999",
            external_number: None,
            created_by: Some(fx.user),
        },
    )
    .await;
    tx.commit().await.expect("commit");
    let err = second.expect_err("a second vendor id must be refused");
    assert!(
        err.to_string().contains("two vendor documents"),
        "the refusal must name the fault, got {err}"
    );
}

/// Mappings are per connection and rewrite in place; the export path reads
/// exactly one row for one Marula code.
#[tokio::test]
async fn tax_mappings_are_per_connection_and_rewrite_in_place() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 96_800, "accounting-e").await;

    let mut tx = org_tx(&db.app, fx.org).await;
    let first = upsert_tax_mapping(
        &mut tx,
        fx.org,
        fx.connection,
        "US-CA-8.25",
        "TAX",
        Some("Standard 8.25%"),
        Some(fx.user),
    )
    .await
    .expect("mapping");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, fx.org).await;
    let rewritten = upsert_tax_mapping(
        &mut tx,
        fx.org,
        fx.connection,
        "US-CA-8.25",
        "TAX2",
        Some("Standard 8.25% (new code)"),
        Some(fx.user),
    )
    .await
    .expect("mapping");
    tx.commit().await.expect("commit");
    assert_eq!(rewritten.id, first.id, "a rewrite is the same row");
    assert_eq!(rewritten.vendor_tax_code, "TAX2");

    let mut tx = org_tx(&db.app, fx.org).await;
    let found = find_tax_mapping(&mut tx, fx.org, fx.connection, "US-CA-8.25")
        .await
        .expect("find")
        .expect("mapping");
    assert_eq!(found.vendor_tax_code, "TAX2");
    let all = list_tax_mappings(&mut tx, fx.org, Some(fx.connection))
        .await
        .expect("list");
    assert_eq!(all.len(), 1, "one answer per Marula code, never two");
    let missing = find_tax_mapping(&mut tx, fx.org, fx.connection, "US-NY-8.875")
        .await
        .expect("find");
    assert!(
        missing.is_none(),
        "no mapping is the held case, not a default"
    );
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, fx.org).await;
    assert!(delete_tax_mapping(&mut tx, fx.org, first.id)
        .await
        .expect("delete"));
    assert!(
        !delete_tax_mapping(&mut tx, fx.org, first.id)
            .await
            .expect("delete"),
        "a second delete reports that nothing was there"
    );
    tx.commit().await.expect("commit");
}

/// RLS: another org's transaction sees none of these rows, and cannot read a
/// receipt by guessing its document id.
#[tokio::test]
async fn another_org_sees_no_runs_receipts_or_mappings() {
    let db = test_db().await;
    let fx = seed_fixture(&db, 97_000, "accounting-f").await;
    let other = fixed_uuid(97_100);
    let mut tx = org_tx(&db.app, other).await;
    orgs::insert_org(&mut *tx, other, "Other Co", "accounting-g", "OTH")
        .await
        .expect("org");
    tx.commit().await.expect("commit");

    let invoice = fixed_uuid(97_200);
    let claimed = claim(
        &db,
        &fx,
        &spec(&fx, RecordKind::Invoice, invoice, None),
        300,
    )
    .await;
    let run = match claimed {
        ClaimOutcome::Claimed(run) => *run,
        other => panic!("expected a claim, got {other:?}"),
    };
    let mut tx = org_tx(&db.app, fx.org).await;
    finish_succeeded(
        &mut tx,
        fx.org,
        run.id,
        &NewReceipt {
            connection_id: fx.connection,
            record_kind: RecordKind::Invoice,
            record_id: invoice,
            external_id: "184",
            external_number: None,
            created_by: Some(fx.user),
        },
    )
    .await
    .expect("receipt");
    upsert_tax_mapping(
        &mut tx,
        fx.org,
        fx.connection,
        "US-CA-8.25",
        "TAX",
        None,
        Some(fx.user),
    )
    .await
    .expect("mapping");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, other).await;
    let runs = list_runs(&mut tx, other, &RunFilters::default(), 50)
        .await
        .expect("list");
    assert!(runs.is_empty(), "another org must not see the run report");
    let receipt = get_receipt(&mut tx, other, fx.connection, RecordKind::Invoice, invoice)
        .await
        .expect("receipt");
    assert!(receipt.is_none(), "another org must not read the receipt");
    let mappings = list_tax_mappings(&mut tx, other, None).await.expect("list");
    assert!(mappings.is_empty());
    let looked_up = get_run(&mut tx, other, run.id).await;
    assert!(looked_up.is_err(), "another org cannot read the run by id");
    tx.commit().await.expect("commit");

    // And the owner still can — the policy filters, it does not break the read.
    let mut tx = org_tx(&db.app, fx.org).await;
    let mine = get_run(&mut tx, fx.org, run.id).await.expect("run");
    assert_eq!(mine.state, ExportState::Succeeded);
    assert!(mine.completed_at.is_some());
    assert!(mine.completed_at.unwrap() >= db_now() - chrono::Duration::minutes(5));
    tx.commit().await.expect("commit");
}
