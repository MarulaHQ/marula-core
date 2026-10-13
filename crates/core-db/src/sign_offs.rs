//! Sign-off queries (Wave 43 track 2 lane 12, migration 0127).
//!
//! Purpose: persistence for `sign_offs`, `sign_off_consents` and
//! `sign_off_revocations`. The sign_offs table is INSERT-ONLY — there is no
//! `update` here, and the caller cannot get one. Revocation inserts a new
//! trail row (`MAX(seq)+1` per sign-off, the `time_entry_approvals` pattern);
//! the "is revoked" and "what superseded this" relations are reads that join
//! back, never writes on the original.
//!
//! Non-purpose: no byte movement (core-store), no admission rules for the PNG
//! (core-domain), no authorization, and no cache.

use crate::{DbError, DbResult};
use core_domain::entities::{
    SignOff, SignOffConsent, SignOffRevocation, SignOffSignerKind, SignOffView,
};
use sqlx::PgConnection;
use uuid::Uuid;

const COLS: &str = "id, org_id, ticket_id, attachment_id, typed_agreement, signer_name, \
                    signer_kind::text AS signer_kind, witnessed_by_user_id, \
                    captured_at, consent_version, consent_text_snapshot, \
                    supersedes_sign_off_id, idempotency_key, created_at";

/// Same projection, qualified for the ticket list query. Unqualified columns
/// are ambiguous there: `attachments`, `sign_off_revocations` and the
/// self-join on `supersedes_sign_off_id` all carry `id`, `org_id` and
/// `created_at` of their own.
const QUALIFIED_COLS: &str = "s.id, s.org_id, s.ticket_id, s.attachment_id, \
                    s.typed_agreement, s.signer_name, \
                    s.signer_kind::text AS signer_kind, s.witnessed_by_user_id, \
                    s.captured_at, s.consent_version, s.consent_text_snapshot, \
                    s.supersedes_sign_off_id, s.idempotency_key, s.created_at";

const REVOCATION_COLS: &str = "id, org_id, sign_off_id, seq, reason, \
                               revoked_by_user_id, created_at";

fn row_to_sign_off(row: sqlx::postgres::PgRow) -> Result<SignOff, sqlx::Error> {
    use sqlx::Row;
    let kind: String = row.get("signer_kind");
    Ok(SignOff {
        id: row.get("id"),
        org_id: row.get("org_id"),
        ticket_id: row.get("ticket_id"),
        attachment_id: row.get("attachment_id"),
        typed_agreement: row.get("typed_agreement"),
        signer_name: row.get("signer_name"),
        signer_kind: kind.parse().map_err(|e: core_domain::DomainError| {
            sqlx::Error::ColumnDecode {
                index: "signer_kind".into(),
                source: Box::new(e),
            }
        })?,
        witnessed_by_user_id: row.get("witnessed_by_user_id"),
        captured_at: row.get("captured_at"),
        consent_version: row.get("consent_version"),
        consent_text_snapshot: row.get("consent_text_snapshot"),
        supersedes_sign_off_id: row.get("supersedes_sign_off_id"),
        idempotency_key: row.get("idempotency_key"),
        created_at: row.get("created_at"),
    })
}

fn row_to_revocation(row: sqlx::postgres::PgRow) -> Result<SignOffRevocation, sqlx::Error> {
    use sqlx::Row;
    Ok(SignOffRevocation {
        id: row.get("id"),
        org_id: row.get("org_id"),
        sign_off_id: row.get("sign_off_id"),
        seq: row.get("seq"),
        reason: row.get("reason"),
        revoked_by_user_id: row.get("revoked_by_user_id"),
        created_at: row.get("created_at"),
    })
}

