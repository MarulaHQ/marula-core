//! KB rich-content query-layer tests (Wave 13 lane 4, KB1/KB2,
//! migration 0069) — seed slots 140–149 are this suite's private range.
//!
//! Purpose: prove the SQL layer — the oEmbed allowlist registry (system
//! seeds visible to every org, org extensions, system rows immutable
//! through the org-scoped write paths), embed instances (unique per
//! article/url), linked external documents (attach/detach, link-check
//! record, URL change resets verification), and the client-facing gates
//! for embeds and documents riding the article's effective rule.
//!
//! Non-purpose: pure allowlist/regex rules (core-kb rich unit tests) and
//! HTTP behavior (core-api kb_rich_flow). Database selection follows the
//! documented contract in tests/common/mod.rs.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{kb, kb_rich, orgs, users};
use core_domain::entities::{ExternalDocUrlKind, KbArticleStatus, LinkCheckStatus};
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Rich Acme", &format!("rich-acme-{n}"), "RCH")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit seed");
    Seeded { org }
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@rich-acme.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

/// Seeds a client-visible category (the other half of the effective
/// gate: uncategorized articles are never client-visible) and an article
/// inside it.
async fn seed_article(
    app: &PgPool,
    s: &Seeded,
    n: u64,
    status: KbArticleStatus,
    visible: bool,
) -> core_domain::entities::KbArticle {
    let author = seed_user(app, s, n + 1).await;
    let mut tx = org_tx(app, s.org).await;
    let category = kb::create_category(
        &mut *tx,
        s.org,
        fixed_uuid(n + 2),
        &format!("Category {n}"),
        &format!("category-{n}"),
        "",
        1,
        true,
    )
    .await
    .expect("category insert");
    let article = kb::create_article(
        &mut *tx,
        s.org,
        fixed_uuid(n),
        Some(category.id),
        &format!("Article {n}"),
        &format!("article-{n}"),
        "Body.",
        status,
        visible,
        author,
    )
    .await
    .expect("article insert");
    tx.commit().await.expect("commit article");
    article
}

