//! Live-database tests for the search indexer's resolve/flush pipeline.
//!
//! Purpose: prove event→target mapping resolves CURRENT rows into correct
//! index ops — including NotFound ⇒ Delete — and that flush batches are
//! deduplicated and grouped per org. Uses the same documented database
//! selection contract as core-db's suite (`MARULA_TEST_DATABASE_URL`,
//! else `DATABASE_URL`); seed slots stay inside the search lane's private
//! 120–129 range.
//!
//! Non-purpose: HTTP transport behavior of `HttpIndexSink` (exercised by
//! the stack smoke test) and SQL-layer query semantics (covered in
//! core-db's db_suite).

use sqlx::{PgPool, Postgres};
use uuid::Uuid;

/// Reset+migrate ONCE per process (tests share one database); pools are
/// created PER CALL because each #[tokio::test] owns its own tokio runtime
/// and a pool must not outlive the runtime that spawned its connection
/// tasks — same contract as core-db's harness.
async fn test_pool() -> PgPool {
    static BOOTSTRAP: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let url = BOOTSTRAP
        .get_or_init(|| async {
            let url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MARULA_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
                     Postgres for worker indexer tests",
                );
            let admin = core_db::connect(&url).await.expect("connect");
            sqlx::query("DROP SCHEMA public CASCADE")
                .execute(&admin)
                .await
                .expect("drop schema");
            sqlx::query("CREATE SCHEMA public")
                .execute(&admin)
                .await
                .expect("create schema");
            core_db::run_migrations(&admin).await.expect("migrations");
            url
        })
        .await;
    core_db::connect(url).await.expect("pool")
}

fn fixed_uuid(n: u64) -> Uuid {
    Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | u128::from(n))
}

/// RLS-scoped transaction helper (mirrors core-db test harness).
async fn org_tx(app: &PgPool, org_id: Uuid) -> sqlx::Transaction<'_, Postgres> {
    let mut tx = app.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    tx
}

struct Seeded {
    org: Uuid,
    board: Uuid,
    entry_status: Uuid,
}

/// Seeds one org with default board/statuses (defaults-first doctrine).
async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    {
        let mut tx = org_tx(app, org).await;
        core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("acme-{n}"), "MER")
            .await
            .expect("org");
        core_db::orgs::seed_default_roles(&mut tx, org)
            .await
            .expect("roles");
        core_db::orgs::seed_default_board(&mut tx, org)
            .await
            .expect("board");
        core_db::orgs::seed_default_sla_policy(&mut *tx, org)
            .await
            .expect("sla");
        tx.commit().await.expect("commit");
    }
    let mut tx = org_tx(app, org).await;
    let board: Uuid = sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1 AND is_default")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .expect("default board");
    let entry_status: Uuid =
        sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry")
            .bind(board)
            .fetch_one(&mut *tx)
            .await
            .expect("entry status");
    tx.commit().await.expect("commit");
    Seeded {
        org,
        board,
        entry_status,
    }
}

/// Records every sink batch so tests can assert orchestration behavior
/// without any HTTP.
#[derive(Default)]
struct RecordingSink {
    batches: std::sync::Mutex<Vec<(Uuid, Vec<worker::search_indexer::IndexOp>)>>,
}

impl RecordingSink {
    fn recorded(&self) -> Vec<(Uuid, Vec<worker::search_indexer::IndexOp>)> {
        self.batches.lock().expect("mutex").clone()
    }
}

impl worker::search_indexer::IndexSink for RecordingSink {
    async fn apply(
        &self,
        org_id: Uuid,
        ops: Vec<worker::search_indexer::IndexOp>,
    ) -> anyhow::Result<()> {
        self.batches.lock().expect("mutex").push((org_id, ops));
        Ok(())
    }
}

use worker::search_indexer::flush;
use worker::search_indexer::{match_event, resolve_target, IndexOp, Target};

fn envelope(event_type: &str, org_id: Uuid, entity_id: Uuid) -> core_events::Envelope {
    core_events::Envelope {
        id: Uuid::now_v7(),
        event_type: event_type.to_string(),
        org_id,
        entity: "test".into(),
        entity_id: Some(entity_id.to_string()),
        occurred_at: chrono::Utc::now(),
        payload_schema_ref: "core-domain/v1".into(),
        payload: serde_json::json!({}),
    }
}

