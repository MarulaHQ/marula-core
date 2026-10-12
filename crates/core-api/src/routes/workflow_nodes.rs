//! Workflow node registry reads (Wave 43 lane 13, `p43-node-registry`).
//!
//! The node catalogue is the single source of truth the whole workflow
//! foundation derives from: the canvas palette, the schema-driven
//! properties panel, the API types, the plugin-action picker and the LLM
//! tool catalogue all read these declarations, and there is no second
//! registry in TypeScript. So these endpoints are reads over DECLARATIONS
//! — Rust-owned, no org data rides in them, and any authed org member may
//! list them, exactly as the plugin surface reads posture.
//!
//! Two families:
//!   * the first-party catalogue, filtered by audience (the palette a
//!     human sees, or the tool catalogue the authoring model sees) and by
//!     tier, with each node's schemas available on demand — the
//!     `get_runnable_details` pattern, so a prompt never grows with the
//!     catalogue;
//!   * the org's plugin-contributed nodes (Tier C), which only the
//!     database can hold because a manifest declares them.
//!
//! Posture: nothing here executes anything. A declaration is data (§4.8).

use crate::auth::middleware::authed;
use crate::error::ApiResult;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::Extensions;
use axum::Json;
use core_domain::node_registry::{
    self, NodeAudience, NodeFamily, NodeTier, NodeType, PluginNodeContract,
};
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

/// One node as the web and the SDK consume it. Schemas ride as JSON so a
/// properties panel renders from them directly instead of hand-writing a
/// form per node.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NodeView {
    pub id: String,
    pub label: String,
    pub agent_description: String,
    pub family: NodeFamily,
    pub tier: NodeTier,
    pub idempotent: bool,
    pub audience: NodeAudience,
    /// The executor path this node rides: a closed domain operation for a
    /// Tier A wrapper, the generic path for Tier B, the plugin surface
    /// kind for Tier C.
    #[schema(value_type = String)]
    pub executor: String,
    #[schema(value_type = Object)]
    pub input_schema: serde_json::Value,
    #[schema(value_type = Object)]
    pub output_schema: serde_json::Value,
    /// The event types this node can produce, the seam the static loop
    /// refusal reads.
    pub emits: Vec<String>,
}

impl From<&NodeType> for NodeView {
    fn from(n: &NodeType) -> Self {
        Self {
            id: n.id.to_string(),
            label: n.label.to_string(),
            agent_description: n.agent_description.to_string(),
            family: n.family,
            tier: n.tier,
            idempotent: n.idempotent,
            audience: n.audience,
            executor: n.executor.as_str().to_string(),
            input_schema: node_registry::input_schema(n.id)
                .unwrap_or_else(|| serde_json::json!({})),
            output_schema: node_registry::output_schema(n.id)
                .unwrap_or_else(|| serde_json::json!({})),
            emits: n.emits.iter().map(|e| e.to_string()).collect(),
        }
    }
}

/// The catalogue read: version, digest, and the nodes the caller asked
/// for. The version and digest let a saved document say which release it
/// was validated against, and make two builds claiming the same release
/// with different declarations detectable.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NodeCatalogueView {
    pub version: String,
    pub digest: String,
    pub node_count: usize,
    pub nodes: Vec<NodeView>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct NodeListQuery {
    /// Which audience's slice to return: `human` (the palette) or `ai`
    /// (the LLM tool catalogue). Defaults to `human`.
    #[serde(default)]
    pub audience: Option<String>,
    /// Restrict to one tier: `a`, `b` or `c`. Defaults to every tier.
    #[serde(default)]
    pub tier: Option<String>,
    /// Restrict to one family: `trigger`, `decision`, `condition`,
    /// `action`, `control` or `value`.
    #[serde(default)]
    pub family: Option<String>,
}

fn parse_audience(value: Option<&str>) -> NodeAudience {
    match value {
        Some("ai") => NodeAudience::Ai,
        Some("both") => NodeAudience::Both,
        _ => NodeAudience::Human,
    }
}

fn parse_tier(value: Option<&str>) -> Option<NodeTier> {
    match value {
        Some("a") => Some(NodeTier::A),
        Some("b") => Some(NodeTier::B),
        Some("c") => Some(NodeTier::C),
        _ => None,
    }
}

fn parse_family(value: Option<&str>) -> Option<NodeFamily> {
    match value {
        Some("trigger") => Some(NodeFamily::Trigger),
        Some("decision") => Some(NodeFamily::Decision),
        Some("condition") => Some(NodeFamily::Condition),
        Some("action") => Some(NodeFamily::Action),
        Some("control") => Some(NodeFamily::Control),
        Some("value") => Some(NodeFamily::Value),
        _ => None,
    }
}

/// GET /api/workflows/nodes — the catalogue, filtered. Declarations carry
/// no org data, so any authed org member may read; each consuming surface
/// keeps its own data gates.
#[utoipa::path(operation_id = "workflow_nodes_list", get, path = "/api/workflows/nodes", tag = "workflows", security(("bearer" = [])),
    params(NodeListQuery),
    responses((status = 200, body = NodeCatalogueView)))]
