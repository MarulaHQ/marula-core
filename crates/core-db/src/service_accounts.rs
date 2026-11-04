//! core-db/service_accounts — persistence for client-certificate
//! authentication service accounts (Wave 47 lane A9, `p47-client-cert-auth`).
//!
//! Purpose: typed queries over the `service_accounts` table from migration
//! 0130. The admin CRUD surface runs org-scoped (RLS via `app.org_id`);
//! the single-row auth lookup rides the SECURITY DEFINER
//! `service_account_auth_lookup` because the presenting certificate
//! resolves the org BEFORE any scope could exist, exactly like the API-key
//! seam it mirrors.
//!
//! Non-purpose: no certificate parsing or fingerprint computation (the
//! edge proxy supplies the fingerprint; core-api compares it here), no
//! permission resolution (the API layer owns the grant→Permission mapping
//! through the existing policy matrix), no logging of fingerprint bodies
//! beyond their identity use.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

/// A service-account row. `scopes` holds the granted permission keys,
/// validated against the existing policy matrix at write time.
#[derive(Debug, Clone)]
pub struct ServiceAccountRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// Lowercase hex SHA-256 of the pinned client certificate.
    pub cert_fingerprint: String,
    /// Display-only subject DN, never matched on.
    pub cert_subject: Option<String>,
    /// Granted permission keys (e.g. "tickets:view").
    pub scopes: Vec<String>,
    /// Admin lifecycle: "active" | "suspended".
    pub status: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revocation_reason: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Derived cert lifecycle for listings/reads: revoked beats expired beats
/// valid, so a revoked account that also lapsed reads as revoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertStatus {
    Valid,
    Expired,
    Revoked,
}

impl CertStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CertStatus::Valid => "valid",
            CertStatus::Expired => "expired",
            CertStatus::Revoked => "revoked",
        }
    }
}

impl ServiceAccountRow {
    /// The derived lifecycle of this account's certificate binding.
    pub fn cert_status(&self, now: DateTime<Utc>) -> CertStatus {
        if self.revoked_at.is_some() {
            CertStatus::Revoked
        } else if self.expires_at.is_some_and(|exp| exp <= now) {
            CertStatus::Expired
        } else {
            CertStatus::Valid
        }
    }

    /// Whether this row may be used for authentication right now: active,
    /// not revoked, and not expired. Mirrors the SQL auth-lookup predicate
    /// so a suspended/lapsed/revoked row is refused before any handler runs.
    pub fn is_authenticatable(&self, now: DateTime<Utc>) -> bool {
        self.status == "active"
            && self.revoked_at.is_none()
            && self.expires_at.is_none_or(|exp| exp > now)
    }
}

/// Raw SELECT shape: `scopes` stays JSONB and is widened to `Vec<String>`
/// in [`service_account_from`].
struct ServiceAccountRecord {
    id: Uuid,
    org_id: Uuid,
    name: String,
    cert_fingerprint: String,
    cert_subject: Option<String>,
    scopes: Value,
    status: String,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    revocation_reason: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
}

fn service_account_from(r: ServiceAccountRecord) -> ServiceAccountRow {
    let scopes = r
        .scopes
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ServiceAccountRow {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        cert_fingerprint: r.cert_fingerprint,
        cert_subject: r.cert_subject,
        scopes,
        status: r.status,
        expires_at: r.expires_at,
        revoked_at: r.revoked_at,
        revocation_reason: r.revocation_reason,
        last_used_at: r.last_used_at,
        created_by: r.created_by,
        created_at: r.created_at,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_service_account<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    name: &str,
    cert_fingerprint: &str,
    cert_subject: Option<&str>,
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
    created_by: Option<Uuid>,
) -> DbResult<ServiceAccountRow>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ServiceAccountRecord,
        r#"INSERT INTO service_accounts
              (id, org_id, name, cert_fingerprint, cert_subject, scopes, expires_at, created_by)
            VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8)
            RETURNING id, org_id, name, cert_fingerprint, cert_subject, scopes, status,
                      expires_at, revoked_at, revocation_reason, last_used_at, created_by,
                      created_at"#,
        id,
        org_id,
        name,
        cert_fingerprint,
        cert_subject,
        serde_json::to_value(scopes).unwrap_or_else(|_| serde_json::json!([])),
        expires_at,
        created_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(service_account_from(row))
}