async fn insert_ticket(pool: &PgPool, s: &Seeded, id: Uuid, title: &str, body: &str) {
    let mut tx = org_tx(pool, s.org).await;
    let (prefix, seq): (String, i64) = core_db::orgs::next_ticket_number(&mut *tx, s.org)
        .await
        .expect("number");
    core_db::tickets::insert_ticket(
        &mut tx,
        core_db::tickets::NewTicket {
            id,
            org_id: s.org,
            number: &format!("{prefix}-{seq}"),
            board_id: s.board,
            status_id: s.entry_status,
            title,
            body: Some(body),
            priority: core_domain::status::Priority::P3,
            client_id: None,
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket insert");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn ticket_events_resolve_to_fresh_upserts_then_delete_when_gone() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 128).await;
    let ticket = fixed_uuid(120);

    // created → upsert with current content
    insert_ticket(
        &pool,
        &s,
        ticket,
        "VPN tunnel flapping",
        "wan circuit errors",
    )
    .await;
    let env = envelope("ticket.created", s.org, ticket);
    let target = match_event(&env).expect("maps to ticket target");
    let op = resolve_target(&pool, s.org, target)
        .await
        .expect("resolve ok")
        .expect("row exists ⇒ upsert");
    match op {
        IndexOp::Upsert {
            client_scope: _,
            entity_kind,
            entity_id,
            parent_id,
            title,
            body,
        } => {
            assert_eq!(entity_kind, "ticket");
            assert_eq!(entity_id, ticket);
            assert_eq!(parent_id, None);
            assert_eq!(title, "VPN tunnel flapping");
            assert_eq!(body, "wan circuit errors");
        }
        other => panic!("expected upsert, got {other:?}"),
    }

    // updated → same key, refreshed content (exactly-once at the index).
    {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::tickets::update_ticket(
            &mut *tx,
            s.org,
            ticket,
            core_db::tickets::TicketPatch {
                title: Some("VPN tunnel stable again"),
                body: Some(Some("monitoring confirms clean")),
                priority: None,
                assignee_id: None,
                client_id: None,
                site_id: None,
                contact_id: None,
                due_at: None,
                type_id: None,
                category_id: None,
            },
        )
        .await
        .expect("patch");
        tx.commit().await.expect("commit");
    }
    let target = match_event(&envelope("ticket.updated", s.org, ticket)).expect("target");
    let op = resolve_target(&pool, s.org, target)
        .await
        .expect("ok")
        .expect("still exists");
    assert!(
        matches!(&op, IndexOp::Upsert { title, .. } if title == "VPN tunnel stable again"),
        "second resolve carries refreshed content"
    );

    // Source deleted → resolver reports None ⇒ flush turns it into Delete.
    {
        let mut tx = org_tx(&pool, s.org).await;
        sqlx::query("DELETE FROM tickets WHERE id = $1")
            .bind(ticket)
            .execute(&mut *tx)
            .await
            .expect("delete");
        tx.commit().await.expect("commit");
    }
    let target = Target::Ticket(ticket);
    let resolved = resolve_target(&pool, s.org, target.clone())
        .await
        .expect("ok");
    assert!(resolved.is_none(), "gone row resolves to nothing");

    let sink = RecordingSink::default();
    flush(&pool, &sink, vec![(s.org, target)])
        .await
        .expect("flush");
    let batches = sink.recorded();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].0, s.org);
    assert_eq!(
        batches[0].1,
        vec![IndexOp::Delete {
            entity_kind: "ticket".into(),
            entity_id: ticket
        }]
    );
}

