//! Agent platform v1 (the private build tracker Phase 3, IF8; docs/10-agent-platform.md
//! §4/§5, docs/instant-first.md "Agent-authored changes default to
//! approval").
//!
//! Three surfaces over migration 0040:
//!
//! 1. **Capability registry** — deny-by-default grants of
//!    `<namespace>:read`/`<namespace>:write` capabilities with the
//!    involvement dial (`proposed` | `autonomous`) per capability.
//!    Owner/admin only; autonomous is legal only on write capabilities.
//! 2. **Agent skills** — append-only versioned manifests. A new version
//!    is a NEW row; rollback publishes an old manifest as a new version.
//!    History is never rewritten.
//! 3. **IF8 proposal pipeline** — agent-authored record changes surface
//!    as proposals carrying the before value, the proposed after value,
//!    the agent identity, skill + version, and the rule-set hash.
//!    Approvals are keyed to the involvement dial: `proposed` (default)
//!    requires a human decision; `autonomous` is an explicit
//!    per-capability grant recorded on the proposal, never a global
//!    switch. A DECLINED proposal produces no write at all; an APPROVED
//!    ticket-field proposal applies through the IF1 instant-first path
//!    (idempotency-keyed, version-gated, write-trail row) so it stays
//!    undoable like any human autosave. v1 restricts the proposal
//!    application surface to ticket updates; the store itself is generic.
//!
//! Provenance audit events ride every proposal action
//! (`agent_proposal.*` / `agent_skill.*` / `mcp_capability.updated`) via
//! the audit trail AND the domain-event contract.
//!
//! Non-purpose: no write-execution tools in the MCP catalog yet (read
//! tools + proposal CRUD only), no layered rules evaluation (§4 rules
//! land with the write-execution lanes), no client-visible bypass —
//! `client_visible`/`external_commit` proposals always thread a human
//! approval whatever the dial says.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim};
use crate::org_tx;
use crate::routes::{board_access::authorize_board_op, ApiGeneration};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use core_db::agent_platform::{
    self, AgentProposalRow, AgentSkillRow, CapabilityGrantRow, ProposalFields,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, BoardOp, Module, Permission};
use core_domain::workflow_approvals;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Wire involvement-dial position of a capability grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMode {
    /// Agent-originated changes surface as proposals awaiting approval
    /// (the default — doctrine #8).
    Proposed,
    /// Explicit per-capability autonomous grant, never a global switch.
    Autonomous,
}

impl CapabilityMode {
    fn as_str(self) -> &'static str {
        match self {
            CapabilityMode::Proposed => "proposed",
            CapabilityMode::Autonomous => "autonomous",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "proposed" => Some(CapabilityMode::Proposed),
            "autonomous" => Some(CapabilityMode::Autonomous),
            _ => None,
        }
    }
}

/// Proposal classification: who will see the change if it lands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalClassification {
    #[default]
    Internal,
    /// Anything a client will see — always threads a human approval.
    ClientVisible,
    /// Anything committing externally (email, portal, booking) — ditto.
    ExternalCommit,
}

impl ProposalClassification {
    fn as_str(self) -> &'static str {
        match self {
            ProposalClassification::Internal => "internal",
            ProposalClassification::ClientVisible => "client_visible",
            ProposalClassification::ExternalCommit => "external_commit",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "internal" => Some(ProposalClassification::Internal),
            "client_visible" => Some(ProposalClassification::ClientVisible),
            "external_commit" => Some(ProposalClassification::ExternalCommit),
            _ => None,
        }
    }
}

/// Proposal lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Proposed,
    Approved,
    Declined,
    Withdrawn,
    Applied,
}

impl ProposalStatus {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "proposed" => Some(ProposalStatus::Proposed),
            "approved" => Some(ProposalStatus::Approved),
            "declined" => Some(ProposalStatus::Declined),
            "withdrawn" => Some(ProposalStatus::Withdrawn),
            "applied" => Some(ProposalStatus::Applied),
            _ => None,
        }
    }
}

// --- capability registry -------------------------------------------------------

/// One granted capability with its dial position.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CapabilityGrantView {
    pub capability: String,
    pub mode: CapabilityMode,
    #[schema(value_type = uuid::Uuid)]
    pub granted_by: Uuid,
    #[schema(value_type = String)]
    pub granted_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl From<CapabilityGrantRow> for CapabilityGrantView {
    fn from(r: CapabilityGrantRow) -> Self {
        Self {
            capability: r.capability,
            mode: CapabilityMode::parse(&r.mode).unwrap_or(CapabilityMode::Proposed),
            granted_by: r.granted_by,
            granted_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CapabilityGrantList {
    pub grants: Vec<CapabilityGrantView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutCapabilityRequest {
    /// `<namespace>:read` | `<namespace>:write`.
    pub capability: String,
    pub mode: CapabilityMode,
}

/// GET /api/v1/mcp/capabilities — every granted capability of the org.
#[utoipa::path(
    operation_id = "mcp_capabilities_list",
    get,
    path = "/api/mcp/capabilities",
    tag = "agent-platform",
    security(("bearer" = [])),
    responses((status = 200, body = CapabilityGrantList))
)]
pub async fn capabilities_list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<CapabilityGrantList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::View,
        },
    )?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        agent_platform::list_capability_grants(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CapabilityGrantList {
        grants: rows.into_iter().map(CapabilityGrantView::from).collect(),
    }))
}