fn row_to_consent(row: sqlx::postgres::PgRow) -> Result<SignOffConsent, sqlx::Error> {
    use sqlx::Row;
    Ok(SignOffConsent {
        id: row.get("id"),
        org_id: row.get("org_id"),
        version: row.get("version"),
        body: row.get("body"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

/// A new sign-off row. The attachment row must already exist — the caller
/// stores the bytes and inserts the payload in the same transaction.
pub struct NewSignOff<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    /// `None` for a typed agreement, which carries no drawn mark.
    pub attachment_id: Option<Uuid>,
    pub typed_agreement: bool,
    pub signer_name: &'a str,
    pub signer_kind: SignOffSignerKind,
    pub witnessed_by_user_id: Option<Uuid>,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub consent_version: i32,
    pub consent_text_snapshot: &'a str,
    pub supersedes_sign_off_id: Option<Uuid>,
    pub idempotency_key: &'a str,
}

/// Inserts one sign-off. Idempotent on (org_id, idempotency_key): a replayed
/// queue op hits the UNIQUE and returns a typed conflict the API maps to 409,
/// so the client's queue recognises its own replay as already-applied.
pub async fn insert_sign_off(
    conn: &mut PgConnection,
    sign_off: NewSignOff<'_>,
) -> DbResult<SignOff> {
    let row = sqlx::query(&format!(
        r#"INSERT INTO sign_offs (id, org_id, ticket_id, attachment_id, typed_agreement,
                                  signer_name, signer_kind, witnessed_by_user_id, captured_at,
                                  consent_version, consent_text_snapshot,
                                  supersedes_sign_off_id, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7::text::sign_off_signer_kind, $8, $9, $10,
                   $11, $12, $13)
           RETURNING {COLS}"#
    ))
    .bind(sign_off.id)
    .bind(sign_off.org_id)
    .bind(sign_off.ticket_id)
    .bind(sign_off.attachment_id)
    .bind(sign_off.typed_agreement)
    .bind(sign_off.signer_name)
    .bind(sign_off.signer_kind.as_str())
    .bind(sign_off.witnessed_by_user_id)
    .bind(sign_off.captured_at)
    .bind(sign_off.consent_version)
    .bind(sign_off.consent_text_snapshot)
    .bind(sign_off.supersedes_sign_off_id)
    .bind(sign_off.idempotency_key)
    .try_map(row_to_sign_off)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        other => other.into(),
    })?;
    Ok(row)
}

/// Fetches one sign-off by id within the org.
pub async fn get_sign_off(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<SignOff> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM sign_offs WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_sign_off)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        other => other.into(),
    })?;
    Ok(row)
}

/// One ticket's sign-offs, newest first, rendered with payload facts plus the
/// revocation and correction relations. No byte movement: the payload's
/// storage facts are enough for a timeline row.
pub async fn list_sign_off_views_for_ticket(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<SignOffView>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS},
                  a.filename AS attachment_filename,
                  a.content_type AS attachment_content_type,
                  a.size_bytes AS attachment_size_bytes,
                  a.sha256 AS attachment_sha256,
                  r.seq AS revocation_seq,
                  r.reason AS revocation_reason,
                  r.revoked_by_user_id AS revocation_by,
                  r.created_at AS revocation_at,
                  r.id AS revocation_id,
                  r.org_id AS revocation_org_id,
                  r.sign_off_id AS revocation_sign_off_id,
                  c.id AS superseded_by
           FROM sign_offs s
           LEFT JOIN attachments a ON a.id = s.attachment_id
           LEFT JOIN LATERAL (
                  SELECT * FROM sign_off_revocations rr
                  WHERE rr.sign_off_id = s.id
                  ORDER BY rr.seq DESC LIMIT 1
                ) r ON TRUE
           LEFT JOIN LATERAL (
                  SELECT cc.id FROM sign_offs cc
                  WHERE cc.supersedes_sign_off_id = s.id
                  ORDER BY cc.created_at ASC LIMIT 1
                ) c ON TRUE
           WHERE s.org_id = $1 AND s.ticket_id = $2
           ORDER BY s.created_at DESC"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .try_map(|row| {
        use sqlx::Row;
        // `PgRow` is not Clone, so every column this projection needs is read
        // BEFORE the row is consumed by `row_to_sign_off`.
        // Typed agreements have no payload, so every field is optional.
        let attachment_filename: Option<String> = row.get("attachment_filename");
        let attachment_content_type: Option<String> = row.get("attachment_content_type");
        let attachment_size_bytes: Option<i64> = row.get("attachment_size_bytes");
        let attachment_sha256: Option<String> = row.get("attachment_sha256");
        let superseded_by: Option<Uuid> = row.get("superseded_by");
        // The trail is joined laterally as ONE row, so its presence is read
        // through that row's own primary key rather than a sentinel column.
        // Its own columns are read by their aliases, never off the sign-off,
        // so a revoked row reports who revoked it rather than who signed.
        let revocation_id: Option<Uuid> = row.try_get("revocation_id").ok().flatten();
        let revocation = revocation_id.map(|id| SignOffRevocation {
            id,
            org_id: row.get("revocation_org_id"),
            sign_off_id: row.get("revocation_sign_off_id"),
            seq: row.try_get("revocation_seq").ok().flatten().unwrap_or(1),
            reason: row.get("revocation_reason"),
            revoked_by_user_id: row.get("revocation_by"),
            created_at: row.get("revocation_at"),
        });
        Ok(SignOffView {
            sign_off: row_to_sign_off(row)?,
            attachment_filename,
            attachment_content_type,
            attachment_size_bytes,
            attachment_sha256,
            revocation,
            superseded_by,
        })
    })
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