/// Org-scoped listing (newest-first). Runs under RLS, so the caller's
/// `app.org_id` already bounds the rows; `org_id` is bound explicitly so
/// the query plans against the org index.
pub async fn list_service_accounts<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<ServiceAccountRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        ServiceAccountRecord,
        r#"SELECT id, org_id, name, cert_fingerprint, cert_subject, scopes, status,
                  expires_at, revoked_at, revocation_reason, last_used_at, created_by, created_at
           FROM service_accounts WHERE org_id = $1 ORDER BY created_at DESC, id DESC"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(service_account_from).collect())
}

/// Org-scoped single-row fetch for the admin read/patch/delete paths.
pub async fn get_service_account<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<ServiceAccountRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ServiceAccountRecord,
        r#"SELECT id, org_id, name, cert_fingerprint, cert_subject, scopes, status,
                  expires_at, revoked_at, revocation_reason, last_used_at, created_by, created_at
           FROM service_accounts WHERE id = $1 AND org_id = $2"#,
        id,
        org_id,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(service_account_from))
}

/// Admin update of the mutable binding fields (scopes, expiry, status).
/// The fingerprint, org binding, subject DN, and creation provenance are
/// immutable — a re-pin is a create + revoke (the rotation story). Returns
/// `NotFound` when the row is not visible in the caller's org scope, which
/// the caller maps to `RlsSparse`.
pub async fn update_service_account<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    scopes: &[String],
    expires_at: Option<DateTime<Utc>>,
    status: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE service_accounts
            SET scopes = $3::jsonb, expires_at = $4, status = $5,
                updated_at = now()
            WHERE id = $1 AND org_id = $2"#,
        id,
        org_id,
        serde_json::to_value(scopes).unwrap_or_else(|_| serde_json::json!([])),
        expires_at,
        status,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Revokes a service account (immediate + idempotent). A second delete of
/// an already-revoked account stays revoked rather than erroring, so the
/// admin surface is safe to retry.
pub async fn revoke_service_account<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    reason: Option<&str>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE service_accounts
            SET revoked_at = COALESCE(revoked_at, now()),
                revocation_reason = COALESCE($3, revocation_reason),
                updated_at = now()
            WHERE id = $1 AND org_id = $2 AND revoked_at IS NULL"#,
        id,
        org_id,
        reason,
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Resolves the presented certificate fingerprint to an authenticatable
/// service account. Cross-org BY DESIGN — the certificate itself selects
/// the account (and therefore the org) and no `app.org_id` scope can exist
/// yet — so this rides the SECURITY DEFINER
/// `service_account_auth_lookup($1)`, which only resolves ACTIVE,
/// non-revoked, unexpired rows. Works identically under the constrained
/// `marula_app` role and the table owner. Returns `None` for an unknown,
/// revoked, suspended, or expired fingerprint (deny-by-default).
pub async fn find_active_service_account_by_fingerprint<'e, E>(
    executor: E,
    cert_fingerprint: &str,
) -> DbResult<Option<ServiceAccountRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ServiceAccountRecord,
        r#"SELECT id AS "id!", org_id AS "org_id!", name AS "name!",
                  cert_fingerprint AS "cert_fingerprint!", cert_subject,
                  scopes AS "scopes!", status AS "status!",
                  expires_at, revoked_at, revocation_reason, last_used_at, created_by,
                  created_at AS "created_at!"
           FROM service_account_auth_lookup($1)"#,
        cert_fingerprint,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(service_account_from))
}

/// One active service account by id, for a key bound to it. Runs before any
/// org is bound, so it rides the SECURITY DEFINER lookup with the same
/// active/status/expiry filter as the certificate path.
pub async fn find_active_service_account_by_id<'e, E>(
    executor: E,
    id: Uuid,
) -> DbResult<Option<ServiceAccountRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ServiceAccountRecord,
        r#"SELECT id AS "id!", org_id AS "org_id!", name AS "name!",
                  cert_fingerprint AS "cert_fingerprint!", cert_subject,
                  scopes AS "scopes!", status AS "status!",
                  expires_at, revoked_at, revocation_reason, last_used_at, created_by,
                  created_at AS "created_at!"
           FROM service_account_key_lookup($1)"#,
        id,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(service_account_from))
}

/// Throttled last-used stamp: at most one write per `min_interval_secs`
/// per account. Rides the SECURITY DEFINER function because the auth path
/// runs before any org is bound (same seam as the api-key seam).
pub async fn touch_service_account_last_used<'e, E>(
    executor: E,
    id: Uuid,
    min_interval_secs: i64,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        "SELECT service_account_touch_last_used($1, $2)",
        id,
        min_interval_secs as i32,
    )
    .execute(executor)
    .await?;
    Ok(())
}
