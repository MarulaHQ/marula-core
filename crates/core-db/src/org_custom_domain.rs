//! Per-org custom domain storage (TASKLIST Wave 47 lane A8,
//! `p47-org-custom-domains`, migration 0135).
//!
//! Four narrow seams over the three `orgs` columns, plus the one read that
//! answers "can a DNS-provider plugin automate this org's records?":
//!
//!   * [`get`] reads the org's current domain, its verification token, and
//!     when it was verified.
//!   * [`set_domain`] writes a normalized domain with a freshly-issued token
//!     and RESETS verification (a changed domain is an unverified domain).
//!   * [`mark_verified`] records the moment Core's system-resolver TXT check
//!     matched the token.
//!   * [`clear`] erases the domain, token, and verification together.
//!   * [`dns_provider_installed`] reports whether the org has an ENABLED
//!     plugin install granted the `org_custom_domain.manage` capability — the
//!     signal that a DNS-provider plugin can automate record creation. It is
//!     a REPORT: no Core code path requires such a plugin, and the manual
//!     paste path is always available.
//!
//! Everything runs on runtime `sqlx::query` runners rather than the
//! `query!` macros: the `orgs` table gains columns this migration adds, and
//! a compile-time macro cache would otherwise need a `sqlx prepare` pass for
//! a shape the offline cache cannot know yet. The callers bind the RLS scope
//! (`app.org_id`) before every statement, so the `org_self` policy on `orgs`
//! and the `plugin_installs_org` policy do the scoping — these functions
//! never filter by org themselves beyond the primary key they are handed.
//!
//! Non-purpose: no TLS, no routing, no certificate issuance (the operator's
//! edge does that, reading the domain back through the API), and no provider
//! registry (a plugin automates records; Core only reports whether one is
//! installed).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::plugins::PluginCapability;
use sqlx::Postgres;
use uuid::Uuid;

/// One org's custom-domain state. All three fields move together: a token
/// exists exactly when a domain does, and `verified_at` is set only after a
/// successful TXT check against that token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgCustomDomain {
    pub org_id: Uuid,
    pub domain: Option<String>,
    pub verification_token: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
}

impl OrgCustomDomain {
    /// Whether the current domain has passed verification. Requires a domain
    /// to be set; a cleared row is never "verified".
    pub fn is_verified(&self) -> bool {
        self.domain.is_some() && self.verified_at.is_some()
    }

    /// The verification status as a stable wire token:
    /// `none` (no domain), `pending` (domain set, not yet verified), or
    /// `verified`.
    pub fn status(&self) -> &'static str {
        match (&self.domain, &self.verified_at) {
            (None, _) => "none",
            (Some(_), None) => "pending",
            (Some(_), Some(_)) => "verified",
        }
    }
}

fn row_to_domain(row: &sqlx::postgres::PgRow) -> DbResult<OrgCustomDomain> {
    use sqlx::Row;
    Ok(OrgCustomDomain {
        org_id: row.try_get("id")?,
        domain: row.try_get("custom_domain")?,
        verification_token: row.try_get("custom_domain_verification_token")?,
        verified_at: row.try_get("custom_domain_verified_at")?,
    })
}

/// The org's current custom-domain row, or `None` when the org is not visible
/// under the caller's RLS scope (an unbound or cross-org session sees nothing).
pub async fn get<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<OrgCustomDomain>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        "SELECT id, custom_domain, custom_domain_verification_token, \
                custom_domain_verified_at \
           FROM orgs WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(executor)
    .await?;
    match row {
        Some(ref r) => Ok(Some(row_to_domain(r)?)),
        None => Ok(None),
    }
}

/// Sets (or replaces) the org's custom domain and issues a fresh verification
/// token, RESETTING `custom_domain_verified_at` — a new or changed domain is
/// unverified until its TXT record is published and checked. `domain` must
/// already be normalized by the caller.
///
/// A unique-violation (SQLSTATE 23505, index `orgs_custom_domain_key`) surfaces
/// as [`DbError::Sql`] and maps to a typed 409 at the API edge.
pub async fn set_domain(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    domain: &str,
    verification_token: &str,
) -> DbResult<()> {
    let result = sqlx::query(
        "UPDATE orgs \
            SET custom_domain = $2, \
                custom_domain_verification_token = $3, \
                custom_domain_verified_at = NULL \
          WHERE id = $1",
    )
    .bind(org_id)
    .bind(domain)
    .bind(verification_token)
    .execute(executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Records a successful verification against the CURRENTLY stored domain.
/// Returns `false` when the org has no domain (nothing to verify) or is not
/// visible under the caller's scope.
pub async fn mark_verified(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<bool> {
    let result = sqlx::query(
        "UPDATE orgs SET custom_domain_verified_at = now() \
          WHERE id = $1 AND custom_domain IS NOT NULL",
    )
    .bind(org_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Clears the domain, its token, and any verification together. Returns
/// `false` when the org is not visible under the caller's scope.
pub async fn clear(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<bool> {
    let result = sqlx::query(
        "UPDATE orgs \
            SET custom_domain = NULL, \
                custom_domain_verification_token = NULL, \
                custom_domain_verified_at = NULL \
          WHERE id = $1",
    )
    .bind(org_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Whether an ENABLED plugin install in this org holds the
/// `org_custom_domain.manage` capability — the signal that a DNS-provider
/// plugin is installed and can automate this org's `_marula-verify` and edge
/// records. Purely informational: no Core endpoint requires it, and the
/// operator can always paste the records by hand.
///
/// `revoked_at IS NULL` is redundant with the `enabled` status today but
/// keeps the answer honest if a future path flips status without clearing
/// revocation. Runs under `plugin_installs_org` RLS, so the caller binds the
/// org scope first.
pub async fn dns_provider_installed<'e, E>(executor: E, org_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let capability = PluginCapability::OrgCustomDomainManage.as_str();
    let wanted = serde_json::json!([capability]).to_string();
    let installed: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM plugin_installs \
              WHERE org_id = $1 \
                AND status = 'enabled' \
                AND revoked_at IS NULL \
                AND granted_capabilities @> $2::jsonb )",
    )
    .bind(org_id)
    .bind(wanted)
    .fetch_one(executor)
    .await?;
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tracks_domain_and_verification_together() {
        let none = OrgCustomDomain {
            org_id: Uuid::nil(),
            domain: None,
            verification_token: None,
            verified_at: None,
        };
        assert_eq!(none.status(), "none");
        assert!(!none.is_verified());

        let pending = OrgCustomDomain {
            domain: Some("support.example.com".into()),
            verification_token: Some("marula-verify-abc".into()),
            ..none.clone()
        };
        assert_eq!(pending.status(), "pending");
        assert!(!pending.is_verified());

        let verified = OrgCustomDomain {
            verified_at: Some(DateTime::<Utc>::UNIX_EPOCH),
            ..pending
        };
        assert_eq!(verified.status(), "verified");
        assert!(verified.is_verified());
    }
}
