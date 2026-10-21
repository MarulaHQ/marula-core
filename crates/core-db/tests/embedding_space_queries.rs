//! The embedding space tag and the re-index run queue (Wave 52 lane R14,
//! migration 0152).
//!
//! Both proofs here need SQL, so both run against a real Postgres:
//!
//! 1. A model switch cannot mix spaces. One row's only vector is from the old
//!    model; ranking it against a query vector from the new model returns a
//!    confident-looking neighbour whose distance is meaningless. The space
//!    predicate turns that into a miss, and the dual-read window keeps the two
//!    populations on separate legs while a re-index runs.
//! 2. The re-index resumes from its selection predicate rather than a cursor,
//!    queues one run per org per switch, and goes dark instead of mixed when
//!    the model id itself changes.
//!
//! The second test owns the instance singleton (`embedding_space_state`), so
//! the whole lifecycle — including the mixed-space matrix — lives in one test:
//! the run queue is instance-wide, and a second test running beside this one
//! would see the org this one deliberately leaves behind.
//!
//! Slot discipline: fixed-UUID slots 94600–94699 only.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::embedding_space::{self, RunProgress, SpaceState};
use core_db::{orgs, search};
use core_domain::embedding::{space_for_model, Catalogue};
use core_domain::SEARCH_EMBEDDING_DIM;
use uuid::Uuid;

const SLOT_ORG: u64 = 94_600;
const SLOT_A: u64 = 94_601;
const SLOT_B: u64 = 94_602;
const SLOT_C: u64 = 94_603;
const SLOT_ORG_SWITCH: u64 = 94_620;
const SLOT_SWITCH_ROWS: u64 = 94_621;

fn shipped_space() -> String {
    space_for_model(&Catalogue::builtin(), "embeddinggemma-2:270m-q8_0-text").id()
}

/// The tag a pre-0152 row gets: model known, build unrecorded.
fn untagged_space(model: &str) -> String {
    format!("{model}@unversioned/{SEARCH_EMBEDDING_DIM}")
}

fn axis(index: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; SEARCH_EMBEDDING_DIM];
    v[index] = 1.0;
    v
}

async fn seed_org(db: &common::TestDb, slot: u64) -> Uuid {
    let org = fixed_uuid(slot);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        &format!("Embedding space org {slot}"),
        &format!("embedding-space-{slot}"),
        "RAG",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit org");
    org
}

async fn index_ticket(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: Uuid,
    id: Uuid,
    title: &str,
    body: &str,
) {
    search::upsert_row(
        &mut **tx,
        org,
        search::SearchKind::Ticket,
        id,
        None,
        title,
        body,
        None,
    )
    .await
    .expect("index row");
}

async fn embed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: Uuid,
    id: Uuid,
    space: &str,
    vector: &[f32],
) {
    // The model id is the space's model segment; the tag is what retrieval
    // filters on, the column is what a support conversation reads.
    let model = space.split('@').next().unwrap_or(space);
    let stored = search::upsert_embedding(
        &mut **tx,
        org,
        search::SearchKind::Ticket,
        id,
        space,
        model,
        vector,
    )
    .await
    .expect("embedding upsert");
    assert!(stored, "the index row must exist before its vector");
}

/// What the vector leg asked before 0152: nearest neighbours over the whole
/// table, with no predicate naming which model wrote the vector.
async fn pre_r14_nearest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: Uuid,
    query: &[f32],
) -> Vec<Uuid> {
    sqlx::query_scalar(
        r#"SELECT entity_id
             FROM search_embeddings
            WHERE org_id = $1 AND entity_kind = 'ticket'
            ORDER BY embedding <=> $2::text::vector, entity_id
            LIMIT 10"#,
    )
    .bind(org)
    .bind(search::embedding_to_pg(query))
    .fetch_all(&mut **tx)
    .await
    .expect("pre-R14 nearest")
}