#[tokio::test]
async fn flush_dedupes_repeats_groups_orgs_and_indexes_comment_parentage() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 129).await;

    // Comment under a ticket: parent_id must carry the ticket id.
    let ticket = fixed_uuid(125);
    insert_ticket(&pool, &s, ticket, "Email relay down", "").await;
    let comment = fixed_uuid(126);
    {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::tickets::insert_comment(
            &mut *tx,
            core_db::tickets::NewTicketComment {
                id: comment,
                org_id: s.org,
                ticket_id: ticket,
                author_id: None,
                kind: core_domain::status::NoteKind::Internal,
                body: "customer confirmed router rebooted cleanly overnight",
                workflow_meta: None,
            },
        )
        .await
        .expect("comment insert");
        tx.commit().await.expect("commit");
    }
    let op = resolve_target(&pool, s.org, Target::Comment(comment))
        .await
        .expect("ok")
        .expect("exists");
    match op {
        IndexOp::Upsert {
            entity_kind,
            parent_id,
            body,
            ..
        } => {
            assert_eq!(entity_kind, "comment");
            assert_eq!(parent_id, Some(ticket));
            assert!(body.contains("router rebooted"));
        }
        other => panic!("expected comment upsert, got {other:?}"),
    }

    // Duplicate events for the same entity collapse into one op per org.
    let dupes = vec![
        (s.org, Target::Comment(comment)),
        (s.org, Target::Comment(comment)),
    ];
    let sink = RecordingSink::default();
    flush(&pool, &sink, dupes).await.expect("flush");
    let batches = sink.recorded();
    assert_eq!(batches.len(), 1, "one org ⇒ one batch");
    assert_eq!(
        batches[0].1.len(),
        1,
        "duplicate targets collapse to one op"
    );
}

/// Native KB v1: article events resolve CURRENT content into `kb_article`
/// upserts, and the deleted event's NotFound leg drops the entry (2.16
/// pattern; seed slot 127 keeps this suite's fixed ids collision-free).
#[tokio::test]
async fn kb_article_events_index_content_and_deleted_drops_it() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 127).await;
    let article = fixed_uuid(122);
    let author = fixed_uuid(123);

    // Seed the article row through core-db (created_by has a users FK).
    let mut tx = org_tx(&pool, s.org).await;
    core_db::users::create_user(
        &mut *tx,
        author,
        s.org,
        "indexer-kb@acme.test",
        "KB Author",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("author insert");
    let article = core_db::kb::create_article(
        &mut *tx,
        s.org,
        article,
        None,
        "Rotate the Wifi password",
        "rotate-the-wifi-password",
        "Quarterly, from the vault.",
        None,
        core_domain::entities::KbArticleStatus::Published,
        true,
        author,
    )
    .await
    .expect("article insert");
    tx.commit().await.expect("commit");

    // created → upsert carrying title + body, category parentage (None).
    let env = envelope("kb_article.created", s.org, article.id);
    let target = match_event(&env).expect("maps to article target");
    let op = resolve_target(&pool, s.org, target)
        .await
        .expect("resolve ok")
        .expect("row exists ⇒ upsert");
    match op {
        IndexOp::Upsert {
            client_scope: _,
            entity_kind,
            entity_id,
            parent_id,
            title,
            body,
        } => {
            assert_eq!(entity_kind, "kb_article");
            assert_eq!(entity_id, article.id);
            assert_eq!(parent_id, None);
            assert_eq!(title, "Rotate the Wifi password");
            assert_eq!(body, "Quarterly, from the vault.");
        }
        other => panic!("expected upsert, got {other:?}"),
    }

    // deleted → resolver NotFound leg ⇒ flush turns it into Delete.
    {
        let mut tx = org_tx(&pool, s.org).await;
        assert!(core_db::kb::delete_article(&mut *tx, s.org, article.id)
            .await
            .expect("delete"));
        tx.commit().await.expect("commit");
    }
    let target = Target::Article(article.id);
    assert!(resolve_target(&pool, s.org, target.clone())
        .await
        .expect("ok")
        .is_none());

    let sink = RecordingSink::default();
    flush(&pool, &sink, vec![(s.org, target)])
        .await
        .expect("flush");
    assert_eq!(
        sink.recorded()[0].1,
        vec![IndexOp::Delete {
            entity_kind: "kb_article".into(),
            entity_id: article.id
        }]
    );
}

