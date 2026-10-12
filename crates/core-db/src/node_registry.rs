//! Workflow node registry persistence (Wave 43 lane 13, `p43-node-registry`).
//!
//! Two things live here, and the split is deliberate. The node
//! declarations themselves live in `core_domain::node_registry` and nowhere
//! else — ADR 0016 rule (c) forbids a second registry, and a table of node
//! rows would be one, going stale on its first schema change. What the
//! database owns is (a) the RELEASE a workflow document was validated
//! against, and (b) the five fields that make a plugin surface entry a
//! workflow node, which only the database can hold because a manifest
//! declares them.
//!
//! Non-purpose: node validation, the loop refusal and the retry refusal
//! (all in core-domain, no I/O); document parsing (lane 15); the run
//! ledger (lane 14).

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

/// One pinned release of the Rust catalogue.
#[derive(Debug, Clone)]
pub struct CataloguePin {
    pub version: String,
    pub digest: String,
    pub node_count: i32,
    pub published_by: Option<String>,
}

/// Records a release. A re-record of the same version is a no-op rather
/// than a rewrite: the pin is history, and a record that must not be
/// edited is the same rule the run ledger follows.
pub async fn record_pin(
    tx: &mut PgConnection,
    version: &str,
    digest: &str,
    node_count: i32,
    published_by: Option<&str>,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO workflow_node_catalogue (version, digest, node_count, published_by)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (version) DO NOTHING"#,
        version,
        digest,
        node_count,
        published_by,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The most recently recorded release, or `None` before the first one.
pub async fn latest_pin<'e, E>(executor: E) -> DbResult<Option<CataloguePin>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT version, digest, node_count, published_by
           FROM workflow_node_catalogue
           ORDER BY recorded_at DESC
           LIMIT 1"#
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| CataloguePin {
        version: r.version,
        digest: r.digest,
        node_count: r.node_count,
        published_by: r.published_by,
    }))
}

/// True when the recorded digest for `version` differs from the one the
/// running catalogue computes — two builds claiming the same release with
/// different declarations. The caller decides what to do about it; this
/// only reports it.
pub async fn pin_matches_digest(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    version: &str,
    digest: &str,
) -> DbResult<bool> {
    let row = sqlx::query!(
        "SELECT digest FROM workflow_node_catalogue WHERE version = $1",
        version
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.is_none_or(|r| r.digest == digest))
}

// ---------------------------------------------------------------------------
// The plugin node contract (A2): the five new columns
// ---------------------------------------------------------------------------

/// A surface entry as a workflow node: the row's own five fields beside
/// the label and the `params_schema` that keeps riding inside
/// `definition`. Read through [`plugin_surface::list_install_entries`]
/// and parsed by `core_domain::node_registry::PluginNodeContract`.
#[derive(Debug, Clone)]
pub struct PluginNodeEntry {
    /// The install the entry belongs to, so a consumer can tell whose node
    /// it is and whether that install is still live.
    pub install_id: Uuid,
    pub entry_key: String,
    pub surface: String,
    pub label: String,
    pub definition: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    /// Conservative by default: a row written before 0128 (or by a path
    /// that does not set it) reads as retry-unsafe, so `flow.retry` and
    /// the operator retry refuse it until a plugin says otherwise.
    pub idempotent: bool,
    pub emits: Vec<String>,
    pub audience: String,
    pub agent_description: Option<String>,
}
