//! Agent-platform persistence (TASKLIST Phase 3, IF8; migration 0040).
//!
//! Purpose: CRUD over the capability registry, the versioned agent-skill
//! store, and the proposal pipeline. Deliberately thin: the involvement
//! dial is DATA here (a `mode` column), decisions are compare-and-set
//! status transitions (a proposal can never be decided twice), and RLS
//! keeps tenancy absolute — authorization itself lives in core-api.
//!
//! Non-purpose: no rule evaluation (§4 layered rules land with the
//! write-execution lanes), no MCP transport state (the adapter is
//! stateless), no business logic (snapshots are opaque JSON the API layer
//! validates).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use uuid::Uuid;

// --- capability registry -------------------------------------------------------

/// One granted capability: the deny-by-default allowlist entry plus the
/// involvement-dial position (`proposed` | `autonomous`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CapabilityGrantRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub capability: String,
    pub mode: String,
    pub granted_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// All capability grants of one org, deterministic by capability slug.
pub async fn list_capability_grants(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<CapabilityGrantRow>> {
    let rows = sqlx::query_as!(
        CapabilityGrantRow,
        r#"SELECT id, org_id, capability, mode, granted_by, created_at, updated_at
           FROM mcp_capability_grants
           WHERE org_id = $1
           ORDER BY capability"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// One capability grant; `None` when the org has not granted it (the
/// deny-by-default case) or when the row is outside the org scope.
pub async fn get_capability_grant(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    capability: &str,
) -> DbResult<Option<CapabilityGrantRow>> {
    let row = sqlx::query_as!(
        CapabilityGrantRow,
        r#"SELECT id, org_id, capability, mode, granted_by, created_at, updated_at
           FROM mcp_capability_grants
           WHERE org_id = $1 AND capability = $2"#,
        org_id,
        capability,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

/// Upserts one capability grant (grant or re-dial). Returns the row.
pub async fn upsert_capability_grant(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    granted_by: Uuid,
    capability: &str,
    mode: &str,
) -> DbResult<CapabilityGrantRow> {
    let row = sqlx::query_as!(
        CapabilityGrantRow,
        r#"INSERT INTO mcp_capability_grants (id, org_id, capability, mode, granted_by)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (org_id, capability)
           DO UPDATE SET mode = EXCLUDED.mode, granted_by = EXCLUDED.granted_by,
                         updated_at = now()
           RETURNING id, org_id, capability, mode, granted_by, created_at, updated_at"#,
        id,
        org_id,
        capability,
        mode,
        granted_by,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Revokes one capability grant (deny again). Idempotent: revoking an
/// ungranted capability succeeds without effect.
pub async fn delete_capability_grant(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    capability: &str,
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM mcp_capability_grants WHERE org_id = $1 AND capability = $2",
        org_id,
        capability,
    )
    .execute(conn)
    .await?;
    Ok(())
}

// --- agent skills ---------------------------------------------------------------

/// One immutable skill version. Rows are append-only: a new version is a
/// NEW row and a rollback creates a NEW version carrying an old manifest.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct AgentSkillRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub slug: String,
    pub version: i32,
    pub manifest: serde_json::Value,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

const SKILL_COLUMNS: &str = "id, org_id, slug, version, manifest, created_by, created_at";

/// Inserts one skill version. The `(org_id, slug, version)` UNIQUE backs
/// the append-only history.
pub async fn insert_skill(
    conn: &mut sqlx::PgConnection,
    row_id: Uuid,
    org_id: Uuid,
    created_by: Uuid,
    slug: &str,
    version: i32,
    manifest: &serde_json::Value,
) -> DbResult<AgentSkillRow> {
    let row = sqlx::query_as!(
        AgentSkillRow,
        r#"INSERT INTO agent_skills (id, org_id, slug, version, manifest, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, slug, version, manifest, created_by, created_at"#,
        row_id,
        org_id,
        slug,
        version,
        manifest,
        created_by,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// One skill version by row id within the org.
pub async fn get_skill(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<AgentSkillRow> {
    let sql = format!("SELECT {SKILL_COLUMNS} FROM agent_skills WHERE org_id = $1 AND id = $2");
    sqlx::query_as::<_, AgentSkillRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// The LATEST version of every skill slug, deterministic by slug — the
/// listing view (history per slug is a separate read).
pub async fn list_latest_skills(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<AgentSkillRow>> {
    let rows = sqlx::query_as!(
        AgentSkillRow,
        r#"SELECT DISTINCT ON (slug) id, org_id, slug, version, manifest, created_by, created_at
           FROM agent_skills
           WHERE org_id = $1
           ORDER BY slug, version DESC"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Every version of one slug, newest first (the diff/rollback view).
pub async fn list_skill_versions(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    slug: &str,
) -> DbResult<Vec<AgentSkillRow>> {
    let rows = sqlx::query_as!(
        AgentSkillRow,
        r#"SELECT id, org_id, slug, version, manifest, created_by, created_at
           FROM agent_skills
           WHERE org_id = $1 AND slug = $2
           ORDER BY version DESC"#,
        org_id,
        slug,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// The highest version number published for a slug (0 when unpublished —
/// the next version is max+1, append-only, no edits in place).
pub async fn max_skill_version(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    slug: &str,
) -> DbResult<i32> {
    let max: Option<i32> =
        sqlx::query_scalar("SELECT MAX(version) FROM agent_skills WHERE org_id = $1 AND slug = $2")
            .bind(org_id)
            .bind(slug)
            .fetch_one(executor)
            .await?;
    Ok(max.unwrap_or(0))
}

// --- agent proposals --------------------------------------------------------------

/// One IF8 proposal with its full snapshot (before/proposed values, agent
/// provenance, dial position) and lifecycle state.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct AgentProposalRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub entity: String,
    pub entity_id: Option<Uuid>,
    pub op: String,
    pub before_value: serde_json::Value,
    pub proposed_value: serde_json::Value,
    pub classification: String,
    pub capability: Option<String>,
    pub dial_mode: String,
    pub base_version: i64,
    pub status: String,
    pub agent_kind: String,
    pub agent_ref: String,
    pub skill_slug: Option<String>,
    pub skill_version: Option<i32>,
    pub rule_set_hash: Option<String>,
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub for_user: Option<Uuid>,
    pub rationale: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const PROPOSAL_COLUMNS: &str = "id, org_id, entity, entity_id, op, before_value, proposed_value, \
     classification, capability, dial_mode, base_version, status, agent_kind, agent_ref, \
     skill_slug, skill_version, rule_set_hash, session_id, tool, for_user, rationale, \
     decided_by, decided_at, decision_note, applied_at, created_at, updated_at";

/// Fields of a proposal insert (everything except id/org/created_at).
#[derive(Debug, Clone)]
pub struct ProposalFields {
    pub entity: String,
    pub entity_id: Option<Uuid>,
    pub op: String,
    pub before_value: serde_json::Value,
    pub proposed_value: serde_json::Value,
    pub classification: String,
    pub capability: Option<String>,
    pub dial_mode: String,
    pub base_version: i64,
    pub agent_kind: String,
    pub agent_ref: String,
    pub skill_slug: Option<String>,
    pub skill_version: Option<i32>,
    pub rule_set_hash: Option<String>,
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub for_user: Option<Uuid>,
    pub rationale: Option<String>,
}

/// Inserts one proposal in the `proposed` state.
pub async fn insert_proposal(
    conn: &mut sqlx::PgConnection,
    row_id: Uuid,
    org_id: Uuid,
    fields: &ProposalFields,
) -> DbResult<AgentProposalRow> {
    let row = sqlx::query_as!(
        AgentProposalRow,
        r#"INSERT INTO agent_proposals
             (id, org_id, entity, entity_id, op, before_value, proposed_value,
              classification, capability, dial_mode, base_version, agent_kind, agent_ref,
              skill_slug, skill_version, rule_set_hash, session_id, tool, for_user, rationale)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                   $18, $19, $20)
           RETURNING id, org_id, entity, entity_id, op, before_value, proposed_value,
             classification, capability, dial_mode, base_version, status, agent_kind, agent_ref,
             skill_slug, skill_version, rule_set_hash, session_id, tool, for_user, rationale,
             decided_by, decided_at, decision_note, applied_at, created_at, updated_at"#,
        row_id,
        org_id,
        fields.entity,
        fields.entity_id,
        fields.op,
        fields.before_value,
        fields.proposed_value,
        fields.classification,
        fields.capability,
        fields.dial_mode,
        fields.base_version,
        fields.agent_kind,
        fields.agent_ref,
        fields.skill_slug,
        fields.skill_version,
        fields.rule_set_hash,
        fields.session_id,
        fields.tool,
        fields.for_user,
        fields.rationale,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// One proposal by id within the org. A miss under RLS (another org's
/// row, deleted, or never existed) is deliberately indistinguishable.
pub async fn get_proposal(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<AgentProposalRow> {
    let sql =
        format!("SELECT {PROPOSAL_COLUMNS} FROM agent_proposals WHERE org_id = $1 AND id = $2");
    sqlx::query_as::<_, AgentProposalRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// Cursor-paged proposal listing, newest first, optionally filtered by
/// status. `after_created`/`after_id` carry the previous page's last row
/// (both None for the first page).
pub async fn list_proposals(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    status: Option<&str>,
    after_created: Option<DateTime<Utc>>,
    after_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<AgentProposalRow>> {
    let sql = format!(
        "SELECT {PROPOSAL_COLUMNS} FROM agent_proposals
         WHERE org_id = $1
           AND ($2::text IS NULL OR status = $2)
           AND (created_at, id) < ($3, $4)
         ORDER BY created_at DESC, id DESC
         LIMIT $5"
    );
    let rows = sqlx::query_as::<_, AgentProposalRow>(&sql)
        .bind(org_id)
        .bind(status)
        .bind(after_created.unwrap_or(DateTime::from_timestamp(0, 0).expect("epoch")))
        .bind(after_id.unwrap_or(Uuid::nil()))
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows)
}

/// Compare-and-set decision: `proposed` → the target status. Returns the
/// updated row; `None` when the proposal was no longer `proposed` (already
/// decided/withdrawn) or left the org scope — the caller surfaces the
/// typed conflict, never a double decision.
pub async fn decide_proposal(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    status: &str,
    decided_by: Uuid,
    note: Option<&str>,
) -> DbResult<Option<AgentProposalRow>> {
    let row = sqlx::query_as!(
        AgentProposalRow,
        r#"UPDATE agent_proposals
           SET status = $3, decided_by = $4, decided_at = now(), decision_note = $5,
               updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'proposed'
           RETURNING id, org_id, entity, entity_id, op, before_value, proposed_value,
             classification, capability, dial_mode, base_version, status, agent_kind, agent_ref,
             skill_slug, skill_version, rule_set_hash, session_id, tool, for_user, rationale,
             decided_by, decided_at, decision_note, applied_at, created_at, updated_at"#,
        org_id,
        id,
        status,
        decided_by,
        note,
    )
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// Marks an approved proposal `applied` after its write landed through the
/// IF1 path. Guarded on `approved` so only a decided proposal can apply.
pub async fn mark_proposal_applied(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    let result = sqlx::query!(
        "UPDATE agent_proposals SET status = 'applied', applied_at = now(), updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND status = 'approved'",
        org_id,
        id,
    )
    .execute(conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The entity write-seq a proposal was created against (0 when the entity
/// has never been written through the instant-first path yet).
pub async fn entity_current_seq(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    entity: &str,
    entity_id: Uuid,
) -> DbResult<i64> {
    let seq: Option<i64> = sqlx::query_scalar(
        "SELECT seq FROM entity_write_seqs WHERE org_id = $1 AND entity = $2 AND entity_id = $3",
    )
    .bind(org_id)
    .bind(entity)
    .bind(entity_id)
    .fetch_optional(executor)
    .await?;
    Ok(seq.unwrap_or(0))
}