/// KB indexing visibility gate (update: Wave 15 lane 4). The staff-only
/// unified index carries DRAFT and PUBLISHED articles — a draft indexes on
/// save so the web /kb FTS preview (which queries /api/search) resolves
/// for the working article; ARCHIVED entries drop again (flush ⇒ Delete).
/// Portal stays gated at its own read layer (published + client-visible),
/// never through this index.
#[tokio::test]
async fn draft_articles_index_and_archived_drop_the_entry() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 121).await;
    let article = fixed_uuid(124);
    let author = fixed_uuid(125);

    let mut tx = org_tx(&pool, s.org).await;
    core_db::users::create_user(
        &mut *tx,
        author,
        s.org,
        "gate-kb@acme.test",
        "Gate Author",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("author insert");
    let article = core_db::kb::create_article(
        &mut *tx,
        s.org,
        article,
        None,
        "Draft runbook",
        "draft-runbook",
        "Work in progress.",
        None,
        core_domain::entities::KbArticleStatus::Draft,
        true,
        author,
    )
    .await
    .expect("article insert");
    tx.commit().await.expect("commit");

    // Draft: the resolver UPserts — the draft is staff-searchable (the
    // /kb FTS preview reads this same index) and portal never consults
    // it, so draft visibility stays staff-only.
    let draft_op = resolve_target(&pool, s.org, Target::Article(article.id))
        .await
        .expect("ok")
        .expect("draft ⇒ upsert");
    assert!(matches!(draft_op, IndexOp::Upsert { .. }));
    // Draft content is upserted verbatim at save time.
    match &draft_op {
        IndexOp::Upsert {
            entity_id,
            title,
            body,
            ..
        } => {
            assert_eq!(*entity_id, article.id);
            assert_eq!(title, "Draft runbook");
            assert_eq!(body, "Work in progress.");
        }
        other => panic!("expected upsert, got {other:?}"),
    }

    // Publish flip → the same event stream re-resolves to a fresh upsert.
    {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::kb::update_article(
            &mut *tx,
            s.org,
            article.id,
            &core_db::kb::ArticlePatch {
                status: Some(core_domain::entities::KbArticleStatus::Published),
                updated_by: Some(author),
                ..Default::default()
            },
        )
        .await
        .expect("publish");
        tx.commit().await.expect("commit");
    }
    let op = resolve_target(&pool, s.org, Target::Article(article.id))
        .await
        .expect("resolve ok")
        .expect("published ⇒ upsert");
    assert!(matches!(op, IndexOp::Upsert { .. }));

    // Archived again → the index entry must be dropped.
    {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::kb::update_article(
            &mut *tx,
            s.org,
            article.id,
            &core_db::kb::ArticlePatch {
                status: Some(core_domain::entities::KbArticleStatus::Archived),
                updated_by: Some(author),
                ..Default::default()
            },
        )
        .await
        .expect("archive");
        tx.commit().await.expect("commit");
    }
    let target = Target::Article(article.id);
    assert!(resolve_target(&pool, s.org, target.clone())
        .await
        .expect("ok")
        .is_none());
    let sink = RecordingSink::default();
    flush(&pool, &sink, vec![(s.org, target)])
        .await
        .expect("flush");
    assert_eq!(
        sink.recorded()[0].1,
        vec![IndexOp::Delete {
            entity_kind: "kb_article".into(),
            entity_id: article.id
        }]
    );
}