pub async fn list_nodes(
    State(_state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<NodeListQuery>,
) -> ApiResult<Json<NodeCatalogueView>> {
    authed::<crate::auth::AuthedUser>(&extensions)?;
    let audience = parse_audience(q.audience.as_deref());
    let tier = parse_tier(q.tier.as_deref());
    let family = parse_family(q.family.as_deref());

    let nodes: Vec<NodeView> = node_registry::all()
        .iter()
        .filter(|n| match audience {
            NodeAudience::Human => n.audience.includes_human(),
            NodeAudience::Ai => n.audience.includes_ai(),
            NodeAudience::Both => true,
        })
        .filter(|n| tier.is_none_or(|t| n.tier == t))
        .filter(|n| family.is_none_or(|f| n.family == f))
        .map(NodeView::from)
        .collect();

    Ok(Json(NodeCatalogueView {
        version: node_registry::CATALOGUE_VERSION.to_string(),
        digest: node_registry::catalogue_digest(),
        node_count: nodes.len(),
        nodes,
    }))
}

/// GET /api/workflows/nodes/{id} — one node with both schemas parsed.
/// The on-demand schema leg the authoring model uses, so exposing the
/// catalogue to a prompt costs one call per node it actually needs rather
/// than the whole catalogue inlined.
#[utoipa::path(operation_id = "workflow_nodes_get", get, path = "/api/workflows/nodes/{id}", tag = "workflows", security(("bearer" = [])),
    params(("id" = String, Path, description = "Node id, e.g. ticket.create")),
    responses((status = 200, body = NodeView)))]
pub async fn get_node(
    State(_state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<String>,
) -> ApiResult<Json<NodeView>> {
    authed::<crate::auth::AuthedUser>(&extensions)?;
    let node = node_registry::require(&id)?;
    Ok(Json(NodeView::from(node)))
}

/// One plugin-contributed node as the palette and the picker consume it.
/// Only live entries are listed — a revoked install's rows are gone, not
/// present-and-broken — and the entry carries its install so the canvas
/// can say whose node it is.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PluginNodeView {
    pub node_id: String,
    /// `action`, `read` or `trigger`. A `trigger` is an EVENT TYPE the
    /// connector declares through the signed ingest, not a node — the
    /// trigger catalogue makes it subscribable.
    pub kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub install_id: Uuid,
    pub label: String,
    pub agent_description: String,
    pub audience: NodeAudience,
    pub idempotent: bool,
    pub emits: Vec<String>,
    #[schema(value_type = Option<Object>, nullable = true)]
    pub input_schema: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>, nullable = true)]
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PluginNodeListView {
    pub nodes: Vec<PluginNodeView>,
    /// The event types the org's plugins declare as triggers. They belong
    /// to the trigger catalogue, not to this list, because a plugin
    /// trigger arrives through the signed ingest and needs no surface
    /// node at all.
    pub declared_triggers: Vec<String>,
}

/// GET /api/workflows/plugin-nodes — the org's plugin-contributed
/// workflow nodes, parsed through the same contract the registry uses.
#[utoipa::path(operation_id = "workflow_plugin_nodes", get, path = "/api/workflows/plugin-nodes", tag = "workflows", security(("bearer" = [])),
    responses((status = 200, body = PluginNodeListView)))]
pub async fn list_plugin_nodes(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<PluginNodeListView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(crate::error::ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;

    let rows = core_db::plugin_surface::list_node_entries(&mut *tx, org_id).await?;
    let triggers = core_db::event_types::list_org_plugin_event_types(&mut *tx, org_id).await?;

    tx.commit().await.map_err(crate::error::ApiError::from)?;

    // Each row parses through the registry's own contract rather than a
    // parallel shape, so a plugin node and a first-party node answer the
    // same questions. A row that fails to parse is dropped from the
    // picker rather than rendered half-declared: the surface must not show
    // a node that cannot run.
    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let contract = match PluginNodeContract::parse(
            &row.surface,
            &row.entry_key,
            &row.label,
            &row.definition,
            row.output_schema.clone(),
            row.idempotent,
            &row.emits,
            &row.audience,
            row.agent_description.as_deref(),
        ) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let declared = contract.as_node_type();
        nodes.push(PluginNodeView {
            node_id: contract.node_id.clone(),
            kind: contract.kind.as_str().to_string(),
            install_id: row.install_id,
            label: declared.label.to_string(),
            agent_description: declared.agent_description.to_string(),
            audience: contract.audience,
            idempotent: contract.idempotent,
            emits: contract.emits.clone(),
            input_schema: contract.input_schema.clone(),
            output_schema: contract.output_schema.clone(),
        });
    }

    Ok(Json(PluginNodeListView {
        nodes,
        declared_triggers: triggers,
    }))
}
