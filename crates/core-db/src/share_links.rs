//! One-time share links + rotation claim reads (Wave 19 lane 3, migration
//! 0096). Implements approach U1 (secret_share_links) and the rotation
//! sweep claim/read helpers.
//!
//! SHARE LINKS — insert (manager), list for subject, get by hash, and
//! the transactional claim. Claiming is the security moment: a single
//! `UPDATE … SET consumed_at WHERE consumed_at IS NULL AND revoked_at IS
//! NULL AND expires_at > now()` statement is the exclusive gate (row is
//! claimed exactly once; two racing claims resolve one consumer). The
//! API layer re-validates role/category scope BEFORE consuming and sets
//! the audit trail + org/user GUC in the SAME transaction the claim
//! runs in, so a denied claim leaves the link untouched.
//!
//! Non-purpose: token generation/hashing (core-api only), booking-style
//! double-claim backstops (the SQL predicate is the backstop), audit
//! row writing (audit.rs), TTL garbage collection (rows persist as the
//! lifecycle ledger).
//!
//! ROTATION — the sweep-side claim reads due rows across BOTH subjects
//! (`secret_records` and `passwords` share the additive columns from
//! 0096) and flips `rotation_reminded_at` in the same statement as the
//! claim for at-most-once semantics; the worker then emits
//! `secret.rotation_due`. Refreshing `rotate_at` forward clears the
//! marker so re-scheduled spins re-remind.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::{SecretShareLink, ShareSubjectKind};
use sqlx::{PgConnection, Postgres, Row};
use uuid::Uuid;

fn share_link_row(row: sqlx::postgres::PgRow) -> SecretShareLink {
    use sqlx::Row as _;
    let kind: String = row.get("subject_kind");
    SecretShareLink {
        id: row.get("id"),
        subject_kind: if kind == "password" {
            ShareSubjectKind::Password
        } else {
            ShareSubjectKind::SecretRecord
        },
        subject_id: row.get("subject_id"),
        label: row.get("label"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        consumed_at: row.get("consumed_at"),
        consumed_by: row.get("consumed_by"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    }
}

const SHARE_COLS: &str = "id, subject_kind, subject_id, label, expires_at, revoked_at, consumed_at, consumed_by, created_by, created_at";

pub struct NewShareLink {
    pub id: Uuid,
    pub org_id: Uuid,
    pub subject_kind: ShareSubjectKind,
    pub subject_id: Uuid,
    /// The SHA-256 token hash (64 lowercase hex); the raw token never
    /// reaches this module.
    pub token_hash: String,
    pub label: String,
    pub created_by: Uuid,
    pub expires_at: DateTime<Utc>,
}

pub async fn insert<'e, E>(executor: E, link: &NewShareLink) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO secret_share_links (id, org_id, subject_kind, subject_id, token_hash,
                                        label, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(link.id)
    .bind(link.org_id)
    .bind(link.subject_kind.as_str())
    .bind(link.subject_id)
    .bind(&link.token_hash)
    .bind(&link.label)
    .bind(Some(link.created_by))
    .bind(link.expires_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_for_subject<'e, E>(
    executor: E,
    org_id: Uuid,
    subject_kind: ShareSubjectKind,
    subject_id: Uuid,
) -> DbResult<Vec<SecretShareLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {SHARE_COLS} FROM secret_share_links
          WHERE org_id = $1 AND subject_kind = $2 AND subject_id = $3
          ORDER BY created_at DESC"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(subject_kind.as_str())
        .bind(subject_id)
        .fetch_all(executor)
        .await?
        .into_iter()
        .map(|row| Ok(share_link_row(row)))
        .collect()
}

pub async fn get_by_id<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SecretShareLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {SHARE_COLS} FROM secret_share_links WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .map(share_link_row)
        .ok_or(DbError::NotFound)
}

/// Look a link up by token hash (token → hash resolved by the caller).
/// The returned row carries the home org so the claim endpoint can verify
/// the claimer belongs to the sharing org BEFORE consuming.
pub async fn get_by_token_hash<'e, E>(executor: E, token_hash: &str) -> DbResult<SecretShareLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {SHARE_COLS} FROM secret_share_links WHERE token_hash = $1");
    sqlx::query(&sql)
        .bind(token_hash)
        .fetch_optional(executor)
        .await?
        .map(share_link_row)
        .ok_or(DbError::NotFound)
}

