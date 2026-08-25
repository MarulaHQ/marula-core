//! Append-only audit event persistence (§6).

use crate::{DbResult};
use core_domain::entities::{Actor, AuditEvent};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn insert(pool: &PgPool, e: &AuditEvent) -> DbResult<()> {
    let (kind, id): (String, Option<String>) = match &e.actor {
        Actor::Human(u) => ("human".into(), Some(u.to_string())),
        Actor::Agent(a) => ("agent".into(), Some(a.clone())),
        Actor::Plugin(p) => ("plugin".into(), Some(p.clone())),
        Actor::Connector(c) => ("connector".into(), Some(c.clone())),
        Actor::System => ("system".into(), None),
    };
    sqlx::query!(
        r#"INSERT INTO audit_events (id, org_id, actor_kind, actor_id, action,
                                     entity_type, entity_id, detail, ip, occurred_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        e.id,
        e.org_id,
        kind,
        id,
        e.action,
        e.entity_type,
        e.entity_id,
        e.detail,
        e.ip,
        e.occurred_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(
    pool: &PgPool,
    org_id: Uuid,
    limit: i64,
    before: Option<chrono::DateTime<chrono::Utc>>,
) -> DbResult<Vec<AuditEvent>> {
    let rows = sqlx::query!(
        r#"SELECT id, org_id, actor_kind, actor_id, action, entity_type, entity_id,
                  detail, ip, occurred_at
           FROM audit_events
           WHERE org_id = $1 AND ($2::timestamptz IS NULL OR occurred_at < $2)
           ORDER BY occurred_at DESC LIMIT $3"#,
        org_id,
        before,
        limit
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let actor = match (row.actor_kind.as_str(), row.actor_id.clone()) {
                ("human", Some(id)) => Actor::Human(id.parse().unwrap_or(Uuid::nil())),
                ("agent", Some(a)) => Actor::Agent(a),
                ("plugin", Some(p)) => Actor::Plugin(p),
                ("connector", Some(c)) => Actor::Connector(c),
                _ => Actor::System,
            };
            AuditEvent {
                id: row.id,
                org_id: row.org_id,
                actor,
                action: row.action,
                entity_type: row.entity_type,
                entity_id: row.entity_id,
                detail: row.detail,
                ip: row.ip,
                occurred_at: row.occurred_at,
            }
        })
        .collect())
}
