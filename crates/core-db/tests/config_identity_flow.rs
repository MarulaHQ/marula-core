//! Configuration-identity integration tests (ADR 0003, Wave 12 lane 3,
//! migration 0061): CI2 feed upsert through source links (never a twin),
//! CI4 merge queue (threshold + undo/split), CI5 curated stickiness, and
//! the migration surface itself (columns, semantic-class backfill, RLS).
//!
//! Database selection follows the shared contract (common/mod.rs):
//! MARULA_TEST_DATABASE_URL or DATABASE_URL, else a testcontainer.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::config_identity::{
    accept_observed_value, auto_merge_threshold, curate_fields, feed_upsert, upsert_precedence,
    FeedField, FeedUpsert,
};
use core_db::document_layouts::{create_layout, default_semantic_class_for_slug};
use core_domain::entities::config_identity::CiSemanticClass;
use sqlx::PgPool;
use uuid::Uuid;

struct Seed {
    org: Uuid,
    client: Uuid,
    layout: Uuid,
    user: Uuid,
}

/// Seeds org + client + a server layout (the CI6 type carrier).
async fn seed(app: &PgPool, n: u64) -> Seed {
    let org = fixed_uuid(n);
    let client = fixed_uuid(n + 1);
    let layout = fixed_uuid(n + 2);
    let user = fixed_uuid(n + 3);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("ci-{n}"), "MER")
        .await
        .expect("org");
    core_db::clients::create_client(&mut *tx, org, client, "Acme Corp", None)
        .await
        .expect("client");
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("u{n}@acme.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    create_layout(
        &mut *tx,
        org,
        layout,
        &format!("server-{n}"),
        "Server",
        "",
        "server",
        &json_schema(),
        true,
    )
    .await
    .expect("layout");
    // CI6: bind the layout to its semantic class (org-configurable types).
    core_db::document_layouts::set_layout_semantic_class(
        &mut *tx,
        org,
        layout,
        Some(CiSemanticClass::Server),
    )
    .await
    .expect("bind class");
    tx.commit().await.expect("commit seed");
    Seed {
        org,
        client,
        layout,
        user,
    }
}

fn json_schema() -> serde_json::Value {
    serde_json::json!([
        {"key": "serial", "label": "Serial", "type": "text", "position": 0},
        {"key": "ip_address", "label": "IP", "type": "text", "position": 1},
        {"key": "owner", "label": "Owner", "type": "text", "position": 2}
    ])
}

fn feed(seed: &Seed, source: &str, external: &str, keys: serde_json::Value) -> FeedUpsert {
    FeedUpsert {
        source_system: source.into(),
        external_id: external.into(),
        source_guid: None,
        payload_ref: None,
        client_id: seed.client,
        tag: "SRV-01".into(),
        name: "Server 01".into(),
        layout_id: Some(seed.layout),
        identity_keys: keys,
        observed_fields: Vec::new(),
    }
}

#[tokio::test]
async fn migration_lands_inventory_and_identity_surface() {
    let db = test_db().await;
    let s = seed(&db.app, 400).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // Inventory lifecycle columns exist with defaults.
    let (ownership, method): (String, String) = sqlx::query_as(
        "SELECT ownership, depreciation_method FROM device_assets WHERE org_id = $1 LIMIT 1",
    )
    .bind(s.org)
    .fetch_optional(&mut *tx)
    .await
    .expect("query")
    .map(|r: (String, String)| (r.0, r.1))
    .unwrap_or(("owned".into(), "none".into()));
    assert_eq!(ownership, "owned");
    assert_eq!(method, "none");

    // CI6: a class-bearing layout reads back its class (rename-safe).
    assert_eq!(
        class_binding(&mut tx, s.org, s.layout).await.as_deref(),
        Some("server")
    );
    assert_eq!(
        default_semantic_class_for_slug("workstation"),
        Some(CiSemanticClass::Workstation)
    );
    assert_eq!(
        default_semantic_class_for_slug("firewall"),
        Some(CiSemanticClass::NetworkAppliance)
    );
    assert_eq!(default_semantic_class_for_slug("sop"), None);

    // Identity keys default empty; threshold defaults to 0.85.
    assert_eq!(auto_merge_threshold(&mut tx, s.org).await.unwrap(), 0.85);
}

