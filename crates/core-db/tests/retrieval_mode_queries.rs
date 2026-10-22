//! The retrieval mode, proved against a real Postgres (Wave 52 lane R15,
//! migration 0153).
//!
//! The row's requirements are all database facts, so all of them run here:
//!
//! 1. `exact` is the default and its result set does not change when an ANN
//!    index exists. That is the one direction the row forbids: an operator who
//!    built an index and then set `exact` must still get the true nearest
//!    neighbours, not a graph walk with a tenant filter.
//! 2. `iterative` refuses before its index exists, names the missing thing, and
//!    is satisfied once the migrate path builds the index.
//! 3. `index` refuses on a database with no VectorChord, names the extension
//!    and the preload, and the ensure function skips instead of failing the
//!    migration.
//! 4. The per-tenant distribution reports every tenant through the
//!    `SECURITY DEFINER` function, while a direct read as the app role sees
//!    only its own org.
//!
//! One test owns the whole lifecycle, the way `embedding_space_queries.rs`
//! does: the HNSW index is a database-wide object, so a second test running
//! beside this one would create or drop it underneath the first.
//!
//! Slot discipline: fixed-UUID slots 94700–94799 only.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::retrieval::{self, EnsureReport};
use core_db::{orgs, search};
use core_domain::embedding::{space_for_model, Catalogue};
use core_domain::retrieval::{Missing, RetrievalMode, HNSW_INDEX, VCHORD_INDEX};
use core_domain::SEARCH_EMBEDDING_DIM;
use uuid::Uuid;

const SLOT_ORG_A: u64 = 94_700;
const SLOT_ORG_B: u64 = 94_701;
const SLOT_ROWS_A: u64 = 94_710;
const SLOT_ROWS_B: u64 = 94_750;

/// Rows in the big tenant. Enough that a plan has something to choose between,
/// small enough that the HNSW build finishes inside a test.
const ORG_A_ROWS: usize = 40;
/// Rows in the small tenant, which is the shape that must stay on `exact`.
const ORG_B_ROWS: usize = 12;

fn shipped_space() -> String {
    space_for_model(&Catalogue::builtin(), "embeddinggemma-2:270m-q8_0-text").id()
}

async fn seed_org(db: &common::TestDb, slot: u64) -> Uuid {
    let org = fixed_uuid(slot);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        &format!("Retrieval mode org {slot}"),
        &format!("retrieval-mode-{slot}"),
        "RAG",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit org");
    org
}

/// A vector whose cosine distance to [`query_axis`] grows with `rank`, so the
/// true nearest-neighbour order is knowable without asking the database.
///
/// The angle step is 0.01 rad, which separates adjacent rows by about 0.004 in
/// cosine similarity — far wider than f32 rounding, so the expected order is
/// not a coin flip between two rows at the same distance.
fn ranked_vector(rank: usize) -> Vec<f32> {
    let theta = (rank as f32) * 0.01;
    let mut v = vec![0.0_f32; SEARCH_EMBEDDING_DIM];
    v[0] = theta.cos();
    v[1] = theta.sin();
    v
}

fn query_axis() -> Vec<f32> {
    let mut v = vec![0.0_f32; SEARCH_EMBEDDING_DIM];
    v[0] = 1.0;
    v
}

async fn index_and_embed(
    db: &common::TestDb,
    org: Uuid,
    base_slot: u64,
    rows: usize,
    space: &str,
) -> Vec<Uuid> {
    let mut ids = Vec::with_capacity(rows);
    let mut tx = org_tx(&db.app, org).await;
    for rank in 0..rows {
        let id = fixed_uuid(base_slot + rank as u64);
        search::upsert_row(
            &mut *tx,
            org,
            search::SearchKind::Ticket,
            id,
            None,
            &format!("Retrieval row {rank}"),
            "body",
            None,
        )
        .await
        .expect("index row");
        let model = space.split('@').next().unwrap_or(space);
        let stored = search::upsert_embedding(
            &mut *tx,
            org,
            search::SearchKind::Ticket,
            id,
            space,
            model,
            &ranked_vector(rank),
        )
        .await
        .expect("embedding upsert");
        assert!(stored, "the index row must exist before its vector");
        ids.push(id);
    }
    tx.commit().await.expect("commit rows");
    ids
}

async fn ensure(db: &common::TestDb, mode: RetrievalMode) -> EnsureReport {
    retrieval::ensure_index(&db.admin, mode)
        .await
        .unwrap_or_else(|error| panic!("ensure_index({}) failed: {error}", mode.as_str()))
}

