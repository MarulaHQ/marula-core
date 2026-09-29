//! Cross-entity dedupe (ADR 0003 extension, Wave 22 lane 2): the
//! source-link + identity-key + merge-candidate pattern applied to
//! clients/contacts. ConfigItems stay on `config_identity` — this module
//! owns the CONTACT line (feeds upsert, deterministic email/phone
//! matching, curated-vs-observed tiers, audited + undoable merges) and
//! the CLIENT org_map reconciliation line (deterministic label proposals
//! become reviewable candidates; an accept writes the durable
//! `client_org_map` and never auto-maps).
//!
//! Queue mechanics mirror `config_identity` exactly: same candidate
//! statuses (pending/deferred/accepted/rejected/auto_merged), same
//! auto-merge threshold (`ci_settings.auto_merge_threshold` — the org's
//! ONE identity-confidence dial), same visible-evidence shape.

use crate::{DbError, DbResult};
use chrono::Utc;
use core_domain::entities::entity_dedupe::{
    match_contact_identity, propose_client_matches, ClientOrgMap, ContactIdentityKeys,
    DedupeEntityKind, EntityMerge, EntityMergeCandidate, EntityMergeStatus, EntitySourceLink,
};
use core_domain::entities::Contact;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

// ---- row mappers ------------------------------------------------------------

const LINK_COLUMNS: &str = "id, org_id, entity_kind, entity_id, source_system, external_id, \
     source_guid, payload_ref, last_seen_at, created_at, updated_at";

fn row_to_link(r: sqlx::postgres::PgRow) -> EntitySourceLink {
    EntitySourceLink {
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        entity_kind: r
            .try_get::<String, _>("entity_kind")
            .ok()
            .and_then(|k| DedupeEntityKind::parse(&k))
            .unwrap_or(DedupeEntityKind::Contact),
        entity_id: r.try_get("entity_id").unwrap_or_default(),
        source_system: r.try_get("source_system").unwrap_or_default(),
        external_id: r.try_get("external_id").unwrap_or_default(),
        source_guid: r.try_get("source_guid").ok(),
        payload_ref: r.try_get("payload_ref").ok(),
        last_seen_at: r.try_get("last_seen_at").unwrap_or_default(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }
}

const CANDIDATE_COLUMNS: &str = "id, org_id, entity_kind, primary_entity_id, candidate_entity_id, \
     source_system, vendor_org_id, vendor_label, confidence::float8 AS confidence, reason, \
     evidence, status, decided_by, decided_at, created_at, updated_at";

fn row_to_candidate(r: sqlx::postgres::PgRow) -> EntityMergeCandidate {
    EntityMergeCandidate {
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        entity_kind: r
            .try_get::<String, _>("entity_kind")
            .ok()
            .and_then(|k| DedupeEntityKind::parse(&k))
            .unwrap_or(DedupeEntityKind::Contact),
        primary_entity_id: r.try_get("primary_entity_id").unwrap_or_default(),
        candidate_entity_id: r.try_get("candidate_entity_id").ok(),
        source_system: r.try_get("source_system").ok(),
        vendor_org_id: r.try_get("vendor_org_id").ok(),
        vendor_label: r.try_get("vendor_label").ok(),
        confidence: r.get::<Option<f64>, _>("confidence").unwrap_or(0.0),
        reason: r.try_get("reason").unwrap_or_default(),
        evidence: r.try_get("evidence").unwrap_or_default(),
        status: r
            .try_get::<String, _>("status")
            .ok()
            .and_then(|s| EntityMergeStatus::parse(&s))
            .unwrap_or(EntityMergeStatus::Pending),
        decided_by: r.try_get("decided_by").ok(),
        decided_at: r.try_get("decided_at").ok(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }
}

const MERGE_COLUMNS: &str = "id, org_id, entity_kind, primary_entity_id, merged_entity_id, \
     candidate_id, confidence::float8 AS confidence, reason, evidence, moved, status, \
     merged_by, merged_at, undone_by, undone_at";

fn row_to_merge(r: sqlx::postgres::PgRow) -> EntityMerge {
    EntityMerge {
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        entity_kind: r
            .try_get::<String, _>("entity_kind")
            .ok()
            .and_then(|k| DedupeEntityKind::parse(&k))
            .unwrap_or(DedupeEntityKind::Contact),
        primary_entity_id: r.try_get("primary_entity_id").unwrap_or_default(),
        merged_entity_id: r.try_get("merged_entity_id").unwrap_or_default(),
        candidate_id: r.try_get("candidate_id").ok(),
        confidence: r.try_get::<Option<f64>, _>("confidence").unwrap_or(None),
        reason: r.try_get("reason").unwrap_or_default(),
        evidence: r.try_get("evidence").unwrap_or_default(),
        moved: r.try_get("moved").unwrap_or_default(),
        status: r.try_get("status").unwrap_or_default(),
        merged_by: r.try_get("merged_by").ok(),
        merged_at: r.try_get("merged_at").unwrap_or_default(),
        undone_by: r.try_get("undone_by").ok(),
        undone_at: r.try_get("undone_at").ok(),
    }
}

const MAP_COLUMNS: &str =
    "id, org_id, source_system, vendor_org_id, client_id, vendor_label, created_by, created_at, updated_at";

fn row_to_map(r: sqlx::postgres::PgRow) -> ClientOrgMap {
    ClientOrgMap {
        id: r.try_get("id").unwrap_or_default(),
        org_id: r.try_get("org_id").unwrap_or_default(),
        source_system: r.try_get("source_system").unwrap_or_default(),
        vendor_org_id: r.try_get("vendor_org_id").unwrap_or_default(),
        client_id: r.try_get("client_id").unwrap_or_default(),
        vendor_label: r.try_get("vendor_label").ok(),
        created_by: r.try_get("created_by").ok(),
        created_at: r.try_get("created_at").unwrap_or_default(),
        updated_at: r.try_get("updated_at").unwrap_or_default(),
    }
}

// ---- source links -----------------------------------------------------------

pub async fn get_entity_link_by_external(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    entity_kind: &str,
    source_system: &str,
    external_id: &str,
) -> DbResult<Option<EntitySourceLink>> {
    let row = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM entity_source_links
         WHERE org_id = $1 AND entity_kind = $2 AND source_system = $3 AND external_id = $4"
    ))
    .bind(org_id)
    .bind(entity_kind)
    .bind(source_system)
    .bind(external_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(row_to_link))
}

