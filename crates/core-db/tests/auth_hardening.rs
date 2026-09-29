//! Stage-2 auth-hardening persistence tests: RLS isolation on every new
//! table, single-use enforcement of recovery codes IN SQL, the TOTP replay
//! watermark, refresh-chain revocation cascades, audit filter/cursor
//! behavior, and OIDC provider/link integrity.
//!
//! Database selection follows the documented contract in
//! `tests/common/mod.rs` (MARULA_TEST_DATABASE_URL → DATABASE_URL →
//! testcontainers).
//!
//! Non-purpose: HTTP-layer behavior (core-api) and protocol crypto.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{audit, mfa, oidc, users, DbError};
use uuid::Uuid;

/// One org + one user with fixed ids derived from `n`.
async fn seed(app: &sqlx::PgPool, n: u64) -> (Uuid, Uuid) {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Hardening Co", &format!("hard-{n}"), "MER")
        .await
        .expect("org");
    let user = fixed_uuid(n + 500);
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("u{n}@hard.test"),
        "User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit seed");
    (org, user)
}

// --- RLS -------------------------------------------------------------------

/// Every new table must be invisible without org scope AND across orgs.
#[tokio::test]
async fn rls_hides_new_auth_tables_without_and_across_org_scope() {
    let db = test_db().await;
    let (org_a, user_a) = seed(&db.app, 700).await;
    let (org_b, _user_b) = seed(&db.app, 800).await;

    // Seed rows in org A under scope.
    let mut tx = org_tx(&db.app, org_a).await;
    mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org_a, user_a, &[1, 2, 3])
        .await
        .expect("totp row");
    mfa::insert_webauthn_credential(
        &mut *tx,
        Uuid::now_v7(),
        org_a,
        user_a,
        "credA",
        &serde_json::json!({"k": "v"}),
        "laptop",
    )
    .await
    .expect("webauthn row");
    mfa::replace_recovery_codes(
        &mut *tx,
        org_a,
        user_a,
        &["h1".to_string(), "h2".to_string()],
        &[Uuid::now_v7(), Uuid::now_v7()],
    )
    .await
    .expect("recovery rows");
    mfa::insert_challenge(
        &mut *tx,
        Uuid::now_v7(),
        org_a,
        user_a,
        "registration",
        &serde_json::json!({}),
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await
    .expect("challenge row");
    oidc::upsert_provider(
        &mut *tx,
        Uuid::now_v7(),
        org_a,
        oidc::NewOidcProvider {
            name: "idp",
            display_name: "IdP",
            issuer: "https://idp",
            discovery_url: "https://idp/.well-known/openid-configuration",
            client_id: "cid",
            client_secret_enc: &[9u8; 40],
            jit_provisioning: false,
            preset: None,
        },
        user_a,
    )
    .await
    .expect("provider");
    tx.commit().await.expect("commit org a");

    // Unscoped session sees NOTHING (default-deny). Uses a VIRGIN pool:
    // Postgres keeps custom GUCs defined after transaction-local sets end,
    // so reused sessions would raise 22P02 instead of filtering.
    {
        let fresh = db.fresh_app_pool().await;
        for (table, expected) in [
            ("totp_secrets", 0i64),
            ("webauthn_credentials", 0),
            ("recovery_codes", 0),
            ("mfa_challenges", 0),
            ("oidc_providers", 0),
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&fresh)
                .await
                .expect("unscoped count");
            assert_eq!(count, expected, "{table} must be empty without org scope");
        }
    }

    // Org B's scope sees none of org A's rows.
    let mut tx_b = org_tx(&db.app, org_b).await;
    assert!(matches!(mfa::find_totp(&mut *tx_b, user_a).await, Ok(None)));
    let providers_b = oidc::list_providers(&mut *tx_b, org_b)
        .await
        .expect("list b");
    assert!(providers_b.is_empty());
    drop(tx_b);

    // And the owning scope still sees them.
    let mut tx_a = org_tx(&db.app, org_a).await;
    assert!(mfa::find_totp(&mut *tx_a, user_a)
        .await
        .expect("own")
        .is_some());
    assert_eq!(
        oidc::list_providers(&mut *tx_a, org_a)
            .await
            .expect("own providers")
            .len(),
        1
    );
}

// --- recovery codes --------------------------------------------------------