fn is_unique_violation(e: &core_db::DbError) -> bool {
    matches!(e, core_db::DbError::Sql(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23505"))
}

#[tokio::test]
async fn provider_registry_lists_system_seeds_for_every_org() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 140).await;
    let mut tx = org_tx(&app, s.org).await;

    let providers = kb_rich::list_providers(&mut *tx, s.org)
        .await
        .expect("list");
    let slugs: Vec<&str> = providers.iter().map(|p| p.slug.as_str()).collect();
    assert!(slugs.contains(&"youtube"));
    assert!(slugs.contains(&"loom"));
    assert!(slugs.contains(&"vimeo"));
    assert!(providers.iter().all(|p| p.org_id.is_none() && p.is_system));

    // A second org sees the SAME system rows (policy: org_id IS NULL OR
    // org_id = caller), and a system row is unreachable from the org's
    // write path.
    let other_org = fixed_uuid(141);
    {
        let mut other_tx = org_tx(&app, other_org).await;
        orgs::insert_org(
            &mut *other_tx,
            other_org,
            "Rich Beta",
            "rich-acme-141",
            "RC2",
        )
        .await
        .expect("second org");
    }
    let again = kb_rich::list_providers(&mut *tx, other_org)
        .await
        .expect("list");
    assert_eq!(again.len(), providers.len());

    let youtube = providers
        .iter()
        .find(|p| p.slug == "youtube")
        .expect("system youtube row");
    let err = kb_rich::update_provider(
        &mut *tx,
        other_org,
        youtube.id,
        &kb_rich::ProviderPatch {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect_err("system rows are not patchable through the org filter");
    assert!(matches!(err, core_db::DbError::NotFound));
}

#[tokio::test]
async fn provider_org_extension_crud_and_slug_gate() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 142).await;
    let _author = seed_user(&app, &s, 143).await;
    let mut tx = org_tx(&app, s.org).await;

    let host = "wiki.acme.test".to_string();
    let provider = kb_rich::create_provider(
        &mut *tx,
        s.org,
        fixed_uuid(144),
        "Acme Wiki",
        "acme-wiki",
        std::slice::from_ref(&host),
        r#"^(https://)?wiki\.acme\.test/page/[0-9]+$"#,
        "https://wiki.acme.test/oembed?url={url}",
        true,
    )
    .await
    .expect("provider insert");
    assert_eq!(provider.org_id, Some(s.org));
    assert!(!provider.is_system);
    tx.commit().await.expect("commit provider");

    // Slug is unique per org (NULLS NOT DISTINCT keeps system rows out of
    // the org's slug space): a duplicate surfaces as a unique violation.
    // Runs on its own transaction — a failed statement aborts the tx it
    // ran in, and the rest of the test keeps using `tx`.
    {
        let mut dup_tx = org_tx(&app, s.org).await;
        let dup = kb_rich::create_provider(
            &mut *dup_tx,
            s.org,
            fixed_uuid(145),
            "Acme Wiki Two",
            "acme-wiki",
            std::slice::from_ref(&host),
            r#"^(https://)?wiki\.acme\.test/page/[0-9]+$"#,
            "https://wiki.acme.test/oembed?url={url}",
            true,
        )
        .await
        .expect_err("duplicate slug");
        assert!(is_unique_violation(&dup));
        dup_tx.rollback().await.expect("rollback dup");
    }

    let mut tx = org_tx(&app, s.org).await;

    // Disable via patch, then delete.
    kb_rich::update_provider(
        &mut *tx,
        s.org,
        provider.id,
        &kb_rich::ProviderPatch {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("patch");
    let disabled = kb_rich::get_provider(&mut *tx, s.org, provider.id)
        .await
        .expect("get");
    assert!(!disabled.enabled);
    assert!(kb_rich::delete_provider(&mut *tx, s.org, provider.id)
        .await
        .expect("delete"));
}

#[tokio::test]
async fn embed_round_trip_unique_gate_and_client_gate() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 146).await;
    let author = seed_user(&app, &s, 147).await;
    let article = seed_article(&app, &s, 148, KbArticleStatus::Published, true).await;
    let mut tx = org_tx(&app, s.org).await;

    let providers = kb_rich::list_providers(&mut *tx, s.org)
        .await
        .expect("list");
    let youtube = providers
        .iter()
        .find(|p| p.slug == "youtube")
        .expect("system provider");

    let embed = kb_rich::create_embed(
        &mut *tx,
        s.org,
        fixed_uuid(149),
        article.id,
        "https://youtu.be/dQw4w9WgXcQ",
        Some(youtube.id),
        Some("youtube"),
        "A runbook video",
        author,
    )
    .await
    .expect("embed insert");
    assert_eq!(embed.provider_slug.as_deref(), Some("youtube"));
    tx.commit().await.expect("commit embed");

    // Same URL on the same article is a unique violation (own tx — a
    // failed statement aborts the tx it ran in).
    {
        let mut dup_tx = org_tx(&app, s.org).await;
        let dup = kb_rich::create_embed(
            &mut *dup_tx,
            s.org,
            fixed_uuid(150),
            article.id,
            "https://youtu.be/dQw4w9WgXcQ",
            Some(youtube.id),
            Some("youtube"),
            "",
            author,
        )
        .await
        .expect_err("duplicate embed url");
        assert!(is_unique_violation(&dup));
        dup_tx.rollback().await.expect("rollback dup");
    }

    let mut tx = org_tx(&app, s.org).await;
    // The same URL on a DIFFERENT article is fine.
    let second = seed_article(&app, &s, 151, KbArticleStatus::Published, true).await;
    let mut tx2 = org_tx(&app, s.org).await;
    kb_rich::create_embed(
        &mut *tx2,
        s.org,
        fixed_uuid(152),
        second.id,
        "https://youtu.be/dQw4w9WgXcQ",
        Some(youtube.id),
        Some("youtube"),
        "",
        author,
    )
    .await
    .expect("same url, other article");

    // Client gate: visible articles expose their embeds; a hidden
    // article's embeds never surface (the gate is the article's).
    let client_visible = kb_rich::list_client_embeds(&mut *tx, s.org, article.id)
        .await
        .expect("client embeds");
    assert_eq!(client_visible.len(), 1);

    // Flip the article to draft → the gate closes.
    {
        let mut tx3 = org_tx(&app, s.org).await;
        kb::update_article(
            &mut *tx3,
            s.org,
            article.id,
            &kb::ArticlePatch {
                status: Some(KbArticleStatus::Draft),
                updated_by: Some(author),
                ..Default::default()
            },
        )
        .await
        .expect("unpublish");
        tx3.commit().await.expect("commit unpublish");
    }
    // Fresh read transaction: the gate must see the committed flip.
    let mut fresh_tx = org_tx(&app, s.org).await;
    let now_hidden = kb_rich::list_client_embeds(&mut *fresh_tx, s.org, article.id)
        .await
        .expect("client embeds");
    assert!(now_hidden.is_empty());
}