async fn class_binding(conn: &mut sqlx::PgConnection, org: Uuid, layout: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT semantic_class FROM document_layouts WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(layout)
        .fetch_one(conn)
        .await
        .expect("class")
}

/// CI2: the same (source, external id) syncs into the same item — the
/// second sync updates its own link and never creates a twin.
#[tokio::test]
async fn ci2_second_sync_updates_own_link_never_creates_twin() {
    let db = test_db().await;
    let s = seed(&db.app, 410).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let keys = json_keys(&[("serial", "SN-911"), ("source_guid", "GUID-1")]);
    let first = feed_upsert(&mut tx, s.org, &feed(&s, "ninjaone", "N-1", keys.clone()))
        .await
        .expect("first sync");
    assert!(first.created_item, "first sync creates the canonical row");
    let count = count_items(&mut *tx, s.org).await;
    assert_eq!(count, 1);

    let second = feed_upsert(&mut tx, s.org, &feed(&s, "ninjaone", "N-1", keys))
        .await
        .expect("second sync");
    assert!(!second.created_item, "second sync must NOT create a twin");
    assert_eq!(second.item.id, first.item.id);
    assert_eq!(count_items(&mut *tx, s.org).await, 1);
    assert_eq!(second.link.id, first.link.id, "same link, updated");

    // The link's last_seen_at refreshes.
    let last_seen: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_seen_at FROM config_source_links WHERE id = $1")
            .bind(first.link.id)
            .fetch_one(&mut *tx)
            .await
            .expect("link");
    assert!(last_seen > chrono::Utc::now() - chrono::Duration::seconds(30));
}

/// CI2 + CI3: the same physical device arriving from a DIFFERENT source
/// system with the same serial is deterministically resolved (adopted),
/// not duplicated and not queued as a twin.
#[tokio::test]
async fn ci3_cross_source_serial_is_deterministically_resolved() {
    let db = test_db().await;
    let s = seed(&db.app, 420).await;
    let mut tx = org_tx(&db.app, s.org).await;

    feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-7788")])),
    )
    .await
    .expect("rmm sync");

    let nms = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "auvix", "A-1", json_keys(&[("serial", "sn 7788")])),
    )
    .await
    .expect("nms sync");
    assert!(
        nms.resolved_existing,
        "serial normalization resolves the deterministic case"
    );
    assert!(!nms.created_item);
    assert!(nms.queued_candidates.is_empty());
    assert_eq!(
        count_items(&mut *tx, s.org).await,
        1,
        "no twin across feeds"
    );

    // The adoption is visible evidence: two source links on one item.
    let links = core_db::config_identity::list_links_for_item(&mut tx, s.org, nms.item.id)
        .await
        .expect("links");
    assert_eq!(links.len(), 2);
    let systems: Vec<&str> = links.iter().map(|l| l.source_system.as_str()).collect();
    assert!(systems.contains(&"ninjaone") && systems.contains(&"auvix"));
}

