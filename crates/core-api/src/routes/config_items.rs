//! Configuration-identity endpoints (ADR 0003, Wave 12 lane 3): the CI
//! facets of the asset surface. The canonical `ConfigItem` IS a
//! `device_assets` row (no parallel registry); `config_source_links` is the
//! ONLY feed write path (lane 8's seam), the merge queue is
//! visible-evidence + audited + undoable, and curated fields are sticky.
//!
//! Envelope: every mutation rides IF1–IF3 (claim → write → seq → emit →
//! record); events register as `config_item.*`.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use core_db::assets::get_asset;
use core_db::config_identity::{FeedField, FeedUpsert, FeedUpsertResult};
use core_domain::entities::config_identity::{
    CiInterface, CiMerge, CiMergeCandidate, CiMergeStatus, CiSemanticClass, ConfigSourceLink,
};
use core_domain::entities::DeviceAsset;
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn ci(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Clients,
        action,
    }
}

// ---- request/response shapes -----------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema, utoipa::IntoParams)]
pub struct ListConfigItemsQuery {
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    /// CI6 semantic class filter (workstation | server | vm |
    /// network_appliance | printer | iot).
    #[serde(default)]
    pub semantic_class: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConfigItemView {
    #[serde(flatten)]
    pub item: DeviceAsset,
    /// The layout's CI6 semantic class (None = untyped).
    pub semantic_class: Option<String>,
    #[serde(default)]
    pub source_links: Vec<ConfigSourceLink>,
    #[serde(default)]
    pub interfaces: Vec<CiInterface>,
}

/// Instant-first mutation response (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConfigItemMutationResponse {
    #[serde(flatten)]
    pub item: DeviceAsset,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CurateFieldsRequest {
    /// Per-field human values; a `null` value un-curates the field.
    pub fields: Value,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AcceptObservedRequest {
    pub field_key: String,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The feeder upsert input — lane 8's seam (NinjaOne / IT Glue / Auvik).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct FeedUpsertRequest {
    pub source_system: String,
    pub external_id: String,
    #[serde(default)]
    pub source_guid: Option<String>,
    #[serde(default)]
    pub payload_ref: Option<String>,
    pub client_id: Uuid,
    /// Display defaults for a freshly created canonical item.
    pub tag: String,
    pub name: String,
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    /// CI3 deterministic identity signals (source GUID, serial, MACs, ...).
    #[serde(default)]
    pub identity_keys: Value,
    /// CI5 observed tier: feed-written, churns with its source.
    #[serde(default)]
    pub observed_fields: Value,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): item fields flattened at the top
/// level plus the additive feed-sync evidence and `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FeedUpsertResponse {
    #[serde(flatten)]
    pub item: DeviceAsset,
    #[serde(default)]
    pub source_link: ConfigSourceLink,
    /// A fresh canonical row was created by THIS sync (no twin risk).
    #[serde(default)]
    pub created_item: bool,
    /// An existing item was deterministically resolved through identity
    /// keys; no second row exists for this source record.
    #[serde(default)]
    pub resolved_existing: bool,
    /// Auto-merged at/above the org confidence threshold.
    #[serde(default)]
    pub auto_merged: bool,
    /// Reviewable candidates queued for a human (visible evidence).
    #[serde(default)]
    pub queued_candidates: Vec<core_domain::entities::CiMergeCandidate>,
    /// Observed values that contradict a curated value (never applied).
    #[serde(default)]
    pub contradictions: Vec<String>,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CandidateView {
    #[serde(flatten)]
    pub candidate: CiMergeCandidate,
    pub primary_tag: String,
    pub candidate_tag: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, utoipa::IntoParams)]
pub struct ListCandidatesQuery {
    /// pending | auto_merged | accepted | rejected
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[schema(as = CiMergeRequest)]
pub struct MergeRequest {
    /// The item to absorb into `{id}`.
    pub merged_config_item_id: Uuid,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub candidate_id: Option<Uuid>,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateInterfaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// Normalized to lower colon form server-side.
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default)]
    pub ips: Vec<String>,
    #[serde(default)]
    pub vlans: Vec<String>,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateSettingsRequest {
    /// Auto-merge only at/above this confidence (0..1).
    pub auto_merge_threshold: f64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CiSettingsView {
    pub auto_merge_threshold: f64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpsertPrecedenceRequest {
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    #[serde(default)]
    pub semantic_class: Option<String>,
    pub field_key: String,
    pub source_system: String,
    pub priority: i16,
}

// ---- helpers ---------------------------------------------------------------

async fn load_view(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<ConfigItemView> {
    let item = get_asset(&mut *conn, org_id, id).await?;
    let semantic_class =
        core_db::config_identity::semantic_class_for_item(&mut *conn, org_id, &item).await?;
    let source_links =
        core_db::config_identity::list_links_for_item(&mut *conn, org_id, id).await?;
    let interfaces = core_db::config_identity::list_interfaces(&mut *conn, org_id, id).await?;
    Ok(ConfigItemView {
        item,
        semantic_class,
        source_links,
        interfaces,
    })
}

fn parse_semantic_class(s: &Option<String>) -> ApiResult<Option<CiSemanticClass>> {
    match s.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(v) => Ok(Some(CiSemanticClass::parse(v).ok_or_else(|| {
            ApiError::Validation(format!("unknown semantic class '{v}'"))
        })?)),
    }
}

// ---- reads -----------------------------------------------------------------

/// GET /api/config-items — active items only (merged tombstones excluded).
#[utoipa::path(operation_id = "config_items_list", get, path = "/api/config-items", tag = "config-items", security(("bearer" = [])),
    params(ListConfigItemsQuery),
    responses((status = 200, body = [ConfigItemView])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListConfigItemsQuery>,
) -> ApiResult<Json<Vec<ConfigItemView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let semantic_class = parse_semantic_class(&q.semantic_class)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::config_identity::list_config_items(
        &mut tx,
        org_id,
        q.client_id,
        q.layout_id,
        semantic_class.as_ref().map(|c| c.as_str()),
        q.q.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|item| ConfigItemView {
                semantic_class: None,
                source_links: Vec::new(),
                interfaces: Vec::new(),
                item,
            })
            .collect(),
    ))
}