#[tokio::test]
async fn external_document_round_trip_attach_detach_and_link_check() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 153).await;
    let author = seed_user(&app, &s, 154).await;
    let article = seed_article(&app, &s, 155, KbArticleStatus::Published, true).await;
    let mut tx = org_tx(&app, s.org).await;

    // Standalone first (staff-only by construction).
    let standalone = kb_rich::create_external_document(
        &mut *tx,
        s.org,
        fixed_uuid(156),
        None,
        "Runbook on SharePoint",
        "https://acme.sharepoint.com/:w:/r/sites/IT/Runbook.docx",
        ExternalDocUrlKind::SharePoint,
        "M365 group ACME-IT",
        author,
    )
    .await
    .expect("external doc insert");
    assert_eq!(standalone.url_kind, ExternalDocUrlKind::SharePoint);
    assert!(standalone.last_verified_at.is_none());

    // Attach it to the article.
    kb_rich::update_external_document(
        &mut *tx,
        s.org,
        standalone.id,
        &kb_rich::ExternalDocumentPatch {
            article_id: Some(Some(article.id)),
            ..Default::default()
        },
    )
    .await
    .expect("attach");

    // A URL change resets the link-check columns.
    kb_rich::record_link_check(
        &mut *tx,
        s.org,
        standalone.id,
        chrono::DateTime::<chrono::Utc>::from_timestamp(1_000, 0).expect("ts"),
        LinkCheckStatus::Ok,
    )
    .await
    .expect("first check");
    kb_rich::update_external_document(
        &mut *tx,
        s.org,
        standalone.id,
        &kb_rich::ExternalDocumentPatch {
            url: Some("https://acme.sharepoint.com/:w:/r/sites/IT/Runbook-v2.docx".into()),
            ..Default::default()
        },
    )
    .await
    .expect("url change");
    let refreshed = kb_rich::get_external_document(&mut *tx, s.org, standalone.id)
        .await
        .expect("get");
    assert!(refreshed.last_verified_at.is_none());
    assert!(refreshed.last_verified_status.is_none());

    // Re-check records the outcome as data.
    kb_rich::record_link_check(
        &mut *tx,
        s.org,
        standalone.id,
        chrono::DateTime::<chrono::Utc>::from_timestamp(2_000, 0).expect("ts"),
        LinkCheckStatus::Unreachable,
    )
    .await
    .expect("second check");
    let checked = kb_rich::get_external_document(&mut *tx, s.org, standalone.id)
        .await
        .expect("get");
    assert_eq!(
        checked.last_verified_status,
        Some(LinkCheckStatus::Unreachable)
    );

    // Client gate: attached to a visible article ⇒ surfaced; standalone
    // rows and rows on hidden articles never surface.
    let visible_docs = kb_rich::list_client_external_documents(&mut *tx, s.org, article.id)
        .await
        .expect("client docs");
    assert_eq!(visible_docs.len(), 1);

    // Delete.
    assert!(
        kb_rich::delete_external_document(&mut *tx, s.org, standalone.id)
            .await
            .expect("delete")
    );
}