/// PUT /api/v1/mcp/capabilities — grant or re-dial one capability.
#[utoipa::path(
    operation_id = "mcp_capability_put",
    put,
    path = "/api/mcp/capabilities",
    tag = "agent-platform",
    security(("bearer" = [])),
    request_body = PutCapabilityRequest,
    responses(
        (status = 200, body = CapabilityGrantView),
        (status = 403, description = "owner/admin surface"),
        (status = 422, description = "invalid capability or dial position")
    )
)]
pub async fn capabilities_put(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutCapabilityRequest>,
) -> ApiResult<Json<CapabilityGrantView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::Edit,
        },
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    validate_capability(&req.capability)?;
    // Autonomous is an explicit per-capability grant for record CHANGES;
    // read capabilities have nothing to act autonomously on.
    if req.mode == CapabilityMode::Autonomous && !req.capability.ends_with(":write") {
        return Err(ApiError::Validation(
            "autonomous mode applies to :write capabilities only".into(),
        ));
    }
    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let row = agent_platform::upsert_capability_grant(
        &mut tx.tx,
        Uuid::now_v7(),
        org_id,
        user_id,
        &req.capability,
        req.mode.as_str(),
    )
    .await?;
    audit(
        &mut tx.tx,
        org_id,
        user_id,
        "mcp_capability.updated",
        &req.capability,
        serde_json::json!({ "capability": req.capability, "mode": req.mode.as_str() }),
    )
    .await?;
    tx.emit(
        types::MCP_CAPABILITY_UPDATED,
        "mcp_capability",
        Some(req.capability.as_str()),
        &serde_json::json!({
            "capability": req.capability,
            "mode": req.mode.as_str(),
            "granted_by": user_id,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(CapabilityGrantView::from(row)))
}

/// DELETE /api/v1/mcp/capabilities/{capability} — revoke (deny again).
#[utoipa::path(
    operation_id = "mcp_capability_revoke",
    delete,
    path = "/api/mcp/capabilities/{capability}",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("capability" = String, Path)),
    responses((status = 204))
)]
pub async fn capabilities_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(capability): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::Delete,
        },
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    validate_capability(&capability)?;
    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    agent_platform::delete_capability_grant(&mut tx.tx, org_id, &capability).await?;
    audit(
        &mut tx.tx,
        org_id,
        user_id,
        "mcp_capability.updated",
        &capability,
        serde_json::json!({ "capability": capability, "revoked": true }),
    )
    .await?;
    tx.emit(
        types::MCP_CAPABILITY_UPDATED,
        "mcp_capability",
        Some(capability.as_str()),
        &serde_json::json!({
            "capability": capability,
            "granted_by": user_id,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- agent skills ---------------------------------------------------------------

/// One immutable skill version.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentSkillView {
    pub id: Uuid,
    pub slug: String,
    pub version: i32,
    /// `{capabilities, prompts, rules}` — declarative manifest (§4).
    #[schema(value_type = Object)]
    pub manifest: Value,
    #[schema(value_type = uuid::Uuid)]
    pub created_by: Uuid,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
}

impl From<AgentSkillRow> for AgentSkillView {
    fn from(r: AgentSkillRow) -> Self {
        Self {
            id: r.id,
            slug: r.slug,
            version: r.version,
            manifest: r.manifest,
            created_by: r.created_by,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentSkillList {
    pub skills: Vec<AgentSkillView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishSkillRequest {
    /// Slug; version is derived server-side (max + 1) — append-only
    /// history, never an in-place edit.
    pub slug: String,
    /// `{capabilities: [...], prompts: [...], rules: [...]}`.
    #[schema(value_type = Object)]
    pub manifest: Value,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct SkillListQuery {
    /// When present, every version of this slug (newest first) instead of
    /// the latest version of each slug.
    #[param(nullable = true)]
    pub slug: Option<String>,
}

/// GET /api/v1/agent-skills — latest version per slug (or one slug's history).
#[utoipa::path(
    operation_id = "agent_skills_list",
    get,
    path = "/api/agent-skills",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(SkillListQuery),
    responses((status = 200, body = AgentSkillList))
)]
pub async fn skills_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<SkillListQuery>,
) -> ApiResult<Json<AgentSkillList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = match q.slug.as_deref() {
        Some(slug) => {
            validate_slug(slug)?;
            agent_platform::list_skill_versions(&mut *tx, org_id, slug).await?
        }
        None => agent_platform::list_latest_skills(&mut *tx, org_id).await?,
    };
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AgentSkillList {
        skills: rows.into_iter().map(AgentSkillView::from).collect(),
    }))
}

/// GET /api/v1/agent-skills/{id} — one immutable version.
#[utoipa::path(
    operation_id = "agent_skills_get",
    get,
    path = "/api/agent-skills/{id}",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AgentSkillView), (status = 404))
)]
pub async fn skills_get(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AgentSkillView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::View,
        },
    )?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let row = agent_platform::get_skill(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AgentSkillView::from(row)))
}