fn missing_names(report: &core_domain::retrieval::RetrievalRefusal) -> Vec<String> {
    report.missing.iter().map(ToString::to_string).collect()
}

#[tokio::test]
async fn the_retrieval_mode_is_probed_refused_and_measured_against_a_real_postgres() {
    let db = test_db().await;
    let space = shipped_space();
    let org_a = seed_org(&db, SLOT_ORG_A).await;
    let org_b = seed_org(&db, SLOT_ORG_B).await;
    let ids_a = index_and_embed(&db, org_a, SLOT_ROWS_A, ORG_A_ROWS, &space).await;
    let _ids_b = index_and_embed(&db, org_b, SLOT_ROWS_B, ORG_B_ROWS, &space).await;

    // ------------------------------------------------------------------
    // 1. `exact` needs nothing, and it is the mode a stock database runs
    // ------------------------------------------------------------------
    let probe = retrieval::probe(&db.admin, RetrievalMode::Exact)
        .await
        .expect("probe the database");
    assert!(
        probe.verdict.is_ok(),
        "exact must be satisfiable on any database: {:?}",
        probe.verdict
    );
    assert!(
        probe.capabilities.pgvector_version.is_some(),
        "the migrated schema needs the vector extension, so the probe must see it"
    );

    // The vector leg before any index exists. This is the population the
    // `exact` result is compared against later.
    let before_index = {
        let mut tx = org_tx(&db.app, org_a).await;
        let keys = search::vector_candidates(
            &mut tx,
            org_a,
            &["ticket".to_string()],
            &space,
            &query_axis(),
            None,
            10,
            RetrievalMode::Exact,
        )
        .await
        .expect("exact vector leg");
        // The mode is scoped to the statement: hydration runs after the leg in
        // the same transaction and must not be forced into a sequential scan.
        let indexscan: String = sqlx::query_scalar("SELECT current_setting('enable_indexscan')")
            .fetch_one(&mut *tx)
            .await
            .expect("read the setting");
        tx.commit().await.expect("commit");
        assert_eq!(
            indexscan, "on",
            "the exact mode must restore index scans for the rest of the transaction"
        );
        keys.into_iter().map(|k| k.entity_id).collect::<Vec<_>>()
    };
    assert_eq!(
        before_index.len(),
        10,
        "exact returns a full page; the graph-walk hazard returns a fraction of one"
    );
    assert_eq!(
        before_index,
        ids_a[..10].to_vec(),
        "exact must return the true nearest neighbours in distance order"
    );

    // ------------------------------------------------------------------
    // 2. `iterative` refuses until the migrate path builds its index
    // ------------------------------------------------------------------
    let refused = retrieval::probe(&db.admin, RetrievalMode::Iterative)
        .await
        .expect("probe");
    let refusal = refused
        .verdict
        .expect_err("iterative has no index yet, so it must refuse");
    assert_eq!(refusal.mode, RetrievalMode::Iterative);
    assert!(
        refusal
            .missing
            .contains(&Missing::Index { name: HNSW_INDEX }),
        "the refusal must name the index it needs: {:?}",
        missing_names(&refusal)
    );
    let message = refusal.to_string();
    assert!(message.contains("search_embeddings_hnsw"), "{message}");
    assert!(
        message.contains("will not fall back to the exact scan"),
        "{message}"
    );

    let report = ensure(&db, RetrievalMode::Iterative).await;
    assert_eq!(report.index, Some(HNSW_INDEX));
    assert_eq!(report.detail, "present", "stock pgvector 0.8 builds HNSW");
    assert!(report.present, "the index exists after the migrate step");

    let satisfied = retrieval::probe(&db.admin, RetrievalMode::Iterative)
        .await
        .expect("probe");
    assert!(
        satisfied.verdict.is_ok(),
        "iterative must be satisfiable once its index exists: {:?}",
        satisfied.verdict
    );
    assert!(
        satisfied.capabilities.has_index(HNSW_INDEX),
        "the probe finds the index by name"
    );

    // The index being built is not allowed to change what `exact` returns.
    let after_index = {
        let mut tx = org_tx(&db.app, org_a).await;
        let keys = search::vector_candidates(
            &mut tx,
            org_a,
            &["ticket".to_string()],
            &space,
            &query_axis(),
            None,
            10,
            RetrievalMode::Exact,
        )
        .await
        .expect("exact vector leg with an index present");
        tx.commit().await.expect("commit");
        keys.into_iter().map(|k| k.entity_id).collect::<Vec<_>>()
    };
    assert_eq!(
        after_index, before_index,
        "an ANN index existing must not change the exact mode's rows"
    );

    // And the indexed mode gives the same page at this size, which is why the
    // mode is a deployment decision rather than a correctness one.
    let iterative_rows = {
        let mut tx = org_tx(&db.app, org_a).await;
        let keys = search::vector_candidates(
            &mut tx,
            org_a,
            &["ticket".to_string()],
            &space,
            &query_axis(),
            None,
            10,
            RetrievalMode::Iterative,
        )
        .await
        .expect("iterative vector leg");
        tx.commit().await.expect("commit");
        keys.into_iter().map(|k| k.entity_id).collect::<Vec<_>>()
    };
    assert_eq!(
        iterative_rows, before_index,
        "at 10 of 40 rows the iterative scan returns the same page; recall only \
         diverges as a tenant grows"
    );

    // ------------------------------------------------------------------
    // 3. `index` refuses here, and the migrate step records why
    // ------------------------------------------------------------------
    let no_vchord = match &retrieval::probe(&db.admin, RetrievalMode::Index).await {
        Ok(probe) => probe.capabilities.clone(),
        Err(error) => panic!("probe: {error}"),
    };
    if !no_vchord.vchord_installed {
        let skipped = ensure(&db, RetrievalMode::Index).await;
        assert_eq!(skipped.index, Some(VCHORD_INDEX));
        assert!(
            skipped.detail.starts_with("skipped:"),
            "a stock database must skip rather than fail the migration: {}",
            skipped.detail
        );
        assert!(
            !skipped.present,
            "no VectorChord index is created on a database without the extension"
        );

        let refusal = retrieval::probe(&db.admin, RetrievalMode::Index)
            .await
            .expect("probe")
            .verdict
            .expect_err("index cannot run without vchord");
        assert!(refusal.missing.contains(&Missing::Extension {
            name: "vchord",
            available: false
        }));
        assert!(refusal
            .missing
            .contains(&Missing::Preload { name: "vchord" }));
        assert!(refusal
            .missing
            .contains(&Missing::Index { name: VCHORD_INDEX }));
        let message = refusal.to_string();
        assert!(message.contains("`vchord` extension"), "{message}");
        assert!(message.contains("shared_preload_libraries"), "{message}");
        assert!(
            message.contains("will not fall back to the exact scan"),
            "{message}"
        );
    }

    // ------------------------------------------------------------------
    // 4. The distribution sees every tenant; the app role does not
    // ------------------------------------------------------------------
    let distribution = retrieval::embedding_distribution(&db.admin)
        .await
        .expect("distribution");
    let find = |org: Uuid| {
        distribution
            .iter()
            .find(|t| t.org_id == org)
            .map(|t| t.embeddings)
    };
    assert_eq!(find(org_a), Some(ORG_A_ROWS as i64));
    assert_eq!(find(org_b), Some(ORG_B_ROWS as i64));
    assert!(
        distribution.len() >= 2,
        "the report lists every tenant with vectors, not a single total"
    );
    assert!(
        distribution
            .windows(2)
            .all(|w| w[0].embeddings >= w[1].embeddings),
        "the report is ordered largest first so the tenant that matters is at the top"
    );

    // Inside one org's RLS scope the app role sees only that org — which is
    // exactly why the report needs the SECURITY DEFINER function.
    let mut tx = org_tx(&db.app, org_a).await;
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM search_embeddings")
        .fetch_one(&mut *tx)
        .await
        .expect("RLS-scoped count");
    let through_function: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT org_id, embeddings FROM search_embedding_distribution()")
            .fetch_all(&mut *tx)
            .await
            .expect("the app role may read the distribution");
    tx.commit().await.expect("commit");
    assert_eq!(
        visible, ORG_A_ROWS as i64,
        "RLS scopes a direct read to the caller's org"
    );
    assert!(
        through_function.iter().any(|(org, _)| *org == org_b),
        "the function reports the other tenant too, which is the whole point"
    );
}

