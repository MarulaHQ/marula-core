//! Append-only audit event persistence (§6).

use crate::DbResult;
use core_domain::entities::{Actor, AuditEvent};
use sqlx::Postgres;
use uuid::Uuid;

/// Filters for the admin audit query API. All optional; unset = unfiltered.
#[derive(Debug, Default, Clone)]
pub struct AuditFilter {
    /// Actor filter: a user uuid, or "system".
    pub actor: Option<String>,
    /// Action prefix match, e.g. "auth" covers every auth.* event.
    pub action: Option<String>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    /// Keyset cursor: strictly older than (occurred_at, id).
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    pub cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
}

pub async fn insert<'e, E>(executor: E, e: &AuditEvent) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let (kind, id): (String, Option<String>) = match &e.actor {
        Actor::Human(u) => ("human".into(), Some(u.to_string())),
        Actor::Agent(a) => ("agent".into(), Some(a.clone())),
        Actor::Plugin(p) => ("plugin".into(), Some(p.clone())),
        Actor::Connector(c) => ("connector".into(), Some(c.clone())),
        Actor::System => ("system".into(), None),
        Actor::PortalContact(u) => ("portal_contact".into(), Some(u.to_string())),
        Actor::PortalApprover(u) => ("portal_approver".into(), Some(u.to_string())),
        Actor::PortalItManager(u) => ("portal_it_manager".into(), Some(u.to_string())),
        // Import runs (TASKLIST D2): descriptor names the driving identity,
        // e.g. "user:<uuid>" or "api-key:<key-id>".
        Actor::Import(desc) => ("import".into(), Some(desc.clone())),
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
    .execute(executor)
    .await?;
    Ok(())
}

fn event_from_row(row: sqlx::postgres::PgRow) -> AuditEvent {
    use sqlx::Row;
    let actor_kind: String = row.get("actor_kind");
    let actor_id: Option<String> = row.get("actor_id");
    let actor = match (actor_kind.as_str(), actor_id.clone()) {
        ("human", Some(id)) => Actor::Human(id.parse().unwrap_or(Uuid::nil())),
        ("agent", Some(a)) => Actor::Agent(a),
        ("plugin", Some(p)) => Actor::Plugin(p),
        ("connector", Some(c)) => Actor::Connector(c),
        ("portal_contact", Some(id)) => Actor::PortalContact(id.parse().unwrap_or(Uuid::nil())),
        ("portal_approver", Some(id)) => Actor::PortalApprover(id.parse().unwrap_or(Uuid::nil())),
        ("portal_it_manager", Some(id)) => {
            Actor::PortalItManager(id.parse().unwrap_or(Uuid::nil()))
        }
        ("import", Some(desc)) => Actor::Import(desc),
        _ => Actor::System,
    };
    AuditEvent {
        id: row.get("id"),
        org_id: row.get("org_id"),
        actor,
        action: row.get("action"),
        entity_type: row.get("entity_type"),
        entity_id: row.get("entity_id"),
        detail: row.get("detail"),
        ip: row.get("ip"),
        occurred_at: row.get("occurred_at"),
    }
}

pub async fn list<'e, E>(
    executor: E,
    org_id: Uuid,
    limit: i64,
    before: Option<chrono::DateTime<chrono::Utc>>,
) -> DbResult<Vec<AuditEvent>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    list_filtered(
        executor,
        org_id,
        AuditFilter {
            before,
            ..AuditFilter::default()
        },
        limit,
    )
    .await
}

/// Admin filtered listing with keyset pagination on (occurred_at DESC, id DESC).
pub async fn list_filtered<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: AuditFilter,
    limit: i64,
) -> DbResult<Vec<AuditEvent>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, actor_kind, actor_id, action, entity_type, entity_id,
                  detail, ip, occurred_at
           FROM audit_events
           WHERE org_id = $1
             AND ($2::text IS NULL OR actor_id = $2 OR ($2 = 'system' AND actor_kind = 'system'))
             AND ($3::text IS NULL OR action LIKE $3 || '%')
             AND ($4::timestamptz IS NULL OR occurred_at >= $4)
             AND ($5::timestamptz IS NULL OR occurred_at <= $5)
             AND ($6::timestamptz IS NULL OR occurred_at < $6)
             AND ($7::timestamptz IS NULL OR $8::uuid IS NULL OR
                  (occurred_at, id) < ($7::timestamptz, $8::uuid))
           ORDER BY occurred_at DESC, id DESC
           LIMIT $9"#,
    )
    .bind(org_id)
    .bind(filter.actor.as_deref())
    .bind(filter.action.as_deref())
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.before)
    .bind(filter.cursor.map(|c| c.0))
    .bind(filter.cursor.map(|c| c.1))
    .bind(limit.clamp(1, 500))
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(event_from_row).collect())
}