/// POST /api/v1/agent-skills — publish a NEW version of a skill.
#[utoipa::path(
    operation_id = "agent_skills_publish",
    post,
    path = "/api/agent-skills",
    tag = "agent-platform",
    security(("bearer" = [])),
    request_body = PublishSkillRequest,
    responses(
        (status = 200, body = AgentSkillView),
        (status = 403, description = "owner/admin surface"),
        (status = 422, description = "invalid slug or manifest")
    )
)]
pub async fn skills_publish(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PublishSkillRequest>,
) -> ApiResult<Json<AgentSkillView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::Create,
        },
    )?;
    let user_id = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let slug = req.slug.trim().to_string();
    validate_slug(&slug)?;
    validate_manifest(&req.manifest)?;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let version = agent_platform::max_skill_version(&mut *tx.tx, org_id, &slug).await? + 1;
    let row = agent_platform::insert_skill(
        &mut tx.tx,
        Uuid::now_v7(),
        org_id,
        user_id,
        &slug,
        version,
        &req.manifest,
    )
    .await?;
    let (event, action): (&'static str, &'static str) = if version == 1 {
        (types::AGENT_SKILL_CREATED, "agent_skill.created")
    } else {
        (types::AGENT_SKILL_UPDATED, "agent_skill.updated")
    };
    audit(
        &mut tx.tx,
        org_id,
        user_id,
        action,
        &row.id.to_string(),
        serde_json::json!({ "slug": slug, "version": version }),
    )
    .await?;
    tx.emit(
        event,
        "agent_skill",
        Some(row.id.to_string().as_str()),
        &serde_json::to_value(&core_domain::events::AgentSkillEventPayload {
            skill_id: row.id,
            slug: row.slug.clone(),
            version: row.version,
            created_by: user_id,
        })
        .expect("typed agent-skill event payload serializes"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(AgentSkillView::from(row)))
}

// --- proposals (IF8) --------------------------------------------------------------