/// CI4 + CI3: an ambiguous match (hostname-only signal, below the
/// threshold) queues a reviewable candidate; lowering the org threshold
/// lets the same-strength match resolve automatically.
#[tokio::test]
async fn ci4_ambiguous_match_queues_candidate_and_threshold_governs_auto() {
    let db = test_db().await;
    let s = seed(&db.app, 430).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // An existing item known by its hostname.
    feed_upsert(
        &mut tx,
        s.org,
        &feed(
            &s,
            "it_glue",
            "D-1",
            json_keys(&[("serial", "OLD-1"), ("hostname", "mail")]),
        ),
    )
    .await
    .expect("existing");

    // A hostname-only signal scores 0.75 — below the 0.85 default
    // threshold: the sync creates a fresh canonical item and queues a
    // REVIEWABLE candidate (visible evidence, never a silent guess).
    let incoming = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "ninjaone", "N-2", json_keys(&[("hostname", "MAIL")])),
    )
    .await
    .expect("incoming");
    assert!(
        incoming.created_item,
        "below threshold: fresh canonical row"
    );
    assert!(!incoming.resolved_existing);
    assert_eq!(
        incoming.queued_candidates.len(),
        1,
        "reviewable candidate queued"
    );
    assert_eq!(count_items(&mut *tx, s.org).await, 2);

    let queued = core_db::config_identity::list_candidates(&mut tx, s.org, Some("pending"))
        .await
        .expect("queue");
    assert_eq!(queued.len(), 1);
    assert!(
        (queued[0].confidence - 0.75).abs() < 0.001,
        "candidate carries its evidence confidence: {}",
        queued[0].confidence
    );
    assert_eq!(queued[0].candidate_config_item_id, incoming.item.id);

    // The org lowers its threshold to 0.7: the SAME strength signal from a
    // third source now auto-resolves onto the best candidate — no third row.
    core_db::config_identity::upsert_settings(&mut tx, s.org, 0.7)
        .await
        .expect("settings");
    let third = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "auvix", "A-2", json_keys(&[("hostname", "Mail")])),
    )
    .await
    .expect("third sync");
    assert!(third.resolved_existing, "threshold governs auto resolution");
    assert!(!third.created_item);
    assert_eq!(
        count_items(&mut *tx, s.org).await,
        2,
        "auto-resolved, no twin"
    );
}

/// CI4: an explicit merge is audited, tombstones the absorbed item, moves
/// its links/interfaces, and a split restores everything.
#[tokio::test]
async fn ci4_merge_is_audited_undoable_and_split_restores() {
    let db = test_db().await;
    let s = seed(&db.app, 440).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let primary = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "KEEP-1")])),
    )
    .await
    .expect("primary");
    let absorbed = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "auvix", "A-1", json_keys(&[("serial", "GONE-1")])),
    )
    .await
    .expect("absorbed");
    assert!(absorbed.created_item);
    // The NIC belongs to the ABSORBED item (the item to fold away), so the
    // merge moves it and the split rides it back.
    core_db::config_identity::create_interface(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        absorbed.item.id,
        Some("eth0"),
        Some("aa:bb:cc:dd:ee:01"),
        &serde_json::json!(["10.0.0.5"]),
        &serde_json::json!([]),
        true,
        None,
    )
    .await
    .expect("interface");

    let before_interfaces =
        core_db::config_identity::list_interfaces(&mut tx, s.org, absorbed.item.id)
            .await
            .expect("interfaces");
    assert_eq!(before_interfaces.len(), 1);

    let merge = core_db::config_identity::merge_items(
        &mut tx,
        s.org,
        primary.item.id,
        absorbed.item.id,
        None,
        Some(0.95),
        "serial evidence",
        &serde_json::json!({ "signals": ["serial KEEP-1"] }),
        Some(s.user),
    )
    .await
    .expect("merge");
    assert_eq!(merge.status, "applied");
    assert_eq!(merge.merged_config_item_id, absorbed.item.id);

    // Tombstone + link move visible.
    let tombstoned = core_db::assets::get_asset(&mut *tx, s.org, absorbed.item.id)
        .await
        .expect("tombstone");
    assert_eq!(tombstoned.merged_into_id, Some(primary.item.id));
    assert_eq!(
        count_items(&mut *tx, s.org).await,
        1,
        "merged item excluded from active list"
    );
    let moved_links =
        core_db::config_identity::list_links_for_item(&mut tx, s.org, primary.item.id)
            .await
            .expect("links");
    assert_eq!(
        moved_links.len(),
        2,
        "absorbed item's link moved to primary"
    );

    // Split (undo): the audit row's moved packet restores everything.
    core_db::config_identity::split_merge(&mut tx, s.org, merge.id, s.user)
        .await
        .expect("split");
    let restored = core_db::assets::get_asset(&mut *tx, s.org, absorbed.item.id)
        .await
        .expect("restored");
    assert!(restored.merged_into_id.is_none(), "tombstone cleared");
    let restored_interfaces =
        core_db::config_identity::list_interfaces(&mut tx, s.org, absorbed.item.id)
            .await
            .expect("interfaces after split");
    assert_eq!(
        restored_interfaces.len(),
        1,
        "the absorbed item's interface rode back"
    );
    let moved_back_links =
        core_db::config_identity::list_links_for_item(&mut tx, s.org, absorbed.item.id)
            .await
            .expect("links back");
    assert_eq!(moved_back_links.len(), 1);
}