/// One test owns the instance singleton, so the mixed-space matrix and the run
/// lifecycle are one function: the queue is instance-wide, and a second sweep
/// test running beside this one would see the other's behind-on-purpose org.
///
/// Section one: the failing (pre-0152) query is the one the code used to issue;
/// the passing case is the one it issues now. They must disagree about which
/// row is nearest.
///
/// Section two: the switch lifecycle in the order the processes run it —
/// core-api records the space, the worker queues and sweeps, core-api finishes
/// the org.
#[tokio::test]
async fn a_model_switch_keeps_spaces_separate_and_the_reindex_resumes() {
    let db = test_db().await;

    // ------------------------------------------------------------------
    // One query vector, two spaces: never ranked together.
    // ------------------------------------------------------------------
    let org = seed_org(&db, SLOT_ORG).await;
    let (a, b, c) = (fixed_uuid(SLOT_A), fixed_uuid(SLOT_B), fixed_uuid(SLOT_C));
    let old_space = untagged_space("nomic-embed-text");
    let new_space = shipped_space();
    let kinds = ["ticket".to_string()];

    let mut tx = org_tx(&db.app, org).await;
    index_ticket(&mut tx, org, a, "Mail relay outage", "smtp failing").await;
    index_ticket(&mut tx, org, b, "Payroll export", "csv export").await;
    index_ticket(&mut tx, org, c, "Network switch", "vlan trunk").await;

    // `a` was embedded by the old model and has not been re-indexed: its only
    // vector is old-space, sitting on the axis the NEW model's query uses.
    // `b` and `c` are already in the new space, near and far respectively.
    embed(&mut tx, org, a, &old_space, &axis(0)).await;
    embed(&mut tx, org, b, &new_space, &axis(0)).await;
    embed(&mut tx, org, c, &new_space, &axis(767)).await;

    let query = axis(0);

    // Failing case: with no space predicate, `a`'s stale vector outranks every
    // comparable one. This is the plausible-but-wrong answer the migration
    // exists to remove.
    let mixed = pre_r14_nearest(&mut tx, org, &query).await;
    assert_eq!(
        mixed.first(),
        Some(&a),
        "the un-filtered nearest-neighbour query serves an old-space row first"
    );

    // Passing case: the same query vector, restricted to the space it was
    // produced in, ranks the two new-space rows and misses `a` honestly.
    let target = search::vector_candidates(&mut *tx, org, &kinds, &new_space, &query, None, 10)
        .await
        .expect("target space");
    let target_ids: Vec<Uuid> = target.iter().map(|key| key.entity_id).collect();
    assert_eq!(target_ids, vec![b, c], "one space, its own order");
    assert!(
        !target_ids.contains(&a),
        "a row with no target-space vector is a miss, not a stale hit"
    );

    // Dual-read: the same two spaces on separate legs. `a` is served from the
    // old leg (it is still all that org has) and `b` appears once — the fusion
    // merges by key, it never adds two distances together.
    let both = [new_space.clone(), old_space.clone()];
    let hits = search::search_hybrid_page(
        &mut tx,
        org,
        &kinds,
        "zzzznomatch",
        false,
        None,
        &both,
        &query,
        10,
        0,
    )
    .await
    .expect("dual-read page");
    let mut ids: Vec<Uuid> = hits.iter().map(|hit| hit.entity_id).collect();
    let unique = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), unique, "a fused row is served once");
    assert_eq!(ids.len(), 3, "both populations are readable: {ids:?}");
    assert!(ids.contains(&a) && ids.contains(&b) && ids.contains(&c));

    // Target-only read: the fused page carries the honest answer, and the
    // count agrees with it.
    let hits = search::search_hybrid_page(
        &mut tx,
        org,
        &kinds,
        "zzzznomatch",
        false,
        None,
        &[new_space.clone()],
        &query,
        10,
        0,
    )
    .await
    .expect("target-only page");
    let ids: Vec<Uuid> = hits.iter().map(|hit| hit.entity_id).collect();
    assert_eq!(ids, vec![b, c]);
    let count = search::search_hybrid_count(
        &mut *tx,
        org,
        &kinds,
        "zzzznomatch",
        false,
        None,
        &[new_space.clone()],
        search::SEARCH_COUNT_CAP,
    )
    .await
    .expect("count");
    assert_eq!(count.total, 2, "only the target space counts");

    // Dark leg: no readable spaces contributes no vector candidates at all,
    // which is the answer for a model change that is not comparable.
    let dark = search::search_hybrid_page(
        &mut tx,
        org,
        &kinds,
        "zzzznomatch",
        false,
        None,
        &[],
        &query,
        10,
        0,
    )
    .await
    .expect("dark page");
    assert!(dark.is_empty(), "no spaces means no vector leg");

    // Leave the first org caught up so the only org behind below is the one
    // this test switches. (The queue is instance-wide by design.)
    embed(&mut tx, org, a, &new_space, &axis(0)).await;
    tx.commit().await.expect("commit");

    // ------------------------------------------------------------------
    // The switch lifecycle: record, queue, lease, resume, finish, go dark.
    // ------------------------------------------------------------------
    let org = seed_org(&db, SLOT_ORG_SWITCH).await;
    let old_space = untagged_space("nomic-embed-text");
    let new_space = shipped_space();
    let rows = [
        fixed_uuid(SLOT_SWITCH_ROWS),
        fixed_uuid(SLOT_SWITCH_ROWS + 1),
        fixed_uuid(SLOT_SWITCH_ROWS + 2),
    ];

    let mut tx = org_tx(&db.app, org).await;
    for (i, id) in rows.iter().enumerate() {
        index_ticket(&mut tx, org, *id, "Relay probe", "network relay body").await;
        // Every row was embedded by the old model before the switch.
        let mut v = vec![0.0_f32; SEARCH_EMBEDDING_DIM];
        v[i] = 1.0;
        embed(&mut tx, org, *id, &old_space, &v).await;
    }
    tx.commit().await.expect("seed rows");

    let mut conn = db.admin.acquire().await.expect("owner connection");

    // First boot after the migration: nothing recorded, nothing to re-index.
    let first = embedding_space::record_switch(&mut conn, &old_space, false)
        .await
        .expect("record first space")
        .expect("a first boot records a switch");
    assert_eq!(first.from_space, None);
    assert!(!first.previous_readable);
    let state: SpaceState = embedding_space::read_state(&db.admin).await.expect("state");
    assert_eq!(state.target_space.as_deref(), Some(old_space.as_str()));
    assert_eq!(state.previous_space, None);
    assert_eq!(state.switch_id, Some(first.switch_id));
    assert!(state.switched_at.is_some());

    // The operator changes the model to another build of the same id: the old
    // population stays readable while the re-index runs.
    let switch = embedding_space::record_switch(&mut conn, &new_space, true)
        .await
        .expect("record switch")
        .expect("a model change is a switch");
    assert_eq!(switch.from_space.as_deref(), Some(old_space.as_str()));
    assert!(switch.previous_readable);
    assert_eq!(
        embedding_space::read_spaces(&db.admin)
            .await
            .expect("spaces"),
        vec![new_space.clone(), old_space.clone()],
        "target first, previous second"
    );

    // A restart that resolves the same target is not a re-index.
    assert!(
        embedding_space::record_switch(&mut conn, &new_space, true)
            .await
            .expect("idempotent record")
            .is_none(),
        "same target writes nothing"
    );

    // The worker queues one run per org that is behind, and a second sweep
    // converges instead of double-queueing.
    let queued = embedding_space::enqueue_missing_runs(&db.admin, switch.switch_id, &new_space)
        .await
        .expect("enqueue");
    assert_eq!(queued, 1, "one org is behind");
    let again = embedding_space::enqueue_missing_runs(&db.admin, switch.switch_id, &new_space)
        .await
        .expect("enqueue again");
    assert_eq!(again, 0, "idempotent on (switch_id, org_id)");
    assert_eq!(
        embedding_space::runs_pending(&db.admin)
            .await
            .expect("pending"),
        1
    );

    let run = embedding_space::next_run(&db.admin)
        .await
        .expect("next run")
        .expect("a queued run");
    assert_eq!(run.org_id, org);
    assert_eq!(run.to_space, new_space);
    assert_eq!(run.from_space.as_deref(), Some(old_space.as_str()));
    assert_eq!(run.total_rows, 3, "the denominator counts the org's rows");

    // A claim is exclusive: a second sweep gets `false` rather than working
    // the same org twice.
    assert!(embedding_space::lease(&mut conn, run.id)
        .await
        .expect("lease"));
    assert!(
        !embedding_space::lease(&mut conn, run.id)
            .await
            .expect("second lease"),
        "a held lease is not claimable"
    );

    // Pass one: one batch, then the count the pass reports. The rate ceiling
    // is a pause between passes, so both passes are bounded by the batch.
    let mut tx = org_tx(&db.app, org).await;
    embed(&mut tx, org, rows[0], &new_space, &axis(0)).await;
    let remaining = search::count_remaining_embeddings(&mut *tx, org, &new_space)
        .await
        .expect("remaining");
    assert_eq!(remaining, 2, "one row moved into the target space");
    tx.commit().await.expect("commit batch");
    let progress: RunProgress =
        embedding_space::record_progress(&mut conn, run.id, 3, remaining, None)
            .await
            .expect("progress");
    assert_eq!(progress.done_rows, 1);
    let running = embedding_space::list_runs(&db.admin, org, 5)
        .await
        .expect("list runs");
    assert_eq!(
        running[0].status, "running",
        "unfinished work stays running"
    );
    assert_eq!(running[0].done_rows, 1);

    // Crash here. The run is still due (the lease was released by the progress
    // write) and the predicate names exactly the two rows still missing — no
    // cursor, no redo of the row already written.
    let resumed = embedding_space::next_run(&db.admin)
        .await
        .expect("next run after a pass")
        .expect("the same run is due again");
    assert_eq!(resumed.id, run.id);
    assert!(embedding_space::lease(&mut conn, run.id)
        .await
        .expect("re-lease"));
    let mut tx = org_tx(&db.app, org).await;
    let due = search::rows_needing_embedding(&mut *tx, org, &new_space, 128)
        .await
        .expect("sweep predicate");
    let mut due_ids: Vec<Uuid> = due.iter().map(|target| target.entity_id).collect();
    due_ids.sort();
    let mut expected = vec![rows[1], rows[2]];
    expected.sort();
    assert_eq!(due_ids, expected, "the predicate is the resume point");

    // Pass two finishes the org. In the real path this is core-api's one
    // transaction: write the last vectors, drop the previous space, then mark
    // the run complete and clear the readable window.
    embed(&mut tx, org, rows[1], &new_space, &axis(1)).await;
    embed(&mut tx, org, rows[2], &new_space, &axis(2)).await;
    let remaining = search::count_remaining_embeddings(&mut *tx, org, &new_space)
        .await
        .expect("remaining");
    assert_eq!(remaining, 0);
    let deleted = search::delete_space_embeddings(&mut *tx, org, &old_space)
        .await
        .expect("delete previous space");
    assert_eq!(
        deleted, 3,
        "the previous space's rows stop occupying the table"
    );
    tx.commit().await.expect("commit completion");
    let progress = embedding_space::record_progress(&mut conn, run.id, 3, 0, None)
        .await
        .expect("complete");
    assert_eq!(progress.done_rows, 3);
    embedding_space::clear_previous_when_done(&db.admin)
        .await
        .expect("clear previous");
    assert_eq!(
        embedding_space::read_spaces(&db.admin)
            .await
            .expect("spaces"),
        vec![new_space.clone()],
        "the readable window closes when the last run finishes"
    );
    assert_eq!(
        embedding_space::org_spaces(&db.admin, org)
            .await
            .expect("org spaces"),
        vec![new_space.clone()],
        "one space per org once the re-index is done"
    );
    assert_eq!(
        embedding_space::runs_pending(&db.admin)
            .await
            .expect("pending"),
        0
    );

    // A later switch to a DIFFERENT model id is not comparable: the previous
    // space is not kept readable, so an un-reindexed row is a miss with a
    // gauge instead of a plausible ranking.
    let dark_space = space_for_model(&Catalogue::builtin(), "embeddinggemma-2:270m-bf16-text").id();
    let dark = embedding_space::record_switch(&mut conn, &dark_space, false)
        .await
        .expect("model change")
        .expect("a different model is a switch");
    assert!(!dark.previous_readable);
    assert_eq!(
        embedding_space::read_spaces(&db.admin)
            .await
            .expect("spaces"),
        vec![dark_space.clone()],
        "a different model id leaves the old population unreadable"
    );
    let queued = embedding_space::enqueue_missing_runs(&db.admin, dark.switch_id, &dark_space)
        .await
        .expect("enqueue after model change");
    assert!(
        queued >= 1,
        "every org holding rows is behind again, and that is visible"
    );
    let runs = embedding_space::list_runs(&db.admin, org, 5)
        .await
        .expect("runs for this org");
    assert_eq!(runs.len(), 2, "one run per switch for this org");
    assert_eq!(runs[0].switch_id, dark.switch_id, "newest first");
    assert_eq!(runs[0].status, "queued");
}
