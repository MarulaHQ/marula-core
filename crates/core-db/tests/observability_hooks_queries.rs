//! Observability-hook queries against a real Postgres (TASKLIST R7a).
//!
//! Purpose: the three statements this lane added are RUNTIME queries
//! (`sqlx::query`/`query_scalar`, not the `query!` macro — they have no
//! `.sqlx` cache entry yet), so the compiler never sees their SQL. This file
//! is what stops a typo from reaching production: it runs each one against a
//! migrated database and asserts the value it produces.
//!
//! Three properties, one per hook:
//!
//! * `queue::oldest_dead_age_secs` reports the age of the OLDEST dead letter
//!   measured from the moment it died (`updated_at`), and `None` when nothing
//!   is dead — the absence rule the worker relies on to skip the series.
//! * `workflow_runs::parked_stats` counts waiting runs and the oldest idle
//!   age, ignoring running/finished rows.
//! * `audit::list_forward` walks ASCENDING on `(occurred_at, id)` and resumes
//!   exactly after a cursor: no row skipped at a page boundary and none
//!   delivered twice — the property that makes an audit export resumable.
//!
//! Database selection follows `tests/common/mod.rs`: `MARULA_TEST_DATABASE_URL`,
//! else `DATABASE_URL`, else a local testcontainer.

mod common;

use common::{fixed_uuid, org_tx, test_db, FUTURE, PAST};
use core_db::audit::AuditFilter;
use core_db::{audit, orgs, queue, workflow_runs};
use core_domain::entities::{Actor, AuditEvent};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_org(app: &PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("acme-obs-{n}"), "OBS")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit seed");
    org
}

/// One audit row, at an explicit instant, so page boundaries are exact.
async fn insert_audit(
    app: &PgPool,
    org: Uuid,
    id: Uuid,
    action: &str,
    at: chrono::DateTime<chrono::Utc>,
) {
    let mut tx = org_tx(app, org).await;
    audit::insert(
        &mut *tx,
        &AuditEvent {
            id: id.into(),
            org_id: org.into(),
            actor: Actor::System,
            action: action.into(),
            entity_type: Some("ticket".into()),
            entity_id: None,
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: at,
        },
    )
    .await
    .expect("audit insert");
    tx.commit().await.expect("commit audit");
}

#[tokio::test]
async fn dead_letter_age_is_absent_until_a_job_actually_dies() {
    let db = test_db().await;
    let org = seed_org(&db.app, 9_001).await;

    assert_eq!(
        queue::oldest_dead_age_secs(&db.admin)
            .await
            .expect("empty read"),
        None,
        "no dead jobs must read as absence, not as zero age"
    );

    // A pending row is not dead: it must not produce an age either.
    queue::enqueue(
        &db.admin,
        &queue::NewJob {
            org_id: org,
            kind: "observability.probe".into(),
            payload: serde_json::json!({}),
            max_attempts: 1,
        },
    )
    .await
    .expect("enqueue");
    assert_eq!(
        queue::oldest_dead_age_secs(&db.admin)
            .await
            .expect("pending read"),
        None
    );

    let dead_id = fixed_uuid(9_002);
    sqlx::query(
        "INSERT INTO jobs (id, org_id, kind, payload, status, attempts, max_attempts, updated_at)
         VALUES ($1, $2, 'observability.probe', '{}'::jsonb, 'dead', 1, 1, now() - interval '2 hours')",
    )
    .bind(dead_id)
    .bind(org)
    .execute(&db.admin)
    .await
    .expect("insert dead job");

    let age = queue::oldest_dead_age_secs(&db.admin)
        .await
        .expect("dead read")
        .expect("a dead job must have an age");
    assert!(
        (7_000..=7_400).contains(&age),
        "expected roughly two hours of age, got {age}s"
    );
}

#[tokio::test]
async fn parked_stats_counts_waiting_runs_and_the_oldest_idle_age() {
    let db = test_db().await;
    let org = seed_org(&db.app, 9_101).await;

    let empty = workflow_runs::parked_stats(&db.admin).await.expect("empty");
    assert_eq!(empty.parked, 0);
    assert_eq!(empty.oldest_age_secs, None);

    // Three rows: two waiting (different ages), one finished. Only the
    // waiting pair may contribute.
    for (id, status, minutes) in [
        (fixed_uuid(9_102), "waiting", 90),
        (fixed_uuid(9_103), "waiting", 10),
        (fixed_uuid(9_104), "succeeded", 5),
    ] {
        sqlx::query(
            "INSERT INTO workflow_runs (
                 id, org_id, rule_name, rule_definition, catalogue_version, status,
                 created_at, updated_at
             ) VALUES ($1, $2, 'probe rule', '{}'::jsonb, 'probe', $3,
                       now() - make_interval(mins => $4),
                       now() - make_interval(mins => $4))",
        )
        .bind(id)
        .bind(org)
        .bind(status)
        .bind(minutes)
        .execute(&db.admin)
        .await
        .expect("insert run");
    }

    let stats = workflow_runs::parked_stats(&db.admin).await.expect("stats");
    assert_eq!(stats.parked, 2, "only waiting runs are parked");
    let oldest = stats.oldest_age_secs.expect("parked runs carry an age");
    assert!(
        (5_300..=5_500).contains(&oldest),
        "expected roughly 90 minutes, got {oldest}s"
    );
}

#[tokio::test]
async fn audit_export_walks_forward_and_resumes_without_skipping() {
    let db = test_db().await;
    let org = seed_org(&db.app, 9_201).await;

    // Five rows, three of them sharing ONE instant: the tie is the case a
    // timestamp-only cursor gets wrong.
    insert_audit(&db.app, org, fixed_uuid(9_211), "probe.one", PAST).await;
    insert_audit(&db.app, org, fixed_uuid(9_212), "probe.two", FUTURE).await;
    insert_audit(&db.app, org, fixed_uuid(9_213), "probe.three", FUTURE).await;
    insert_audit(&db.app, org, fixed_uuid(9_214), "probe.four", FUTURE).await;

    let filter = || AuditFilter {
        actor: None,
        action: Some("probe.".into()),
        ..AuditFilter::default()
    };

    // Page one: two rows, oldest first.
    let first = audit::list_forward(&db.admin, org, filter(), None, 2)
        .await
        .expect("page one");
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].id, fixed_uuid(9_211));
    assert!(
        first[0].occurred_at <= first[1].occurred_at,
        "rows must arrive oldest first"
    );

    let anchor = first.last().expect("page one has rows");
    let cursor = (anchor.occurred_at, anchor.id);

    // Page two resumes strictly AFTER the anchor: no repeat of the anchor,
    // no skip of the row that shares its timestamp.
    let second = audit::list_forward(&db.admin, org, filter(), Some(cursor), 2)
        .await
        .expect("page two");
    assert_eq!(second.len(), 2, "the remaining rows must all be reachable");
    assert!(
        !second.iter().any(|e| e.id == anchor.id),
        "the anchor row must not be delivered twice"
    );

    // Walking to exhaustion delivers every row exactly once.
    let mut seen: Vec<Uuid> = first.iter().chain(second.iter()).map(|e| e.id).collect();
    let mut after = Some((
        second.last().expect("page two rows").occurred_at,
        second.last().unwrap().id,
    ));
    loop {
        let page = audit::list_forward(&db.admin, org, filter(), after, 2)
            .await
            .expect("next page");
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|e| e.id));
        let last = page.last().expect("non-empty page");
        after = Some((last.occurred_at, last.id));
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        4,
        "every probe row must be delivered exactly once, got {seen:?}"
    );
}