/// GET /api/config-items/{id} — the CI with its source links + interfaces.
#[utoipa::path(operation_id = "config_items_get", get, path = "/api/config-items/{id}", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ConfigItemView),
              (status = 404, description = "config item not visible in scope")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ConfigItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let view = load_view(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// GET /api/config-items/{id}/merges — the item's merge audit trail.
#[utoipa::path(operation_id = "config_items_list_merges", get, path = "/api/config-items/{id}/merges", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [CiMerge])))]
pub async fn list_merges(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<CiMerge>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    get_asset(&mut *tx, org_id, id).await?;
    let merges = core_db::config_identity::list_merges_for_item(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(merges))
}

/// GET /api/config-items/precedence — org overrides (shipped ADR defaults
/// resolve in Rust when no row matches).
#[utoipa::path(operation_id = "config_items_list_precedence", get, path = "/api/config-items/precedence", tag = "config-items", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::CiFieldPrecedence])))]
pub async fn list_precedence(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::CiFieldPrecedence>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::config_identity::list_precedence(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// GET /api/config-items/settings — the org's auto-merge threshold.
#[utoipa::path(operation_id = "config_items_get_settings", get, path = "/api/config-items/settings", tag = "config-items", security(("bearer" = [])),
    responses((status = 200, body = CiSettingsView)))]
pub async fn get_settings(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<CiSettingsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let threshold = core_db::config_identity::auto_merge_threshold(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(CiSettingsView {
        auto_merge_threshold: threshold,
    }))
}

// ---- writes (IF1–IF3) ------------------------------------------------------

/// GET /api/config-items/merge-candidates — the review queue.
#[utoipa::path(operation_id = "config_items_list_candidates", get, path = "/api/config-items/merge-candidates", tag = "config-items", security(("bearer" = [])),
    params(ListCandidatesQuery),
    responses((status = 200, body = [CandidateView])))]
pub async fn list_candidates(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListCandidatesQuery>,
) -> ApiResult<Json<Vec<CandidateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let candidates =
        core_db::config_identity::list_candidates(&mut tx, org_id, q.status.as_deref()).await?;
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        let primary_tag = get_asset(&mut *tx, org_id, c.primary_config_item_id)
            .await
            .map(|a| a.tag)
            .unwrap_or_default();
        let candidate_tag = get_asset(&mut *tx, org_id, c.candidate_config_item_id)
            .await
            .map(|a| a.tag)
            .unwrap_or_default();
        out.push(CandidateView {
            candidate: c,
            primary_tag,
            candidate_tag,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(out))
}