/// The `index` mode's own proof. It needs a database that carries VectorChord
/// and preloads it, which the stock test image does not:
///
/// ```text
/// docker run -d --name r15-vchord -e POSTGRES_PASSWORD=marula \
///   -e POSTGRES_USER=marula -e POSTGRES_DB=marula_r15 -p 15439:5432 \
///   timescale/timescaledb-ha:pg18
/// MARULA_TEST_DATABASE_URL=postgres://marula:marula@localhost:15439/marula_r15 \
///   cargo test -p core-db --test retrieval_mode_queries -- --ignored
/// ```
///
/// It is ignored rather than skipped because a skipped test reports nothing;
/// an operator running the gate names the database and sees the result.
#[tokio::test]
#[ignore = "needs Postgres with vchord preloaded (VectorChord image); see the doc comment"]
async fn index_mode_is_satisfied_and_returns_the_whole_small_tenant_page() {
    let db = test_db().await;
    let space = shipped_space();
    let org_small = seed_org(&db, SLOT_ORG_B).await;
    let small_ids = index_and_embed(&db, org_small, SLOT_ROWS_B, ORG_B_ROWS, &space).await;

    // An `index` deployment starts where this database starts: the image
    // carries the extension, the server preloads it, and nothing has installed
    // or indexed it yet. The boot refuses in that state, which is the point of
    // the probe — the operator reads why before `migrate` fixes it.
    let before = retrieval::probe(&db.admin, RetrievalMode::Index)
        .await
        .expect("probe");
    assert!(
        before.capabilities.vchord_available,
        "this test needs a database whose image carries vchord: {:?}",
        before.capabilities
    );
    assert!(
        before.capabilities.preloads("vchord"),
        "vchord must be in shared_preload_libraries, or its index cannot be maintained"
    );
    assert!(
        before.verdict.is_err(),
        "index mode must refuse before the migrate step installs and indexes vchord: {:?}",
        before.verdict
    );

    // The migrate step: one call installs the extension and builds the index,
    // because migration 0153's function can see availability and preload in the
    // same transaction it runs the DDL in.
    let report = ensure(&db, RetrievalMode::Index).await;
    assert_eq!(report.index, Some(VCHORD_INDEX));
    assert!(report.present, "the index exists: {}", report.detail);
    assert_eq!(
        report.detail, "present",
        "the function built it: {}",
        report.detail
    );

    let satisfied = retrieval::probe(&db.admin, RetrievalMode::Index)
        .await
        .expect("probe");
    assert!(
        satisfied.capabilities.vchord_installed,
        "the migrate step installed the extension: {:?}",
        satisfied.capabilities
    );
    assert!(
        satisfied.verdict.is_ok(),
        "index mode must be satisfiable here: {:?}",
        satisfied.verdict
    );

    let rows = {
        let mut tx = org_tx(&db.app, org_small).await;
        let keys = search::vector_candidates(
            &mut tx,
            org_small,
            &["ticket".to_string()],
            &space,
            &query_axis(),
            None,
            10,
            RetrievalMode::Index,
        )
        .await
        .expect("index vector leg");
        tx.commit().await.expect("commit");
        keys.into_iter().map(|k| k.entity_id).collect::<Vec<_>>()
    };
    assert_eq!(
        rows,
        small_ids[..10].to_vec(),
        "the mode returns the tenant's true nearest neighbours"
    );

    // The rows alone do not prove the index ran: at twelve rows the planner's
    // cheaper plan is the tenant btree plus a sort, and every mode returns the
    // same page that way. With the scan and the sort both disabled, the only
    // way left to answer the ORDER BY is an index that orders by distance —
    // and the VectorChord index the migrate step built is it.
    let plan = {
        let mut tx = org_tx(&db.app, org_small).await;
        for setting in [
            "SET LOCAL enable_seqscan = off",
            "SET LOCAL enable_sort = off",
            "SET LOCAL vchordrq.prefilter = on",
        ] {
            sqlx::query(setting)
                .execute(&mut *tx)
                .await
                .expect("session setting");
        }
        let literal = query_axis()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let lines: Vec<String> = sqlx::query_scalar(&format!(
            "EXPLAIN SELECT e.entity_id FROM search_embeddings e \
             WHERE e.org_id = '{org_small}' AND e.entity_kind = 'ticket' \
               AND e.space = '{space}' \
             ORDER BY e.embedding <=> '[{literal}]'::vector LIMIT 10"
        ))
        .fetch_all(&mut *tx)
        .await
        .expect("explain");
        tx.commit().await.expect("commit");
        lines.join("\n")
    };
    assert!(
        plan.contains(VCHORD_INDEX),
        "the order-by is served by the VectorChord index: {plan}"
    );
}
