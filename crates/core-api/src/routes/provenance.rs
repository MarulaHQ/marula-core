//! Provenance query endpoint (the private build tracker IF9, doctrine #9): the audit-log
//! query API (2.3) extended with provenance-grade filters over the
//! materialized per-field projection of the instant-first write trail.
//!
//! Where `GET /api/audit-events` answers "what audit events happened"
//! (security authority, action-prefixed), THIS surface answers the
//! interaction-trail questions doctrine #9 names — who changed which
//! record/field, the before and after values, the actor kind
//! (human | agent | ...), the agent session/skill/rule-set hash when
//! present, undo reverts, and when — as indexed lookups over
//! `provenance_events` (migration 0043), never raw-row scans.
//!
//! Conventions mirror the 2.3 audit API exactly: newest first, opaque
//! base64 keyset cursor, `from`/`to` bounds, clamped limit — extended with
//! the third cursor component (field_seq) that per-field rows need.
//!
//! RBAC: holders of `ViewAuditLog` (owner/admin and any custom role
//! granted it) query org-wide provenance; every other authenticated user
//! sees ONLY their own actions — the handler forces the actor filter to
//! the caller's id and ignores a client-supplied `actor`, so a
//! non-manager can neither widen nor impersonate.
//!
//! Non-purpose: no audit-event semantics here (that surface keeps its own
//! contract), no mutations, and no agent-context WRITES — IF8 approvals
//! populate session/skill/ruleset_hash through the envelope's provenance
//! context; this endpoint only reads them.

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::instant::ACTOR_KINDS;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use core_db::provenance::{ProvenanceFilter, ProvenanceRow};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ProvenanceQuery {
    /// Keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Actor-kind filter (the write-trail vocabulary: human, agent,
    /// import, ...).
    pub actor_kind: Option<String>,
    /// The "who" filter: a user uuid. Ignored (forced to the caller) for
    /// callers without the audit permission.
    pub actor: Option<Uuid>,
    /// Entity family, e.g. "ticket". Required by `entity_id` and `field`.
    pub entity: Option<String>,
    /// Record id; requires `entity`.
    pub entity_id: Option<Uuid>,
    /// Single-field provenance; requires `entity`.
    pub field: Option<String>,
    /// Agent session filter (rows carry it when present).
    pub session_id: Option<Uuid>,
    /// Agent skill filter (rows carry it when present).
    pub skill: Option<String>,
    /// Rule-set hash filter (rows carry it when present).
    pub ruleset_hash: Option<String>,
    /// Restrict to undo reverts (`true`) or exclude them (`false`).
    pub undo: Option<bool>,
    /// Inclusive lower bound on occurred_at.
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// Inclusive upper bound on occurred_at.
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

fn decode_cursor(raw: &str) -> Option<(chrono::DateTime<chrono::Utc>, Uuid, i32)> {
    let decoded = B64.decode(raw.as_bytes()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (ts, rest) = text.split_once('|')?;
    let (id, seq) = rest.split_once('|')?;
    Some((ts.parse().ok()?, id.parse().ok()?, seq.parse().ok()?))
}

fn encode_cursor(at: chrono::DateTime<chrono::Utc>, id: Uuid, field_seq: i32) -> String {
    B64.encode(format!("{}|{}|{}", at.to_rfc3339(), id, field_seq).as_bytes())
}

/// GET /api/provenance — the provenance read model: who / record+field /
/// before-after / actor kind / agent session+skill+rule-set hash / when,
/// newest first, filterable, cursor-paged. Extends the 2.3 audit-log
/// query API with provenance-grade filters backed by the materialized
/// projection (no raw-row scans).
#[utoipa::path(operation_id = "provenance_list", get, path = "/api/provenance", tag = "audit", security(("bearer" = [])),
    params(ProvenanceQuery),
    responses((status = 200, body = ProvenancePage)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ProvenanceQuery>,
) -> ApiResult<Json<ProvenancePage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    // Manager+ (the audit permission) sees org provenance; everyone else
    // is pinned to their own actions.
    let org_wide = require_permission(
        &authed.principal,
        core_domain::policy::Permission::ViewAuditLog,
    )
    .is_ok();

    if let Some(kind) = q.actor_kind.as_deref() {
        if !ACTOR_KINDS.contains(&kind) {
            return Err(crate::error::ApiError::Validation(format!(
                "unknown actor_kind {kind:?}"
            )));
        }
    }
    if (q.entity_id.is_some() || q.field.is_some()) && q.entity.is_none() {
        return Err(crate::error::ApiError::Validation(
            "entity_id and field filters require entity".into(),
        ));
    }

    let actor_id = if org_wide {
        q.actor
    } else {
        Some(authed.principal.user.id)
    };

    let cursor = q.cursor.as_deref().and_then(decode_cursor);
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(crate::error::ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(crate::error::ApiError::from)?;
    let rows = core_db::provenance::list_filtered(
        &mut *tx,
        authed.principal.user.org_id,
        ProvenanceFilter {
            actor_kind: q.actor_kind,
            actor_id,
            entity: q.entity,
            entity_id: q.entity_id,
            field: q.field,
            session_id: q.session_id,
            skill: q.skill,
            ruleset_hash: q.ruleset_hash,
            is_undo: q.undo,
            from: q.from,
            to: q.to,
            cursor,
        },
        q.limit.clamp(1, 500),
    )
    .await?;
    tx.commit().await.map_err(crate::error::ApiError::from)?;
    // A full page hints another exists; anchor it to the last row.
    let next_cursor = if rows.len() as i64 >= q.limit.clamp(1, 500) {
        rows.last()
            .map(|r| encode_cursor(r.occurred_at, r.write_id, r.field_seq))
    } else {
        None
    };
    Ok(Json(ProvenancePage {
        events: rows.into_iter().map(provenance_from_row).collect(),
        next_cursor,
    }))
}

fn provenance_from_row(row: ProvenanceRow) -> ProvenanceEvent {
    ProvenanceEvent {
        write_id: row.write_id,
        entity: row.entity,
        entity_id: row.entity_id,
        seq: row.seq,
        op: row.op,
        actor_kind: row.actor_kind,
        actor_id: row.actor_id,
        field: row.field,
        before: row.before,
        after: row.after,
        session_id: row.session_id,
        skill: row.skill,
        ruleset_hash: row.ruleset_hash,
        undo: row.is_undo,
        idempotency_key: row.idempotency_key,
        occurred_at: row.occurred_at,
    }
}

/// One provenance entry: a single field change within one applied write.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProvenanceEvent {
    pub write_id: Uuid,
    /// Entity family, e.g. "ticket".
    pub entity: String,
    #[schema(value_type = uuid::Uuid)]
    pub entity_id: Uuid,
    /// The write's per-entity sequence (mirrors `mutation.version`).
    pub seq: i64,
    /// Machine-readable operation ("create", "update", "status_change", ...).
    pub op: String,
    /// Actor kind from the write-trail vocabulary (human, agent, ...).
    pub actor_kind: String,
    /// The acting user (or machine actor) id, when attributed.
    #[schema(value_type = uuid::Uuid, nullable = true)]
    pub actor_id: Option<Uuid>,
    /// The changed field.
    pub field: String,
    /// Prior value; null when the field is new.
    #[schema(value_type = Object, nullable = true)]
    pub before: Option<serde_json::Value>,
    /// Applied value; null when the field was removed in this write.
    #[schema(value_type = Object, nullable = true)]
    pub after: Option<serde_json::Value>,
    /// Agent session, when present (IF8-approved writes).
    #[schema(value_type = uuid::Uuid, nullable = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    /// Skill that authored the write, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// Rule-set hash that governed the write, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ruleset_hash: Option<String>,
    /// True for undo reverts (IF7) riding the normal write path.
    pub undo: bool,
    /// The idempotency key that claimed the write, when the caller
    /// supplied one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[schema(value_type = String)]
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProvenancePage {
    pub events: Vec<ProvenanceEvent>,
    /// Opaque cursor for the next page; absent when exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