/// POST /api/config-items/feed-upsert — CI2: the single feeder write path
/// (lane 8's seam). Never blind-inserts: existing (source_system,
/// external_id) links resolve to their own item; identity keys resolve the
/// deterministic case; ambiguity queues reviewable candidates and only
/// auto-merges at/above the org threshold.
#[utoipa::path(operation_id = "config_items_feed_upsert", post, path = "/api/config-items/feed-upsert", tag = "config-items", security(("bearer" = [])),
    request_body = FeedUpsertRequest,
    responses((status = 200, body = FeedUpsertResponse),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "source system / external id validation")))]
pub async fn feed_upsert(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<FeedUpsertRequest>,
) -> ApiResult<Json<FeedUpsertResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let source_system = req.source_system.trim().to_ascii_lowercase();
    let external_id = req.external_id.trim().to_string();
    if source_system.is_empty() || external_id.is_empty() {
        return Err(ApiError::Validation(
            "source_system and external_id are required".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let observed_fields: Vec<FeedField> = req
        .observed_fields
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| FeedField {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let input = FeedUpsert {
        source_system: source_system.clone(),
        external_id: external_id.clone(),
        source_guid: req.source_guid.clone(),
        payload_ref: req.payload_ref.clone(),
        client_id: req.client_id,
        tag: req.tag.trim().to_string(),
        name: req.name.trim().to_string(),
        layout_id: req.layout_id,
        identity_keys: req.identity_keys.clone(),
        observed_fields,
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "config_item",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        // Per-connector gate (Wave 15 lane 4): the org-level
        // `connectors.*` toggles decide whether the feeder source may
        // write at all here — an org that turned a connector off gets a
        // typed 403, and an instant-first replay of a request that WAS
        // accepted while the connector was on stays replayable.
        let overrides = core_db::settings::list_org_overrides(&mut *ctx.tx, org_id).await?;
        let rows: Vec<(String, serde_json::Value)> =
            overrides.into_iter().map(|o| (o.key, o.value)).collect();
        if !core_domain::settings::connector_enabled(&source_system, &rows) {
            return Err(ApiError::Forbidden(format!(
                "connector for source system `{source_system}` is disabled for this org \
                 (settings key: {})",
                core_domain::settings::connector_key(&source_system).unwrap_or("connectors.*"),
            )));
        }

        let result: FeedUpsertResult =
            core_db::config_identity::feed_upsert(&mut ctx.tx, org_id, &input).await?;
        let item_id = result.item.id;

        // Event emission: created vs updated-on-adopt.
        if result.created_item {
            ctx.emit(
                types::CONFIG_ITEM_CREATED,
                "config_item",
                Some(item_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ConfigItemCreated {
                    config_item_id: item_id,
                    org_id,
                    client_id: req.client_id,
                    source_link_id: result.link.id,
                    origin: "feed".into(),
                    source_system: source_system.clone(),
                    occurred_at: chrono::Utc::now(),
                })
                .unwrap_or_default(),
            )
            .await?;
        } else {
            ctx.emit(
                types::CONFIG_ITEM_UPDATED,
                "config_item",
                Some(item_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ConfigItemUpdated {
                    config_item_id: item_id,
                    org_id,
                    source_link_id: result.link.id,
                    change: if result.resolved_existing {
                        "adopted_existing".into()
                    } else {
                        "feed".into()
                    },
                    fields_changed: Vec::new(),
                    contradictions: result.contradictions.clone(),
                    occurred_at: chrono::Utc::now(),
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "config_item",
                entity_id: item_id,
                version: req.instant.version,
            },
        )
        .await?;
        let response_item = get_asset(&mut *ctx.tx, org_id, item_id).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "config_item",
                entity_id: item_id,
                op: "feed_upsert",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&response_item).unwrap_or(Value::Null),
                updated_at: response_item.updated_at,
                key: key.as_deref(),
                payload: &response_item,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(FeedUpsertResponse {
                item: response_item,
                source_link: result.link,
                created_item: result.created_item,
                resolved_existing: result.resolved_existing,
                auto_merged: result.auto_merged,
                queued_candidates: result.queued_candidates,
                contradictions: result.contradictions,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/config-items/{id} — human curation (CI5): curated values are
/// sticky forever; feeds never auto-overwrite them afterwards.
#[utoipa::path(operation_id = "config_items_curate", patch, path = "/api/config-items/{id}", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CurateFieldsRequest,
    responses((status = 200, body = ConfigItemMutationResponse),
              (status = 404, description = "config item not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn curate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CurateFieldsRequest>,
) -> ApiResult<Json<ConfigItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "config_item",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let changed = core_db::config_identity::curate_fields(&mut ctx.tx, org_id, id, &req.fields)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "config_item",
                entity_id: id,
                version,
            },
        )
        .await?;
        let curated_keys: Vec<String> = req
            .fields
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        ctx.emit(
            types::CONFIG_ITEM_UPDATED,
            "config_item",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemUpdated {
                config_item_id: id,
                org_id,
                source_link_id: Uuid::nil(),
                change: "curated".into(),
                fields_changed: curated_keys.clone(),
                contradictions: Vec::new(),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "config_item.curated".into(),
                entity_type: Some("config_item".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "fields": curated_keys }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "config_item",
                entity_id: id,
                op: "curate",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&changed).unwrap_or(Value::Null),
                updated_at: changed.updated_at,
                key: key.as_deref(),
                payload: &changed,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ConfigItemMutationResponse {
                item: changed,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/config-items/{id}/accept-observed — CI5 one-click "accept the
/// new value": promotes the observed value into the sticky curated tier.
#[utoipa::path(operation_id = "config_items_accept_observed", post, path = "/api/config-items/{id}/accept-observed", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AcceptObservedRequest,
    responses((status = 200, body = ConfigItemMutationResponse),
              (status = 404, description = "config item not visible in scope"),
              (status = 409, description = "no observed value for that field / version conflict")))]
pub async fn accept_observed(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AcceptObservedRequest>,
) -> ApiResult<Json<ConfigItemMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;
    let field_key = req.field_key.trim().to_string();
    if field_key.is_empty() {
        return Err(ApiError::Validation("field_key is required".into()));
    }

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "config_item",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let item =
            core_db::config_identity::accept_observed_value(&mut ctx.tx, org_id, id, &field_key)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "config_item",
                entity_id: id,
                version,
            },
        )
        .await?;
        ctx.emit(
            types::CONFIG_ITEM_UPDATED,
            "config_item",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemUpdated {
                config_item_id: id,
                org_id,
                source_link_id: Uuid::nil(),
                change: "accept_observed".into(),
                fields_changed: vec![field_key.clone()],
                contradictions: Vec::new(),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "config_item.accepted_observed".into(),
                entity_type: Some("config_item".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "field": field_key }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "config_item",
                entity_id: id,
                op: "accept_observed",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&item).unwrap_or(Value::Null),
                updated_at: item.updated_at,
                key: key.as_deref(),
                payload: &item,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ConfigItemMutationResponse { item, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/config-items/{id}/merge — an explicit human merge (the queue's
/// accept path rides this too). Audited + undoable (split).
#[utoipa::path(operation_id = "config_items_merge", post, path = "/api/config-items/{id}/merge", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = MergeRequest,
    responses((status = 200, body = CiMerge),
              (status = 404, description = "config item not visible in scope"),
              (status = 409, description = "cannot merge an item into itself / already merged")))]
pub async fn merge(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MergeRequest>,
) -> ApiResult<Json<CiMerge>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let reason = req
        .reason
        .clone()
        .unwrap_or_else(|| "manual merge".to_string());

    let merge = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        get_asset(&mut *ctx.tx, org_id, req.merged_config_item_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let merge = core_db::config_identity::merge_items(
            &mut ctx.tx,
            org_id,
            id,
            req.merged_config_item_id,
            None,
            None,
            &reason,
            &serde_json::json!({ "origin": "manual" }),
            Some(user_id),
        )
        .await?;
        ctx.emit(
            types::CONFIG_ITEM_MERGED,
            "config_item",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemMerged {
                primary_config_item_id: id,
                merged_config_item_id: req.merged_config_item_id,
                org_id,
                candidate_id: None,
                confidence: None,
                reason: reason.clone(),
                auto: false,
                merged_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "config_item.merged".into(),
                entity_type: Some("config_item".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "merged": req.merged_config_item_id, "reason": reason }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, merge))
    })
    .await?;

    Ok(Json(merge))
}

/// POST /api/config-items/merges/{id}/split — undo a merge (IF7): links,
/// interfaces, and field tiers ride back from the audit row's packet.
#[utoipa::path(operation_id = "config_items_split", post, path = "/api/config-items/merges/{id}/split", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = CiMerge),
              (status = 404, description = "merge not visible in scope"),
              (status = 409, description = "merge already undone")))]
pub async fn split(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CiMerge>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let merge = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::config_identity::get_merge(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let merge = core_db::config_identity::split_merge(&mut ctx.tx, org_id, id, user_id).await?;
        ctx.emit(
            types::CONFIG_ITEM_MERGE_UNDONE,
            "config_item",
            Some(merge.primary_config_item_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemMergeUndone {
                primary_config_item_id: merge.primary_config_item_id,
                restored_config_item_id: merge.merged_config_item_id,
                org_id,
                merge_id: merge.id,
                undone_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "config_item.merge_undone".into(),
                entity_type: Some("config_item".into()),
                entity_id: Some(merge.primary_config_item_id.to_string()),
                detail: serde_json::json!({ "restored": merge.merged_config_item_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, merge))
    })
    .await?;

    Ok(Json(merge))
}

/// POST /api/config-items/merge-candidates/{id}/accept — fold the candidate
/// into the primary with the candidate's evidence attached.
#[utoipa::path(operation_id = "config_items_accept_candidate", post, path = "/api/config-items/merge-candidates/{id}/accept", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = CiMerge),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate already decided")))]
pub async fn accept_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CiMerge>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let merge = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let candidate = core_db::config_identity::get_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let merge = core_db::config_identity::merge_items(
            &mut ctx.tx,
            org_id,
            candidate.primary_config_item_id,
            candidate.candidate_config_item_id,
            Some(candidate.id),
            Some(candidate.confidence),
            &candidate.reason,
            &candidate.evidence,
            Some(user_id),
        )
        .await?;
        core_db::config_identity::decide_candidate(
            &mut ctx.tx,
            org_id,
            candidate.id,
            CiMergeStatus::Accepted.as_str(),
            user_id,
        )
        .await?;
        ctx.emit(
            types::CONFIG_ITEM_MERGED,
            "config_item",
            Some(candidate.primary_config_item_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemMerged {
                primary_config_item_id: candidate.primary_config_item_id,
                merged_config_item_id: candidate.candidate_config_item_id,
                org_id,
                candidate_id: Some(candidate.id),
                confidence: Some(candidate.confidence),
                reason: candidate.reason.clone(),
                auto: false,
                merged_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, merge))
    })
    .await?;

    Ok(Json(merge))
}

/// POST /api/config-items/merge-candidates/{id}/reject — keep both items.
#[utoipa::path(operation_id = "config_items_reject_candidate", post, path = "/api/config-items/merge-candidates/{id}/reject", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = CiMergeCandidate),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate already decided")))]
pub async fn reject_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CiMergeCandidate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let candidate = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::config_identity::get_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let candidate = core_db::config_identity::decide_candidate(
            &mut ctx.tx,
            org_id,
            id,
            CiMergeStatus::Rejected.as_str(),
            user_id,
        )
        .await?;
        Ok((ctx, candidate))
    })
    .await?;

    Ok(Json(candidate))
}

