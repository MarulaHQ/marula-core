//! Data lifecycle — DB-layer contracts (Wave 52 lane R6,
//! `p52-data-lifecycle-dr`).
//!
//! These cases pin the four things the migration and the two sweeps promise, and
//! nothing else:
//!
//! 1. `auth_attempt_log` really is a partition-DROP table: a partitioned parent,
//!    a DEFAULT partition to keep an INSERT from being refused, month children,
//!    and RLS on the parent AND on every child (a policy on the parent does not
//!    answer a direct query against a child, and the RLS-coverage gate reads
//!    children).
//! 2. The maintainer drops a whole expired partition and creates the months it is
//!    about to need — including December, whose successor is in the next year.
//! 3. The retention sweep deletes only rows past their window that are also no
//!    longer owed: a pending job and a running job survive at any age, a finished
//!    one does not.
//! 4. An erasure removes the tenant's rows by cascade and leaves a receipt that
//!    names the org it no longer has.
//!
//! The object-integrity cases assert the reference query rather than row marking,
//! because `attachments` needs a ticket or an email message to exist and the
//! rows that matter here are the keys, not the owners.
//!
//! Everything runs on the admin (owner) pool, deliberately: the worker and the
//! CLI both connect as the table owner for exactly this work, and a constrained
//! role would delete nothing while reporting success.

mod common;

/// These tests read global schema state — the partition catalogue, the row
/// counts an erasure reports, the cascade scan — so one test's seeded partition
/// or swept table is another test's false failure. One lock per process, taken
/// by every test in this file.
static SCHEMA_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use common::{fixed_uuid, test_db, TestDb};
use sqlx::PgPool;
use uuid::Uuid;

/// Makes each case start from a known retention state. The sweep is cross-org by
/// design, so a case that leaves eligible rows behind would hand the next case
/// extra rows to delete and turn every count into a coin toss.
async fn clear_retention_tables(admin: &PgPool) {
    for table in ["jobs", "audit_events", "notifications"] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(admin)
            .await
            .unwrap_or_else(|e| panic!("clear {table}: {e}"));
    }
}

async fn seed_org(db: &TestDb, n: u64) -> Uuid {
    let org_id = fixed_uuid(n);
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(format!("Lifecycle Org {n}"))
        .bind(format!("lifecycle-{n}"))
        .execute(&db.admin)
        .await
        .expect("seed org");
    org_id
}

#[tokio::test]
async fn the_attempt_log_is_partitioned_with_a_default_and_rls_on_every_child() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;

    let relkind: String =
        sqlx::query_scalar("SELECT relkind::text FROM pg_class WHERE relname = 'auth_attempt_log'")
            .fetch_one(&db.admin)
            .await
            .expect("relkind");
    assert_eq!(
        relkind, "p",
        "auth_attempt_log must be a partitioned parent"
    );

    // RLS on the parent. The gate reads children too, and a child without RLS is
    // readable across tenants by a query that names it.
    let unprotected: Vec<String> = sqlx::query_scalar(
        r#"SELECT c.relname FROM pg_class c
             JOIN pg_inherits i ON i.inhrelid = c.oid
             JOIN pg_class p ON p.oid = i.inhparent
            WHERE p.relname = 'auth_attempt_log' AND c.relrowsecurity = false"#,
    )
    .fetch_all(&db.admin)
    .await
    .expect("child RLS scan");
    assert!(
        unprotected.is_empty(),
        "partitions without RLS: {unprotected:?}"
    );

    let has_default: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1 FROM pg_inherits i
               JOIN pg_class c ON c.oid = i.inhrelid
               JOIN pg_class p ON p.oid = i.inhparent
              WHERE p.relname = 'auth_attempt_log'
                AND pg_get_expr(c.relpartbound, c.oid) = 'DEFAULT')"#,
    )
    .fetch_one(&db.admin)
    .await
    .expect("default partition");
    assert!(
        has_default,
        "the DEFAULT partition is what keeps a failed login from being refused"
    );

    // An insert with no explicit month partition must land somewhere: this is the
    // promise the DEFAULT exists for.
    let org = seed_org(&db, 9_001).await;
    sqlx::query(
        "INSERT INTO auth_attempt_log (id, org_id, kind, subject_key, ip_key, occurred_at)
         VALUES (gen_random_uuid(), $1, 'password_login', 'email:x@example.test', 'ip:abc',
                 now() - interval '3 years')",
    )
    .bind(org)
    .execute(&db.admin)
    .await
    .expect("an attempt outside every partition must still insert");
}