/// One IF8 proposal with the full snapshot the approval decision needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentProposalView {
    pub id: Uuid,
    /// Record coordinates (v1 applies `ticket`/`update` only; the store is
    /// generic).
    pub entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub entity_id: Option<Uuid>,
    pub op: String,
    /// The value the agent saw (`null` when the record did not exist).
    #[schema(value_type = Object, nullable = true)]
    pub before_value: Value,
    /// The proposed after value.
    #[schema(value_type = Object)]
    pub proposed_value: Value,
    pub classification: ProposalClassification,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub capability: Option<String>,
    /// Involvement-dial snapshot at proposal time.
    pub dial_mode: CapabilityMode,
    /// The entity write-seq the proposal was created against.
    pub base_version: i64,
    pub status: ProposalStatus,
    pub agent_kind: String,
    pub agent_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub skill_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub skill_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub rule_set_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub for_user: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub rationale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub decided_by: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = true)]
    pub decision_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub applied_at: Option<DateTime<Utc>>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl From<AgentProposalRow> for AgentProposalView {
    fn from(r: AgentProposalRow) -> Self {
        Self {
            id: r.id,
            entity: r.entity,
            entity_id: r.entity_id,
            op: r.op,
            before_value: r.before_value,
            proposed_value: r.proposed_value,
            classification: ProposalClassification::parse(&r.classification)
                .unwrap_or(ProposalClassification::Internal),
            capability: r.capability,
            dial_mode: CapabilityMode::parse(&r.dial_mode).unwrap_or(CapabilityMode::Proposed),
            base_version: r.base_version,
            status: ProposalStatus::parse(&r.status).unwrap_or(ProposalStatus::Proposed),
            agent_kind: r.agent_kind,
            agent_ref: r.agent_ref,
            skill_slug: r.skill_slug,
            skill_version: r.skill_version,
            rule_set_hash: r.rule_set_hash,
            session_id: r.session_id,
            tool: r.tool,
            for_user: r.for_user,
            rationale: r.rationale,
            decided_by: r.decided_by,
            decided_at: r.decided_at,
            decision_note: r.decision_note,
            applied_at: r.applied_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// GET /api/agent-proposals query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ProposalListQuery {
    /// Filter by lifecycle state.
    #[param(nullable = true)]
    pub status: Option<String>,
    /// Opaque keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, clamped to [1, 200].
    pub limit: Option<i64>,
}

/// GET /api/agent-proposals envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentProposalPage {
    pub proposals: Vec<AgentProposalView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// POST /api/agent-proposals body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProposalRequest {
    pub entity: String,
    #[serde(default)]
    #[schema(nullable = true)]
    pub entity_id: Option<Uuid>,
    pub op: String,
    /// The value the agent saw when it drafted the change (the IF8
    /// before-snapshot). `null` for creates. Application re-reads the
    /// live entity, so this is decision context, not a blind write.
    #[serde(default)]
    #[schema(value_type = Object, nullable = true)]
    pub before_value: Option<Value>,
    #[schema(value_type = Object)]
    pub proposed_value: Value,
    #[serde(default)]
    pub classification: ProposalClassification,
    /// Write capability this change belongs to (must be granted).
    #[serde(default)]
    #[schema(nullable = true)]
    pub capability: Option<String>,
    /// Agent provenance (§5). Optional: the machine identity defaults to
    /// the caller's (`api-key:<id>` / `user:<id>`).
    #[serde(default)]
    #[schema(nullable = true)]
    pub agent_ref: Option<String>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub skill_slug: Option<String>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub skill_version: Option<i32>,
    /// Hash of the evaluated rule set at proposal time.
    #[serde(default)]
    #[schema(nullable = true)]
    pub rule_set_hash: Option<String>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub session_id: Option<String>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub tool: Option<String>,
    /// The user the agent acted on behalf of.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub for_user: Option<Uuid>,
    #[serde(default)]
    #[schema(nullable = true)]
    pub rationale: Option<String>,
}

/// POST /api/agent-proposals response: the stored proposal.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateProposalResponse {
    pub proposal: AgentProposalView,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecideProposalRequest {
    pub note: Option<String>,
}

/// GET /api/agent-proposals — cursor-paged listing, newest first.
#[utoipa::path(
    operation_id = "agent_proposals_list",
    get,
    path = "/api/agent-proposals",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(ProposalListQuery),
    responses((status = 200, body = AgentProposalPage))
)]
pub async fn proposals_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ProposalListQuery>,
) -> ApiResult<Json<AgentProposalPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let (after_created, after_id) = match q.cursor.as_deref().map(decode_cursor) {
        None => (None, None),
        Some(None) => return Err(ApiError::Validation("malformed cursor".into())),
        Some(Some(pair)) => pair,
    };
    let status = match q.status.as_deref() {
        None => None,
        Some(raw) => {
            let parsed = ProposalStatus::parse(raw)
                .ok_or_else(|| ApiError::Validation(format!("unknown status {raw:?}")))?;
            Some(
                match parsed {
                    ProposalStatus::Proposed => "proposed",
                    ProposalStatus::Approved => "approved",
                    ProposalStatus::Declined => "declined",
                    ProposalStatus::Withdrawn => "withdrawn",
                    ProposalStatus::Applied => "applied",
                }
                .to_string(),
            )
        }
    };
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = agent_platform::list_proposals(
        &mut *tx,
        org_id,
        status.as_deref(),
        after_created,
        after_id,
        limit,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    let next_cursor = if rows.len() as i64 >= limit {
        rows.last().map(|r| encode_cursor(r.created_at, r.id))
    } else {
        None
    };
    Ok(Json(AgentProposalPage {
        proposals: rows.into_iter().map(AgentProposalView::from).collect(),
        next_cursor,
    }))
}

/// GET /api/agent-proposals/{id}.
#[utoipa::path(
    operation_id = "agent_proposals_get",
    get,
    path = "/api/agent-proposals/{id}",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AgentProposalView), (status = 404))
)]
pub async fn proposals_get(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AgentProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::View,
        },
    )?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let row = agent_platform::get_proposal(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AgentProposalView::from(row)))
}

