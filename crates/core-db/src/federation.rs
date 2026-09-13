//! Identity-federation bundle storage: edge-assertion auth sources and
//! the conservative link rows shared by non-OIDC bundle sources.
//!
//! Purpose: org-level config + link persistence for the versioned
//! federation bundle interface (core-api `auth::federation`).
//! Deny-by-default lives in the schema: edge sources are created
//! DISABLED and the pre-auth resolver (`app_resolve_edge_source`)
//! returns only ENABLED sources, so an unknown or disabled source is
//! uniformly rejected.
//! Non-purpose: protocol verification (core-api) and the OIDC provider
//! flow (see `oidc`); federated logins resolve into the same session
//! machinery as every other login, never a parallel one.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

// --- edge-assertion sources -------------------------------------------------

#[derive(Debug, Clone)]
pub struct EdgeSourceMeta {
    pub id: Uuid,
    pub name: String,
    pub display_name: String,
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NewEdgeSource<'a> {
    pub name: &'a str,
    pub display_name: &'a str,
    pub issuer: &'a str,
    pub audience: &'a str,
    pub jwks_url: &'a str,
    /// Deny-by-default: callers pass the admin's EXPLICIT enable
    /// decision; `false` unless the request carried `enabled: true`.
    pub enabled: bool,
}

/// Creates or updates an edge-assertion source by (org, name). Creation
/// timestamps are preserved across replaces so tie-breaking in the
/// pre-auth resolver stays deterministic.
pub async fn upsert_edge_source<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    source: NewEdgeSource<'_>,
    created_by: Uuid,
) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let NewEdgeSource {
        name,
        display_name,
        issuer,
        audience,
        jwks_url,
        enabled,
    } = source;
    let row: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO edge_assertion_configs
             (id, org_id, name, display_name, issuer, audience, jwks_url,
              enabled, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (org_id, name) DO UPDATE SET
             display_name = EXCLUDED.display_name,
             issuer = EXCLUDED.issuer,
             audience = EXCLUDED.audience,
             jwks_url = EXCLUDED.jwks_url,
             enabled = EXCLUDED.enabled,
             updated_at = now()
           RETURNING id"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(name)
    .bind(display_name)
    .bind(issuer)
    .bind(audience)
    .bind(jwks_url)
    .bind(enabled)
    .bind(created_by)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

/// Admin listing — pins (issuer/audience/jwks_url) are admin-visible
/// configuration, but nothing here enables an unauthenticated caller to
/// use the source: the exchange path re-checks `enabled`.
pub async fn list_edge_sources<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<EdgeSourceMeta>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        EdgeSourceMeta,
        r#"SELECT id, name, display_name, issuer, audience, jwks_url,
                  enabled, created_at
           FROM edge_assertion_configs WHERE org_id = $1 ORDER BY created_at"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

pub async fn delete_edge_source<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM edge_assertion_configs WHERE id = $2 AND org_id = $1",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// --- federation links (non-OIDC bundle sources) ------------------------------

#[derive(Debug, Clone)]
pub struct FederationLink {
    pub id: Uuid,
    pub user_id: Uuid,
    pub subject: String,
    pub email: String,
    pub linked_by: Option<Uuid>,
    pub linked_at: chrono::DateTime<chrono::Utc>,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct IdpIdentity<'a> {
    pub subject: &'a str,
    pub email: &'a str,
}

/// Creates the admin-explicit link for a bundle source identity. A
/// duplicate (source_kind, source_id, subject) is the typed
/// `federation_link_taken` conflict, never a silent rebind.
pub async fn create_link<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    source_kind: &str,
    source_id: Uuid,
    user_id: Uuid,
    identity: IdpIdentity<'_>,
    linked_by: Uuid,
) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO federation_links
             (id, org_id, source_kind, source_id, user_id, subject, email, linked_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(source_kind)
    .bind(source_id)
    .bind(user_id)
    .bind(identity.subject)
    .bind(identity.email)
    .bind(linked_by)
    .execute(executor)
    .await?;
    Ok(id)
}

pub async fn list_links<'e, E>(
    executor: E,
    org_id: Uuid,
    source_kind: &str,
    source_id: Uuid,
) -> DbResult<Vec<FederationLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as!(
        FederationLink,
        r#"SELECT id, user_id, subject, email, linked_by, linked_at, last_login_at
           FROM federation_links
           WHERE org_id = $1 AND source_kind = $2 AND source_id = $3
           ORDER BY linked_at"#,
        org_id,
        source_kind,
        source_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Resolves the linked local user for an asserted (source, subject)
/// pair. `None` means LinkRequired under the conservative policy.
pub async fn find_link_user<'e, E>(
    executor: E,
    source_kind: &str,
    source_id: Uuid,
    subject: &str,
) -> DbResult<Option<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT user_id FROM federation_links \
         WHERE source_kind = $1 AND source_id = $2 AND subject = $3",
    )
    .bind(source_kind)
    .bind(source_id)
    .bind(subject)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|(u,)| u))
}

pub async fn touch_link_login<'e, E>(
    executor: E,
    source_kind: &str,
    source_id: Uuid,
    subject: &str,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE federation_links SET last_login_at = now() \
         WHERE source_kind = $1 AND source_id = $2 AND subject = $3",
    )
    .bind(source_kind)
    .bind(source_id)
    .bind(subject)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

pub async fn delete_link<'e, E>(
    executor: E,
    org_id: Uuid,
    link_id: Uuid,
    source_kind: &str,
    source_id: Uuid,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "DELETE FROM federation_links \
         WHERE id = $2 AND org_id = $1 AND source_kind = $3 AND source_id = $4",
    )
    .bind(org_id)
    .bind(link_id)
    .bind(source_kind)
    .bind(source_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_source_meta_shape_is_stable() {
        // Pin the shape the pre-auth bridge and admin API both rely on.
        let m = EdgeSourceMeta {
            id: Uuid::now_v7(),
            name: "corp-access".into(),
            display_name: "Corp Access".into(),
            issuer: "https://team.cloudflareaccess.com".into(),
            audience: "aud.tag".into(),
            jwks_url: "https://team.cloudflareaccess.com/cdn-cgi/access/certs".into(),
            enabled: false,
            created_at: chrono::Utc::now(),
        };
        assert_eq!(m.name, "corp-access");
        // Deny-by-default is representable and the default posture.
        assert!(!m.enabled);
    }
}