/// The transactional claim: consume A live link (not consumed, not
/// revoked, not expired, org matches) exactly once. `Ok(Some(tuple))`
/// means consumed by THIS caller — the tuple carries the home org since
/// a cross-org caller (same DB) resolved nothing. `Ok(None)` means the
/// link was already dead to claim — the API layer resolves the honest
/// message per case.
pub async fn claim(tx: &mut PgConnection, org_id: Uuid, id: Uuid, claimer: Uuid) -> DbResult<bool> {
    // The predicate carries the whole liveness gate; FOR UPDATE rows are
    // serialized so double-claim races resolve one winner. The org
    // binding rides the org GUC set for this transaction (RLS), plus an
    // explicit org match for defense in depth.
    let r = sqlx::query(
        "UPDATE secret_share_links
            SET consumed_at = now(), consumed_by = $3
          WHERE org_id = $1 AND id = $2
            AND consumed_at IS NULL AND revoked_at IS NULL
            AND expires_at > now()",
    )
    .bind(org_id)
    .bind(id)
    .bind(claimer)
    .execute(tx)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn revoke(tx: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let r = sqlx::query(
        "UPDATE secret_share_links
            SET revoked_at = now()
          WHERE org_id = $1 AND id = $2
            AND consumed_at IS NULL AND revoked_at IS NULL",
    )
    .bind(org_id)
    .bind(id)
    .execute(tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- rotation sweep (claims) --------------------------------------------------

/// One due-rotation claim of THESE subjects: `subject_kind` = `secret_record`
/// or `password`. Claims rows whose `rotate_at <= now()` and
/// `rotation_reminded_at IS NULL`, stamping `rotation_reminded_at` in the
/// SAME statement — at-most-once per schedule: concurrent sweeps lock the
/// row once, so only one emitter ever loads it. `last_rotated_at IS NULL OR
/// last_rotated_at < rotate_at` keeps a re-scheduled row honest: an update
/// that moved `rotate_at` forward that ALREADY stamped `last_rotated_at` in
/// that update shouldn't be re-reminded mid-flight.
#[derive(Debug)]
pub struct RotationClaim {
    pub org_id: Uuid,
    pub subject_kind: ShareSubjectKind,
    pub subject_id: Uuid,
    pub title: String,
    pub rotate_at: DateTime<Utc>,
}

pub async fn claim_due_rotation(
    pool: &sqlx::PgPool,
    batch_limit: i64,
) -> DbResult<Vec<RotationClaim>> {
    // One statement per subject: `UPDATE … WHERE due AND unreminded
    // RETURNING` is the at-most-once claim (the stamp IS the claim), and
    // a concurrent sweep serializes on the row locks. Sweep reads run
    // unscoped (worker posture — worker reads never carry an org GUC).
    let secret_rows = sqlx::query(
        "UPDATE secret_records
            SET rotation_reminded_at = now()
          WHERE ctid IN (
                SELECT ctid
                  FROM secret_records
                 WHERE rotate_at IS NOT NULL
                   AND rotate_at <= now()
                   AND (
                     rotation_reminded_at IS NULL
                     OR rotation_reminded_at < rotate_at
                   )
                 LIMIT $1
               )
          RETURNING id, org_id, title, rotate_at",
    )
    .bind(batch_limit)
    .fetch_all(pool)
    .await?;
    let password_rows = sqlx::query(
        "UPDATE passwords
            SET rotation_reminded_at = now()
          WHERE ctid IN (
                SELECT ctid
                  FROM passwords
                 WHERE rotate_at IS NOT NULL
                   AND rotate_at <= now()
                   AND (
                     rotation_reminded_at IS NULL
                     OR rotation_reminded_at < rotate_at
                   )
                 LIMIT $1
               )
          RETURNING id, org_id, title, rotate_at",
    )
    .bind(batch_limit)
    .fetch_all(pool)
    .await?;
    let mut claims = Vec::with_capacity(secret_rows.len() + password_rows.len());
    for row in secret_rows.into_iter() {
        claims.push(RotationClaim {
            org_id: row.get("org_id"),
            subject_kind: ShareSubjectKind::SecretRecord,
            subject_id: row.get("id"),
            title: row.get("title"),
            rotate_at: row.get("rotate_at"),
        });
    }
    for row in password_rows.into_iter() {
        claims.push(RotationClaim {
            org_id: row.get("org_id"),
            subject_kind: ShareSubjectKind::Password,
            subject_id: row.get("id"),
            title: row.get("title"),
            rotate_at: row.get("rotate_at"),
        });
    }
    Ok(claims)
}