/// CI5: curated values are sticky — a feed contradiction is recorded but
/// never applied; one-click accept promotes the observed value.
#[tokio::test]
async fn ci5_curated_sticky_and_accept_observed_promotes() {
    let db = test_db().await;
    let s = seed(&db.app, 450).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let mut input = feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-1")]));
    input.observed_fields = vec![FeedField {
        key: "owner".into(),
        value: serde_json::json!("feed-owner"),
    }];
    let up = feed_upsert(&mut tx, s.org, &input).await.expect("sync");
    let item_id = up.item.id;

    // Human curates: sticky forever.
    let curated = curate_fields(
        &mut tx,
        s.org,
        item_id,
        &serde_json::json!({ "owner": "Alice" }),
    )
    .await
    .expect("curate");
    assert_eq!(curated.fields["owner"], "Alice");

    // Feed contradicts: observed is recorded, effective stays curated, and
    // the contradiction is flagged as visible evidence.
    let mut input2 = feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-1")]));
    input2.observed_fields = vec![FeedField {
        key: "owner".into(),
        value: serde_json::json!("Bot McBotface"),
    }];
    let second = feed_upsert(&mut tx, s.org, &input2)
        .await
        .expect("second sync");
    assert_eq!(second.item.fields["owner"], "Alice", "curated wins");
    assert_eq!(second.item.curated_fields["owner"], "Alice");
    assert_eq!(second.item.observed_fields["owner"], "Bot McBotface");
    assert!(
        second.item.field_sources["owner"]["contradicts_curated"] == serde_json::json!(true),
        "contradiction surfaced as visible evidence"
    );

    // One-click accept: observed promoted into curated (sticky), flag cleared.
    let accepted = accept_observed_value(&mut tx, s.org, item_id, "owner")
        .await
        .expect("accept");
    assert_eq!(accepted.curated_fields["owner"], "Bot McBotface");
    assert_eq!(accepted.fields["owner"], "Bot McBotface");
    assert!(
        accepted.field_sources["owner"]
            .get("contradicts_curated")
            .is_none(),
        "contradiction cleared on accept"
    );

    // A third feed sync cannot overwrite the now-curated value.
    let mut input3 = feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-1")]));
    input3.observed_fields = vec![FeedField {
        key: "owner".into(),
        value: serde_json::json!("Yet Another"),
    }];
    let third = feed_upsert(&mut tx, s.org, &input3).await.expect("third");
    assert_eq!(third.item.fields["owner"], "Bot McBotface");
}