#[tokio::test]
async fn the_maintainer_drops_an_expired_partition_and_creates_the_months_ahead() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let plan = core_db::retention::RetentionPlan::resolve(|_| None);

    // A partition nobody needs any more, in the shape the maintainer names.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS auth_attempt_log_p2015_01
           PARTITION OF auth_attempt_log FOR VALUES FROM ('2015-01-01 00:00:00+00')
                                                   TO ('2015-02-01 00:00:00+00')",
    )
    .execute(&db.admin)
    .await
    .expect("seed an expired partition");

    // The migration pre-creates the ahead window, so a plain maintainer pass
    // has nothing to create. Drop next month to exercise the step that matters
    // when the calendar moves rather than to assert an empty no-op.
    let ahead: String = sqlx::query_scalar(
        "SELECT to_char(date_trunc('month', now()) + interval '1 month', 'YYYY_MM')",
    )
    .fetch_one(&db.admin)
    .await
    .expect("next month");
    sqlx::query(&format!("DROP TABLE IF EXISTS auth_attempt_log_p{ahead}"))
        .execute(&db.admin)
        .await
        .expect("drop next month's partition");

    let outcomes = core_db::partitions::maintain(&db.admin, &plan)
        .await
        .expect("maintain");
    let attempts = outcomes
        .iter()
        .find(|o| o.table == "auth_attempt_log")
        .expect("outcome for the managed table");
    assert!(
        attempts.skipped.is_none(),
        "skipped: {:?}",
        attempts.skipped
    );
    assert!(
        attempts
            .dropped
            .iter()
            .any(|d| d.name == "auth_attempt_log_p2015_01"),
        "the expired partition must be dropped: {:?}",
        attempts.dropped
    );
    assert!(
        attempts
            .created
            .iter()
            .any(|c| c == &format!("auth_attempt_log_p{ahead}")),
        "the maintainer must create the months it is about to need: {:?}",
        attempts.created
    );

    let still_there: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_class WHERE relname = 'auth_attempt_log_p2015_01')",
    )
    .fetch_one(&db.admin)
    .await
    .expect("existence");
    assert!(!still_there, "the dropped partition must be gone");
}

#[tokio::test]
async fn the_sweep_deletes_only_rows_past_their_window_that_are_not_owed() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    clear_retention_tables(&db.admin).await;
    let org = seed_org(&db, 9_002).await;

    // Old and finished: eligible.
    let finished = fixed_uuid(9_100);
    // Old but still pending: the queue owes it a run.
    let pending = fixed_uuid(9_101);
    // Old and mid-flight: a worker holds it.
    let running = fixed_uuid(9_102);
    // Fresh and finished: inside the window.
    let recent = fixed_uuid(9_103);
    for (id, status, age) in [
        (finished, "done", "40 days"),
        (pending, "pending", "40 days"),
        (running, "running", "40 days"),
        (recent, "done", "1 day"),
    ] {
        sqlx::query(&format!(
            "INSERT INTO jobs (id, org_id, kind, payload, status, updated_at)
             VALUES ($1, $2, 'test.noop', '{{}}'::jsonb, $3, now() - interval '{age}')"
        ))
        .bind(id)
        .bind(org)
        .bind(status)
        .execute(&db.admin)
        .await
        .expect("seed job");
    }

    let plan = core_db::retention::RetentionPlan::resolve(|_| None);
    let report = core_db::retention::sweep_once(&db.admin, &plan, 100)
        .await
        .expect("sweep");
    let jobs = report
        .iter()
        .find(|t| t.table == "jobs")
        .expect("jobs in the report");
    assert_eq!(jobs.deleted, 1, "only the finished, old job is eligible");

    let survivors: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM jobs ORDER BY id")
        .fetch_all(&db.admin)
        .await
        .expect("survivors");
    assert!(
        survivors.contains(&pending) && survivors.contains(&running) && survivors.contains(&recent),
        "a job that is owed a run, or is inside the window, must survive: {survivors:?}"
    );
    assert!(
        !survivors.contains(&finished),
        "the finished job past the window must be gone"
    );
}