/// Latest consent version for the org. The migration seeds version 1 for every
/// existing org, so this only fails when the row was deleted out from under us.
pub async fn current_consent(conn: &mut PgConnection, org_id: Uuid) -> DbResult<SignOffConsent> {
    let row = sqlx::query(
        r#"SELECT id, org_id, version, body, created_by, created_at
           FROM sign_off_consents
           WHERE org_id = $1
           ORDER BY version DESC
           LIMIT 1"#,
    )
    .bind(org_id)
    .try_map(row_to_consent)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        other => other.into(),
    })?;
    Ok(row)
}

/// Appends the next consent version for the org. The version is computed by
/// the caller from `current_consent`; the UNIQUE (org_id, version) makes a
/// double-submit a typed conflict instead of two versions racing to 2.
pub async fn insert_consent(
    conn: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    version: i32,
    body: &str,
    created_by: Uuid,
) -> DbResult<SignOffConsent> {
    let row = sqlx::query(
        r#"INSERT INTO sign_off_consents (id, org_id, version, body, created_by)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, org_id, version, body, created_by, created_at"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(version)
    .bind(body)
    .bind(created_by)
    .try_map(row_to_consent)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Appends one revocation to a sign-off's trail. `MAX(seq)+1` per sign-off,
/// so a second revocation attempt is a UNIQUE violation rather than a silent
/// overwrite of the first reason — the original row is never touched.
pub async fn insert_revocation(
    conn: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    sign_off_id: Uuid,
    reason: &str,
    revoked_by_user_id: Uuid,
) -> DbResult<SignOffRevocation> {
    let row = sqlx::query(&format!(
        r#"INSERT INTO sign_off_revocations (id, org_id, sign_off_id, seq, reason,
                                             revoked_by_user_id)
           SELECT $1, $2, $3, COALESCE(MAX(seq), 0) + 1, $4, $5
           FROM sign_off_revocations WHERE sign_off_id = $3
           RETURNING {REVOCATION_COLS}"#
    ))
    .bind(id)
    .bind(org_id)
    .bind(sign_off_id)
    .bind(reason)
    .bind(revoked_by_user_id)
    .try_map(row_to_revocation)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        other => other.into(),
    })?;
    Ok(row)
}

/// The newest revocation for one sign-off, when it has been revoked.
pub async fn latest_revocation(
    conn: &mut PgConnection,
    sign_off_id: Uuid,
) -> DbResult<Option<SignOffRevocation>> {
    let row = sqlx::query(&format!(
        "SELECT {REVOCATION_COLS} FROM sign_off_revocations
         WHERE sign_off_id = $1 ORDER BY seq DESC LIMIT 1"
    ))
    .bind(sign_off_id)
    .try_map(row_to_revocation)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// A sign-off's immutable ticket subject. Used by the revoke route to prove
/// the row belongs to the ticket named in the path before appending to the trail.
pub async fn sign_off_ticket(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<Uuid> {
    let row: Uuid =
        sqlx::query_scalar("SELECT ticket_id FROM sign_offs WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .fetch_one(conn)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => DbError::NotFound,
                other => other.into(),
            })?;
    Ok(row)
}

/// The attachment payload for one sign-off, for the download route.
#[derive(Debug)]
pub struct SignOffPayload {
    pub attachment_id: Uuid,
    pub storage_key: String,
    pub content_type: String,
    pub filename: String,
    /// Integrity anchor: the digest of the exact bytes behind `storage_key`.
    pub sha256: String,
}

pub async fn payload_for_sign_off(
    conn: &mut PgConnection,
    org_id: Uuid,
    sign_off_id: Uuid,
) -> DbResult<SignOffPayload> {
    let row = sqlx::query(
        // The inner JOIN is the point: a typed agreement has no payload, so
        // it yields no row and the route answers 404 rather than inventing
        // bytes for a signature that was never drawn.
        r#"SELECT s.attachment_id, a.storage_key, a.content_type, a.filename, a.sha256
           FROM sign_offs s
           JOIN attachments a ON a.id = s.attachment_id
           WHERE s.org_id = $1 AND s.id = $2"#,
    )
    .bind(org_id)
    .bind(sign_off_id)
    .try_map(|row: sqlx::postgres::PgRow| {
        use sqlx::Row;
        Ok(SignOffPayload {
            attachment_id: row.get("attachment_id"),
            storage_key: row.get("storage_key"),
            content_type: row.get("content_type"),
            filename: row.get("filename"),
            sha256: row.get("sha256"),
        })
    })
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        other => other.into(),
    })?;
    Ok(row)
}