/// POST /api/agent-proposals — file a proposal (IF8). Accepts human-JWT
/// callers for administrative simulation, but the API-key/machine path is
/// the intended caller: the machine principal's key id becomes the
/// agent_ref when none is supplied. The dial position is captured from
/// the capability's grant row at creation time.
#[utoipa::path(
    operation_id = "agent_proposals_create",
    post,
    path = "/api/agent-proposals",
    tag = "agent-platform",
    security(("bearer" = [])),
    request_body = CreateProposalRequest,
    responses(
        (status = 200, body = CreateProposalResponse),
        (status = 403, description = "capability not granted (deny-by-default)"),
        (status = 422, description = "validation failed")
    )
)]
pub async fn proposals_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateProposalRequest>,
) -> ApiResult<Json<CreateProposalResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let caller_id = authed.principal.user.id;

    let classification = req.classification;
    let entity = req.entity.trim().to_string();
    let op = req.op.trim().to_string();
    if entity.is_empty() || entity.len() > 64 {
        return Err(ApiError::Validation("entity must be 1..=64 chars".into()));
    }
    if !matches!(op.as_str(), "create" | "update" | "delete") {
        return Err(ApiError::Validation(
            "op must be create, update, or delete".into(),
        ));
    }
    if op == "update" && req.entity_id.is_none() {
        return Err(ApiError::Validation(
            "update proposals need entity_id".into(),
        ));
    }
    if !matches!(req.proposed_value, Value::Object(_) | Value::Null) {
        return Err(ApiError::Validation(
            "proposed_value must be an object".into(),
        ));
    }
    let agent_ref = normalize_agent_ref(&authed, req.agent_ref.as_deref())?;
    let agent_kind = if authed.via_api_key {
        "api-key"
    } else {
        "agent"
    };

    // Capability gate + dial capture. Deny-by-default: no grant row = 403.
    let capability = match req.capability.as_deref() {
        Some(c) => {
            validate_capability(c)?;
            Some(c.to_string())
        }
        None => None,
    };
    // RLS-scoped reads (pool carries no app.org_id GUC).
    let mut gate_tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut gate_tx, org_id, caller_id)
        .await
        .map_err(ApiError::from)?;
    let dial_mode = match &capability {
        Some(cap) => {
            let grant = agent_platform::get_capability_grant(&mut *gate_tx, org_id, cap)
                .await?
                .ok_or_else(|| {
                    ApiError::Forbidden(format!("capability {cap} is not granted for this org"))
                })?;
            CapabilityMode::parse(&grant.mode).unwrap_or(CapabilityMode::Proposed)
        }
        None => CapabilityMode::Proposed,
    };

    // `for_user` names the human the agent acted on behalf of. A key
    // principal IS the agent (not a user row), so it never defaults to
    // itself here; a supplied id must resolve to an org user or the
    // filing is rejected before the FK ever sees it.
    let for_user = match req.for_user {
        Some(id) => {
            core_db::users::get_user(&mut *gate_tx, org_id, id)
                .await
                .map_err(|_| {
                    ApiError::Validation("for_user does not name a user of this org".into())
                })?;
            Some(id)
        }
        None if !authed.via_api_key => Some(caller_id),
        None => None,
    };

    // Drift provenance: the write-seq the record sits at right now.
    let base_version = match (op.as_str(), req.entity_id) {
        ("update", Some(id)) => {
            agent_platform::entity_current_seq(&mut *gate_tx, org_id, &entity, id).await?
        }
        _ => 0,
    };
    gate_tx.commit().await.map_err(ApiError::from)?;

    let mut tx = org_tx::begin(&state, org_id, caller_id).await?;
    let fields = ProposalFields {
        entity: entity.clone(),
        entity_id: req.entity_id,
        op: op.clone(),
        // before_value is captured as `null` at filing time; the approval
        // flow re-reads the live entity (application re-validates), so
        // this is the decision record's "what the agent saw" slot.
        before_value: req.before_value.clone().unwrap_or(Value::Null),
        proposed_value: req.proposed_value.clone(),
        classification: classification.as_str().to_string(),
        capability: capability.clone(),
        dial_mode: dial_mode.as_str().to_string(),
        base_version,
        agent_kind: agent_kind.to_string(),
        agent_ref: agent_ref.clone(),
        skill_slug: req.skill_slug.clone(),
        skill_version: req.skill_version,
        rule_set_hash: req.rule_set_hash.clone(),
        session_id: req.session_id.clone(),
        tool: req.tool.clone(),
        for_user,
        rationale: req.rationale.clone(),
    };
    let row = agent_platform::insert_proposal(&mut tx.tx, Uuid::now_v7(), org_id, &fields).await?;
    audit(
        &mut tx.tx,
        org_id,
        caller_id,
        "agent_proposal.created",
        &row.id.to_string(),
        serde_json::json!({
            "entity": entity,
            "entity_id": req.entity_id,
            "op": op,
            "classification": classification.as_str(),
            "dial_mode": dial_mode.as_str(),
            "agent_ref": agent_ref,
        }),
    )
    .await?;
    tx.emit(
        types::AGENT_PROPOSAL_CREATED,
        "agent_proposal",
        Some(row.id.to_string().as_str()),
        &serde_json::json!({
            "proposal_id": row.id,
            "entity": entity,
            "entity_id": req.entity_id,
            "op": op,
            "classification": classification.as_str(),
            "dial_mode": dial_mode.as_str(),
            "agent_ref": agent_ref,
            "skill_slug": req.skill_slug,
            "skill_version": req.skill_version,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(CreateProposalResponse {
        proposal: AgentProposalView::from(row),
    }))
}

/// POST /api/agent-proposals/{id}/approve — decide FOR. Approved ticket
/// update proposals apply immediately through the IF1 instant-first path
/// in the SAME transaction (idempotency-keyed, version-gated, write-trail
/// row — stays undoable); other entity/op combinations stay approved-
/// pending until the write-execution lanes widen application.
#[utoipa::path(
    operation_id = "agent_proposals_approve",
    post,
    path = "/api/agent-proposals/{id}/approve",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = DecideProposalRequest,
    responses(
        (status = 200, body = AgentProposalView),
        (status = 403, description = "not a permitted approver"),
        (status = 404, description = "not in scope"),
        (status = 409, description = "proposal already decided")
    )
)]
pub async fn proposals_approve(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DecideProposalRequest>,
) -> ApiResult<Json<AgentProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let view = decide_and_maybe_apply(&state, &authed, id, "approved", &req.note).await?;
    Ok(Json(view))
}