/// DocumentVersion entities (D4): document events index the CURRENT
/// version's content — a new version re-resolves to refreshed content.
#[tokio::test]
async fn document_events_index_the_current_version() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 126).await;
    let author = fixed_uuid(124);

    let (doc, _v1) = {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::users::create_user(
            &mut *tx,
            author,
            s.org,
            "indexer-docs@acme.test",
            "Doc Author",
            "argon2hash",
            "UTC",
        )
        .await
        .expect("author insert");
        let pair = core_db::documents::create_document(
            &mut tx,
            s.org,
            fixed_uuid(121),
            None,
            None,
            &serde_json::Value::Null,
            "Escalation policy",
            "P1 pages on-call.",
            "initial",
            author,
        )
        .await
        .expect("document + v1");
        tx.commit().await.expect("commit");
        pair
    };

    // created → upsert with the CURRENT version content.
    let env = envelope("document.created", s.org, doc.id);
    let target = match_event(&env).expect("maps to document target");
    let op = resolve_target(&pool, s.org, target)
        .await
        .expect("resolve ok")
        .expect("document exists");
    match op {
        IndexOp::Upsert {
            entity_kind,
            title,
            body,
            ..
        } => {
            assert_eq!(entity_kind, "document");
            assert_eq!(title, "Escalation policy");
            assert_eq!(body, "P1 pages on-call.");
        }
        other => panic!("expected upsert, got {other:?}"),
    }

    // An edit APPENDS a version; the indexer must pick up the new head.
    {
        let mut tx = org_tx(&pool, s.org).await;
        core_db::documents::insert_version(
            &mut *tx,
            s.org,
            doc.id,
            2,
            "Escalation policy v2",
            "P1 pages on-call. P2 alerts in-app.",
            "added P2",
            author,
            None,
        )
        .await
        .expect("v2 append");
        tx.commit().await.expect("commit");
    }
    let target = match_event(&envelope("document.updated", s.org, doc.id)).expect("target");
    let op = resolve_target(&pool, s.org, target)
        .await
        .expect("ok")
        .expect("still exists");
    assert!(
        matches!(&op, IndexOp::Upsert { title, body, .. }
            if title == "Escalation policy v2" && body.contains("P2 alerts in-app")),
        "second resolve carries the appended head content"
    );

    // restored also maps to the document target (re-index after restore).
    assert!(matches!(
        match_event(&envelope("document.restored", s.org, doc.id)),
        Some(Target::Document(_))
    ));
}

// --- P12 seam-share measurement ---------------------------------------------
//
// Ignored by default: this is the measured evidence for the P12 row ("every
// indexed row crosses the internal seam as its own HTTP call"). It seeds a
// bulk fixture, then prices three shapes against a counting stub that stands
// in for core-api's `/internal/search/*`:
//
//   * the production shape — `follow_stream` hands `flush` one XREAD reply of
//     up to COUNT entries, so N rows leave as ceil(N / COUNT) upsert POSTs;
//   * one batch of all N ops in a single call — the floor the seam can reach;
//   * one `flush` per target — the per-row shape the row fears.
//
// Run it with the exact command in design/qa-p52-internal-seam-batching/GATES.md.

/// A stub for core-api's internal search surface that counts the POSTs
/// `HttpIndexSink` makes. The real route answers a JSON body the sink
/// discards, so the stub only needs a 200 and a counter.
#[derive(Default)]
struct SeamTally {
    upsert_calls: std::sync::atomic::AtomicUsize,
    delete_calls: std::sync::atomic::AtomicUsize,
}