/// POST /api/config-items/{id}/interfaces — record a NIC (ADR 0003
/// lifecycle rule: NICs are attributes, not their own CIs).
#[utoipa::path(operation_id = "config_items_add_interface", post, path = "/api/config-items/{id}/interfaces", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateInterfaceRequest,
    responses((status = 200, body = [CiInterface]),
              (status = 404, description = "config item not visible in scope")))]
pub async fn add_interface(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateInterfaceRequest>,
) -> ApiResult<Json<Vec<CiInterface>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mac = req
        .mac
        .as_deref()
        .map(core_domain::entities::config_identity::normalize_mac);
    let ips = serde_json::to_value(&req.ips).unwrap_or(Value::Null);
    let vlans = serde_json::to_value(&req.vlans).unwrap_or(Value::Null);

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        get_asset(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::config_identity::create_interface(
            &mut ctx.tx,
            org_id,
            Uuid::now_v7(),
            id,
            req.name.as_deref(),
            mac.flatten().as_deref(),
            &ips,
            &vlans,
            req.is_primary,
            None,
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    list_interfaces_response(&state, &extensions, id).await
}

/// DELETE /api/config-items/interfaces/{id}
#[utoipa::path(operation_id = "config_items_delete_interface", delete, path = "/api/config-items/interfaces/{id}", tag = "config-items", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Value)))]
pub async fn delete_interface(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::config_identity::delete_interface(&mut ctx.tx, org_id, id).await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// PUT /api/config-items/precedence — a per-type/per-source precedence
/// override (CI5). Upserts by (layout, field, source).
#[utoipa::path(operation_id = "config_items_upsert_precedence", put, path = "/api/config-items/precedence", tag = "config-items", security(("bearer" = [])),
    request_body = UpsertPrecedenceRequest,
    responses((status = 200, body = core_domain::entities::CiFieldPrecedence),
              (status = 422, description = "unknown semantic class")))]