/// POST /api/agent-proposals/{id}/decline — decide AGAINST. A declined
/// proposal produces NO write at all; the row is the audit record.
#[utoipa::path(
    operation_id = "agent_proposals_decline",
    post,
    path = "/api/agent-proposals/{id}/decline",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = DecideProposalRequest,
    responses(
        (status = 200, body = AgentProposalView),
        (status = 403, description = "not a permitted approver"),
        (status = 404, description = "not in scope"),
        (status = 409, description = "proposal already decided")
    )
)]
pub async fn proposals_decline(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<DecideProposalRequest>,
) -> ApiResult<Json<AgentProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let view = decide_and_maybe_apply(&state, &authed, id, "declined", &req.note).await?;
    Ok(Json(view))
}

/// POST /api/agent-proposals/{id}/withdraw — the proposing agent (or an
/// admin) cancels its own open proposal. No write ever happened.
#[utoipa::path(
    operation_id = "agent_proposals_withdraw",
    post,
    path = "/api/agent-proposals/{id}/withdraw",
    tag = "agent-platform",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = AgentProposalView),
        (status = 403, description = "only the proposing agent or an admin may withdraw"),
        (status = 404, description = "not in scope"),
        (status = 409, description = "proposal already decided")
    )
)]
pub async fn proposals_withdraw(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AgentProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let caller_ref = machine_ref(&authed);

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let current = agent_platform::get_proposal(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        // Only the proposing agent (same machine identity or same user)
        // or an owner/admin may withdraw.
        let is_proposer = current.agent_ref == caller_ref || current.for_user == Some(ctx.user_id);
        if !is_proposer && !ctx_can_admin(&authed) {
            return Err(ApiError::Forbidden(
                "only the proposing agent or an admin may withdraw".into(),
            ));
        }
        let row = agent_platform::decide_proposal(
            &mut ctx.tx,
            ctx.org_id,
            id,
            "withdrawn",
            ctx.user_id,
            None,
        )
        .await?
        .ok_or(ApiError::Conflict("proposal already decided".into()))?;
        audit(
            &mut ctx.tx,
            ctx.org_id,
            ctx.user_id,
            "agent_proposal.withdrawn",
            &id.to_string(),
            serde_json::json!({ "withdrawn_by": caller_ref }),
        )
        .await?;
        ctx.emit(
            types::AGENT_PROPOSAL_WITHDRAWN,
            "agent_proposal",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "proposal_id": id,
                "entity": row.entity,
                "withdrawn_by": caller_ref,
            }),
        )
        .await?;
        Ok((ctx, AgentProposalView::from(row)))
    })
    .await?;
    Ok(Json(outcome))
}

// --- decision + application -------------------------------------------------------

/// Shared decide flow: permission, compare-and-set transition, and — for
/// approved ticket updates — the IF1 application.
async fn decide_and_maybe_apply(
    state: &AppState,
    authed: &crate::auth::AuthedUser,
    id: Uuid,
    decision: &str,
    note: &Option<String>,
) -> ApiResult<AgentProposalView> {
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        let current = agent_platform::get_proposal(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        ensure_can_decide(authed, &current)?;

        let row = agent_platform::decide_proposal(
            &mut ctx.tx,
            ctx.org_id,
            id,
            decision,
            ctx.user_id,
            note.as_deref(),
        )
        .await?
        .ok_or(ApiError::Conflict("proposal already decided".into()))?;

        // IF8 application: an APPROVED ticket update lands as a normal
        // autosave write through the IF1 path (idempotent, version-gated,
        // write-trail row) so it stays undoable like any human edit.
        let (view, applied) = if decision == "approved" && row.entity == "ticket" && row.op == "update"
        {
            apply_ticket_proposal(&mut ctx, &authed.principal, &row).await?;
            agent_platform::mark_proposal_applied(&mut ctx.tx, ctx.org_id, id).await?;
            let final_row = agent_platform::get_proposal(&mut *ctx.tx, ctx.org_id, id).await?;
            (AgentProposalView::from(final_row), true)
        } else {
            (AgentProposalView::from(row), false)
        };

        // Provenance audit + contract event ride the SAME transaction as
        // the decision and (when applicable) the IF1 write — approval,
        // application, and provenance commit atomically or not at all.
        audit(
            &mut ctx.tx,
            ctx.org_id,
            ctx.user_id,
            if decision == "approved" {
                "agent_proposal.approved"
            } else {
                "agent_proposal.declined"
            },
            &id.to_string(),
            serde_json::json!({ "decided_by": ctx.user_id, "decision": decision, "applied": applied }),
        )
        .await?;
        if decision == "approved" {
            ctx.emit(
                types::AGENT_PROPOSAL_APPROVED,
                "agent_proposal",
                Some(id.to_string().as_str()),
                &serde_json::json!({
                    "proposal_id": id,
                    "entity": view.entity,
                    "entity_id": view.entity_id,
                    "approved_by": ctx.user_id,
                    "applied": applied,
                }),
            )
            .await?;
        } else {
            ctx.emit(
                types::AGENT_PROPOSAL_DECLINED,
                "agent_proposal",
                Some(id.to_string().as_str()),
                &serde_json::json!({
                    "proposal_id": id,
                    "entity": view.entity,
                    "declined_by": ctx.user_id,
                }),
            )
            .await?;
        }
        Ok((ctx, view))
    })
    .await?;
    Ok(outcome)
}