#[tokio::test]
async fn recovery_codes_are_single_use_in_sql() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 710).await;

    let mut tx = org_tx(&db.app, org).await;
    let id = Uuid::now_v7();
    mfa::replace_recovery_codes(&mut *tx, org, user, &["hash-of-code".into()], &[id])
        .await
        .expect("seed codes");
    assert_eq!(
        mfa::count_unused_recovery_codes(&mut *tx, user)
            .await
            .unwrap(),
        1
    );

    // First consume wins.
    assert_eq!(mfa::consume_recovery_code(&mut *tx, id).await.unwrap(), 1);
    // Second consume of the SAME code loses the used_at IS NULL race.
    assert_eq!(mfa::consume_recovery_code(&mut *tx, id).await.unwrap(), 0);
    assert_eq!(
        mfa::count_unused_recovery_codes(&mut *tx, user)
            .await
            .unwrap(),
        0
    );

    // Regeneration wipes old codes entirely.
    mfa::replace_recovery_codes(
        &mut *tx,
        org,
        user,
        &vec!["new-hash".to_string(); 3],
        &[Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()],
    )
    .await
    .expect("regen");
    assert_eq!(
        mfa::count_unused_recovery_codes(&mut *tx, user)
            .await
            .unwrap(),
        3
    );
}

// --- TOTP watermark --------------------------------------------------------

#[tokio::test]
async fn totp_replay_watermark_rejects_stale_steps() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 720).await;

    let mut tx = org_tx(&db.app, org).await;
    mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org, user, &[4, 5, 6])
        .await
        .expect("pending");
    // Confirming consumes the enabling step atomically.
    assert_eq!(mfa::confirm_totp(&mut *tx, user, 100).await.unwrap(), 1);
    // Re-confirming is impossible.
    assert_eq!(mfa::confirm_totp(&mut *tx, user, 101).await.unwrap(), 0);

    // Newer steps advance; equal-or-older are rejected (0 rows).
    assert_eq!(
        mfa::consume_totp_step(&mut *tx, user, 101).await.unwrap(),
        1
    );
    assert_eq!(
        mfa::consume_totp_step(&mut *tx, user, 101).await.unwrap(),
        0
    );
    assert_eq!(
        mfa::consume_totp_step(&mut *tx, user, 100).await.unwrap(),
        0
    );
    assert_eq!(mfa::consume_totp_step(&mut *tx, user, 99).await.unwrap(), 0);
    assert_eq!(
        mfa::consume_totp_step(&mut *tx, user, 102).await.unwrap(),
        1
    );
}

// --- challenges ------------------------------------------------------------

#[tokio::test]
async fn ceremonies_are_one_shot_and_expiry_bound() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 730).await;
    let challenge = Uuid::now_v7();

    let mut tx = org_tx(&db.app, org).await;
    mfa::insert_challenge(
        &mut *tx,
        challenge,
        org,
        user,
        "authentication",
        &serde_json::json!({"challenge": [1, 2]}),
        chrono::Utc::now() + chrono::Duration::seconds(120),
    )
    .await
    .expect("insert");
    // Inserting another ceremony for the same user sweeps the first.
    mfa::insert_challenge(
        &mut *tx,
        Uuid::now_v7(),
        org,
        user,
        "authentication",
        &serde_json::json!({"challenge": [3]}),
        chrono::Utc::now() + chrono::Duration::seconds(120),
    )
    .await
    .expect("insert supersedes");
    assert!(
        mfa::take_challenge(&mut *tx, challenge, user, "authentication")
            .await
            .unwrap()
            .is_none()
    );

    let fresh = Uuid::now_v7();
    mfa::insert_challenge(
        &mut *tx,
        fresh,
        org,
        user,
        "authentication",
        &serde_json::json!({"challenge": [9]}),
        chrono::Utc::now() + chrono::Duration::seconds(120),
    )
    .await
    .expect("insert fresh");
    assert!(
        mfa::take_challenge(&mut *tx, fresh, user, "authentication")
            .await
            .unwrap()
            .is_some(),
        "first take succeeds"
    );
    assert!(
        mfa::take_challenge(&mut *tx, fresh, user, "authentication")
            .await
            .unwrap()
            .is_none(),
        "second take is refused — ceremonies cannot be replayed"
    );
}

// --- audit filters ---------------------------------------------------------

