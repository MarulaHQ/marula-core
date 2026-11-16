//! Knowledge-home persistence: client documentation homes, rack layouts,
//! network records, domain records, procedures, expiry rollups, the generic
//! knowledge edge and documentation templates.
//!
//! Purpose: every query behind `routes::knowledge_home`. The read models here
//! exist so the client-first home answers "which client" in one round trip,
//! and so a record page is a single fetch rather than a fan-out of counts.
//!
//! Non-purpose: no authorization decisions. Every function takes an executor
//! that is already inside the org-scoped transaction (RLS set), and every
//! call site is gated by the route's permission check. Nothing here knows
//! what a role is.
//!
//! Honesty rules that shaped the SQL:
//!
//! * Completeness is a counted set of legs with names, never a bare
//!   percentage. [`Completeness`] carries both the count and the gaps.
//! * Rotation age reads `secret_records.last_rotated_at` (migration 0096), not
//!   `updated_at`, because a metadata edit is not a rotation.
//! * Expiries name where the date came from, so a warranty read out of an
//!   asset's typed fields is never presented as a registry fact.

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::Postgres;
use uuid::Uuid;

/* ------------------------------------------------------------------------- */
/* The client home                                                           */
/* ------------------------------------------------------------------------- */

/// One counted lift of every fact the home and the hub need for a client.
#[derive(Debug, Clone, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientFacts {
    pub client_id: Uuid,
    pub contacts_n: i64,
    pub sites_n: i64,
    pub assets_n: i64,
    pub documents_n: i64,
    pub articles_n: i64,
    pub secrets_n: i64,
    pub racks_n: i64,
    pub networks_n: i64,
    pub domains_n: i64,
    pub procedures_n: i64,
    pub stale_secrets_n: i64,
    pub expiring_n: i64,
    pub latest_document_at: Option<DateTime<Utc>>,
    pub latest_article_at: Option<DateTime<Utc>>,
    pub latest_rack_at: Option<DateTime<Utc>>,
    pub latest_procedure_at: Option<DateTime<Utc>>,
}

