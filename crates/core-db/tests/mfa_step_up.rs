//! Wave 55 lane `p55-mfa-step-up` persistence tests.
//!
//! The three facts the lane turns on, each pinned at the database layer where
//! it is actually enforced:
//!
//! 1. A pending enrolment can never downgrade a CONFIRMED TOTP row (the SQL
//!    half of "beginning enrolment cannot replace a live factor").
//! 2. A step-up is fresh inside its window, stale after it, survives a refresh
//!    rotation through its chain, and is invisible to a different login.
//! 3. An admin reset removes every factor, clears every proof, and leaves the
//!    account owing a factor rather than merely factor-free.
//!
//! Non-purpose: HTTP status codes and the permission gate around the reset —
//! `crates/core-api/tests/mfa_step_up_flow.rs` owns those.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb, FUTURE};
use core_db::{mfa, users};
use uuid::Uuid;

/// One org, one active user, one live session whose chain root is its own id.
async fn seed(n: u64) -> (TestDb, Uuid, Uuid, Uuid) {
    let db = test_db().await;
    let org = fixed_uuid(n);
    let user = fixed_uuid(n + 500);
    let session = fixed_uuid(n + 900);

    let mut tx = org_tx(&db.app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Step-Up Co", &format!("step-{n}"), "MER")
        .await
        .expect("org");
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("u{n}@step.test"),
        "User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("seed commit");

    users::create_session(&db.app, session, user, "refresh-hash", None, FUTURE)
        .await
        .expect("session");
    (db, org, user, session)
}

// --- the confirmed-factor guard -------------------------------------------

/// The bug in DECISIONS record 1 was one `ON CONFLICT DO UPDATE` that set
/// `confirmed = false` on a live row. The handler's step-up check is one half
/// of the fix; this is the other, and it holds even if a future caller forgets
/// the first.
#[tokio::test]
async fn a_confirmed_totp_is_never_downgraded_by_a_pending_write() {
    let (db, org, user, _session) = seed(900).await;
    let mut tx = org_tx(&db.app, org).await;

    let first = mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org, user, &[1, 2, 3])
        .await
        .expect("pending enrolment");
    assert_eq!(first, 1, "the first enrolment writes the row");

    assert_eq!(
        mfa::confirm_totp(&mut *tx, user, 42)
            .await
            .expect("confirm"),
        1
    );

    let refused = mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org, user, &[9, 9, 9])
        .await
        .expect("second pending write is answered, not fatal");
    assert_eq!(
        refused, 0,
        "a confirmed factor must not be writable back to pending"
    );

    let row = mfa::find_totp(&mut *tx, user)
        .await
        .expect("read")
        .expect("row survives");
    assert!(row.confirmed, "the factor is still live");
    assert_eq!(row.secret_enc, vec![1, 2, 3], "the secret is untouched");
    assert_eq!(row.last_used_step, 42, "the replay watermark is untouched");

    tx.commit().await.expect("commit");
}

// --- step-up records -------------------------------------------------------