impl SeamTally {
    fn upserts(&self) -> usize {
        self.upsert_calls.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn deletes(&self) -> usize {
        self.delete_calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

type TallyState = std::sync::Arc<SeamTally>;

async fn tally_upsert(
    axum::extract::State(tally): axum::extract::State<TallyState>,
) -> axum::Json<serde_json::Value> {
    tally
        .upsert_calls
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    axum::Json(serde_json::json!({ "upserted": 1 }))
}

async fn tally_delete(
    axum::extract::State(tally): axum::extract::State<TallyState>,
) -> axum::Json<serde_json::Value> {
    tally
        .delete_calls
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    axum::Json(serde_json::json!({ "deleted": 1 }))
}

/// Serves the two internal routes on an ephemeral loopback port.
async fn seam_probe() -> (
    String,
    std::sync::Arc<SeamTally>,
    tokio::task::JoinHandle<()>,
) {
    let tally = std::sync::Arc::new(SeamTally::default());
    let router = axum::Router::new()
        .route("/internal/search/upsert", axum::routing::post(tally_upsert))
        .route("/internal/search/delete", axum::routing::post(tally_delete))
        .with_state(std::sync::Arc::clone(&tally));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind seam probe");
    let addr = listener.local_addr().expect("probe addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{addr}"), tally, server)
}

fn round_ms(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

#[ignore = "measurement (P12 seam share): needs MARULA_TEST_DATABASE_URL; see design/qa-p52-internal-seam-batching/GATES.md"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seam_share_over_a_bulk_seed() {
    use worker::search_indexer::{flush as flush_batch, HttpIndexSink, IndexSink};

    // Mirrors COUNT in crates/worker/src/search_indexer.rs:41 — the number of
    // stream entries one XREAD reply carries into a single flush.
    const STREAM_BATCH: usize = 200;

    let rows: usize = std::env::var("SEAM_ROWS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(2_000);
    let pool = test_pool().await;
    // Slot 122 is free in this file's 120–129 range; a shared org id would
    // collide when the whole suite runs with --include-ignored.
    let s = seed_org(&pool, 122).await;

    let mut targets: Vec<(Uuid, Target)> = Vec::with_capacity(rows);
    for index in 0..rows {
        let id = Uuid::now_v7();
        insert_ticket(
            &pool,
            &s,
            id,
            &format!("Seam ticket {index}"),
            "network outage on the west bridge",
        )
        .await;
        targets.push((s.org, Target::Ticket(id)));
    }

    // What the current rows resolve to, recorded without any HTTP.
    let recorder = RecordingSink::default();
    let start = std::time::Instant::now();
    flush_batch(&pool, &recorder, targets.clone())
        .await
        .expect("resolve into ops");
    let resolve_ms = elapsed_ms(start);
    let ops: Vec<IndexOp> = recorder
        .recorded()
        .into_iter()
        .flat_map(|(_, batch)| batch)
        .collect();
    assert_eq!(ops.len(), rows, "every seeded row resolves to one op");

    let (base_url, tally, server) = seam_probe().await;
    let sink = HttpIndexSink::new(&base_url, "seam-probe-token");

    // Floor: all N ops in one POST.
    let before = tally.upserts();
    let start = std::time::Instant::now();
    sink.apply(s.org, ops.clone()).await.expect("one big apply");
    let batched_apply_ms = elapsed_ms(start);
    let batched_apply_calls = tally.upserts() - before;
    assert_eq!(batched_apply_calls, 1, "one batch is one POST");

    // Production shape: one flush per XREAD-sized window, as follow_stream
    // runs it.
    let before = tally.upserts();
    let start = std::time::Instant::now();
    for window in targets.chunks(STREAM_BATCH) {
        flush_batch(&pool, &sink, window.to_vec())
            .await
            .expect("window flush");
    }
    let production_window_ms = elapsed_ms(start);
    let production_calls = tally.upserts() - before;
    let expected_windows = rows.div_ceil(STREAM_BATCH);
    assert_eq!(
        production_calls, expected_windows,
        "the production shape spends one POST per stream window, not one per row"
    );

    // The per-row shape the row describes: one resolve + one POST per entity.
    let before = tally.upserts();
    let start = std::time::Instant::now();
    for target in &targets {
        flush_batch(&pool, &sink, vec![target.clone()])
            .await
            .expect("per-row flush");
    }
    let per_row_flush_ms = elapsed_ms(start);
    let per_row_flush_calls = tally.upserts() - before;
    assert_eq!(per_row_flush_calls, rows, "the per-row shape is N POSTs");

    // Transport alone, with no database work: N single-op POSTs.
    let one_op = ops.first().expect("at least one op").clone();
    let start = std::time::Instant::now();
    for _ in 0..rows {
        sink.apply(s.org, vec![one_op.clone()])
            .await
            .expect("single apply");
    }
    let per_row_apply_ms = elapsed_ms(start);

    server.abort();

    let requests_per_row = production_calls as f64 / rows as f64;
    println!(
        "SEAM_SHARE {}",
        serde_json::json!({
            "rows": rows,
            "stream_batch": STREAM_BATCH,
            "resolve_ms": round_ms(resolve_ms),
            "resolve_ms_per_row": round_ms(resolve_ms / rows as f64),
            "batched_apply_ms": round_ms(batched_apply_ms),
            "batched_apply_calls": batched_apply_calls,
            "production_window_ms": round_ms(production_window_ms),
            "production_calls": production_calls,
            "production_calls_per_row": round_ms(requests_per_row),
            "production_ms_per_row": round_ms(production_window_ms / rows as f64),
            "per_row_flush_ms": round_ms(per_row_flush_ms),
            "per_row_flush_calls": per_row_flush_calls,
            "per_row_flush_ms_per_row": round_ms(per_row_flush_ms / rows as f64),
            "per_row_apply_ms": round_ms(per_row_apply_ms),
            "per_row_apply_ms_per_call": round_ms(per_row_apply_ms / rows as f64),
            "deletes": tally.deletes(),
        })
    );
}