/// Applies an approved ticket-field proposal through the instant-first
/// write path. The proposal snapshot is the DECISION record; application
/// re-reads the live entity, so a proposal made against a drifted record
/// still lands exactly what was approved (fields merge on top of the
/// current state — nothing blind is replayed).
async fn apply_ticket_proposal(
    ctx: &mut org_tx::OrgTx<'_>,
    principal: &core_domain::entities::Principal,
    proposal: &AgentProposalRow,
) -> ApiResult<()> {
    use crate::routes::tickets::PatchTicketRequest;

    let entity_id = proposal
        .entity_id
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("approved ticket proposal has no id")))?;

    let before = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, entity_id)
        .await
        .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;

    // The approved change rides the SAME handler machinery as a human
    // autosave: patch shape validated, board RBAC re-checked, version
    // gate enforced, write-trail row appended.
    let req: PatchTicketRequest =
        serde_json::from_value(proposal.proposed_value.clone()).map_err(|e| {
            ApiError::Validation(format!(
                "approved proposal is not a valid ticket patch: {e}"
            ))
        })?;
    authorize_board_op(&mut ctx.tx, principal, before.board_id, BoardOp::WorkTicket).await?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let idempotency_key = format!("agent-proposal-{}", proposal.id);
    if let InstantClaim::Replay(_) = instant::claim(
        ctx,
        instant::ClaimSpec {
            entity: "ticket",
            entity_id: Some(entity_id),
            key: Some(&idempotency_key),
            fingerprint: &fingerprint,
        },
    )
    .await?
    {
        // Already applied (crash-retry of this same approval): idempotent.
        return Ok(());
    }
    let write_seq = instant::entity_seq(
        ctx,
        instant::SeqSpec {
            entity: "ticket",
            entity_id,
            version: None,
        },
    )
    .await?;
    core_db::tickets::update_ticket(
        &mut *ctx.tx,
        ctx.org_id,
        entity_id,
        core_db::tickets::TicketPatch {
            title: req.title.as_deref(),
            body: req.body.as_deref().map(Some),
            priority: req.priority,
            assignee_id: req.assignee_id,
            client_id: req.client_id,
            site_id: req.site_id,
            contact_id: req.contact_id,
            due_at: req.due_at,

            type_id: None,
            category_id: None,
        },
    )
    .await?;
    let after = core_db::tickets::get_ticket(&mut *ctx.tx, ctx.org_id, entity_id).await?;
    ctx.emit(
        types::TICKET_UPDATED,
        "ticket",
        Some(after.id.to_string().as_str()),
        &serde_json::json!({
            "ticket_id": after.id,
            "board_id": after.board_id,
            "assignee_id": after.assignee_id,
        }),
    )
    .await?;
    let _mutation = instant::record(
        ctx,
        instant::RecordSpec {
            entity: "ticket",
            entity_id,
            op: "update",
            seq: write_seq,
            before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
            after: serde_json::to_value(&after).unwrap_or(Value::Null),
            updated_at: after.updated_at,
            key: Some(&idempotency_key),
            // IF8 approve-apply is a human decision write; agent linkage
            // lives on the proposal row, not on this trail entry.
            provenance: Default::default(),
            payload: &after,
        },
    )
    .await?;
    Ok(())
}

// --- helpers ----------------------------------------------------------------------

const ENTITY_PROPOSAL: &str = "agent_proposal";

async fn audit(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    actor_id: Uuid,
    action: &str,
    entity_id: &str,
    detail: Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        conn,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(actor_id),
            action: action.into(),
            entity_type: Some(ENTITY_PROPOSAL.into()),
            entity_id: Some(entity_id.to_string()),
            detail,
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    Ok(())
}

/// The decision plane: owner/admin, the proposal's for-user, or (for
/// proposals filed on the human's own records) that user. Dial position
/// does NOT change who may approve — it changes whether approval is
/// required for the WRITE-EXECUTION lanes; on this surface a `proposed`
/// proposal must always be decided by a human regardless.
fn ensure_can_decide(
    authed: &crate::auth::AuthedUser,
    proposal: &AgentProposalRow,
) -> ApiResult<()> {
    // A workflow gate is the one proposal that names its own approvers, and
    // that list is the authority: the document's author chose who may open
    // THIS gate, so an owner/admin the gate did not name is not entitled, and
    // a named technician is — without an owner having to decide for them.
    // A gate row whose approver list will not parse entitles nobody, which is
    // the fail-closed reading: the run expires instead.
    if proposal.entity == workflow_approvals::GATE_ENTITY {
        let approvers = workflow_approvals::GateRequest::approvers_of(&proposal.proposed_value);
        let role_keys: Vec<String> = authed
            .principal
            .roles
            .iter()
            .filter_map(|role| role.builtin.map(|builtin| builtin.as_str().to_string()))
            .collect();
        let role_names: Vec<String> = authed
            .principal
            .roles
            .iter()
            .map(|role| role.name.clone())
            .collect();
        if workflow_approvals::entitled(
            &approvers,
            authed.principal.user.id,
            &role_keys,
            &role_names,
        ) {
            return Ok(());
        }
        return Err(ApiError::Forbidden(
            "this workflow gate names its own approvers; you are not one of them".into(),
        ));
    }
    if ctx_can_admin(authed) {
        return Ok(());
    }
    if proposal.for_user == Some(authed.principal.user.id) {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "only the proposal's owner or an owner/admin may decide it".into(),
    ))
}

