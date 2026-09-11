//! Sales-pipeline persistence (TASKLIST Phase 3, Wave 12 lane 2;
//! migration 0060): stages, deals, and the append-only transition
//! ledger.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate).
//! Stage moves are one guarded UPDATE + one ledger INSERT inside the
//! caller's transaction: the `None` return means the deal or target stage
//! left its expected state mid-flight, which the API maps to a 409.

use crate::{DbError, DbResult};
use core_domain::entities::{
    DealStatus, PipelineDeal, PipelineDealTransition, PipelineStage, StageType,
};
use sqlx::Postgres;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct StageRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    position: i32,
    stage_type: String,
    active: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn stage_row(r: StageRow) -> PipelineStage {
    PipelineStage {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        position: r.position,
        stage_type: StageType::from_str(&r.stage_type).unwrap_or(StageType::Open),
        active: r.active,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const STAGE_COLUMNS: &str =
    "id, org_id, name, position, stage_type, active, created_at, updated_at";

pub async fn insert_stage<'e, E>(executor: E, stage: &PipelineStage) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO pipeline_stages (id, org_id, name, position, stage_type, active)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(stage.id)
    .bind(stage.org_id)
    .bind(stage.name.trim())
    .bind(stage.position)
    .bind(stage.stage_type.as_str())
    .bind(stage.active)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_stage<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PipelineStage>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {STAGE_COLUMNS} FROM pipeline_stages WHERE org_id = $1 AND id = $2");
    let row = sqlx::query_as::<_, StageRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(stage_row(row))
}

pub async fn list_stages<'e, E>(
    executor: E,
    org_id: Uuid,
    include_inactive: bool,
) -> DbResult<Vec<PipelineStage>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {STAGE_COLUMNS} FROM pipeline_stages
         WHERE org_id = $1 AND ($2::boolean = false OR active)
         ORDER BY position, name, id"
    );
    let rows = sqlx::query_as::<_, StageRow>(&sql)
        .bind(org_id)
        .bind(include_inactive)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(stage_row).collect())
}

/// Full-row update. `None` = the stage does not exist.
pub async fn update_stage<'e, E>(
    executor: E,
    stage: &PipelineStage,
) -> DbResult<Option<PipelineStage>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE pipeline_stages SET name = $3, position = $4, stage_type = $5,
                                    active = $6, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {STAGE_COLUMNS}"
    );
    let row = sqlx::query_as::<_, StageRow>(&sql)
        .bind(stage.org_id)
        .bind(stage.id)
        .bind(stage.name.trim())
        .bind(stage.position)
        .bind(stage.stage_type.as_str())
        .bind(stage.active)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(stage_row))
}

// --- deals ---------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct DealRow {
    id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    quote_id: Option<Uuid>,
    stage_id: Uuid,
    name: String,
    amount_minor: i64,
    currency: String,
    status: String,
    expected_close_on: Option<chrono::NaiveDate>,
    owner_user_id: Option<Uuid>,
    notes: Option<String>,
    closed_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn deal_row(r: DealRow) -> PipelineDeal {
    PipelineDeal {
        id: r.id,
        org_id: r.org_id,
        client_id: r.client_id,
        quote_id: r.quote_id,
        stage_id: r.stage_id,
        name: r.name,
        amount_minor: r.amount_minor,
        currency: r.currency,
        status: DealStatus::from_str(&r.status).unwrap_or(DealStatus::Open),
        expected_close_on: r.expected_close_on,
        owner_user_id: r.owner_user_id,
        notes: r.notes,
        closed_at: r.closed_at,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const DEAL_COLUMNS: &str = "id, org_id, client_id, quote_id, stage_id, name, amount_minor,
                             currency, status, expected_close_on, owner_user_id, notes,
                             closed_at, created_by, created_at, updated_at";

pub async fn insert_deal<'e, E>(executor: E, deal: &PipelineDeal) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO pipeline_deals (id, org_id, client_id, quote_id, stage_id, name,
                                     amount_minor, currency, status, expected_close_on,
                                     owner_user_id, notes, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(deal.id)
    .bind(deal.org_id)
    .bind(deal.client_id)
    .bind(deal.quote_id)
    .bind(deal.stage_id)
    .bind(deal.name.trim())
    .bind(deal.amount_minor)
    .bind(&deal.currency)
    .bind(deal.status.as_str())
    .bind(deal.expected_close_on)
    .bind(deal.owner_user_id)
    .bind(deal.notes.as_deref())
    .bind(deal.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_deal<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PipelineDeal>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {DEAL_COLUMNS} FROM pipeline_deals WHERE org_id = $1 AND id = $2");
    let row = sqlx::query_as::<_, DealRow>(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(deal_row(row))
}

pub struct DealFilters {
    pub client_id: Option<Uuid>,
    pub stage_id: Option<Uuid>,
    pub quote_id: Option<Uuid>,
    /// Exact status: `open|won|lost`.
    pub status: Option<String>,
    /// Restrict to one owner.
    pub owner_user_id: Option<Uuid>,
}

pub async fn list_deals<'e, E>(
    executor: E,
    org_id: Uuid,
    filters: &DealFilters,
    limit: i64,
) -> DbResult<Vec<PipelineDeal>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {DEAL_COLUMNS} FROM pipeline_deals
         WHERE org_id = $1
           AND ($2::uuid IS NULL OR client_id = $2)
           AND ($3::uuid IS NULL OR stage_id = $3)
           AND ($4::uuid IS NULL OR quote_id = $4)
           AND ($5::text IS NULL OR status = $5)
           AND ($6::uuid IS NULL OR owner_user_id = $6)
         ORDER BY created_at DESC, id DESC
         LIMIT $7"
    );
    let rows = sqlx::query_as::<_, DealRow>(&sql)
        .bind(org_id)
        .bind(filters.client_id)
        .bind(filters.stage_id)
        .bind(filters.quote_id)
        .bind(filters.status.as_deref())
        .bind(filters.owner_user_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.into_iter().map(deal_row).collect())
}