#[tokio::test]
async fn a_step_up_is_fresh_inside_its_window_and_stale_after_it() {
    let (db, org, user, session) = seed(910).await;
    let mut tx = org_tx(&db.app, org).await;

    mfa::record_step_up(
        &mut *tx,
        mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id: org,
            user_id: user,
            session_id: session,
            method: "totp",
            factor_ref: None,
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    )
    .await
    .expect("record");

    let row = mfa::latest_step_up(&mut *tx, session, user)
        .await
        .expect("read")
        .expect("recorded");
    assert!(row.fresh, "a proof inside its window is fresh");
    assert_eq!(row.method, "totp");

    // A different login is a different chain, so it cannot borrow this proof.
    let other_session = Uuid::now_v7();
    users::create_session(&db.app, other_session, user, "other-hash", None, FUTURE)
        .await
        .expect("second login");
    assert!(
        mfa::latest_step_up(&mut *tx, other_session, user)
            .await
            .expect("read other")
            .is_none(),
        "another session must not see this chain's proof"
    );

    // `record_step_up` cannot write a proof that is born expired: 0161's
    // `mfa_step_ups_window_positive` requires `expires_at > verified_at`.
    // The stale branch is still reachable the way production reaches it — a
    // proof whose window ran out after it was written — so the row goes in
    // with a window in the past, which the constraint admits. The fresh proof
    // above has to go first, or the read returns it: it sorts ahead of any row
    // whose window has already closed.
    sqlx::query("DELETE FROM mfa_step_ups WHERE session_id = $1")
        .bind(session)
        .execute(&mut *tx)
        .await
        .expect("clear the fresh proof");
    sqlx::query(
        "INSERT INTO mfa_step_ups \
           (id, org_id, user_id, session_id, chain_id, verified_at, expires_at, method, factor_ref) \
         SELECT $1, $2, $3, a.id, a.chain_id, now() - interval '10 minutes', \
                now() - interval '5 minutes', 'recovery_code', 'code-row' \
           FROM auth_sessions a WHERE a.id = $4",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(user)
    .bind(session)
    .execute(&mut *tx)
    .await
    .expect("record stale");

    let row = mfa::latest_step_up(&mut *tx, session, user)
        .await
        .expect("read stale")
        .expect("still a row");
    assert!(
        !row.fresh,
        "an expired proof is reported as stale, not as absent — the two \
         answers tell the client different things"
    );
    assert_eq!(row.method, "recovery_code");

    tx.commit().await.expect("commit");
}

/// Refresh rotation inserts a NEW session row on the same chain. A proof taken
/// minutes ago has to survive that, or "step up then change a factor" breaks
/// whenever the client happens to refresh in between.
#[tokio::test]
async fn a_rotated_session_keeps_the_step_up_through_its_chain() {
    let (db, org, user, session) = seed(920).await;
    let mut tx = org_tx(&db.app, org).await;

    mfa::record_step_up(
        &mut *tx,
        mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id: org,
            user_id: user,
            session_id: session,
            method: "webauthn",
            factor_ref: Some("cred-1"),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    )
    .await
    .expect("record");

    let parent = users::RefreshSession {
        id: session,
        user_id: user,
        refresh_token_hash: "refresh-hash".into(),
        user_agent: None,
        chain_id: session,
        rotated_from: None,
        expires_at: FUTURE,
        created_at: chrono::Utc::now(),
        revoked_at: None,
    };
    let child = Uuid::now_v7();
    assert!(
        users::rotate_session_chain(&mut *tx, &parent, child, "child-hash", FUTURE)
            .await
            .expect("rotate"),
        "the rotation lands"
    );

    let row = mfa::latest_step_up(&mut *tx, child, user)
        .await
        .expect("read")
        .expect("proof survives rotation");
    assert!(row.fresh);
    assert_eq!(row.factor_ref.as_deref(), Some("cred-1"));

    tx.commit().await.expect("commit");
}

// --- admin factor reset ----------------------------------------------------

#[tokio::test]
async fn an_admin_reset_clears_every_factor_and_leaves_the_account_owing_one() {
    let (db, org, user, session) = seed(930).await;
    let mut tx = org_tx(&db.app, org).await;

    // A realistic account: confirmed TOTP, one passkey, two recovery codes and
    // a live proof.
    assert_eq!(
        mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org, user, &[7])
            .await
            .expect("totp"),
        1
    );
    assert_eq!(
        mfa::confirm_totp(&mut *tx, user, 9).await.expect("confirm"),
        1
    );
    mfa::insert_webauthn_credential(
        &mut *tx,
        Uuid::now_v7(),
        org,
        user,
        "cred-reset",
        &serde_json::json!({ "k": "v" }),
        "laptop",
    )
    .await
    .expect("passkey");
    mfa::replace_recovery_codes(
        &mut *tx,
        org,
        user,
        &["h1".to_string(), "h2".to_string()],
        &[Uuid::now_v7(), Uuid::now_v7()],
    )
    .await
    .expect("codes");
    mfa::record_step_up(
        &mut *tx,
        mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id: org,
            user_id: user,
            session_id: session,
            method: "totp",
            factor_ref: None,
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    )
    .await
    .expect("step-up");

    let before = mfa::mfa_capabilities(&mut *tx, user).await.expect("caps");
    assert!(before.any(), "the account starts with working factors");

    let counts = mfa::reset_user_factors(&mut *tx, user)
        .await
        .expect("reset");
    assert_eq!(counts.totp_removed, 1);
    assert_eq!(counts.passkeys_removed, 1);
    assert_eq!(counts.recovery_codes_removed, 2);

    assert!(mfa::find_totp(&mut *tx, user)
        .await
        .expect("read totp")
        .is_none());
    assert!(mfa::list_webauthn_credentials(&mut *tx, user)
        .await
        .expect("read passkeys")
        .is_empty());
    let after = mfa::mfa_capabilities(&mut *tx, user).await.expect("caps");
    assert!(!after.any(), "no working factor is left behind");
    assert!(
        mfa::latest_step_up(&mut *tx, session, user)
            .await
            .expect("read step-up")
            .is_none(),
        "a proof minted before the reset must not survive it"
    );

    assert!(
        mfa::user_mfa_reenroll_required(&mut *tx, user)
            .await
            .expect("flag"),
        "the account owes a factor, which is what stops this being a silent \
         downgrade to password-only"
    );

    // The handler revokes sessions in the same transaction; assert the
    // statement the reset path depends on actually does that.
    let revoked = users::revoke_all_user_sessions(&mut *tx, user)
        .await
        .expect("revoke");
    assert_eq!(revoked, 1);

    mfa::clear_mfa_reenroll_required(&mut *tx, user)
        .await
        .expect("clear");
    assert!(!mfa::user_mfa_reenroll_required(&mut *tx, user)
        .await
        .expect("flag after enrolment"));

    tx.commit().await.expect("commit");
}