pub async fn list_entity_links(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> DbResult<Vec<EntitySourceLink>> {
    let rows = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM entity_source_links
         WHERE org_id = $1 AND entity_kind = $2 AND entity_id = $3
         ORDER BY last_seen_at DESC"
    ))
    .bind(org_id)
    .bind(entity_kind)
    .bind(entity_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_link).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_entity_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    source_system: &str,
    external_id: &str,
    source_guid: Option<&str>,
    payload_ref: Option<&str>,
) -> DbResult<EntitySourceLink> {
    let row = sqlx::query(&format!(
        "INSERT INTO entity_source_links
           (id, org_id, entity_kind, entity_id, source_system, external_id, source_guid, payload_ref)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (org_id, entity_kind, source_system, external_id) DO UPDATE
           SET entity_id = EXCLUDED.entity_id, source_guid = EXCLUDED.source_guid,
               payload_ref = EXCLUDED.payload_ref, last_seen_at = now(), updated_at = now()
         RETURNING {LINK_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(entity_kind)
    .bind(entity_id)
    .bind(source_system)
    .bind(external_id)
    .bind(source_guid)
    .bind(payload_ref)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_link(row))
}

/// Moves every link of one entity onto another (the merge substrate).
pub async fn move_entity_links(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    from_entity: Uuid,
    to_entity: Uuid,
) -> DbResult<Vec<Uuid>> {
    let moved = sqlx::query_scalar(
        "UPDATE entity_source_links SET entity_id = $3, updated_at = now()
         WHERE org_id = $1 AND entity_id = $2 RETURNING id",
    )
    .bind(org_id)
    .bind(from_entity)
    .bind(to_entity)
    .fetch_all(&mut *conn)
    .await?;
    Ok(moved)
}

// ---- candidates -------------------------------------------------------------

pub async fn list_entity_candidates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    entity_kind: Option<&str>,
    status: Option<&str>,
) -> DbResult<Vec<EntityMergeCandidate>> {
    let rows = sqlx::query(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM entity_merge_candidates
         WHERE org_id = $1
           AND ($2::text IS NULL OR entity_kind = $2)
           AND ($3::text IS NULL OR status = $3)
         ORDER BY confidence DESC, created_at DESC"
    ))
    .bind(org_id)
    .bind(entity_kind)
    .bind(status)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_candidate).collect())
}

pub async fn get_entity_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<EntityMergeCandidate> {
    sqlx::query(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM entity_merge_candidates WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::NotFound)
}

