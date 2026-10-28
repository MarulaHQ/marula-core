//! The org entitlement artifact store (RFC 0023).
//!
//! Purpose: persistence for the signed artifact an org currently holds — the
//! row behind `GET`/`PUT /orgs/{org_id}/entitlements`. One row per org, holding
//! the latest adopted revision. Verification, shape checks and authorization
//! live in core-api; this module verifies nothing and interprets nothing.
//!
//! The monotonic guard lives in [`adopt`]'s write statement rather than in the
//! caller: `ON CONFLICT ... WHERE EXCLUDED.revision > org_entitlements.revision`
//! is atomic, so two concurrent adoptions cannot both think they are newest.
//! The statement reports refusal by returning no row, which lets the caller
//! distinguish a replay (same revision, same content) from a conflict (same
//! revision, different content) by reading the row it kept.
//!
//! Non-purpose: entitlement semantics (flags, seats, allowances), feature
//! gating, pricing, and history. Core reports the revision it holds; what the
//! payload means belongs to whoever issued it.
//!
//! Queries are runtime-checked (`query_as::<_, Row>`) rather than the
//! `query_as!` macro, following `document_attachments`/`decision`: the column
//! list is fixed and small, and this keeps the offline `.sqlx` cache out of the
//! change set for a table that did not exist when it was prepared.

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

/// One org's adopted artifact, as stored.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EntitlementRow {
    pub org_id: Uuid,
    pub revision: i64,
    /// The signed payload, verbatim. Opaque to core by design.
    pub payload: serde_json::Value,
    pub payload_sha256: String,
    pub artifact_sha256: String,
    /// The trust identity that verified, when one did.
    pub key_id: Option<String>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub adopted_by: Option<Uuid>,
    pub adopted_at: chrono::DateTime<chrono::Utc>,
}

/// What an adoption proposes to store. Digests are already verified against
/// the signed envelope by the time this exists — this module stores them.
pub struct AdoptCandidate<'a> {
    pub revision: i64,
    pub payload: &'a serde_json::Value,
    pub payload_sha256: &'a str,
    pub artifact: &'a serde_json::Value,
    pub artifact_sha256: &'a str,
    pub key_id: Option<&'a str>,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub adopted_by: Uuid,
}

/// Reads the artifact an org holds; `None` when it has never adopted one.
pub async fn get<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<EntitlementRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as::<_, EntitlementRow>(ENTITLEMENT_COLUMNS_SELECT)
        .bind(org_id)
        .fetch_optional(executor)
        .await?;
    Ok(row)
}

/// Adopts `candidate` when its revision is strictly greater than the stored
/// one (or when the org holds nothing yet), atomically.
///
/// Returns the stored row on success and `None` when the monotonic guard
/// refused — meaning a row already exists at the same or a higher revision.
/// The caller reads that row to tell a replay from a conflict; this function
/// does not decide which, because "same content" is a comparison the route
/// owns (it holds the candidate digests already).
pub async fn adopt(
    conn: &mut PgConnection,
    org_id: Uuid,
    candidate: &AdoptCandidate<'_>,
) -> DbResult<Option<EntitlementRow>> {
    let row = sqlx::query_as::<_, EntitlementRow>(&format!(
        "INSERT INTO org_entitlements
             (id, org_id, revision, payload, payload_sha256,
              artifact, artifact_sha256, key_id, issued_at, adopted_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (org_id) DO UPDATE
           SET revision = EXCLUDED.revision,
               payload = EXCLUDED.payload,
               payload_sha256 = EXCLUDED.payload_sha256,
               artifact = EXCLUDED.artifact,
               artifact_sha256 = EXCLUDED.artifact_sha256,
               key_id = EXCLUDED.key_id,
               issued_at = EXCLUDED.issued_at,
               adopted_by = EXCLUDED.adopted_by,
               adopted_at = now()
           WHERE EXCLUDED.revision > org_entitlements.revision
         RETURNING {ENTITLEMENT_COLUMNS}"
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(candidate.revision)
    .bind(candidate.payload)
    .bind(candidate.payload_sha256)
    .bind(candidate.artifact)
    .bind(candidate.artifact_sha256)
    .bind(candidate.key_id)
    .bind(candidate.issued_at)
    .bind(candidate.adopted_by)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

const ENTITLEMENT_COLUMNS: &str = "org_id, revision, payload, payload_sha256, \
     artifact_sha256, key_id, issued_at, adopted_by, adopted_at";

const ENTITLEMENT_COLUMNS_SELECT: &str = "SELECT org_id, revision, payload, \
     payload_sha256, artifact_sha256, key_id, issued_at, adopted_by, adopted_at \
     FROM org_entitlements WHERE org_id = $1";

#[cfg(test)]
mod tests {
    use super::*;

    fn column_set(list: &str) -> Vec<String> {
        let mut columns: Vec<String> = list
            .split(',')
            .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        columns.sort();
        columns
    }

    /// The `SELECT` and `RETURNING` lists must carry the same columns. A
    /// `RETURNING` list that drifts from the `SELECT` list decodes into a
    /// row missing a field at runtime, and the compiler cannot see either
    /// string. This is the cheapest place to catch that.
    #[test]
    fn select_and_returning_column_lists_agree() {
        let select = ENTITLEMENT_COLUMNS_SELECT
            .split_once("SELECT ")
            .and_then(|(_, rest)| rest.split_once(" FROM"))
            .map(|(cols, _)| cols)
            .expect("select list");
        assert_eq!(column_set(select), column_set(ENTITLEMENT_COLUMNS));
    }

    /// And both lists match `EntitlementRow`'s fields, so a column added to
    /// one place and not the other fails here rather than at decode time.
    #[test]
    fn column_lists_match_the_row_fields() {
        let fields = [
            "org_id",
            "revision",
            "payload",
            "payload_sha256",
            "artifact_sha256",
            "key_id",
            "issued_at",
            "adopted_by",
            "adopted_at",
        ];
        let mut expected: Vec<String> = fields.iter().map(|f| (*f).to_string()).collect();
        expected.sort();
        assert_eq!(column_set(ENTITLEMENT_COLUMNS), expected);
    }
}