/// Editable-field update (everything except stage/status — those move via
/// [`move_deal_stage`]). `None` = the deal does not exist.
pub async fn update_deal<'e, E>(executor: E, deal: &PipelineDeal) -> DbResult<Option<PipelineDeal>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE pipeline_deals SET client_id = $3, quote_id = $4, name = $5,
                                   amount_minor = $6, currency = $7,
                                   expected_close_on = $8, owner_user_id = $9,
                                   notes = $10, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {DEAL_COLUMNS}"
    );
    let row = sqlx::query_as::<_, DealRow>(&sql)
        .bind(deal.org_id)
        .bind(deal.id)
        .bind(deal.client_id)
        .bind(deal.quote_id)
        .bind(deal.name.trim())
        .bind(deal.amount_minor)
        .bind(&deal.currency)
        .bind(deal.expected_close_on)
        .bind(deal.owner_user_id)
        .bind(deal.notes.as_deref())
        .fetch_optional(executor)
        .await?;
    Ok(row.map(deal_row))
}

/// Moves a deal to `to_stage_id`, re-deriving `status` from the target
/// stage's kind and stamping `closed_at` when the deal closes. Guarded on
/// the CURRENT stage (`expected_from_stage_id`): a concurrent move wins,
/// the loser gets `None` (typed 409 at the API layer). The caller inserts
/// the ledger row in the same transaction ([`insert_transition`]) — the
/// ledger is the audit trail, this UPDATE is the state.
pub async fn move_deal_stage<'e, E>(
    executor: E,
    org_id: Uuid,
    deal_id: Uuid,
    expected_from_stage_id: Uuid,
    to_stage_id: Uuid,
    status: &str,
) -> DbResult<Option<PipelineDeal>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE pipeline_deals SET stage_id = $3, status = $4,
                     closed_at = CASE WHEN $4 IN ('won','lost') THEN now() ELSE NULL END,
                     updated_at = now()
         WHERE org_id = $1 AND id = $2 AND stage_id = $5
         RETURNING {DEAL_COLUMNS}"
    );
    let row = sqlx::query_as::<_, DealRow>(&sql)
        .bind(org_id)
        .bind(deal_id)
        .bind(to_stage_id)
        .bind(status)
        .bind(expected_from_stage_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.map(deal_row))
}

/// Appends one immutable transition row. `from_stage_id` is `None` for
/// the initial placement recorded at deal creation.
pub async fn insert_transition<'e, E>(
    executor: E,
    transition: &PipelineDealTransition,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO pipeline_deal_transitions (id, org_id, deal_id, from_stage_id,
                                                to_stage_id, note, actor_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(transition.id)
    .bind(transition.org_id)
    .bind(transition.deal_id)
    .bind(transition.from_stage_id)
    .bind(transition.to_stage_id)
    .bind(transition.note.as_deref())
    .bind(transition.actor_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_transitions<'e, E>(
    executor: E,
    org_id: Uuid,
    deal_id: Uuid,
) -> DbResult<Vec<PipelineDealTransition>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    #[derive(Debug, sqlx::FromRow)]
    struct TransitionRow {
        id: Uuid,
        org_id: Uuid,
        deal_id: Uuid,
        from_stage_id: Option<Uuid>,
        to_stage_id: Uuid,
        note: Option<String>,
        actor_id: Option<Uuid>,
        created_at: chrono::DateTime<chrono::Utc>,
    }
    let rows = sqlx::query_as::<_, TransitionRow>(
        r#"SELECT id, org_id, deal_id, from_stage_id, to_stage_id, note, actor_id, created_at
           FROM pipeline_deal_transitions
           WHERE org_id = $1 AND deal_id = $2
           ORDER BY created_at DESC, id DESC"#,
    )
    .bind(org_id)
    .bind(deal_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PipelineDealTransition {
            id: r.id,
            org_id: r.org_id,
            deal_id: r.deal_id,
            from_stage_id: r.from_stage_id,
            to_stage_id: r.to_stage_id,
            note: r.note,
            actor_id: r.actor_id,
            created_at: r.created_at,
        })
        .collect())
}