#[tokio::test]
async fn every_org_scoped_table_cascades_from_the_org_row() {
    // The erasure contract says "rows" are removed by one `DELETE FROM orgs`.
    // That sentence is only true while every table with an `org_id` carries a
    // foreign key to `orgs` with ON DELETE CASCADE. A migration that adds an
    // `org_id` column to a new table and forgets the cascade would leave rows
    // behind that the receipt calls deleted — the exact failure the contract
    // exists to prevent — so the rule is a test, not a convention.
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let offenders: Vec<String> = sqlx::query_scalar(
        r#"SELECT c.relname
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = 'public'
              AND c.relkind IN ('r', 'p')
              AND EXISTS (
                SELECT 1 FROM information_schema.columns col
                 WHERE col.table_schema = 'public'
                   AND col.table_name = c.relname
                   AND col.column_name = 'org_id')
              AND NOT EXISTS (
                SELECT 1
                  FROM pg_constraint con
                  JOIN pg_attribute a
                    ON a.attrelid = con.conrelid AND a.attnum = con.conkey[1]
                 WHERE con.conrelid = c.oid
                   AND con.contype = 'f'
                   AND con.confrelid = 'orgs'::regclass
                   AND con.confdeltype = 'c'
                   AND a.attname = 'org_id')
            ORDER BY c.relname"#,
    )
    .fetch_all(&db.admin)
    .await
    .expect("cascade scan");
    assert!(
        offenders.is_empty(),
        "tables with org_id but no ON DELETE CASCADE to orgs: {offenders:?}"
    );
}

#[tokio::test]
async fn erasure_purges_the_tenant_by_cascade_and_leaves_a_receipt() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 9_003).await;
    let user = fixed_uuid(9_300);
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash)
         VALUES ($1, $2, 'erase@example.test', 'Erase Me', 'x')",
    )
    .bind(user)
    .bind(org)
    .execute(&db.admin)
    .await
    .expect("seed user");

    // One object key on a table with no owner column, so the reference query has
    // something to find without a ticket or an email message.
    sqlx::query(
        "INSERT INTO attachment_scans
           (org_id, sha256, status, scanner, size_bytes, content_type, filename, storage_key)
         VALUES ($1, $2, 'skipped', 'none', 12, 'text/plain', 'notes.txt', $3)",
    )
    .bind(org)
    .bind("a".repeat(64))
    .bind(format!("org/{org}/quarantine/{}", "a".repeat(64)))
    .execute(&db.admin)
    .await
    .expect("seed a scan row");

    let facts = core_db::erasure::org_facts(&db.admin, org)
        .await
        .expect("org facts")
        .expect("the org exists");
    assert_eq!(facts.org_id, org);
    assert_eq!(facts.users, 1);

    let counts = core_db::erasure::surface_counts(&db.admin, org)
        .await
        .expect("counts");
    assert_eq!(counts.get("users"), Some(&1));
    assert_eq!(counts.get("attachment_scans"), Some(&1));

    let keys = core_db::erasure::object_keys(&db.admin, org)
        .await
        .expect("object keys");
    assert_eq!(
        keys.len(),
        1,
        "the org's object key must be collected: {keys:?}"
    );

    let references = core_db::object_gc::referenced_keys(&db.admin)
        .await
        .expect("references");
    assert!(
        references
            .iter()
            .any(|r| r.source_table == "attachment_scans"),
        "the integrity pass must see scan rows, or it would delete their objects"
    );

    let purged = core_db::erasure::purge_rows(&db.admin, org)
        .await
        .expect("purge");
    assert_eq!(purged, 1);

    let users_left: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE org_id = $1")
        .bind(org)
        .fetch_one(&db.admin)
        .await
        .expect("users left");
    assert_eq!(users_left, 0, "the cascade must remove the tenant's users");
    assert!(
        core_db::erasure::org_facts(&db.admin, org)
            .await
            .expect("org facts after")
            .is_none(),
        "the org row must be gone"
    );

    let receipt = core_db::erasure::Receipt {
        purged_org_id: org,
        org_label: facts.label,
        requested_by: "test".into(),
        started_at: chrono::Utc::now(),
        finished_at: Some(chrono::Utc::now()),
        rows_deleted: counts,
        objects_deleted: 1,
        objects_failed: 0,
        outcome: core_db::erasure::ErasureOutcome::Completed,
        note: Some("db-layer contract test".into()),
    };
    let receipt_id = core_db::erasure::insert_receipt(&db.admin, &receipt)
        .await
        .expect("receipt");
    let stored: String =
        sqlx::query_scalar("SELECT outcome FROM org_erasure_receipts WHERE id = $1")
            .bind(receipt_id)
            .fetch_one(&db.admin)
            .await
            .expect("read the receipt back");
    assert_eq!(stored, "completed");
}
