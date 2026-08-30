//! Knowledge-base query-layer tests (native KB v1, TASKLIST Phase 3) —
//! seed slots 130–139 are this suite's private range.
//!
//! Purpose: prove the SQL layer — category/article CRUD, the client-visible
//! gate join, filter composition, slug uniqueness, RLS isolation, and the
//! 0031-widened unified-search vocabulary (`kb_article`, `document`).
//!
//! Non-purpose: domain rules (core-kb unit tests) and HTTP behavior
//! (core-api kb_flow). Database selection follows the documented contract
//! in tests/common/mod.rs.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{kb, orgs, search, users, DbError};
use core_domain::entities::KbArticleStatus;
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@kb-acme.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

async fn seed_org(app: &sqlx::PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "KB Acme", &format!("kb-acme-{n}"), "KBX")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit seed");
    Seeded { org }
}

#[tokio::test]
async fn category_and_article_round_trip() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 130).await;
    let _author = seed_user(&app, &s, 132).await;
    let mut tx = org_tx(&app, s.org).await;

    let category = kb::create_category(
        &mut *tx,
        s.org,
        fixed_uuid(131),
        "How-Tos",
        "how-tos",
        "Step-by-step guides",
        2,
        true,
    )
    .await
    .expect("category insert");
    assert_eq!(category.slug, "how-tos");
    assert!(category.client_visible);

    let author = fixed_uuid(132);
    let article = kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(133),
        Some(category.id),
        "Reset a lost password",
        "reset-a-lost-password",
        "# Steps\n1. open the portal",
        KbArticleStatus::Published,
        true,
        author,
    )
    .await
    .expect("article insert");
    assert_eq!(article.status, KbArticleStatus::Published);
    assert_eq!(article.created_by, Some(author));
    assert_eq!(article.updated_by, Some(author));

    // List ordering: categories by position then name.
    let _low = kb::create_category(
        &mut *tx,
        s.org,
        fixed_uuid(134),
        "Announcements",
        "announcements",
        "",
        1,
        false,
    )
    .await
    .expect("second category");
    let categories = kb::list_categories(&mut *tx, s.org).await.expect("list");
    assert_eq!(categories.len(), 2);
    assert_eq!(categories[0].slug, "announcements");

    // Update (patch semantics: untouched columns stay).
    kb::update_article(
        &mut *tx,
        s.org,
        article.id,
        &kb::ArticlePatch {
            body: Some("v2 body".into()),
            status: Some(KbArticleStatus::Draft),
            updated_by: Some(author),
            ..Default::default()
        },
    )
    .await
    .expect("article update");
    let reloaded = kb::get_article(&mut *tx, s.org, article.id)
        .await
        .expect("reload");
    assert_eq!(reloaded.body, "v2 body");
    assert_eq!(reloaded.status, KbArticleStatus::Draft);
    assert_eq!(reloaded.title, "Reset a lost password");

    // Delete category → article survives with a NULL category.
    assert!(kb::delete_category(&mut *tx, s.org, category.id)
        .await
        .expect("delete category"));
    let orphaned = kb::get_article(&mut *tx, s.org, article.id)
        .await
        .expect("article survives");
    assert_eq!(orphaned.category_id, None);

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn article_filters_compose_and_client_gate_needs_the_category() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 135).await;
    let author = seed_user(&app, &s, 139).await;
    let mut tx = org_tx(&app, s.org).await;

    let visible_cat = kb::create_category(
        &mut *tx,
        s.org,
        fixed_uuid(136),
        "Client Guides",
        "client-guides",
        "",
        0,
        true,
    )
    .await
    .expect("visible category");
    let hidden_cat = kb::create_category(
        &mut *tx,
        s.org,
        fixed_uuid(137),
        "Internal",
        "internal",
        "",
        1,
        false,
    )
    .await
    .expect("hidden category");

    kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(138),
        Some(visible_cat.id),
        "Onboard a new site",
        "onboard-a-new-site",
        "content",
        KbArticleStatus::Published,
        true,
        author,
    )
    .await
    .expect("published client-visible article");
    // Flagged but DRAFT: gate must exclude it.
    kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1340),
        Some(visible_cat.id),
        "Draft playbook",
        "draft-playbook",
        "content",
        KbArticleStatus::Draft,
        true,
        author,
    )
    .await
    .expect("flagged draft");
    // Published + flagged but category hidden: gate must exclude it.
    kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1342),
        Some(hidden_cat.id),
        "Internal runbook",
        "internal-runbook",
        "content",
        KbArticleStatus::Published,
        true,
        author,
    )
    .await
    .expect("hidden-category article");
    // No category at all: gate must exclude it.
    kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1344),
        None,
        "Uncategorized",
        "uncategorized",
        "content",
        KbArticleStatus::Published,
        true,
        author,
    )
    .await
    .expect("uncategorized article");

    let client_view = kb::list_articles(
        &mut *tx,
        s.org,
        &kb::ArticleFilter {
            client_visible: true,
            ..Default::default()
        },
    )
    .await
    .expect("client-visible list");
    assert_eq!(client_view.len(), 1, "only the fully-gated article passes");
    assert_eq!(client_view[0].slug, "onboard-a-new-site");

    let drafts = kb::list_articles(
        &mut *tx,
        s.org,
        &kb::ArticleFilter {
            status: Some(KbArticleStatus::Draft),
            ..Default::default()
        },
    )
    .await
    .expect("draft list");
    assert_eq!(drafts.len(), 1);

    let by_category = kb::list_articles(
        &mut *tx,
        s.org,
        &kb::ArticleFilter {
            category_id: Some(hidden_cat.id),
            ..Default::default()
        },
    )
    .await
    .expect("category list");
    assert_eq!(by_category.len(), 1);

    let matched = kb::list_articles(
        &mut *tx,
        s.org,
        &kb::ArticleFilter {
            q: Some("runbook".into()),
            ..Default::default()
        },
    )
    .await
    .expect("q list");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].slug, "internal-runbook");

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn slug_uniqueness_is_enforced_per_org() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 1346).await;
    let author = seed_user(&app, &s, 1348).await;
    let mut tx = org_tx(&app, s.org).await;

    kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1347),
        None,
        "Duplicate",
        "same-slug",
        "",
        KbArticleStatus::Draft,
        false,
        fixed_uuid(1348),
    )
    .await
    .expect("first insert");

    let second = kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1349),
        None,
        "Duplicate Two",
        "same-slug",
        "",
        KbArticleStatus::Draft,
        false,
        author,
    )
    .await;
    match second {
        Err(DbError::Sql(sqlx::Error::Database(db_err))) => {
            assert!(
                db_err.constraint().unwrap_or("").contains("slug"),
                "violation must name the slug constraint"
            );
        }
        other => panic!("expected slug unique violation, got {other:?}"),
    }
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn rls_blocks_cross_org_kb_reads() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 1351).await;
    let author = seed_user(&app, &s, 1353).await;
    let mut tx = org_tx(&app, s.org).await;
    let article = kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(1352),
        None,
        "Secret handshakes",
        "secret-handshakes",
        "internal",
        KbArticleStatus::Draft,
        false,
        author,
    )
    .await
    .expect("insert");
    tx.commit().await.expect("commit");

    // A foreign org sees nothing.
    let other = seed_org(&app, 1354).await;
    let mut tx2 = org_tx(&app, other.org).await;
    assert!(matches!(
        kb::get_article(&mut *tx2, other.org, article.id).await,
        Err(DbError::NotFound)
    ));
    assert!(
        kb::list_articles(&mut *tx2, other.org, &kb::ArticleFilter::default())
            .await
            .expect("list is empty for foreign org")
            .is_empty()
    );
    tx2.commit().await.expect("commit");
}

#[tokio::test]
async fn unified_search_accepts_kb_and_document_kinds() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 1355).await;
    let mut tx = org_tx(&app, s.org).await;

    // 0031 widened the kind vocabulary; the indexer's upsert path must
    // accept both new kinds (the CHECK is the authority).
    search::upsert_row(
        &mut *tx,
        s.org,
        search::SearchKind::KbArticle,
        fixed_uuid(1356),
        None,
        "Wifi password rotation",
        "Rotate the guest wifi password quarterly.",
    )
    .await
    .expect("kb_article index row");
    search::upsert_row(
        &mut *tx,
        s.org,
        search::SearchKind::Document,
        fixed_uuid(1357),
        None,
        "Escalation policy",
        "P1 incidents page the on-call immediately.",
    )
    .await
    .expect("document index row");

    let hits = search::search_page(
        &mut *tx,
        s.org,
        &["kb_article".to_string(), "document".to_string()],
        "wifi password",
        false,
        10,
        0,
    )
    .await
    .expect("fts hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, "kb_article");

    assert!(search::delete_row(
        &mut *tx,
        s.org,
        search::SearchKind::Document,
        fixed_uuid(1357)
    )
    .await
    .expect("delete indexed document"));
    tx.commit().await.expect("commit");
}
