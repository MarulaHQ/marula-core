//! SLA policies and per-ticket SLA clock state.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::{BusinessCalendar, SlaPolicy, SlaTargets, TicketSlaState};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn get_default_policy(pool: &PgPool, org_id: Uuid) -> DbResult<SlaPolicy> {
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, is_default, targets, calendar
           FROM sla_policies WHERE org_id = $1 AND is_default = true LIMIT 1"#,
        org_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::NotFound)?;
    let targets: HashMap<String, SlaTargets> = serde_json::from_value(row.targets)
        .map_err(|e| DbError::Sql(sqlx::Error::ColumnDecode {
            index: "targets".into(),
            source: Box::new(e),
        }))?;
    let by_priority = targets
        .into_iter()
        .filter_map(|(k, v)| k.parse::<core_domain::Priority>().ok().map(|p| (p, v)))
        .collect();
    let calendar: BusinessCalendar = serde_json::from_value(row.calendar)
        .map_err(|e| DbError::Sql(sqlx::Error::ColumnDecode {
            index: "calendar".into(),
            source: Box::new(e),
        }))?;
    Ok(SlaPolicy {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        is_default: row.is_default,
        targets: by_priority,
        calendar,
    })
}

pub async fn upsert_ticket_state(pool: &PgPool, s: &TicketSlaState) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO ticket_sla_state
             (ticket_id, org_id, policy_id, first_response_due, resolution_due,
              first_responded_at, paused_business_secs, paused_since, breached, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
           ON CONFLICT (ticket_id) DO UPDATE SET
             policy_id = $3, first_response_due = $4, resolution_due = $5,
             first_responded_at = $6, paused_business_secs = $7,
             paused_since = $8, breached = $9, updated_at = now()"#,
        s.ticket_id,
        s.org_id,
        s.policy_id,
        s.first_response_due,
        s.resolution_due,
        s.first_responded_at,
        s.paused_business_secs,
        s.paused_since,
        s.breached,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_ticket_state(pool: &PgPool, ticket_id: Uuid) -> DbResult<Option<TicketSlaState>> {
    let row = sqlx::query!(
        r#"SELECT ticket_id, org_id, policy_id, first_response_due, resolution_due,
                  first_responded_at, paused_business_secs, paused_since, breached
           FROM ticket_sla_state WHERE ticket_id = $1"#,
        ticket_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| TicketSlaState {
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        policy_id: row.policy_id,
        first_response_due: row.first_response_due,
        resolution_due: row.resolution_due,
        first_responded_at: row.first_responded_at,
        paused_business_secs: row.paused_business_secs,
        paused_since: row.paused_since,
        breached: row.breached,
    }))
}

pub async fn mark_first_response(pool: &PgPool, ticket_id: Uuid, at: DateTime<Utc>) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE ticket_sla_state
           SET first_responded_at = COALESCE(first_responded_at, $2), updated_at = now()
           WHERE ticket_id = $1"#,
        ticket_id,
        at
    )
    .execute(pool)
    .await?;
    Ok(())
}
