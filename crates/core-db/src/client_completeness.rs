//! Client documentation-completeness scoring (Wave 17 lane 13, migration
//! 0089): the BACKEND read model behind the client page's completeness
//! card. This replaces the honest client-side metadata scoring the clients
//! lane shipped first (`apps/web/lib/docs-completeness.ts`) — same shape,
//! same weights, now computed from the org's live rows instead of the
//! page's already-loaded arrays.
//!
//! Purpose: one aggregate query per client returning the counted facts
//! the scoring needs, plus the weight table + pro-ration that turn facts
//! into the 0-100 score. The weights live HERE (single source of truth);
//! the gap vocabulary matches the client card's keys so rendering is a
//! pure mapping.
//!
//! Non-purpose: no per-user gating of score LEGS here — the same facts
//! are visible to every staff viewer of the client (the route is
//! clients:view scoped, so the facts it reads are in scope by
//! construction); no freshness computation beyond the 180-day window.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

/// The freshness window for the documentation leg (matches the client
/// card's `FRESH_WINDOW_DAYS` semantics).
pub const FRESH_WINDOW_DAYS: i64 = 180;

/// One aggregate lift of every scoring fact for a client — all counts are
/// org- AND client-scoped by construction (the caller sits inside the
/// RLS-scoped transaction).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClientCompletenessFacts {
    pub contacts_n: i64,
    pub contacts_with_email_n: i64,
    pub primary_contacts_n: i64,
    pub sites_n: i64,
    pub assets_n: i64,
    pub documents_n: i64,
    pub typed_documents_n: i64,
    pub latest_document_at: Option<chrono::DateTime<chrono::Utc>>,
    pub portal_users_n: i64,
}

pub async fn load_facts<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<ClientCompletenessFacts>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let facts = sqlx::query_as::<_, ClientCompletenessFacts>(
        r#"SELECT
             (SELECT count(*) FROM contacts WHERE org_id = $1 AND client_id = $2) AS contacts_n,
             (SELECT count(*) FROM contacts
               WHERE org_id = $1 AND client_id = $2
                 AND coalesce(email, '') <> '') AS contacts_with_email_n,
             (SELECT count(*) FROM contacts
               WHERE org_id = $1 AND client_id = $2 AND is_primary) AS primary_contacts_n,
             (SELECT count(*) FROM sites WHERE org_id = $1 AND client_id = $2) AS sites_n,
             (SELECT count(*) FROM device_assets WHERE org_id = $1 AND client_id = $2) AS assets_n,
             (SELECT count(*) FROM documents WHERE org_id = $1 AND client_id = $2) AS documents_n,
             (SELECT count(*) FROM documents
               WHERE org_id = $1 AND client_id = $2 AND layout_id IS NOT NULL) AS typed_documents_n,
             (SELECT max(updated_at) FROM documents WHERE org_id = $1 AND client_id = $2)
               AS latest_document_at,
             (SELECT count(*) FROM portal_users WHERE org_id = $1 AND client_id = $2)
               AS portal_users_n"#,
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_one(executor)
    .await?;
    Ok(facts)
}