/// Upserts one candidate against the partial open-pair index: a pending
/// pair refreshes evidence in place; a decided pair never flips back
/// silently (a feed contradiction does NOT un-reject a human decision);
/// a deferred pair stays parked.
#[allow(clippy::too_many_arguments)]
pub async fn create_entity_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    entity_kind: &str,
    primary_entity_id: Uuid,
    candidate_entity_id: Option<Uuid>,
    source_system: Option<&str>,
    vendor_org_id: Option<&str>,
    vendor_label: Option<&str>,
    confidence: f64,
    reason: &str,
    evidence: &Value,
    status: &str,
) -> DbResult<EntityMergeCandidate> {
    let row = sqlx::query(&format!(
        "INSERT INTO entity_merge_candidates
           (id, org_id, entity_kind, primary_entity_id, candidate_entity_id,
            source_system, vendor_org_id, vendor_label, confidence, reason, evidence, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (org_id, entity_kind, primary_entity_id, candidate_entity_id,
                      source_system, vendor_org_id) DO UPDATE
           SET confidence = EXCLUDED.confidence,
                reason = EXCLUDED.reason,
                evidence = EXCLUDED.evidence,
                status = CASE WHEN entity_merge_candidates.status = 'pending'
                              THEN EXCLUDED.status
                              ELSE entity_merge_candidates.status END,
                updated_at = now()
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(entity_kind)
    .bind(primary_entity_id)
    .bind(candidate_entity_id)
    .bind(source_system)
    .bind(vendor_org_id)
    .bind(vendor_label)
    .bind(confidence)
    .bind(reason)
    .bind(evidence)
    .bind(status)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_candidate(row))
}

/// Marks a candidate decided; only pending/auto_merged rows can flip.
pub async fn decide_entity_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    status: &str,
    decided_by: Uuid,
) -> DbResult<EntityMergeCandidate> {
    let row = sqlx::query(&format!(
        "UPDATE entity_merge_candidates
         SET status = $3, decided_by = $4, decided_at = now(), updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status IN ('pending', 'auto_merged')
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(status)
    .bind(decided_by)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::Conflict("merge candidate already decided".into()))?;
    Ok(row)
}

/// Re-opens a DEFERRED candidate (the un-defer affordance). Decided rows
/// stay decided — undo for accepted candidates rides the merge's split.
pub async fn restore_entity_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<EntityMergeCandidate> {
    let row = sqlx::query(&format!(
        "UPDATE entity_merge_candidates
         SET status = 'pending', decided_by = NULL, decided_at = NULL, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'deferred'
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::Conflict("candidate is not deferred".into()))?;
    Ok(row)
}

// ---- contact merges (audit + undo substrate) ---------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn record_entity_merge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    entity_kind: &str,
    primary_entity_id: Uuid,
    merged_entity_id: Uuid,
    candidate_id: Option<Uuid>,
    confidence: Option<f64>,
    reason: &str,
    evidence: &Value,
    moved: &Value,
    merged_by: Option<Uuid>,
) -> DbResult<EntityMerge> {
    let row = sqlx::query(&format!(
        "INSERT INTO entity_merges
           (id, org_id, entity_kind, primary_entity_id, merged_entity_id, candidate_id,
            confidence, reason, evidence, moved, status, merged_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'applied', $11)
         RETURNING {MERGE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(entity_kind)
    .bind(primary_entity_id)
    .bind(merged_entity_id)
    .bind(candidate_id)
    .bind(confidence)
    .bind(reason)
    .bind(evidence)
    .bind(moved)
    .bind(merged_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_merge(row))
}

pub async fn get_entity_merge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<EntityMerge> {
    sqlx::query(&format!(
        "SELECT {MERGE_COLUMNS} FROM entity_merges WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_merge)
    .ok_or(DbError::NotFound)
}

pub async fn list_merges_for_contact(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    contact_id: Uuid,
) -> DbResult<Vec<EntityMerge>> {
    let rows = sqlx::query(&format!(
        "SELECT {MERGE_COLUMNS} FROM entity_merges
         WHERE org_id = $1 AND (primary_entity_id = $2 OR merged_entity_id = $2)
         ORDER BY merged_at DESC"
    ))
    .bind(org_id)
    .bind(contact_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_merge).collect())
}

// ---- org map ----------------------------------------------------------------

pub async fn get_org_map(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    source_system: &str,
    vendor_org_id: &str,
) -> DbResult<Option<ClientOrgMap>> {
    let row = sqlx::query(&format!(
        "SELECT {MAP_COLUMNS} FROM client_org_map
         WHERE org_id = $1 AND source_system = $2 AND vendor_org_id = $3"
    ))
    .bind(org_id)
    .bind(source_system)
    .bind(vendor_org_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(row_to_map))
}

/// Upsert the operator-curated vendor-org → client mapping (the client
/// reconciliation decision). Idempotent per (org, source, vendor org).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_org_map(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    source_system: &str,
    vendor_org_id: &str,
    client_id: Uuid,
    vendor_label: Option<&str>,
    created_by: Uuid,
) -> DbResult<ClientOrgMap> {
    let row = sqlx::query(&format!(
        "INSERT INTO client_org_map
           (id, org_id, source_system, vendor_org_id, client_id, vendor_label, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (org_id, source_system, vendor_org_id) DO UPDATE
           SET client_id = EXCLUDED.client_id,
                vendor_label = EXCLUDED.vendor_label,
                created_by = EXCLUDED.created_by,
                updated_at = now()
         RETURNING {MAP_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(source_system)
    .bind(vendor_org_id)
    .bind(client_id)
    .bind(vendor_label)
    .bind(created_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_map(row))
}

// ---- the contact feed write path ---------------------------------------------

/// One contact feed sync's outcome — the visible-evidence packet.
#[derive(Debug, Clone)]
pub struct ContactFeedUpsertResult {
    pub contact: Contact,
    pub link: EntitySourceLink,
    pub created_contact: bool,
    pub resolved_existing: bool,
    pub auto_merged: bool,
    pub queued_candidates: Vec<EntityMergeCandidate>,
    /// Observed values that contradict a curated value (never applied).
    pub contradictions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContactFeedUpsert {
    pub source_system: String,
    pub external_id: String,
    pub source_guid: Option<String>,
    pub payload_ref: Option<String>,
    pub client_id: Uuid,
    /// Display defaults for a freshly created contact.
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub title: Option<String>,
}

impl ContactFeedUpsert {
    /// The observed tier arriving with THIS feed: only present, non-null
    /// keys (verbatim, like every other connector's observed tier).
    pub fn observed_fields(&self) -> Vec<(&'static str, Value)> {
        let mut out: Vec<(&'static str, Value)> = Vec::new();
        if !self.name.trim().is_empty() {
            out.push(("name", json!(self.name.trim())));
        }
        if let Some(e) = self
            .email
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            out.push(("email", json!(e)));
        }
        if let Some(p) = self
            .phone
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            out.push(("phone", json!(p)));
        }
        if let Some(t) = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            out.push(("title", json!(t)));
        }
        out
    }

    fn identity(&self) -> ContactIdentityKeys {
        ContactIdentityKeys::from_value(&json!({
            "email": self.email.clone(),
            "phone": self.phone.clone(),
        }))
    }
}

/// Follows merge tombstones to the canonical contact.
async fn contact_canonical(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    mut contact: Contact,
) -> DbResult<Contact> {
    for _ in 0..4 {
        match contact.merged_into_id {
            Some(target) => {
                contact = crate::clients::get_contact(&mut *conn, org_id, target).await?
            }
            None => return Ok(contact),
        }
    }
    Err(DbError::Conflict("merge tombstone chain too deep".into()))
}

/// Applies one observed contact field under CI5 rules: curated is sticky
/// (record + contradiction flag, never applied); feeds write the observed
/// tier and project the effective view onto the base columns when they
/// own the field. Contacts have no per-source precedence table: the FIRST
/// feed to write a field owns it; later feeds' values land in the
/// observed tier (visible in the workbench), and curated edits (the
/// human `update_contact` path) always win after that.
async fn apply_contact_observed(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    contact_id: Uuid,
    link_id: Uuid,
    source_system: &str,
    key: &str,
    value: &Value,
) -> DbResult<bool> {
    let contact = crate::clients::get_contact(&mut *conn, org_id, contact_id).await?;
    let curated_obj = contact
        .curated_fields
        .as_object()
        .cloned()
        .unwrap_or_default();
    let contradicts = matches!(curated_obj.get(key), Some(existing) if existing != value);

    let mut observed = contact
        .observed_fields
        .as_object()
        .cloned()
        .unwrap_or_default();
    observed.insert(key.to_string(), value.clone());

    let mut sources = contact
        .field_sources
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut entry = sources
        .get(key)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let first_write = entry.is_empty();
    entry.insert("source_system".into(), json!(source_system));
    entry.insert("source_link_id".into(), json!(link_id));
    entry.insert("observed_at".into(), json!(Utc::now().to_rfc3339()));
    if contradicts {
        entry.insert("contradicts_curated".into(), json!(true));
    } else {
        entry.remove("contradicts_curated");
    }
    sources.insert(key.to_string(), Value::Object(entry));

    let mut effective_name = contact.name;
    let mut effective_email = contact.email;
    let mut effective_phone = contact.phone;
    let mut effective_title = contact.title;
    if !contradicts && first_write {
        let owned = curated_obj.get(key).is_none();
        if owned {
            match key {
                "name" => effective_name = value.as_str().unwrap_or(&effective_name).to_string(),
                "email" => effective_email = value.as_str().map(str::to_string),
                "phone" => effective_phone = value.as_str().map(str::to_string),
                "title" => effective_title = value.as_str().map(str::to_string),
                _ => {}
            }
        }
    }

    sqlx::query(
        "UPDATE contacts SET observed_fields = $3, field_sources = $4,
            name = $5, email = $6, phone = $7, title = $8, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(contact_id)
    .bind(Value::Object(observed))
    .bind(Value::Object(sources))
    .bind(effective_name)
    .bind(effective_email)
    .bind(effective_phone)
    .bind(effective_title)
    .execute(&mut *conn)
    .await?;
    Ok(contradicts)
}

/// Deterministic contact matching: email/phone identity keys over the
/// org's non-tombstoned contacts. Signals below 0.6 are noise.
async fn find_contact_matches(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    keys: &ContactIdentityKeys,
) -> DbResult<
    Vec<(
        Contact,
        core_domain::entities::config_identity::IdentityMatch,
    )>,
> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        "SELECT {} FROM contacts WHERE org_id = $1 AND merged_into_id IS NULL",
        crate::clients::CONTACT_COLUMNS
    ))
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let contact = crate::clients::row_to_contact(&row);
        let stored = ContactIdentityKeys::from_value(&json!({
            "email": contact.email.clone(),
            "phone": contact.phone.clone(),
        }));
        if let Some(m) = match_contact_identity(keys, &stored) {
            if m.confidence >= 0.6 {
                out.push((contact, m));
            }
        }
    }
    Ok(out)
}

/// CI2+CI3+CI4 for contacts — mirrors `config_identity::feed_upsert`
/// exactly: own link first, deterministic matching second, threshold
/// third, fresh row + reviewable candidates last. The same
/// `ci_settings.auto_merge_threshold` drives the contact queue (the org's
/// ONE identity-confidence dial, per ADR 0003).
pub async fn contact_feed_upsert(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    input: &ContactFeedUpsert,
) -> DbResult<ContactFeedUpsertResult> {
    // 1. own link first: upsert, never blind-insert.
    if let Some(link) = get_entity_link_by_external(
        &mut *conn,
        org_id,
        DedupeEntityKind::Contact.as_str(),
        &input.source_system,
        &input.external_id,
    )
    .await?
    {
        let bound = crate::clients::get_contact(&mut *conn, org_id, link.entity_id).await?;
        let contact = contact_canonical(&mut *conn, org_id, bound).await?;
        let link = if contact.id == link.entity_id {
            create_entity_link(
                &mut *conn,
                org_id,
                link.id,
                DedupeEntityKind::Contact.as_str(),
                contact.id,
                &input.source_system,
                &input.external_id,
                input.source_guid.as_deref(),
                input.payload_ref.as_deref(),
            )
            .await?
        } else {
            // The link points at a tombstone (pre-merge): heal it.
            create_entity_link(
                &mut *conn,
                org_id,
                link.id,
                DedupeEntityKind::Contact.as_str(),
                contact.id,
                &input.source_system,
                &input.external_id,
                input.source_guid.as_deref(),
                input.payload_ref.as_deref(),
            )
            .await?
        };
        let mut contradictions = Vec::new();
        for (key, value) in input.observed_fields() {
            if apply_contact_observed(
                &mut *conn,
                org_id,
                contact.id,
                link.id,
                &input.source_system,
                key,
                &value,
            )
            .await?
            {
                contradictions.push(key.to_string());
            }
        }
        let contact = crate::clients::get_contact(&mut *conn, org_id, contact.id).await?;
        return Ok(ContactFeedUpsertResult {
            contact,
            link,
            created_contact: false,
            resolved_existing: false,
            auto_merged: false,
            queued_candidates: Vec::new(),
            contradictions,
        });
    }

    // 2. deterministic matching.
    let keys = input.identity();
    let mut candidates = find_contact_matches(&mut *conn, org_id, &keys).await?;
    candidates.sort_by(|a, b| {
        b.1.confidence
            .partial_cmp(&a.1.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let threshold = crate::config_identity::auto_merge_threshold(&mut *conn, org_id).await?;
    let mut queued_candidates: Vec<EntityMergeCandidate> = Vec::new();

    let (target, created_contact, resolved_existing) = match candidates.first() {
        Some((contact, m)) if m.confidence >= threshold => (contact.id, false, true),
        _ => {
            // Below threshold (ambiguous) or no signal: fresh canonical row.
            let id = Uuid::now_v7();
            crate::clients::create_contact(
                &mut *conn,
                org_id,
                id,
                input.client_id,
                None,
                &input.name,
                input.email.as_deref(),
                input.phone.as_deref(),
                input.title.as_deref(),
                false,
            )
            .await?;
            // Queue the reviewable candidates against the new row.
            for (contact, m) in &candidates {
                let candidate = create_entity_candidate(
                    &mut *conn,
                    org_id,
                    Uuid::now_v7(),
                    DedupeEntityKind::Contact.as_str(),
                    contact.id,
                    Some(id),
                    Some(&input.source_system),
                    None,
                    None,
                    m.confidence,
                    &m.reason,
                    &m.evidence,
                    "pending",
                )
                .await?;
                queued_candidates.push(candidate);
            }
            (id, true, false)
        }
    };

    let link = create_entity_link(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        DedupeEntityKind::Contact.as_str(),
        target,
        &input.source_system,
        &input.external_id,
        input.source_guid.as_deref(),
        input.payload_ref.as_deref(),
    )
    .await?;
    let mut contradictions = Vec::new();
    for (key, value) in input.observed_fields() {
        if apply_contact_observed(
            &mut *conn,
            org_id,
            target,
            link.id,
            &input.source_system,
            key,
            &value,
        )
        .await?
        {
            contradictions.push(key.to_string());
        }
    }
    let contact = crate::clients::get_contact(&mut *conn, org_id, target).await?;
    Ok(ContactFeedUpsertResult {
        contact,
        link,
        created_contact,
        resolved_existing,
        auto_merged: resolved_existing && !created_contact,
        queued_candidates,
        contradictions,
    })
}

// ---- contact merge / split ----------------------------------------------------

/// Folds `merged_id` into `primary_id`: ticket references relink, source
/// links move, tiers fill gaps only, and the absorbed row becomes a
/// tombstone. Returns the audit row (the split substrate).
#[allow(clippy::too_many_arguments)]
pub async fn merge_contacts(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    primary_id: Uuid,
    merged_id: Uuid,
    candidate_id: Option<Uuid>,
    confidence: Option<f64>,
    reason: &str,
    evidence: &Value,
    merged_by: Option<Uuid>,
) -> DbResult<EntityMerge> {
    if primary_id == merged_id {
        return Err(DbError::Conflict(
            "cannot merge a contact into itself".into(),
        ));
    }
    let primary = crate::clients::get_contact(&mut *conn, org_id, primary_id).await?;
    let absorbed = crate::clients::get_contact(&mut *conn, org_id, merged_id).await?;
    if absorbed.merged_into_id.is_some() {
        return Err(DbError::Conflict("contact already merged".into()));
    }

    let moved_tickets: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE tickets SET contact_id = $3 WHERE org_id = $1 AND contact_id = $2 RETURNING id",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(primary_id)
    .fetch_all(&mut *conn)
    .await?;
    let moved_links: Vec<Uuid> =
        move_entity_links(&mut *conn, org_id, merged_id, primary_id).await?;

    let mut observed = primary
        .observed_fields
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(incoming) = absorbed.observed_fields.as_object() {
        for (k, v) in incoming {
            observed.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let mut curated = primary
        .curated_fields
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(incoming) = absorbed.curated_fields.as_object() {
        for (k, v) in incoming {
            curated.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    sqlx::query(
        "UPDATE contacts SET observed_fields = $3, curated_fields = $4, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(primary_id)
    .bind(Value::Object(observed))
    .bind(Value::Object(curated))
    .execute(&mut *conn)
    .await?;

    let moved = json!({
        "ticket_ids": moved_tickets,
        "source_link_ids": moved_links,
        "observed_before": absorbed.observed_fields,
        "curated_before": absorbed.curated_fields,
    });

    // Tombstone the absorbed row LAST (it is the split substrate).
    sqlx::query(
        "UPDATE contacts SET merged_into_id = $3, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(primary_id)
    .execute(&mut *conn)
    .await?;

    record_entity_merge(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        DedupeEntityKind::Contact.as_str(),
        primary_id,
        merged_id,
        candidate_id,
        confidence,
        reason,
        evidence,
        &moved,
        merged_by,
    )
    .await
}

/// Split (undo): restores the absorbed contact from the audit row's
/// `moved` packet — ticket refs and links ride back, tiers are restored
/// from the snapshot, and the tombstone clears.
pub async fn split_entity_merge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    merge_id: Uuid,
    undone_by: Uuid,
) -> DbResult<EntityMerge> {
    let merge = get_entity_merge(&mut *conn, org_id, merge_id).await?;
    if merge.status != "applied" {
        return Err(DbError::Conflict("merge already undone".into()));
    }
    let merged_id = merge.merged_entity_id;

    let parse_ids = |v: Option<&Value>| -> Vec<Uuid> {
        v.and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect()
            })
            .unwrap_or_default()
    };
    for ticket_id in parse_ids(merge.moved.get("ticket_ids")) {
        sqlx::query("UPDATE tickets SET contact_id = $3 WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(ticket_id)
            .bind(merged_id)
            .execute(&mut *conn)
            .await?;
    }
    for link_id in parse_ids(merge.moved.get("source_link_ids")) {
        sqlx::query(
            "UPDATE entity_source_links SET entity_id = $3, updated_at = now()
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(link_id)
        .bind(merged_id)
        .execute(&mut *conn)
        .await?;
    }

    let observed_before = merge
        .moved
        .get("observed_before")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let curated_before = merge
        .moved
        .get("curated_before")
        .cloned()
        .unwrap_or_else(|| json!({}));
    sqlx::query(
        "UPDATE contacts SET observed_fields = $3, curated_fields = $4,
            merged_into_id = NULL, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(observed_before)
    .bind(curated_before)
    .execute(&mut *conn)
    .await?;

    let row = sqlx::query(&format!(
        "UPDATE entity_merges
         SET status = 'undone', undone_by = $3, undone_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'applied'
         RETURNING {MERGE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(merge_id)
    .bind(undone_by)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_merge)
    .ok_or(DbError::Conflict("merge already undone".into()))?;
    Ok(row)
}

// ---- client org_map reconciliation --------------------------------------------

/// Queues the reviewable candidates for one unmapped vendor org: the
/// deterministic label proposals (core-domain) score every Marula
/// client; the best-scoring proposal above 0.6 queues ONE candidate
/// (never more — a second candidate for the same vendor org replaces the
/// open one's evidence via the pair upsert). Already-mapped orgs
/// short-circuit: the durable map is the operator's word, never re-litigated
/// by a candidate.
pub async fn queue_client_reconciliation(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    source_system: &str,
    vendor_org_id: &str,
    vendor_label: Option<&str>,
) -> DbResult<Vec<EntityMergeCandidate>> {
    let mapped = get_org_map(&mut *conn, org_id, source_system, vendor_org_id).await?;
    if mapped.is_some() {
        return Ok(Vec::new());
    }
    let Some(label) = vendor_label.map(str::trim).filter(|l| !l.is_empty()) else {
        // No label — the connector's report stays the truth surface; there
        // is nothing deterministic to propose (org_map's never-guess rule).
        return Ok(Vec::new());
    };
    let clients = crate::clients::list_clients(&mut *conn, org_id).await?;
    let pairs: Vec<(Uuid, String)> = clients.iter().map(|c| (c.id, c.name.clone())).collect();
    let proposals = propose_client_matches(label, &pairs);
    let mut queued = Vec::new();
    if let Some(best) = proposals.first() {
        let candidate = create_entity_candidate(
            &mut *conn,
            org_id,
            Uuid::now_v7(),
            DedupeEntityKind::Client.as_str(),
            best.client_id,
            None,
            Some(source_system),
            Some(vendor_org_id),
            Some(label),
            best.confidence,
            &best.reason,
            &json!({
                "signals": [
                    { "vendor_org_id": vendor_org_id, "vendor_label": label },
                ],
            }),
            "pending",
        )
        .await?;
        queued.push(candidate);
    }
    Ok(queued)
}

/// A reconciliation accept WRITES the durable operator map (audited
/// upstream) and marks the candidate accepted. Client candidates are
/// NEVER auto-accepted: a mapping is an operator decision by definition.
pub async fn accept_client_reconciliation(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    candidate_id: Uuid,
    decided_by: Uuid,
) -> DbResult<(EntityMergeCandidate, ClientOrgMap)> {
    let candidate = get_entity_candidate(&mut *conn, org_id, candidate_id).await?;
    if candidate.entity_kind != DedupeEntityKind::Client {
        return Err(DbError::Conflict(
            "candidate is not a client reconciliation".into(),
        ));
    }
    let (source_system, vendor_org_id) = match (
        candidate.source_system.as_deref(),
        candidate.vendor_org_id.as_deref(),
    ) {
        (Some(s), Some(v)) => (s.to_string(), v.to_string()),
        _ => {
            return Err(DbError::Conflict(
                "reconciliation candidate carries no vendor org".into(),
            ))
        }
    };
    let map = upsert_org_map(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        &source_system,
        &vendor_org_id,
        candidate.primary_entity_id,
        candidate.vendor_label.as_deref(),
        decided_by,
    )
    .await?;
    let candidate = decide_entity_candidate(
        &mut *conn,
        org_id,
        candidate.id,
        EntityMergeStatus::Accepted.as_str(),
        decided_by,
    )
    .await?;
    Ok((candidate, map))
}

/// Records a client-side source link for a mapped vendor org (the
/// connectors write one per pull once the map resolves; the workbench
/// shows the org's per-source provenance on the client card).
pub async fn link_client_org(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    source_system: &str,
    vendor_org_id: &str,
) -> DbResult<EntitySourceLink> {
    let mapped = get_org_map(conn, org_id, source_system, vendor_org_id).await?;
    if mapped.is_none() {
        return Err(DbError::NotFound);
    }
    create_entity_link(
        conn,
        org_id,
        Uuid::now_v7(),
        DedupeEntityKind::Client.as_str(),
        client_id,
        source_system,
        vendor_org_id,
        None,
        None,
    )
    .await
}

// ---- the Jev dedupe workflow --------------------------------------------------

use core_domain::entities::entity_dedupe::DedupeWorkflow;

const WORKFLOW_COLUMNS: &str =
    "org_id, enabled, confidence_threshold::float8 AS confidence_threshold, \
     mode, interval_hours, notify_user_ids, entity_kinds, next_run_at, last_run_at, \
     last_result, updated_by, created_at, updated_at";

fn row_to_workflow(r: sqlx::postgres::PgRow) -> DedupeWorkflow {
    DedupeWorkflow {
        org_id: r.get("org_id"),
        enabled: r.get("enabled"),
        confidence_threshold: r
            .get::<Option<f64>, _>("confidence_threshold")
            .unwrap_or(0.8),
        mode: r
            .get::<String, _>("mode")
            .parse_dedupe_mode()
            .unwrap_or(core_domain::entities::entity_dedupe::DedupeRunMode::ReportOnly),
        interval_hours: r.get("interval_hours"),
        notify_user_ids: r.get("notify_user_ids"),
        entity_kinds: r.get("entity_kinds"),
        next_run_at: r.get("next_run_at"),
        last_run_at: r.get("last_run_at"),
        last_result: r.get("last_result"),
        updated_by: r.get("updated_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// Tiny helper trait so the mode string parses without importing a
/// FromStr impl that doesn't exist for the mode type.
trait ParseDedupeMode {
    fn parse_dedupe_mode(self) -> Option<core_domain::entities::entity_dedupe::DedupeRunMode>;
}

impl ParseDedupeMode for String {
    fn parse_dedupe_mode(self) -> Option<core_domain::entities::entity_dedupe::DedupeRunMode> {
        core_domain::entities::entity_dedupe::DedupeRunMode::parse(&self)
    }
}

pub async fn get_dedupe_workflow(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<DedupeWorkflow> {
    let row = sqlx::query(&format!(
        "SELECT {WORKFLOW_COLUMNS} FROM dedupe_workflows WHERE org_id = $1"
    ))
    .bind(org_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_to_workflow(row))
}

/// Upserts the workflow row. The DECISION GATE lives upstream (only the
/// API layer can see the provider config); this stores the validated
/// payload. Enabling with a schedule stamps the first next_run_at.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_dedupe_workflow(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    enabled: bool,
    confidence_threshold: f64,
    mode: &str,
    interval_hours: Option<i32>,
    notify_user_ids: &[Uuid],
    entity_kinds: &[String],
    updated_by: Uuid,
) -> DbResult<DedupeWorkflow> {
    let row = sqlx::query(&format!(
        "INSERT INTO dedupe_workflows
           (org_id, enabled, confidence_threshold, mode, interval_hours,
            notify_user_ids, entity_kinds, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (org_id) DO UPDATE
           SET enabled = EXCLUDED.enabled,
                confidence_threshold = EXCLUDED.confidence_threshold,
                mode = EXCLUDED.mode,
                interval_hours = EXCLUDED.interval_hours,
                notify_user_ids = EXCLUDED.notify_user_ids,
                entity_kinds = EXCLUDED.entity_kinds,
                updated_by = EXCLUDED.updated_by,
                -- A schedule change re-anchors the cadence; disabling clears it.
                next_run_at = CASE
                  WHEN EXCLUDED.enabled AND EXCLUDED.interval_hours IS NOT NULL
                    THEN now() + make_interval(hours => EXCLUDED.interval_hours)
                  ELSE NULL END,
                updated_at = now()
         RETURNING {WORKFLOW_COLUMNS}"
    ))
    .bind(org_id)
    .bind(enabled)
    .bind(confidence_threshold)
    .bind(mode)
    .bind(interval_hours)
    .bind(notify_user_ids)
    .bind(entity_kinds)
    .bind(updated_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_workflow(row))
}

/// Atomically claims a DUE scheduled run (the pm_sweep claim pattern):
/// only the row whose next_run_at has passed advances its stamp — the
/// claim prevents double-running across worker replicas. Returns the
/// workflow when claimed.
pub async fn claim_due_dedupe_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<Option<DedupeWorkflow>> {
    let row = sqlx::query(&format!(
        "UPDATE dedupe_workflows
         SET next_run_at = now() + make_interval(hours => interval_hours),
             last_run_at = now(), updated_at = now()
         WHERE org_id = $1 AND enabled AND interval_hours IS NOT NULL
           AND next_run_at IS NOT NULL AND next_run_at <= now()
         RETURNING {WORKFLOW_COLUMNS}"
    ))
    .bind(org_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(row_to_workflow))
}

/// Lists every org with a due scheduled run (the sweep's claim source).
pub async fn list_due_dedupe_orgs(
    conn: &mut sqlx::PgConnection,
    limit: i64,
) -> DbResult<Vec<Uuid>> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT org_id FROM dedupe_workflows
         WHERE enabled AND interval_hours IS NOT NULL
           AND next_run_at IS NOT NULL AND next_run_at <= now()
         ORDER BY next_run_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids)
}

/// Persists one pass's report on the workflow row.
pub async fn record_dedupe_run(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    report: &core_domain::entities::entity_dedupe::DedupeRunReport,
) -> DbResult<()> {
    sqlx::query(
        "UPDATE dedupe_workflows
         SET last_run_at = now(), last_result = $2, updated_at = now()
         WHERE org_id = $1",
    )
    .bind(org_id)
    .bind(serde_json::to_value(report).unwrap_or_default())
    .execute(&mut *conn)
    .await?;
    Ok(())
}