#[tokio::test]
async fn audit_filtered_listing_supports_actor_action_range_cursor() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 740).await;
    let other_org = fixed_uuid(741);
    {
        let mut tx = org_tx(&db.app, other_org).await;
        core_db::orgs::insert_org(&mut *tx, other_org, "Elsewhere", "elsewhere", "MER")
            .await
            .expect("other org");
        tx.commit().await.expect("commit other org");
    }

    let mut tx = org_tx(&db.app, org).await;
    for i in 0..6i64 {
        audit::insert(
            &mut *tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id: org,
                actor: if i % 2 == 0 {
                    core_domain::entities::Actor::Human(user)
                } else {
                    core_domain::entities::Actor::System
                },
                action: if i < 3 { "auth.login" } else { "ticket.update" }.into(),
                entity_type: None,
                entity_id: None,
                detail: serde_json::json!({ "i": i }),
                ip: None,
                occurred_at: chrono::Utc::now() + chrono::Duration::seconds(i),
            },
        )
        .await
        .expect("audit insert");
    }
    tx.commit().await.expect("commit audits");
    // Cross-org noise that must never appear — inserted via the admin pool
    // (RLS would rightly reject it through the app role).
    audit::insert(
        &db.admin,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id: other_org,
            actor: core_domain::entities::Actor::System,
            action: "auth.login".into(),
            entity_type: None,
            entity_id: None,
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("other org noise");

    let mut tx = org_tx(&db.app, org).await;
    let all = audit::list_filtered(&mut *tx, org, audit::AuditFilter::default(), 100)
        .await
        .expect("all events");
    assert_eq!(all.len(), 6, "exactly this org's events");
    assert!(
        all.windows(2).all(|w| w[0].occurred_at >= w[1].occurred_at),
        "ordered newest first"
    );

    // Action prefix.
    let auth_only = audit::list_filtered(
        &mut *tx,
        org,
        audit::AuditFilter {
            action: Some("auth".into()),
            ..Default::default()
        },
        100,
    )
    .await
    .unwrap();
    assert_eq!(auth_only.len(), 3);
    assert!(auth_only.iter().all(|e| e.action.starts_with("auth.")));

    // Actor filter (system rows).
    let system_only = audit::list_filtered(
        &mut *tx,
        org,
        audit::AuditFilter {
            actor: Some("system".into()),
            ..Default::default()
        },
        100,
    )
    .await
    .unwrap();
    assert_eq!(system_only.len(), 3);

    // Time range (inclusive lower bound; newest-first list ⇒ all[2] is the
    // third-newest event, so exactly three events sit at or after it).
    let mid = all[2].occurred_at;
    let since_mid = audit::list_filtered(
        &mut *tx,
        org,
        audit::AuditFilter {
            from: Some(mid),
            ..Default::default()
        },
        100,
    )
    .await
    .unwrap();
    assert_eq!(since_mid.len(), 3);

    // Cursor pagination walks the whole set exactly once.
    let mut seen: Vec<Uuid> = Vec::new();
    let mut anchor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> = None;
    loop {
        let page = audit::list_filtered(
            &mut *tx,
            org,
            audit::AuditFilter {
                cursor: anchor,
                ..Default::default()
            },
            2,
        )
        .await
        .unwrap();
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|e| e.id));
        let last = page.last().unwrap();
        anchor = Some((last.occurred_at, last.id));
        if page.len() < 2 {
            // Last partial page: one more call must come back empty.
            assert!(audit::list_filtered(
                &mut *tx,
                org,
                audit::AuditFilter {
                    cursor: anchor,
                    ..Default::default()
                },
                2,
            )
            .await
            .unwrap()
            .is_empty());
            break;
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen.len(),
        6,
        "cursor walk covers each event once, no overlap"
    );
}

// --- OIDC integrity --------------------------------------------------------

