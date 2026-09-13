//! Live-database tests for the search indexer's resolve/flush pipeline.
//!
//! Purpose: prove event→target mapping resolves CURRENT rows into correct
//! index ops — including NotFound ⇒ Delete — and that flush batches are
//! deduplicated and grouped per org. Uses the same documented database
//! selection contract as core-db's suite (`MERIDIAN_TEST_DATABASE_URL`,
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
            let url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MERIDIAN_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
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
            comment,
            s.org,
            ticket,
            None,
            core_domain::status::NoteKind::Internal,
            "customer confirmed router rebooted cleanly overnight",
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

/// KB indexing visibility gate (Wave 13 lane 4): the unified index
/// mirrors PUBLISHED articles only — a draft stays out, the publish flip
/// mirrors it, and the archive flip drops the entry again (flush ⇒
/// Delete).
#[tokio::test]
async fn draft_and_archived_articles_stay_out_of_the_index() {
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
        core_domain::entities::KbArticleStatus::Draft,
        true,
        author,
    )
    .await
    .expect("article insert");
    tx.commit().await.expect("commit");

    // Draft: the resolver returns None, which flush() reads as the
    // delete leg — a draft never enters the index.
    assert!(resolve_target(&pool, s.org, Target::Article(article.id))
        .await
        .expect("ok")
        .is_none());

    // Publish flip → the same event stream re-resolves to an upsert.
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