fn ctx_can_admin(authed: &crate::auth::AuthedUser) -> bool {
    authed.principal.is_owner_or_admin()
}

/// Stable machine identity for an authenticated caller: the API-key id
/// for machine keys, the user id otherwise (§5 provenance).
fn machine_ref(authed: &crate::auth::AuthedUser) -> String {
    if authed.via_api_key {
        format!("api-key:{}", authed.principal.user.id)
    } else {
        format!("user:{}", authed.principal.user.id)
    }
}

/// Resolves the caller-supplied agent_ref (defaulting to the machine
/// identity), pinning its kind so provenance is always attributable.
fn normalize_agent_ref(
    authed: &crate::auth::AuthedUser,
    supplied: Option<&str>,
) -> ApiResult<String> {
    match supplied.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) if r.len() <= 200 => Ok(r.to_string()),
        Some(_) => Err(ApiError::Validation(
            "agent_ref must be at most 200 characters".into(),
        )),
        None => Ok(machine_ref(authed)),
    }
}

/// `<namespace>:read` | `<namespace>:write` — the capability slug grammar
/// (mirrors migration 0040's CHECK).
fn validate_capability(capability: &str) -> ApiResult<()> {
    // Row A29: the registry may only express what the derived catalog can
    // serve. Before this, any plausible namespace was accepted, so an admin
    // could grant `invoices:read`, watch the tool appear in `tools/list`, and
    // then see it 401 two layers down.
    if crate::mcp_catalog::is_grantable(capability) {
        return Ok(());
    }
    Err(ApiError::Validation(format!(
        "capability {capability:?} is not in the MCP catalog; grantable capabilities: {}",
        crate::mcp_catalog::families()
            .iter()
            .map(|f| f.capability.as_str())
            .chain(
                crate::mcp_catalog::sensitive_capabilities()
                    .iter()
                    .map(|s| s.capability.as_str())
            )
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn validate_slug(slug: &str) -> ApiResult<()> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid {
        return Err(ApiError::Validation(
            "slug must be lowercase alphanumeric/underscore/dash, 1..=64 chars".into(),
        ));
    }
    Ok(())
}

/// Manifest shape validation (§4): an object whose members, when present,
/// are the declared arrays. Unknown members are tolerated (additive).
fn validate_manifest(manifest: &Value) -> ApiResult<()> {
    let Some(obj) = manifest.as_object() else {
        return Err(ApiError::Validation("manifest must be an object".into()));
    };
    for key in ["capabilities", "prompts", "rules"] {
        if let Some(v) = obj.get(key) {
            if !v.is_array() {
                return Err(ApiError::Validation(format!(
                    "manifest.{key} must be an array"
                )));
            }
        }
    }
    Ok(())
}

fn decode_cursor(raw: &str) -> Option<(Option<DateTime<Utc>>, Option<Uuid>)> {
    let decoded = B64.decode(raw.as_bytes()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (ts, id) = text.split_once('|')?;
    Some((ts.parse().ok(), id.parse().ok()))
}

fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> String {
    B64.encode(format!("{}|{id}", created_at.to_rfc3339()).as_bytes())
}

// --- router -----------------------------------------------------------------------

/// The agent-platform slice of the `/api` surface, built per generation.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/mcp/capabilities"),
            get(capabilities_list).put(capabilities_put),
        )
        .route(
            &format!("{base}/mcp/capabilities/{{capability}}"),
            axum::routing::delete(capabilities_delete),
        )
        .route(
            &format!("{base}/agent-skills"),
            get(skills_list).post(skills_publish),
        )
        .route(&format!("{base}/agent-skills/{{id}}"), get(skills_get))
        .route(
            &format!("{base}/agent-proposals"),
            get(proposals_list).post(proposals_create),
        )
        .route(
            &format!("{base}/agent-proposals/{{id}}"),
            get(proposals_get),
        )
        .route(
            &format!("{base}/agent-proposals/{{id}}/approve"),
            post(proposals_approve),
        )
        .route(
            &format!("{base}/agent-proposals/{{id}}/decline"),
            post(proposals_decline),
        )
        .route(
            &format!("{base}/agent-proposals/{{id}}/withdraw"),
            post(proposals_withdraw),
        )
}