const FACTS_SQL: &str = r#"
SELECT
  c.id AS client_id,
  (SELECT count(*) FROM contacts x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS contacts_n,
  (SELECT count(*) FROM sites x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS sites_n,
  (SELECT count(*) FROM device_assets x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS assets_n,
  (SELECT count(*) FROM documents x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS documents_n,
  (SELECT count(*) FROM kb_articles x
     WHERE x.org_id = c.org_id AND x.client_visible
       AND x.status = 'published') AS articles_n,
  (SELECT count(*) FROM secret_records x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS secrets_n,
  (SELECT count(*) FROM knowledge_racks x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS racks_n,
  (SELECT count(*) FROM knowledge_networks x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS networks_n,
  (SELECT count(*) FROM knowledge_domains x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS domains_n,
  (SELECT count(*) FROM knowledge_procedures x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS procedures_n,
  (SELECT count(*) FROM secret_records x
     WHERE x.org_id = c.org_id AND x.client_id = c.id
       AND coalesce(x.last_rotated_at, x.created_at) < now() - interval '180 days') AS stale_secrets_n,
  (
    (SELECT count(*) FROM knowledge_domains d
       WHERE d.org_id = c.org_id AND d.client_id = c.id
         AND (d.registrar_expiry < current_date + 60
              OR d.ssl_expiry < current_date + 60)) +
    (SELECT count(*) FROM documents d
       WHERE d.org_id = c.org_id AND d.client_id = c.id
         AND d.expires_at IS NOT NULL AND d.expires_at < now() + interval '60 days')
  ) AS expiring_n,
  (SELECT max(x.updated_at) FROM documents x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS latest_document_at,
  (SELECT max(x.updated_at) FROM kb_articles x
     WHERE x.org_id = c.org_id AND x.client_visible
       AND x.status = 'published') AS latest_article_at,
  (SELECT max(x.updated_at) FROM knowledge_racks x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS latest_rack_at,
  (SELECT max(x.updated_at) FROM knowledge_procedures x
     WHERE x.org_id = c.org_id AND x.client_id = c.id) AS latest_procedure_at
FROM clients c
WHERE c.org_id = $1
"#;

pub async fn list_client_facts<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<ClientFacts>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("{FACTS_SQL} ORDER BY c.name");
    Ok(sqlx::query_as::<_, ClientFacts>(&sql)
        .bind(org_id)
        .fetch_all(executor)
        .await?)
}

pub async fn client_facts<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<ClientFacts>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("{FACTS_SQL} AND c.id = $2");
    sqlx::query_as::<_, ClientFacts>(&sql)
        .bind(org_id)
        .bind(client_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

pub async fn client_name<'e, E>(executor: E, org_id: Uuid, client_id: Uuid) -> DbResult<String>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar("SELECT name FROM clients WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(client_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)
}

/// The named documentation legs. A leg is complete when its count is at least
/// one; the reason is the sentence a technician reads on the hub, so the hub
/// never shows a percentage without saying what is missing.
pub const COMPLETENESS_LEGS: [(&str, &str); 8] = [
    ("contacts", "Add a contact, so an escalation has a name"),
    ("sites", "Add a site, so every record has a place"),
    ("assets", "Add a configuration, so the estate is counted"),
    (
        "documents",
        "Write or link a document, so the process is written down",
    ),
    (
        "secrets",
        "Store a credential, so access does not live in a chat",
    ),
    (
        "network",
        "Document the network, so an address conflict is visible",
    ),
    (
        "domains",
        "Add a domain or certificate, so an expiry is not a surprise",
    ),
    (
        "procedures",
        "Write a procedure, so the next person can repeat it",
    ),
];

#[derive(Debug, Clone)]
pub struct Completeness {
    pub complete: i64,
    pub total: i64,
    pub gaps: Vec<String>,
}

impl ClientFacts {
    pub fn completeness(&self) -> Completeness {
        let legs = [
            self.contacts_n > 0,
            self.sites_n > 0,
            self.assets_n > 0,
            self.documents_n > 0 || self.articles_n > 0,
            self.secrets_n > 0,
            self.networks_n > 0,
            self.domains_n > 0,
            self.procedures_n > 0,
        ];
        let gaps = legs
            .iter()
            .zip(COMPLETENESS_LEGS.iter())
            .filter(|(done, _)| !**done)
            .map(|(_, (_, reason))| (*reason).to_string())
            .collect();
        Completeness {
            complete: legs.iter().filter(|done| **done).count() as i64,
            total: legs.len() as i64,
            gaps,
        }
    }

    pub fn last_activity(&self) -> Option<DateTime<Utc>> {
        [
            self.latest_document_at,
            self.latest_article_at,
            self.latest_rack_at,
            self.latest_procedure_at,
        ]
        .into_iter()
        .flatten()
        .max()
    }
}

/* ------------------------------------------------------------------------- */
/* Activity and expiries                                                     */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecentChange {
    pub kind: String,
    pub title: String,
    pub entity_id: Uuid,
    pub actor_name: Option<String>,
    pub changed_at: DateTime<Utc>,
}

/// Recent documentation movement for one client: the newest write per record,
/// read from the record tables themselves. A delete removes the row from this
/// feed; the deletion act survives in the audit trail, which the record
/// surface does not read.
pub async fn recent_changes<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    limit: i64,
) -> DbResult<Vec<RecentChange>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, RecentChange>(
        r#"SELECT * FROM (
             SELECT 'document'::text AS kind, d.title, d.id AS entity_id,
                    u.display_name AS actor_name, d.updated_at AS changed_at
               FROM documents d
               LEFT JOIN users u ON u.id = d.updated_by
              WHERE d.org_id = $1 AND d.client_id = $2
             UNION ALL
             SELECT 'article', a.title, a.id,
                    u.display_name, a.updated_at
               FROM kb_articles a
               LEFT JOIN users u ON u.id = a.updated_by
              WHERE a.org_id = $1 AND a.client_visible
                AND a.status = 'published'
             UNION ALL
             SELECT 'asset', a.name, a.id, NULL, a.updated_at
               FROM device_assets a
              WHERE a.org_id = $1 AND a.client_id = $2
             UNION ALL
             SELECT 'rack', r.name, r.id,
                    u.display_name, r.updated_at
               FROM knowledge_racks r
               LEFT JOIN users u ON u.id = r.updated_by
              WHERE r.org_id = $1 AND r.client_id = $2
             UNION ALL
             SELECT 'network', n.name, n.id,
                    u.display_name, n.updated_at
               FROM knowledge_networks n
               LEFT JOIN users u ON u.id = n.updated_by
              WHERE n.org_id = $1 AND n.client_id = $2
             UNION ALL
             SELECT 'domain', d.domain, d.id,
                    u.display_name, d.updated_at
               FROM knowledge_domains d
               LEFT JOIN users u ON u.id = d.updated_by
              WHERE d.org_id = $1 AND d.client_id = $2
             UNION ALL
             SELECT 'procedure', p.title, p.id,
                    u.display_name, p.updated_at
               FROM knowledge_procedures p
               LEFT JOIN users u ON u.id = p.updated_by
              WHERE p.org_id = $1 AND p.client_id = $2
           ) change
          ORDER BY changed_at DESC
          LIMIT $3"#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(limit)
    .fetch_all(executor)
    .await?)
}

/// One expiring thing, with its date and who owns it.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExpiryRow {
    pub kind: String,
    pub title: String,
    pub client_id: Uuid,
    pub client_name: String,
    pub due_on: NaiveDate,
    pub owner: Option<String>,
    pub source_id: Option<Uuid>,
    /// Why this date is trusted: a registry of record, or a field someone
    /// typed. The hub prints this so a typed warranty never reads as a probe.
    pub provenance: String,
}

/// Every expiring thing in scope, soonest first. One query: domains carry
/// registrar and TLS dates, documents carry a review date, credentials carry
/// a rotation date, and an asset carries its warranty end in its typed fields.
pub async fn expirations<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
    horizon_days: i32,
) -> DbResult<Vec<ExpiryRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, ExpiryRow>(
        r#"SELECT * FROM (
             SELECT 'domain_registrar'::text AS kind, d.domain AS title,
                    d.client_id, c.name AS client_name,
                    d.registrar_expiry AS due_on,
                    coalesce(d.registrar, 'No registrar recorded') AS owner,
                    d.id AS source_id,
                    'registrar record'::text AS provenance
               FROM knowledge_domains d
               JOIN clients c ON c.id = d.client_id
              WHERE d.org_id = $1 AND d.registrar_expiry IS NOT NULL
             UNION ALL
             SELECT 'domain_tls', d.domain, d.client_id, c.name,
                    d.ssl_expiry,
                    coalesce(d.ssl_issuer, 'No issuer recorded'),
                    d.id, 'certificate record'
               FROM knowledge_domains d
               JOIN clients c ON c.id = d.client_id
              WHERE d.org_id = $1 AND d.ssl_expiry IS NOT NULL
             UNION ALL
             SELECT 'document_review', d.title, d.client_id, c.name,
                    (d.expires_at AT TIME ZONE 'UTC')::date,
                    coalesce(u.display_name, 'No owner'),
                    d.id, 'document review date'
               FROM documents d
               JOIN clients c ON c.id = d.client_id
               LEFT JOIN users u ON u.id = d.updated_by
              WHERE d.org_id = $1 AND d.expires_at IS NOT NULL
             UNION ALL
             -- A warranty date is typed by a person, so it is parsed here
             -- rather than cast: a shape filter still admits 2026-02-30, and
             -- `::date` on that raises and fails the whole rollup for the
             -- org. A value that is not a real calendar date yields no
             -- due_on and drops out of the horizon filter instead.
             SELECT 'asset_warranty', w.name, w.client_id, w.client_name,
                    CASE WHEN to_char(w.candidate, 'YYYY-MM-DD') = w.raw
                         THEN w.candidate END,
                    w.tag, w.id, 'asset field'
               FROM (
                 SELECT a.name, a.client_id, c.name AS client_name, a.tag, a.id,
                        v.raw,
                        CASE WHEN substr(v.raw, 1, 4)::int BETWEEN 1 AND 9999
                              AND substr(v.raw, 6, 2)::int BETWEEN 1 AND 12
                              AND substr(v.raw, 9, 2)::int BETWEEN 1 AND 31
                             THEN make_date(substr(v.raw, 1, 4)::int,
                                            substr(v.raw, 6, 2)::int, 1)
                                  + (substr(v.raw, 9, 2)::int - 1)
                        END AS candidate
                   FROM device_assets a
                   JOIN clients c ON c.id = a.client_id
                   CROSS JOIN LATERAL (
                     SELECT CASE WHEN a.fields->>'warranty_end'
                                       ~ '^\d{4}-\d{2}-\d{2}$'
                                 THEN a.fields->>'warranty_end' END AS raw
                   ) v
                  WHERE a.org_id = $1
               ) w
             UNION ALL
             SELECT 'secret_rotation', r.title, r.client_id, c.name,
                    (coalesce(r.last_rotated_at, r.created_at) + interval '180 days')::date,
                    'Vault category owner', r.id, 'rotation policy'
               FROM secret_records r
               JOIN clients c ON c.id = r.client_id
              WHERE r.org_id = $1
           ) e
          WHERE e.due_on <= current_date + $3::int
            AND ($2::uuid IS NULL OR e.client_id = $2::uuid)
          ORDER BY e.due_on ASC, e.title ASC"#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(horizon_days)
    .fetch_all(executor)
    .await?)
}

/// The vault glance on the hub: counts only. Values never leave the vault, and
/// this query never touches `secret_fields`.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretsGlance {
    pub total_n: i64,
    pub stale_n: i64,
    pub expiring_n: i64,
    pub revealed_week_n: i64,
}

pub async fn secrets_glance<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<SecretsGlance>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, SecretsGlance>(
        r#"SELECT
             (SELECT count(*) FROM secret_records r
               WHERE r.org_id = $1 AND r.client_id = $2) AS total_n,
             (SELECT count(*) FROM secret_records r
               WHERE r.org_id = $1 AND r.client_id = $2
                 AND coalesce(r.last_rotated_at, r.created_at) < now() - interval '180 days') AS stale_n,
             (SELECT count(*) FROM secret_records r
               WHERE r.org_id = $1 AND r.client_id = $2
                 AND r.expires_at IS NOT NULL
                 AND r.expires_at < now() + interval '30 days') AS expiring_n,
             (SELECT count(*) FROM audit_events a
               WHERE a.org_id = $1
                 AND a.action = 'secret.revealed'
                 AND a.entity_id IN (
                   SELECT r.id::text FROM secret_records r
                    WHERE r.org_id = $1 AND r.client_id = $2
                 )
                 AND a.occurred_at > now() - interval '7 days') AS revealed_week_n"#,
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_one(executor)
    .await?)
}

/* ------------------------------------------------------------------------- */
/* Racks                                                                     */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RackRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub site_id: Option<Uuid>,
    pub name: String,
    pub code: Option<String>,
    pub units: i16,
    pub width_mm: Option<i32>,
    pub depth_mm: Option<i32>,
    pub power_capacity_kw: Option<f64>,
    pub power_used_kw: Option<f64>,
    pub cooling: Option<String>,
    pub location_note: Option<String>,
    pub notes: Option<String>,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const RACK_COLS: &str = "id, org_id, client_id, site_id, name, code, units, width_mm, depth_mm, \
     power_capacity_kw::double precision AS power_capacity_kw, \
     power_used_kw::double precision AS power_used_kw, cooling, location_note, notes, fields, \
     created_at, updated_at";

pub async fn list_racks<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<RackRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, RackRecord>(&format!(
        "SELECT {RACK_COLS} FROM knowledge_racks
          WHERE org_id = $1 AND client_id = $2 ORDER BY name"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

pub async fn get_rack<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<RackRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, RackRecord>(&format!(
        "SELECT {RACK_COLS} FROM knowledge_racks WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

/// The rack row, held for the rest of the caller's transaction. A placement
/// write reads the rack and then asks whether the units it wants are free;
/// without the lock two requests can both see the same units free on
/// overlapping ranges and both commit.
pub async fn lock_rack<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<RackRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, RackRecord>(&format!(
        "SELECT {RACK_COLS} FROM knowledge_racks WHERE org_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RackDeviceRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub rack_id: Uuid,
    pub asset_id: Option<Uuid>,
    pub name: String,
    pub device_type: String,
    pub face: String,
    pub unit_start: i16,
    pub unit_height: i16,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub status: String,
    pub power_notes: Option<String>,
    pub network_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const RACK_DEVICE_COLS: &str = "id, org_id, rack_id, asset_id, name, device_type, face, \
     unit_start, unit_height, model, serial, status, power_notes, network_notes, \
     created_at, updated_at";

pub async fn list_rack_devices<'e, E>(
    executor: E,
    org_id: Uuid,
    rack_id: Uuid,
) -> DbResult<Vec<RackDeviceRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, RackDeviceRecord>(&format!(
        "SELECT {RACK_DEVICE_COLS} FROM knowledge_rack_devices
          WHERE org_id = $1 AND rack_id = $2 ORDER BY face, unit_start DESC"
    ))
    .bind(org_id)
    .bind(rack_id)
    .fetch_all(executor)
    .await?)
}

/// True when this placement really is on that rack. A path names both, and a
/// device from another rack must not be movable or deletable through it.
pub async fn rack_device_on<'e, E>(
    executor: E,
    org_id: Uuid,
    rack_id: Uuid,
    device_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let found: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1 FROM knowledge_rack_devices
              WHERE org_id = $1 AND rack_id = $2 AND id = $3
           )"#,
    )
    .bind(org_id)
    .bind(rack_id)
    .bind(device_id)
    .fetch_one(executor)
    .await?;
    Ok(found)
}

/// True when the proposed placement overlaps an existing one on the same face.
/// The unique index catches the same start unit; this catches every other
/// overlap a person can create by hand.
pub async fn rack_slot_taken<'e, E>(
    executor: E,
    org_id: Uuid,
    rack_id: Uuid,
    face: &str,
    unit_start: i16,
    unit_height: i16,
    exclude: Option<Uuid>,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let taken: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
             SELECT 1 FROM knowledge_rack_devices
              WHERE org_id = $1 AND rack_id = $2 AND face = $3
                AND ($6::uuid IS NULL OR id <> $6::uuid)
                AND unit_start < $4::int + $5::int
                AND unit_start + unit_height > $4::int
           )"#,
    )
    .bind(org_id)
    .bind(rack_id)
    .bind(face)
    .bind(unit_start)
    .bind(unit_height)
    .bind(exclude)
    .fetch_one(executor)
    .await?;
    Ok(taken)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_rack<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    name: &str,
    code: Option<&str>,
    units: i16,
    width_mm: Option<i32>,
    depth_mm: Option<i32>,
    power_capacity_kw: Option<f64>,
    power_used_kw: Option<f64>,
    cooling: Option<&str>,
    location_note: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<RackRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, RackRecord>(&format!(
        "INSERT INTO knowledge_racks
           (id, org_id, client_id, site_id, name, code, units, width_mm, depth_mm,
            power_capacity_kw, power_used_kw, cooling, location_note, notes, fields,
            created_by, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::numeric,$11::numeric,$12,$13,$14,$15,$16,$16)
         RETURNING {RACK_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(site_id)
    .bind(name)
    .bind(code)
    .bind(units)
    .bind(width_mm)
    .bind(depth_mm)
    .bind(power_capacity_kw)
    .bind(power_used_kw)
    .bind(cooling)
    .bind(location_note)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_rack<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    site_id: Option<Uuid>,
    name: &str,
    code: Option<&str>,
    units: i16,
    width_mm: Option<i32>,
    depth_mm: Option<i32>,
    power_capacity_kw: Option<f64>,
    power_used_kw: Option<f64>,
    cooling: Option<&str>,
    location_note: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<RackRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, RackRecord>(&format!(
        "UPDATE knowledge_racks SET site_id = $3, name = $4, code = $5, units = $6,
            width_mm = $7, depth_mm = $8, power_capacity_kw = $9::numeric,
            power_used_kw = $10::numeric, cooling = $11, location_note = $12,
            notes = $13, fields = $14, updated_by = $15, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {RACK_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(site_id)
    .bind(name)
    .bind(code)
    .bind(units)
    .bind(width_mm)
    .bind(depth_mm)
    .bind(power_capacity_kw)
    .bind(power_used_kw)
    .bind(cooling)
    .bind(location_note)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete_rack(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    // The placements leave with the rack (their foreign key cascades), so the
    // edges that name them are dropped while they can still be matched.
    sqlx::query(
        "DELETE FROM knowledge_links l
           USING knowledge_rack_devices d
          WHERE d.org_id = $1 AND d.rack_id = $2
            AND ((l.from_kind = 'rack_device' AND l.from_id = d.id)
              OR (l.to_kind = 'rack_device' AND l.to_id = d.id))",
    )
    .bind(org_id)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let r = sqlx::query("DELETE FROM knowledge_racks WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "rack", id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_rack_device<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    rack_id: Uuid,
    asset_id: Option<Uuid>,
    name: &str,
    device_type: &str,
    face: &str,
    unit_start: i16,
    unit_height: i16,
    model: Option<&str>,
    serial: Option<&str>,
    status: &str,
    power_notes: Option<&str>,
    network_notes: Option<&str>,
) -> DbResult<RackDeviceRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, RackDeviceRecord>(&format!(
        "INSERT INTO knowledge_rack_devices
           (id, org_id, rack_id, asset_id, name, device_type, face, unit_start,
            unit_height, model, serial, status, power_notes, network_notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
         RETURNING {RACK_DEVICE_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(rack_id)
    .bind(asset_id)
    .bind(name)
    .bind(device_type)
    .bind(face)
    .bind(unit_start)
    .bind(unit_height)
    .bind(model)
    .bind(serial)
    .bind(status)
    .bind(power_notes)
    .bind(network_notes)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_rack_device<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    asset_id: Option<Uuid>,
    name: &str,
    device_type: &str,
    face: &str,
    unit_start: i16,
    unit_height: i16,
    model: Option<&str>,
    serial: Option<&str>,
    status: &str,
    power_notes: Option<&str>,
    network_notes: Option<&str>,
) -> DbResult<RackDeviceRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, RackDeviceRecord>(&format!(
        "UPDATE knowledge_rack_devices SET asset_id = $3, name = $4, device_type = $5,
            face = $6, unit_start = $7, unit_height = $8, model = $9, serial = $10,
            status = $11, power_notes = $12, network_notes = $13, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {RACK_DEVICE_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(asset_id)
    .bind(name)
    .bind(device_type)
    .bind(face)
    .bind(unit_start)
    .bind(unit_height)
    .bind(model)
    .bind(serial)
    .bind(status)
    .bind(power_notes)
    .bind(network_notes)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete_rack_device(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM knowledge_rack_devices WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "rack_device", id).await
}

/* ------------------------------------------------------------------------- */
/* Networks                                                                  */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetworkRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub site_id: Option<Uuid>,
    pub name: String,
    pub kind: String,
    pub gateway: Option<String>,
    pub dns: Option<String>,
    pub vlan_zone: Option<String>,
    pub notes: Option<String>,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const NETWORK_COLS: &str = "id, org_id, client_id, site_id, name, kind, gateway, dns, \
     vlan_zone, notes, fields, created_at, updated_at";

pub async fn list_networks<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<NetworkRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, NetworkRecord>(&format!(
        "SELECT {NETWORK_COLS} FROM knowledge_networks
          WHERE org_id = $1 AND client_id = $2 ORDER BY name"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

pub async fn get_network<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<NetworkRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, NetworkRecord>(&format!(
        "SELECT {NETWORK_COLS} FROM knowledge_networks WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_network<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    name: &str,
    kind: &str,
    gateway: Option<&str>,
    dns: Option<&str>,
    vlan_zone: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<NetworkRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, NetworkRecord>(&format!(
        "INSERT INTO knowledge_networks
           (id, org_id, client_id, site_id, name, kind, gateway, dns, vlan_zone,
            notes, fields, created_by, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12)
         RETURNING {NETWORK_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(site_id)
    .bind(name)
    .bind(kind)
    .bind(gateway)
    .bind(dns)
    .bind(vlan_zone)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_network<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    site_id: Option<Uuid>,
    name: &str,
    kind: &str,
    gateway: Option<&str>,
    dns: Option<&str>,
    vlan_zone: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<NetworkRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, NetworkRecord>(&format!(
        "UPDATE knowledge_networks SET site_id = $3, name = $4, kind = $5, gateway = $6,
            dns = $7, vlan_zone = $8, notes = $9, fields = $10, updated_by = $11,
            updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {NETWORK_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(site_id)
    .bind(name)
    .bind(kind)
    .bind(gateway)
    .bind(dns)
    .bind(vlan_zone)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete_network(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    // Subnets leave with the network (their foreign key cascades), so the
    // edges that name them are dropped while they can still be matched.
    sqlx::query(
        "DELETE FROM knowledge_links l
           USING knowledge_subnets s
          WHERE s.org_id = $1 AND s.network_id = $2
            AND ((l.from_kind = 'subnet' AND l.from_id = s.id)
              OR (l.to_kind = 'subnet' AND l.to_id = s.id))",
    )
    .bind(org_id)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    let r = sqlx::query("DELETE FROM knowledge_networks WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "network", id).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubnetRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub network_id: Uuid,
    pub cidr: String,
    pub vlan_id: Option<i32>,
    pub name: Option<String>,
    pub gateway: Option<String>,
    pub used_for: Option<String>,
    pub dhcp: bool,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const SUBNET_COLS: &str = "id, org_id, client_id, network_id, cidr::text AS cidr, vlan_id, \
     name, gateway, used_for, dhcp, notes, created_at, updated_at";

pub async fn list_subnets<'e, E>(
    executor: E,
    org_id: Uuid,
    network_id: Uuid,
) -> DbResult<Vec<SubnetRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, SubnetRecord>(&format!(
        "SELECT {SUBNET_COLS} FROM knowledge_subnets
          WHERE org_id = $1 AND network_id = $2 ORDER BY knowledge_subnets.cidr"
    ))
    .bind(org_id)
    .bind(network_id)
    .fetch_all(executor)
    .await?)
}

pub async fn list_subnets_for_client<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<SubnetRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, SubnetRecord>(&format!(
        "SELECT {SUBNET_COLS} FROM knowledge_subnets
          WHERE org_id = $1 AND client_id = $2 ORDER BY knowledge_subnets.cidr"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_subnet<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    network_id: Uuid,
    cidr: &str,
    vlan_id: Option<i32>,
    name: Option<&str>,
    gateway: Option<&str>,
    used_for: Option<&str>,
    dhcp: bool,
    notes: Option<&str>,
) -> DbResult<SubnetRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, SubnetRecord>(&format!(
        "INSERT INTO knowledge_subnets
           (id, org_id, client_id, network_id, cidr, vlan_id, name, gateway, used_for, dhcp, notes)
         VALUES ($1,$2,$3,$4,$5::cidr,$6,$7,$8,$9,$10,$11)
         RETURNING {SUBNET_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(network_id)
    .bind(cidr)
    .bind(vlan_id)
    .bind(name)
    .bind(gateway)
    .bind(used_for)
    .bind(dhcp)
    .bind(notes)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_subnet<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    cidr: &str,
    vlan_id: Option<i32>,
    name: Option<&str>,
    gateway: Option<&str>,
    used_for: Option<&str>,
    dhcp: bool,
    notes: Option<&str>,
) -> DbResult<SubnetRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, SubnetRecord>(&format!(
        "UPDATE knowledge_subnets SET cidr = $3::cidr, vlan_id = $4, name = $5,
            gateway = $6, used_for = $7, dhcp = $8, notes = $9, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {SUBNET_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(cidr)
    .bind(vlan_id)
    .bind(name)
    .bind(gateway)
    .bind(used_for)
    .bind(dhcp)
    .bind(notes)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete_subnet(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM knowledge_subnets WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "subnet", id).await
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IpReservationRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub subnet_id: Uuid,
    pub address: String,
    pub hostname: Option<String>,
    pub asset_id: Option<Uuid>,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const IP_COLS: &str = "id, org_id, client_id, subnet_id, host(address) AS address, hostname, \
     asset_id, status, notes, created_at, updated_at";

pub async fn list_reservations<'e, E>(
    executor: E,
    org_id: Uuid,
    subnet_id: Uuid,
) -> DbResult<Vec<IpReservationRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, IpReservationRecord>(&format!(
        "SELECT {IP_COLS} FROM knowledge_ip_reservations
          WHERE org_id = $1 AND subnet_id = $2 ORDER BY knowledge_ip_reservations.address"
    ))
    .bind(org_id)
    .bind(subnet_id)
    .fetch_all(executor)
    .await?)
}

/// The address is stored as the host itself: `10.0.0.7` and `10.0.0.7/24` are
/// one reservation, not two that read back the same.
#[allow(clippy::too_many_arguments)]
pub async fn insert_reservation<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    subnet_id: Uuid,
    address: &str,
    hostname: Option<&str>,
    asset_id: Option<Uuid>,
    status: &str,
    notes: Option<&str>,
) -> DbResult<IpReservationRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, IpReservationRecord>(&format!(
        "INSERT INTO knowledge_ip_reservations
           (id, org_id, client_id, subnet_id, address, hostname, asset_id, status, notes)
         VALUES ($1,$2,$3,$4,host($5::inet)::inet,$6,$7,$8,$9)
         RETURNING {IP_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(subnet_id)
    .bind(address)
    .bind(hostname)
    .bind(asset_id)
    .bind(status)
    .bind(notes)
    .fetch_one(executor)
    .await?)
}

pub async fn delete_reservation<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM knowledge_ip_reservations WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/* ------------------------------------------------------------------------- */
/* Domains                                                                   */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DomainRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub domain: String,
    pub kind: String,
    pub registrar: Option<String>,
    pub registrar_expiry: Option<NaiveDate>,
    pub ssl_expiry: Option<NaiveDate>,
    pub ssl_issuer: Option<String>,
    pub dns_provider: Option<String>,
    pub hosting: Option<String>,
    pub manages_dns: bool,
    pub monitoring: bool,
    pub status_url: Option<String>,
    pub notes: Option<String>,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const DOMAIN_COLS: &str = "id, org_id, client_id, domain, kind, registrar, registrar_expiry, \
     ssl_expiry, ssl_issuer, dns_provider, hosting, manages_dns, monitoring, status_url, \
     notes, fields, created_at, updated_at";

pub async fn list_domains<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<DomainRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, DomainRecord>(&format!(
        "SELECT {DOMAIN_COLS} FROM knowledge_domains
          WHERE org_id = $1 AND client_id = $2 ORDER BY domain"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

pub async fn get_domain<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<DomainRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, DomainRecord>(&format!(
        "SELECT {DOMAIN_COLS} FROM knowledge_domains WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_domain<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    domain: &str,
    kind: &str,
    registrar: Option<&str>,
    registrar_expiry: Option<NaiveDate>,
    ssl_expiry: Option<NaiveDate>,
    ssl_issuer: Option<&str>,
    dns_provider: Option<&str>,
    hosting: Option<&str>,
    manages_dns: bool,
    monitoring: bool,
    status_url: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<DomainRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, DomainRecord>(&format!(
        "INSERT INTO knowledge_domains
           (id, org_id, client_id, domain, kind, registrar, registrar_expiry, ssl_expiry,
            ssl_issuer, dns_provider, hosting, manages_dns, monitoring, status_url, notes,
            fields, created_by, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17)
         RETURNING {DOMAIN_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(domain)
    .bind(kind)
    .bind(registrar)
    .bind(registrar_expiry)
    .bind(ssl_expiry)
    .bind(ssl_issuer)
    .bind(dns_provider)
    .bind(hosting)
    .bind(manages_dns)
    .bind(monitoring)
    .bind(status_url)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_domain<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    domain: &str,
    kind: &str,
    registrar: Option<&str>,
    registrar_expiry: Option<NaiveDate>,
    ssl_expiry: Option<NaiveDate>,
    ssl_issuer: Option<&str>,
    dns_provider: Option<&str>,
    hosting: Option<&str>,
    manages_dns: bool,
    monitoring: bool,
    status_url: Option<&str>,
    notes: Option<&str>,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<DomainRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, DomainRecord>(&format!(
        "UPDATE knowledge_domains SET domain = $3, kind = $4, registrar = $5,
            registrar_expiry = $6, ssl_expiry = $7, ssl_issuer = $8, dns_provider = $9,
            hosting = $10, manages_dns = $11, monitoring = $12, status_url = $13,
            notes = $14, fields = $15, updated_by = $16, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {DOMAIN_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(domain)
    .bind(kind)
    .bind(registrar)
    .bind(registrar_expiry)
    .bind(ssl_expiry)
    .bind(ssl_issuer)
    .bind(dns_provider)
    .bind(hosting)
    .bind(manages_dns)
    .bind(monitoring)
    .bind(status_url)
    .bind(notes)
    .bind(fields)
    .bind(user_id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn delete_domain(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM knowledge_domains WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "domain", id).await
}

/* ------------------------------------------------------------------------- */
/* Procedures                                                                */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProcedureRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub title: String,
    pub summary: Option<String>,
    pub body: String,
    pub owner_user_id: Option<Uuid>,
    pub owner_name: Option<String>,
    pub is_checklist: bool,
    pub client_visible: bool,
    pub version: i32,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const PROCEDURE_COLS: &str = "p.id, p.org_id, p.client_id, p.title, p.summary, p.body, \
     p.owner_user_id, u.display_name AS owner_name, p.is_checklist, p.client_visible, p.version, \
     p.fields, p.created_at, p.updated_at";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProcedureStepRecord {
    pub id: Uuid,
    pub procedure_id: Uuid,
    pub position: i32,
    pub title: String,
    pub body: Option<String>,
    pub required: bool,
}

pub async fn list_procedures<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<ProcedureRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, ProcedureRecord>(&format!(
        "SELECT {PROCEDURE_COLS} FROM knowledge_procedures p
           LEFT JOIN users u ON u.id = p.owner_user_id
          WHERE p.org_id = $1 AND p.client_id = $2
          ORDER BY p.title"
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

pub async fn get_procedure<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<ProcedureRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, ProcedureRecord>(&format!(
        "SELECT {PROCEDURE_COLS} FROM knowledge_procedures p
           LEFT JOIN users u ON u.id = p.owner_user_id
          WHERE p.org_id = $1 AND p.id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

pub async fn list_procedure_steps<'e, E>(
    executor: E,
    org_id: Uuid,
    procedure_id: Uuid,
) -> DbResult<Vec<ProcedureStepRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, ProcedureStepRecord>(
        "SELECT id, procedure_id, position, title, body, required
           FROM knowledge_procedure_steps
          WHERE org_id = $1 AND procedure_id = $2
          ORDER BY position",
    )
    .bind(org_id)
    .bind(procedure_id)
    .fetch_all(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_procedure(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    title: &str,
    summary: Option<&str>,
    body: &str,
    owner_user_id: Option<Uuid>,
    is_checklist: bool,
    client_visible: bool,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<ProcedureRecord> {
    sqlx::query(
        "INSERT INTO knowledge_procedures
           (id, org_id, client_id, title, summary, body, owner_user_id, is_checklist,
            client_visible, fields, created_by, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11)",
    )
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(title)
    .bind(summary)
    .bind(body)
    .bind(owner_user_id)
    .bind(is_checklist)
    .bind(client_visible)
    .bind(fields)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    get_procedure(&mut *conn, org_id, id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_procedure<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    title: &str,
    summary: Option<&str>,
    body: &str,
    owner_user_id: Option<Uuid>,
    is_checklist: bool,
    client_visible: bool,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<ProcedureRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query_as::<_, ProcedureRecord>(
        "UPDATE knowledge_procedures p SET title = $3, summary = $4, body = $5,
            owner_user_id = $6, is_checklist = $7, client_visible = $8, fields = $9,
            version = p.version + 1, updated_by = $10, updated_at = now()
          WHERE p.org_id = $1 AND p.id = $2
          RETURNING p.id, p.org_id, p.client_id, p.title, p.summary, p.body,
            p.owner_user_id, (SELECT display_name FROM users WHERE id = p.owner_user_id) AS owner_name,
            p.is_checklist, p.client_visible, p.version, p.fields, p.created_at, p.updated_at",
    )
    .bind(org_id)
    .bind(id)
    .bind(title)
    .bind(summary)
    .bind(body)
    .bind(owner_user_id)
    .bind(is_checklist)
    .bind(client_visible)
    .bind(fields)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    r.ok_or(DbError::NotFound)
}

pub async fn delete_procedure(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM knowledge_procedures WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    delete_links_for(conn, org_id, "procedure", id).await
}

/// Replaces the step list wholesale. A procedure's steps are one authored
/// sequence, so a partial merge would leave a position gap the next edit has
/// to guess at.
pub async fn replace_procedure_steps(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    procedure_id: Uuid,
    steps: &[(String, Option<String>, bool)],
) -> DbResult<Vec<ProcedureStepRecord>> {
    sqlx::query("DELETE FROM knowledge_procedure_steps WHERE org_id = $1 AND procedure_id = $2")
        .bind(org_id)
        .bind(procedure_id)
        .execute(&mut *conn)
        .await?;
    for (position, (title, body, required)) in steps.iter().enumerate() {
        sqlx::query(
            "INSERT INTO knowledge_procedure_steps
               (id, org_id, procedure_id, position, title, body, required)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6)",
        )
        .bind(org_id)
        .bind(procedure_id)
        .bind(position as i32)
        .bind(title)
        .bind(body)
        .bind(required)
        .execute(&mut *conn)
        .await?;
    }
    list_procedure_steps(&mut *conn, org_id, procedure_id).await
}

/* ------------------------------------------------------------------------- */
/* The generic knowledge edge                                                */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LinkRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Option<Uuid>,
    pub from_kind: String,
    pub from_id: Uuid,
    pub to_kind: String,
    pub to_id: Uuid,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

const LINK_COLS: &str =
    "id, org_id, client_id, from_kind, from_id, to_kind, to_id, label, created_at";

/// Drops the edges one record owned. Neither endpoint can be a foreign key —
/// a link names a record by kind and id — so deleting the record has to delete
/// them; a leftover edge reads as a relationship to something that is gone.
async fn delete_links_for(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    kind: &str,
    id: Uuid,
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM knowledge_links
          WHERE org_id = $1
            AND ((from_kind = $2 AND from_id = $3) OR (to_kind = $2 AND to_id = $3))",
    )
    .bind(org_id)
    .bind(kind)
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Every edge touching this record, in either direction, as one list. A
/// relationship is not directional to a reader, so the surface prints both
/// sides with the direction named in the label rather than in a column.
pub async fn links_for<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: &str,
    id: Uuid,
) -> DbResult<Vec<LinkRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, LinkRecord>(&format!(
        "SELECT {LINK_COLS} FROM knowledge_links
          WHERE org_id = $1
            AND ((from_kind = $2 AND from_id = $3) OR (to_kind = $2 AND to_id = $3))
          ORDER BY created_at DESC"
    ))
    .bind(org_id)
    .bind(kind)
    .bind(id)
    .fetch_all(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_link<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    from_kind: &str,
    from_id: Uuid,
    to_kind: &str,
    to_id: Uuid,
    label: Option<&str>,
    user_id: Uuid,
) -> DbResult<LinkRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, LinkRecord>(&format!(
        "INSERT INTO knowledge_links
           (id, org_id, client_id, from_kind, from_id, to_kind, to_id, label, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (org_id, from_kind, from_id, to_kind, to_id) DO UPDATE
           SET label = EXCLUDED.label
         RETURNING {LINK_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(from_kind)
    .bind(from_id)
    .bind(to_kind)
    .bind(to_id)
    .bind(label)
    .bind(user_id)
    .fetch_one(executor)
    .await?)
}

pub async fn delete_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM knowledge_links WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/* ------------------------------------------------------------------------- */
/* Hub settings                                                              */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientSettingsRecord {
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub owner_name: Option<String>,
    pub pinned_document_ids: Vec<Uuid>,
    pub review_note: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_client_settings<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Option<ClientSettingsRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, ClientSettingsRecord>(
        "SELECT s.org_id, s.client_id, s.owner_user_id, u.display_name AS owner_name,
                s.pinned_document_ids, s.review_note, s.updated_at
           FROM knowledge_client_settings s
           LEFT JOIN users u ON u.id = s.owner_user_id
          WHERE s.org_id = $1 AND s.client_id = $2",
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_optional(executor)
    .await?)
}

pub async fn upsert_client_settings(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    owner_user_id: Option<Uuid>,
    pinned_document_ids: &[Uuid],
    review_note: Option<&str>,
    user_id: Uuid,
) -> DbResult<ClientSettingsRecord> {
    sqlx::query(
        "INSERT INTO knowledge_client_settings
           (org_id, client_id, owner_user_id, pinned_document_ids, review_note, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (org_id, client_id) DO UPDATE
           SET owner_user_id = EXCLUDED.owner_user_id,
               pinned_document_ids = EXCLUDED.pinned_document_ids,
               review_note = EXCLUDED.review_note,
               updated_by = EXCLUDED.updated_by,
               updated_at = now()",
    )
    .bind(org_id)
    .bind(client_id)
    .bind(owner_user_id)
    .bind(pinned_document_ids)
    .bind(review_note)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(get_client_settings(&mut *conn, org_id, client_id)
        .await?
        .unwrap_or(ClientSettingsRecord {
            org_id,
            client_id,
            owner_user_id,
            owner_name: None,
            pinned_document_ids: pinned_document_ids.to_vec(),
            review_note: review_note.map(str::to_string),
            updated_at: Utc::now(),
        }))
}

/* ------------------------------------------------------------------------- */
/* Templates                                                                 */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TemplateRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Option<Uuid>,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub body: String,
    pub fields: serde_json::Value,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const TEMPLATE_COLS: &str = "id, org_id, client_id, kind, name, description, body, fields, \
     version, created_at, updated_at";

pub async fn list_templates<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: Option<&str>,
) -> DbResult<Vec<TemplateRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, TemplateRecord>(&format!(
        "SELECT {TEMPLATE_COLS} FROM knowledge_templates
          WHERE org_id = $1 AND ($2::text IS NULL OR kind = $2)
          ORDER BY name"
    ))
    .bind(org_id)
    .bind(kind)
    .fetch_all(executor)
    .await?)
}

pub async fn get_template<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<TemplateRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, TemplateRecord>(&format!(
        "SELECT {TEMPLATE_COLS} FROM knowledge_templates WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_template<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    kind: &str,
    name: &str,
    description: Option<&str>,
    body: &str,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<TemplateRecord>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, TemplateRecord>(&format!(
        "INSERT INTO knowledge_templates
           (id, org_id, client_id, kind, name, description, body, fields, created_by, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)
         RETURNING {TEMPLATE_COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(kind)
    .bind(name)
    .bind(description)
    .bind(body)
    .bind(fields)
    .bind(user_id)
    .fetch_one(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_template(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: Option<&str>,
    body: &str,
    fields: &serde_json::Value,
    user_id: Uuid,
) -> DbResult<TemplateRecord> {
    // The row is locked for the rest of the caller's transaction: two editors
    // that both read the version they are replacing would snapshot the same
    // number, and the later update would overwrite the earlier one's content
    // without ever recording it.
    let current = sqlx::query_as::<_, TemplateRecord>(&format!(
        "SELECT {TEMPLATE_COLS} FROM knowledge_templates
          WHERE org_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    // Snapshot the version being replaced before it is overwritten, so a
    // document written from an older template can still be read against the
    // words it actually used.
    sqlx::query(
        "INSERT INTO knowledge_template_versions
           (id, org_id, template_id, version, name, body, fields, author_id)
         VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (template_id, version) DO NOTHING",
    )
    .bind(org_id)
    .bind(id)
    .bind(current.version)
    .bind(&current.name)
    .bind(&current.body)
    .bind(&current.fields)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query_as::<_, TemplateRecord>(&format!(
        "UPDATE knowledge_templates SET name = $3, description = $4, body = $5,
            fields = $6, version = version + 1, updated_by = $7, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING {TEMPLATE_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(body)
    .bind(fields)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)
}

/* ------------------------------------------------------------------------- */
/* Two-step seeds                                                            */
/* ------------------------------------------------------------------------- */

#[derive(Debug, Clone, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretTotpRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub record_id: Uuid,
    pub seed_enc: Vec<u8>,
    pub digits: i16,
    pub period_seconds: i16,
    pub algorithm: String,
    pub issuer: Option<String>,
    pub account: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn get_secret_totp<'e, E>(
    executor: E,
    org_id: Uuid,
    record_id: Uuid,
) -> DbResult<Option<SecretTotpRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, SecretTotpRecord>(
        "SELECT id, org_id, record_id, seed_enc, digits, period_seconds, algorithm,
                issuer, account, created_at
           FROM knowledge_secret_totp WHERE org_id = $1 AND record_id = $2",
    )
    .bind(org_id)
    .bind(record_id)
    .fetch_optional(executor)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_secret_totp<'e, E>(
    executor: E,
    org_id: Uuid,
    record_id: Uuid,
    seed_enc: &[u8],
    digits: i16,
    period_seconds: i16,
    algorithm: &str,
    issuer: Option<&str>,
    account: Option<&str>,
    user_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO knowledge_secret_totp
           (id, org_id, record_id, seed_enc, digits, period_seconds, algorithm, issuer,
            account, created_by)
         VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (record_id) DO UPDATE
           SET seed_enc = EXCLUDED.seed_enc, digits = EXCLUDED.digits,
               period_seconds = EXCLUDED.period_seconds, algorithm = EXCLUDED.algorithm,
               issuer = EXCLUDED.issuer, account = EXCLUDED.account, updated_at = now()",
    )
    .bind(org_id)
    .bind(record_id)
    .bind(seed_enc)
    .bind(digits)
    .bind(period_seconds)
    .bind(algorithm)
    .bind(issuer)
    .bind(account)
    .bind(user_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn delete_secret_totp<'e, E>(executor: E, org_id: Uuid, record_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM knowledge_secret_totp WHERE org_id = $1 AND record_id = $2")
        .bind(org_id)
        .bind(record_id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/* ------------------------------------------------------------------------- */
/* Document visibility (portal)                                              */
/* ------------------------------------------------------------------------- */

pub async fn document_visibility<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
) -> DbResult<(Uuid, bool)>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT client_id, client_visible FROM documents
          WHERE org_id = $1 AND id = $2 AND client_id IS NOT NULL",
    )
    .bind(org_id)
    .bind(document_id)
    .fetch_optional(executor)
    .await?;
    row.ok_or(DbError::NotFound)
}

pub async fn set_document_visibility<'e, E>(
    executor: E,
    org_id: Uuid,
    document_id: Uuid,
    visible: bool,
    user_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(
        "UPDATE documents SET client_visible = $3, updated_by = $4, updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(document_id)
    .bind(visible)
    .bind(user_id)
    .execute(executor)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The documents and articles a client may see in its portal, newest first./// Two sources, one shape: the portal does not care which store wrote it, and
/// a client reading "Network policy" should not have to know either.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PortalDocRow {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub href: String,
    pub updated_at: DateTime<Utc>,
}

pub async fn portal_documents<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<PortalDocRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, PortalDocRow>(
        r#"SELECT * FROM (
             SELECT d.id, 'document'::text AS kind, d.title,
                    coalesce(substring(v.body from 1 for 180), '') AS summary,
                    '/portal/knowledge/documents/' || d.id::text AS href,
                    d.updated_at
               FROM documents d
               LEFT JOIN document_versions v
                 ON v.document_id = d.id AND v.version = d.current_version
              WHERE d.org_id = $1 AND d.client_id = $2 AND d.client_visible
             UNION ALL
             SELECT a.id, 'article', a.title,
                    coalesce(substring(a.body from 1 for 180), ''),
                    '/portal/kb/articles/' || a.id::text,
                    a.updated_at
               FROM kb_articles a
              WHERE a.org_id = $1 AND a.client_visible
                AND a.status = 'published'
           ) d
          ORDER BY updated_at DESC"#,
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?)
}

/* ------------------------------------------------------------------------- */
/* Unified search                                                            */
/* ------------------------------------------------------------------------- */

/// One search hit across the documentation stores. `subtitle` carries the
/// context a technician needs to tell two same-named records apart, and for a
/// secret it deliberately says only what the record is — never a value, never
/// a username from a sealed field.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KnowledgeSearchHit {
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    pub subtitle: String,
    pub client_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub href: String,
}

/// Every documentation store in one ranked list. Ranking is a kind weight
/// first (a client and a secret outrank a document) and recency second, which
/// is the order an MSP technician searches in: "the client, the password for
/// it, then the runbook".
///
/// `client_id` narrows the answer to one client. The filter rides EVERY store
/// leg, before the ranking and the limit, rather than trimming the finished
/// list: a post-limit filter spends the `limit` on rows the caller cannot see
/// and answers a scoped search with fewer hits than it asked for. Stores that
/// belong to no client (the org knowledge base, org documents, the org vault)
/// drop out when a client is named — "scoped to Acme" is Acme's records, not
/// everything Acme might read.
pub async fn search<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
    pattern: &str,
    limit: i64,
) -> DbResult<Vec<KnowledgeSearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, KnowledgeSearchHit>(
        r#"SELECT kind, id, title, subtitle, client_id, client_name, href FROM (
             SELECT 'client'::text AS kind, c.id, c.name AS title,
                    'Client' AS subtitle,
                    c.id AS client_id, c.name AS client_name,
                    '/kb/clients/' || c.id::text AS href, 0 AS rank,
                    c.updated_at AS touched
               FROM clients c
              WHERE c.org_id = $1 AND c.name ILIKE $3
                AND ($2::uuid IS NULL OR c.id = $2)
             UNION ALL
             SELECT 'secret', r.id, r.title, 'Credential · value hidden',
                    r.client_id, c.name,
                    '/kb/secrets?record=' || r.id::text, 1, r.updated_at
               FROM secret_records r
               LEFT JOIN clients c ON c.id = r.client_id
              WHERE r.org_id = $1 AND r.title ILIKE $3
                AND ($2::uuid IS NULL OR r.client_id = $2)
             UNION ALL
             SELECT 'asset', a.id, a.name, coalesce(nullif(a.kind, ''), 'Configuration'),
                    a.client_id, c.name, '/assets/' || a.id::text, 2, a.updated_at
               FROM device_assets a
               LEFT JOIN clients c ON c.id = a.client_id
              WHERE a.org_id = $1 AND a.name ILIKE $3
                AND ($2::uuid IS NULL OR a.client_id = $2)
             UNION ALL
             SELECT 'rack', r.id, r.name, 'Rack',
                    r.client_id, c.name, '/kb/racks/' || r.id::text, 2, r.updated_at
               FROM knowledge_racks r
               JOIN clients c ON c.id = r.client_id
              WHERE r.org_id = $1 AND r.name ILIKE $3
                AND ($2::uuid IS NULL OR r.client_id = $2)
             UNION ALL
             SELECT 'network', n.id, n.name, 'Network',
                    n.client_id, c.name, '/kb/networks/' || n.id::text, 2, n.updated_at
               FROM knowledge_networks n
               JOIN clients c ON c.id = n.client_id
              WHERE n.org_id = $1 AND n.name ILIKE $3
                AND ($2::uuid IS NULL OR n.client_id = $2)
             UNION ALL
             SELECT 'domain', d.id, d.domain, 'Domain or certificate',
                    d.client_id, c.name, '/kb/domains?domain=' || d.id::text, 2, d.updated_at
               FROM knowledge_domains d
               JOIN clients c ON c.id = d.client_id
              WHERE d.org_id = $1 AND d.domain ILIKE $3
                AND ($2::uuid IS NULL OR d.client_id = $2)
             UNION ALL
             SELECT 'procedure', p.id, p.title, 'Procedure',
                    p.client_id, c.name, '/kb/procedures/' || p.id::text, 3, p.updated_at
               FROM knowledge_procedures p
               JOIN clients c ON c.id = p.client_id
              WHERE p.org_id = $1 AND p.title ILIKE $3
                AND ($2::uuid IS NULL OR p.client_id = $2)
             UNION ALL
             -- The knowledge base is org-wide (kb_articles has no client), so
             -- a client scope excludes it rather than pretending it belongs.
             SELECT 'article', a.id, a.title, 'Knowledge article',
                    NULL::uuid, NULL::text, '/kb/articles?article=' || a.id::text, 3, a.updated_at
               FROM kb_articles a
              WHERE a.org_id = $1 AND a.title ILIKE $3
                AND $2::uuid IS NULL
             UNION ALL
             SELECT 'document', d.id, d.title, 'Document',
                    d.client_id, c.name, '/documents/' || d.id::text, 3, d.updated_at
               FROM documents d
               LEFT JOIN clients c ON c.id = d.client_id
              WHERE d.org_id = $1 AND d.title ILIKE $3
                AND ($2::uuid IS NULL OR d.client_id = $2)
           ) hit
          ORDER BY rank ASC, touched DESC
          LIMIT $4"#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(pattern)
    .bind(limit)
    .fetch_all(executor)
    .await?)
}

/* ------------------------------------------------------------------------- */
/* Small lifts the home needs                                                */
/* ------------------------------------------------------------------------- */

pub async fn list_client_names<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<(Uuid, String)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(
        sqlx::query_as("SELECT id, name FROM clients WHERE org_id = $1 ORDER BY name")
            .bind(org_id)
            .fetch_all(executor)
            .await?,
    )
}

/// Org-level documentation, the half of the home that does not belong to a
/// client: the shared knowledge base, org documents, templates and the org
/// vault.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OrgKnowledgeCounts {
    pub published_articles_n: i64,
    pub draft_articles_n: i64,
    pub org_documents_n: i64,
    pub templates_n: i64,
    pub org_secrets_n: i64,
}

pub async fn org_knowledge_counts<'e, E>(executor: E, org_id: Uuid) -> DbResult<OrgKnowledgeCounts>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, OrgKnowledgeCounts>(
        r#"SELECT
             (SELECT count(*) FROM kb_articles WHERE org_id = $1 AND status = 'published')
               AS published_articles_n,
             (SELECT count(*) FROM kb_articles WHERE org_id = $1 AND status = 'draft')
               AS draft_articles_n,
             (SELECT count(*) FROM documents WHERE org_id = $1 AND client_id IS NULL)
               AS org_documents_n,
             (SELECT count(*) FROM knowledge_templates WHERE org_id = $1) AS templates_n,
             (SELECT count(*) FROM secret_records WHERE org_id = $1 AND client_id IS NULL)
               AS org_secrets_n"#,
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?)
}

/// Every domain in the org, newest expiry first. The domains page is org-wide
/// because the question it answers ("what is about to expire?") is.
pub async fn list_domains_all<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<DomainRecord>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, DomainRecord>(&format!(
        "SELECT {DOMAIN_COLS} FROM knowledge_domains
          WHERE org_id = $1
          ORDER BY coalesce(ssl_expiry, registrar_expiry) NULLS LAST, domain"
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(document_articles: (i64, i64)) -> ClientFacts {
        ClientFacts {
            client_id: Uuid::nil(),
            contacts_n: 0,
            sites_n: 0,
            assets_n: 0,
            documents_n: document_articles.0,
            articles_n: document_articles.1,
            secrets_n: 0,
            racks_n: 0,
            networks_n: 0,
            domains_n: 0,
            procedures_n: 0,
            stale_secrets_n: 0,
            expiring_n: 0,
            latest_document_at: None,
            latest_article_at: None,
            latest_rack_at: None,
            latest_procedure_at: None,
        }
    }

    #[test]
    fn empty_client_names_every_gap() {
        let c = facts((0, 0)).completeness();
        assert_eq!(c.complete, 0);
        assert_eq!(c.total, 8);
        assert_eq!(c.gaps.len(), 8);
        assert!(c.gaps.iter().any(|g| g.contains("contact")));
        assert!(c.gaps.iter().any(|g| g.contains("procedure")));
    }

    #[test]
    fn a_document_leg_is_satisfied_by_an_article() {
        let c = facts((0, 3)).completeness();
        assert_eq!(c.complete, 1);
        assert!(!c.gaps.iter().any(|g| g.contains("document")));
    }

    #[test]
    fn every_leg_done_leaves_no_gap() {
        let mut f = facts((1, 1));
        f.contacts_n = 1;
        f.sites_n = 1;
        f.assets_n = 1;
        f.secrets_n = 1;
        f.networks_n = 1;
        f.domains_n = 1;
        f.procedures_n = 1;
        let c = f.completeness();
        assert_eq!(c.complete, 8);
        assert!(c.gaps.is_empty());
    }

    #[test]
    fn last_activity_is_the_newest_write_not_the_first_field() {
        let mut f = facts((0, 0));
        let older = "2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let newer = "2026-06-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        f.latest_document_at = Some(older);
        f.latest_procedure_at = Some(newer);
        assert_eq!(f.last_activity(), Some(newer));
        assert_eq!(facts((0, 0)).last_activity(), None);
    }
}
