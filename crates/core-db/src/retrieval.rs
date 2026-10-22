//! Retrieval-mode capabilities and the per-tenant embedding distribution
//! (Wave 52 lane R15, `p52-retrieval-mode-config`).
//!
//! Purpose: read the four facts the mode check needs from the connected
//! database, and the one number the operator needs to decide when to change the
//! mode. The policy lives in [`core_domain::retrieval`]; this module only
//! supplies the evidence, so boot, `marula deploy describe` and the distribution
//! gauge all agree.
//!
//! Three decisions worth the ink:
//!
//! - **The probe reads, it never changes.** Creating the extension or the index
//!   is [`ensure_index`], called from the one path that already owns DDL (the
//!   `migrate` step, as the schema owner). A boot that could create indexes
//!   would be a boot that runs DDL as the RLS role, which this repository does
//!   not allow.
//! - **The distribution is a `SECURITY DEFINER` function, not a wide query.**
//!   `search_embeddings` carries RLS on `org_id`, so the app role can only count
//!   its own org. Migration 0153 installs
//!   `search_embedding_distribution()` owned by the migration role, which is
//!   what lets one gauge report every tenant. Its output is a count per org, not
//!   content.
//! - **An index that is missing is a refusal, not a slower path.** The check
//!   looks the index up by name, so "the mode is on but the index is not built"
//!   is caught at boot instead of becoming a sequential scan that nobody
//!   notices.
//!
//! Non-purpose: no query planning, no index creation policy (the mode decides
//! which index; the migration function builds it), and no per-tenant
//! configuration.

use sqlx::PgPool;
use uuid::Uuid;

use core_domain::retrieval::{
    check, DatabaseCapabilities, RetrievalMode, RetrievalRefusal, HNSW_INDEX, VCHORD_INDEX,
};

/// What one deployment's database answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The raw facts, in the shared shape the policy checks.
    pub capabilities: DatabaseCapabilities,
    /// Whether the mode can run, with every missing requirement when it cannot.
    pub verdict: Result<(), RetrievalRefusal>,
}

/// One tenant's stored embedding count.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TenantEmbeddings {
    /// The tenant.
    pub org_id: Uuid,
    /// Rows in `search_embeddings` for that tenant, across spaces.
    pub embeddings: i64,
}

/// Read the capability facts for one mode. `mode` is only used to compute the
/// verdict, so a caller that wants the raw facts can pass [`RetrievalMode::Exact`].
///
/// # Errors
///
/// Returns a [`DbError`] when the catalogue tables cannot be read, which is a
/// connection or permission fault and must not be mistaken for an unsatisfied
/// mode.
pub async fn probe(pool: &PgPool, mode: RetrievalMode) -> crate::DbResult<Probe> {
    let capabilities = capabilities(pool).await?;
    let verdict = check(mode, &capabilities);
    Ok(Probe {
        capabilities,
        verdict,
    })
}

/// Read the capability facts without judging a mode.
///
/// # Errors
///
/// Returns a [`DbError`] when the catalogue tables cannot be read.
pub async fn capabilities(pool: &PgPool) -> crate::DbResult<DatabaseCapabilities> {
    let pgvector_version: Option<String> =
        sqlx::query_scalar("SELECT extversion FROM pg_extension WHERE extname = 'vector'")
            .fetch_optional(pool)
            .await?;
    let vchord_installed: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vchord')")
            .fetch_one(pool)
            .await?;
    let vchord_available: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vchord')",
    )
    .fetch_one(pool)
    .await?;
    let preload: String = sqlx::query_scalar(
        "SELECT coalesce(current_setting('shared_preload_libraries', true), '')",
    )
    .fetch_one(pool)
    .await?;
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes \
          WHERE schemaname = current_schema() AND tablename = 'search_embeddings'",
    )
    .fetch_all(pool)
    .await?;
    Ok(DatabaseCapabilities {
        pgvector_version,
        vchord_installed,
        vchord_available,
        preload_libraries: preload
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect(),
        indexes,
    })
}

/// One tenant's embedding count and whether it is past the threshold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TenantDistribution {
    /// The tenant.
    pub org_id: Uuid,
    /// Its stored embeddings.
    pub embeddings: i64,
    /// At or past [`core_domain::retrieval::INDEX_THRESHOLD_EMBEDDINGS`].
    pub index_recommended: bool,
}

/// Every tenant's embedding count, largest first, with the threshold marked.
/// Reads the `SECURITY DEFINER` function migration 0153 installs, so it reports
/// every tenant rather than the caller's org.
///
/// # Errors
///
/// Returns a [`DbError`] when the function is missing (the database is not
/// migrated) or cannot be read.
pub async fn embedding_distribution(pool: &PgPool) -> crate::DbResult<Vec<TenantDistribution>> {
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT org_id, embeddings FROM search_embedding_distribution() \
         ORDER BY embeddings DESC, org_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(org_id, embeddings)| TenantDistribution {
            org_id,
            embeddings,
            index_recommended: core_domain::retrieval::index_recommended(
                u64::try_from(embeddings).unwrap_or(0),
            ),
        })
        .collect())
}

/// What [`ensure_index`] did, so the migrate step can say it in a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureReport {
    /// The mode the function was asked to prepare for.
    pub mode: RetrievalMode,
    /// The index the mode needs.
    pub index: Option<&'static str>,
    /// Whether the index exists after the call.
    pub present: bool,
    /// The function's own account: `created`, `present`, `skipped: <reason>`.
    pub detail: String,
}

/// Materialise the index a mode needs. Runs as the schema owner, from the
/// `migrate` path only.
///
/// The work is done by migration 0153's `marula_ensure_retrieval_index(text)`
/// because that function can look at `pg_available_extensions` and
/// `shared_preload_libraries` in the same transaction it creates the extension
/// in. This wrapper exists so the Rust side names the mode and reports a fact
/// rather than a string.
///
/// # Errors
///
/// Returns a [`DbError`] when the database refuses the DDL, which includes the
/// honest case: the extension is not available on this image.
pub async fn ensure_index(pool: &PgPool, mode: RetrievalMode) -> crate::DbResult<EnsureReport> {
    let detail: String = sqlx::query_scalar("SELECT marula_ensure_retrieval_index($1)")
        .bind(mode.as_str())
        .fetch_one(pool)
        .await?;
    let index = match mode {
        RetrievalMode::Exact => None,
        RetrievalMode::Iterative => Some(HNSW_INDEX),
        RetrievalMode::Index => Some(VCHORD_INDEX),
    };
    let present = match index {
        None => true,
        Some(name) => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_indexes \
                  WHERE schemaname = current_schema() \
                    AND tablename = 'search_embeddings' AND indexname = $1)",
            )
            .bind(name)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(EnsureReport {
        mode,
        index,
        present,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_rows_mark_the_threshold() {
        let row = TenantDistribution {
            org_id: Uuid::nil(),
            embeddings: 20_000,
            index_recommended: core_domain::retrieval::index_recommended(20_000),
        };
        assert!(row.index_recommended);
        let row = TenantDistribution {
            org_id: Uuid::nil(),
            embeddings: 19_999,
            index_recommended: core_domain::retrieval::index_recommended(19_999),
        };
        assert!(!row.index_recommended);
    }
}