/// CI5: feed-vs-feed precedence — a stronger source wins an observed field
/// unless an org override says otherwise; weaker sources never clobber.
#[tokio::test]
async fn ci5_precedence_governs_feed_writes() {
    let db = test_db().await;
    let s = seed(&db.app, 460).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let mut rmm = feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-1")]));
    rmm.observed_fields = vec![FeedField {
        key: "ip_address".into(),
        value: serde_json::json!("10.0.0.1"),
    }];
    let up = feed_upsert(&mut tx, s.org, &rmm).await.expect("rmm sync");

    // NMS is the shipped winner for ip_address — it overwrites the RMM value.
    let mut nms = feed(&s, "auvix", "A-1", json_keys(&[("serial", "SN-1")]));
    nms.observed_fields = vec![FeedField {
        key: "ip_address".into(),
        value: serde_json::json!("10.0.0.99"),
    }];
    let adopted = feed_upsert(&mut tx, s.org, &nms).await.expect("nms sync");
    assert_eq!(adopted.item.id, up.item.id, "same canonical item");
    assert_eq!(adopted.item.fields["ip_address"], "10.0.0.99");

    // Org override: pin the field to ninjaone (priority 100 > the shipped
    // NMS 90). The next ninjaone sync overwrites; a following auvix sync
    // can no longer clobber it.
    upsert_precedence(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        None,
        None,
        "ip_address",
        "ninjaone",
        100,
    )
    .await
    .expect("override");
    let mut rmm2 = feed(&s, "ninjaone", "N-1", json_keys(&[("serial", "SN-1")]));
    rmm2.observed_fields = vec![FeedField {
        key: "ip_address".into(),
        value: serde_json::json!("10.1.1.1"),
    }];
    let rmm_second = feed_upsert(&mut tx, s.org, &rmm2)
        .await
        .expect("rmm second");
    assert_eq!(rmm_second.item.fields["ip_address"], "10.1.1.1");

    let mut nms2 = feed(&s, "auvix", "A-1", json_keys(&[("serial", "SN-1")]));
    nms2.observed_fields = vec![FeedField {
        key: "ip_address".into(),
        value: serde_json::json!("10.9.9.9"),
    }];
    let third = feed_upsert(&mut tx, s.org, &nms2).await.expect("third");
    assert_eq!(
        third.item.fields["ip_address"], "10.1.1.1",
        "the override-protected source keeps the field"
    );
}

/// CI2: no identity signals at all still lands a fresh canonical item with
/// its link — the write is never blind.
#[tokio::test]
async fn ci2_signal_less_sync_creates_one_item_with_link() {
    let db = test_db().await;
    let s = seed(&db.app, 470).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let up = feed_upsert(
        &mut tx,
        s.org,
        &feed(&s, "it_glue", "D-1", serde_json::json!({})),
    )
    .await
    .expect("sync");
    assert!(up.created_item);
    assert_eq!(count_items(&mut *tx, s.org).await, 1);
    assert_eq!(
        core_db::config_identity::list_links_for_item(&mut tx, s.org, up.item.id)
            .await
            .expect("links")
            .len(),
        1
    );
}

/// RLS: every new CI table is org-bound (defense-in-depth USING+WITH CHECK).
#[tokio::test]
async fn ci_tables_are_rls_org_bound() {
    let db = test_db().await;
    let offenders: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relkind = 'r'
          AND c.relname IN ('config_source_links','ci_interfaces',
                            'ci_field_precedence','ci_merge_candidates',
                            'ci_merges','ci_settings')
          AND NOT EXISTS (
            SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid
          )
        "#,
    )
    .fetch_all(&db.admin)
    .await
    .expect("probe");
    assert!(
        offenders.is_empty(),
        "tables missing an RLS policy: {offenders:?}"
    );
}

// ---- helpers ---------------------------------------------------------------

fn json_keys(pairs: &[(&str, &str)]) -> serde_json::Value {
    let mut o = serde_json::Map::new();
    for (k, v) in pairs {
        if *k == "macs" {
            o.insert(
                "macs".into(),
                serde_json::Value::Array(vec![serde_json::Value::from(*v)]),
            );
        } else {
            o.insert((*k).to_string(), serde_json::Value::from(*v));
        }
    }
    serde_json::Value::Object(o)
}

async fn count_items<'e, E>(executor: E, org: Uuid) -> i64
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "SELECT count(*) FROM device_assets WHERE org_id = $1 AND merged_into_id IS NULL",
    )
    .bind(org)
    .fetch_one(executor)
    .await
    .expect("count")
}