pub async fn upsert_precedence(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpsertPrecedenceRequest>,
) -> ApiResult<Json<core_domain::entities::CiFieldPrecedence>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let semantic_class = parse_semantic_class(&req.semantic_class)?;
    let field_key = req.field_key.trim().to_string();
    let source_system = req.source_system.trim().to_ascii_lowercase();
    if field_key.is_empty() || source_system.is_empty() {
        return Err(ApiError::Validation(
            "field_key and source_system are required".into(),
        ));
    }

    let row = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row = core_db::config_identity::upsert_precedence(
            &mut ctx.tx,
            org_id,
            Uuid::now_v7(),
            req.layout_id,
            semantic_class.as_ref().map(|c| c.as_str()),
            &field_key,
            &source_system,
            req.priority,
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "config_item.precedence_upserted".into(),
                entity_type: Some("config_item".into()),
                entity_id: Some(row.id.to_string()),
                detail: serde_json::json!({
                    "field_key": field_key,
                    "source_system": source_system,
                    "priority": req.priority
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, row))
    })
    .await?;

    Ok(Json(row))
}

/// PUT /api/config-items/settings — the org's auto-merge confidence
/// threshold (CI4).
#[utoipa::path(operation_id = "config_items_put_settings", put, path = "/api/config-items/settings", tag = "config-items", security(("bearer" = [])),
    request_body = UpdateSettingsRequest,
    responses((status = 200, body = CiSettingsView),
              (status = 422, description = "threshold out of range")))]