#[tokio::test]
async fn oidc_provider_meta_never_exposes_secret_and_links_are_unique() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 750).await;

    let mut tx = org_tx(&db.app, org).await;
    let pid = oidc::upsert_provider(
        &mut *tx,
        Uuid::now_v7(),
        org,
        oidc::NewOidcProvider {
            name: "corp",
            display_name: "Corp IdP",
            issuer: "https://corp",
            discovery_url: "https://corp/.well-known/openid-configuration",
            client_id: "client-1",
            client_secret_enc: b"s3cr3t-bytes",
            jit_provisioning: false,
            preset: None,
        },
        user,
    )
    .await
    .expect("provider upsert");

    // Metadata path carries no ciphertext column at all.
    let meta = oidc::get_provider_meta(&mut *tx, org, pid)
        .await
        .expect("meta");
    assert_eq!(meta.client_id, "client-1");
    assert_eq!(meta.name, "corp");

    // Full row exists for the login flow only.
    let full = oidc::find_provider_with_secret(&mut *tx, org, pid)
        .await
        .expect("full row");
    assert_eq!(full.client_secret_enc, b"s3cr3t-bytes");

    // Links unique per (provider, subject).
    let subject = "sub-123";
    oidc::create_link(
        &mut *tx,
        Uuid::now_v7(),
        org,
        pid,
        user,
        oidc::IdpIdentity {
            subject,
            email: "u750@hard.test",
        },
        Some(user),
    )
    .await
    .expect("first link");
    tx.commit().await.expect("commit link");

    // Duplicate binding fails on its own transaction (unique violation).
    let mut dup = org_tx(&db.app, org).await;
    assert!(oidc::create_link(
        &mut *dup,
        Uuid::now_v7(),
        org,
        pid,
        user,
        oidc::IdpIdentity {
            subject,
            email: "u750@hard.test"
        },
        Some(user),
    )
    .await
    .is_err());
    drop(dup);

    let mut tx = org_tx(&db.app, org).await;
    let found = oidc::find_link(&mut *tx, pid, subject)
        .await
        .expect("lookup")
        .expect("link exists");
    assert_eq!(found.user_id, user);
}

#[tokio::test]
async fn sso_handoff_code_is_single_use_and_expiry_gated() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 760).await;
    let code = Uuid::now_v7();

    let mut tx = org_tx(&db.app, org).await;
    mfa::insert_challenge(
        &mut *tx,
        code,
        org,
        user,
        "sso_code",
        &serde_json::json!({"tokens": true}),
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await
    .expect("expired insert");
    assert!(
        mfa::take_sso_code(&mut *tx, code).await.unwrap().is_none(),
        "expired codes are dead"
    );

    let live = Uuid::now_v7();
    mfa::insert_challenge(
        &mut *tx,
        live,
        org,
        user,
        "sso_code",
        &serde_json::json!({"tokens": true}),
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .await
    .expect("live insert");
    assert!(mfa::take_sso_code(&mut *tx, live).await.unwrap().is_some());
    assert!(mfa::take_sso_code(&mut *tx, live).await.unwrap().is_none());
}

// --- refresh chain cascade -------------------------------------------------

#[tokio::test]
async fn chain_revocation_kills_every_descendant() {
    let db = test_db().await;
    let (org, user) = seed(&db.app, 770).await;

    let root = fixed_uuid(771);
    let mut tx = org_tx(&db.app, org).await;
    users::create_session(&mut *tx, root, user, "root-hash", None, common::FUTURE)
        .await
        .expect("root session");

    // Build a 3-node chain: root → child-0 → child-1.
    let mut hashes = vec!["root-hash".to_string()];
    let mut parent = users::find_session_any(&mut *tx, root, "root-hash")
        .await
        .expect("root row");
    for i in 0..2u32 {
        let child_id = fixed_uuid(772 + i as u64);
        let child_hash = format!("child-{i}-hash");
        users::rotate_session_chain(&mut *tx, &parent, child_id, &child_hash, common::FUTURE)
            .await
            .expect("rotate");
        parent = users::find_session_any(&mut *tx, child_id, &child_hash)
            .await
            .expect("child row");
        hashes.push(child_hash);
    }
    tx.commit().await.expect("commit chain");

    // Presenting the ROOT (two rotations stale) ⇒ the chain dies. The two
    // ancestor rows were already revoked BY rotation; exactly the live leaf
    // remains for reuse detection to kill.
    let mut tx = org_tx(&db.app, org).await;
    let killed = users::revoke_chain(&mut *tx, parent.chain_id)
        .await
        .expect("chain kill");
    assert_eq!(killed, 1);
    for hash in &hashes {
        let active: Vec<sqlx::postgres::PgRow> = sqlx::query(
            "SELECT id FROM auth_sessions WHERE refresh_token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(hash)
        .fetch_all(&mut *tx)
        .await
        .expect("scan");
        assert!(active.is_empty(), "no active session may hold {hash}");
    }
    let root_lookup = users::find_active_session(&mut *tx, root, "root-hash").await;
    assert!(matches!(root_lookup, Err(DbError::NotFound)));
}