pub async fn put_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpdateSettingsRequest>,
) -> ApiResult<Json<CiSettingsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, ci(Action::Edit))?;
    if !(0.0..=1.0).contains(&req.auto_merge_threshold) {
        return Err(ApiError::Validation(
            "auto_merge_threshold must be within 0.0..=1.0".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let threshold = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let t = core_db::config_identity::upsert_settings(
            &mut ctx.tx,
            org_id,
            req.auto_merge_threshold,
        )
        .await?;
        Ok((ctx, t))
    })
    .await?;
    Ok(Json(CiSettingsView {
        auto_merge_threshold: threshold,
    }))
}

async fn list_interfaces_response(
    state: &AppState,
    extensions: &Extensions,
    id: Uuid,
) -> ApiResult<Json<Vec<CiInterface>>> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    require_permission(&authed.principal, ci(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::config_identity::list_interfaces(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

// ---- router ----------------------------------------------------------------

/// The module-owned router (static segments before `{id}`).
pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/config-items"), get(list))
        .route(
            &format!("{base}/config-items/feed-upsert"),
            post(feed_upsert),
        )
        .route(
            &format!("{base}/config-items/merge-candidates"),
            get(list_candidates),
        )
        .route(
            &format!("{base}/config-items/merge-candidates/{{id}}/accept"),
            post(accept_candidate),
        )
        .route(
            &format!("{base}/config-items/merge-candidates/{{id}}/reject"),
            post(reject_candidate),
        )
        .route(
            &format!("{base}/config-items/merges/{{id}}/split"),
            post(split),
        )
        .route(
            &format!("{base}/config-items/interfaces/{{id}}"),
            axum::routing::delete(delete_interface),
        )
        .route(
            &format!("{base}/config-items/precedence"),
            get(list_precedence).put(upsert_precedence),
        )
        .route(
            &format!("{base}/config-items/settings"),
            get(get_settings).put(put_settings),
        )
        .route(
            &format!("{base}/config-items/{{id}}"),
            get(get_one).patch(curate),
        )
        .route(
            &format!("{base}/config-items/{{id}}/accept-observed"),
            post(accept_observed),
        )
        .route(
            &format!("{base}/config-items/{{id}}/interfaces"),
            post(add_interface),
        )
        .route(
            &format!("{base}/config-items/{{id}}/merges"),
            get(list_merges),
        )
        .route(&format!("{base}/config-items/{{id}}/merge"), post(merge))
}
