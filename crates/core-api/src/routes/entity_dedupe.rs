//! Cross-entity dedupe routes (Wave 22 lane 2, ADR 0003 extension): the
//! client/contact line of the merge workbench. Same queue mechanics as
//! the config-items line — same statuses, same `ci_settings`
//! auto-merge threshold, same visible-evidence shape — mounted under
//! `/api/entity-dedupe` plus two ingest paths (`/api/contacts/feed-upsert`
//! for connector contact feeds and `/api/clients/org-reconcile` for
//! unmapped vendor orgs riding the adapters-core org_map pattern).
//!
//! Posture: contact MERGES are audited + undoable (split), but they are
//! audit-only (no domain events) — the `entity_merges` row IS the event
//! record for queue lifecycle actions, the same posture the export paths
//! take. Client reconciliation is NEVER auto-applied: an accept writes
//! the durable `client_org_map` and nothing else moves.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant;
use crate::org_tx;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::Extensions;
use axum::Json;
use core_db::entity_dedupe::{ContactFeedUpsert, ContactFeedUpsertResult};
use core_domain::entities::entity_dedupe::{DedupeEntityKind, EntityMergeStatus};
use core_domain::events as event_types;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;

fn clients_view_permission() -> core_domain::policy::Permission {
    core_domain::policy::Permission::Matrix {
        module: core_domain::policy::Module::Clients,
        action: core_domain::policy::Action::View,
    }
}

fn clients_edit_permission() -> core_domain::policy::Permission {
    core_domain::policy::Permission::Matrix {
        module: core_domain::policy::Module::Clients,
        action: core_domain::policy::Action::Create,
    }
}

// ---- wire shapes -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntityCandidateView {
    #[serde(flatten)]
    pub candidate: core_domain::entities::entity_dedupe::EntityMergeCandidate,
    /// Human labels for the card: contact candidates carry both contact
    /// names; client candidates carry the client name + vendor label.
    pub primary_label: String,
    pub candidate_label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, utoipa::IntoParams)]
pub struct ListEntityCandidatesQuery {
    /// contact | client
    #[serde(default)]
    pub kind: Option<String>,
    /// pending | auto_merged | accepted | rejected | deferred
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityWorkbenchSide {
    #[cfg_attr(feature = "openapi", schema(value_type = Option<serde_json::Value>))]
    pub contact: Option<core_domain::entities::Contact>,
    pub source_links: Vec<core_domain::entities::entity_dedupe::EntitySourceLink>,
    pub observed_fields: serde_json::Value,
    pub curated_fields: serde_json::Value,
    pub field_sources: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityWorkbenchFieldRow {
    pub key: String,
    pub primary_observed: Option<serde_json::Value>,
    pub primary_curated: Option<serde_json::Value>,
    pub primary_source: Option<serde_json::Value>,
    pub candidate_observed: Option<serde_json::Value>,
    pub candidate_curated: Option<serde_json::Value>,
    pub candidate_source: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityWorkbenchDecision {
    pub surface: String,
    pub mode: String,
    pub provider: String,
    pub model: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub answers: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityWorkbenchView {
    pub candidate: EntityCandidateView,
    pub primary: EntityWorkbenchSide,
    pub candidate_side: EntityWorkbenchSide,
    pub fields: Vec<EntityWorkbenchFieldRow>,
    /// The org's auto-merge threshold, surfaced inline (why was this
    /// queued instead of auto-merged).
    pub auto_merge_threshold: f64,
    /// Suggestion-grade `decision_record_dedupe` judgment; `None` (silent
    /// in the UI) whenever the decision model is unconfigured — ADR 0010.
    pub decision: Option<EntityWorkbenchDecision>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ContactFeedUpsertRequest {
    pub source_system: String,
    pub external_id: String,
    #[serde(default)]
    pub source_guid: Option<String>,
    #[serde(default)]
    pub payload_ref: Option<String>,
    pub client_id: uuid::Uuid,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContactFeedUpsertResponse {
    pub contact: core_domain::entities::Contact,
    pub source_link: core_domain::entities::entity_dedupe::EntitySourceLink,
    #[serde(default)]
    pub created_contact: bool,
    #[serde(default)]
    pub resolved_existing: bool,
    #[serde(default)]
    pub auto_merged: bool,
    #[serde(default)]
    pub queued_candidates: Vec<core_domain::entities::entity_dedupe::EntityMergeCandidate>,
    #[serde(default)]
    pub contradictions: Vec<String>,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct OrgReconcileRequest {
    pub source_system: String,
    pub vendor_org_id: String,
    #[serde(default)]
    pub vendor_label: Option<String>,
    /// Instant-first mutation parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrgReconcileResponse {
    pub queued_candidates: Vec<EntityCandidateView>,
    /// Whether the vendor org was ALREADY mapped (idempotent no-op).
    pub already_mapped: bool,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EntityBatchRequest {
    pub ids: Vec<uuid::Uuid>,
    /// accept | reject | defer
    pub action: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityBatchOutcome {
    pub id: uuid::Uuid,
    /// "applied" | "skipped" | "failed"
    pub outcome: String,
    pub merge: Option<core_domain::entities::entity_dedupe::EntityMerge>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EntityBatchResponse {
    pub accepted: usize,
    pub rejected: usize,
    pub deferred: usize,
    pub skipped: usize,
    pub failed: usize,
    pub outcomes: Vec<EntityBatchOutcome>,
}

// ---- reads -------------------------------------------------------------------

/// GET /api/entity-dedupe/candidates — the client/contact review queue.
#[utoipa::path(operation_id = "entity_dedupe_list_candidates", get, path = "/api/entity-dedupe/candidates", tag = "entity-dedupe", security(("bearer" = [])),
    params(ListEntityCandidatesQuery),
    responses((status = 200, body = [EntityCandidateView])))]
pub async fn list_candidates(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListEntityCandidatesQuery>,
) -> ApiResult<Json<Vec<EntityCandidateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_view_permission())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::entity_dedupe::list_entity_candidates(
        &mut tx,
        org_id,
        q.kind.as_deref(),
        q.status.as_deref(),
    )
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for c in rows {
        let primary_label = match c.entity_kind {
            DedupeEntityKind::Contact => {
                core_db::clients::get_contact(&mut *tx, org_id, c.primary_entity_id)
                    .await
                    .map(|ct| ct.name)
                    .unwrap_or_default()
            }
            DedupeEntityKind::Client => {
                core_db::clients::get_client(&mut *tx, org_id, c.primary_entity_id)
                    .await
                    .map(|cl| cl.name)
                    .unwrap_or_default()
            }
        };
        let candidate_label = match (c.entity_kind, c.candidate_entity_id) {
            (DedupeEntityKind::Contact, Some(id)) => Some(
                core_db::clients::get_contact(&mut *tx, org_id, id)
                    .await
                    .map(|ct| ct.name)
                    .unwrap_or_default(),
            ),
            _ => c.vendor_label.clone(),
        };
        out.push(EntityCandidateView {
            candidate: c,
            primary_label,
            candidate_label,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(out))
}

/// GET /api/entity-dedupe/candidates/{id}/workbench — the client/contact
/// side of the guided merge workbench.
#[utoipa::path(operation_id = "entity_dedupe_candidate_workbench", get, path = "/api/entity-dedupe/candidates/{id}/workbench", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = EntityWorkbenchView),
              (status = 404, description = "candidate not visible in scope")))]
pub async fn candidate_workbench(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<EntityWorkbenchView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_view_permission())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;

    let candidate = core_db::entity_dedupe::get_entity_candidate(&mut tx, org_id, id).await?;
    let primary = side_for(&mut tx, org_id, &candidate).await;
    let candidate_side = candidate_side_for(&mut tx, org_id, &candidate).await;

    let keys: Vec<String> = [field_keys(&primary), field_keys(&candidate_side)]
        .concat()
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut fields = Vec::with_capacity(keys.len());
    for key in keys.clone() {
        fields.push(EntityWorkbenchFieldRow {
            primary_observed: primary.observed_fields.get(&key).cloned(),
            primary_curated: primary.curated_fields.get(&key).cloned(),
            primary_source: primary.field_sources.get(&key).cloned(),
            candidate_observed: candidate_side.observed_fields.get(&key).cloned(),
            candidate_curated: candidate_side.curated_fields.get(&key).cloned(),
            candidate_source: candidate_side.field_sources.get(&key).cloned(),
            key,
        });
    }

    let entity_type = "contact_merge_candidate";
    let entity_id = candidate.id;
    let decision = core_db::decision::latest_answers_for_entity(
        &mut *tx,
        org_id,
        "decision_record_dedupe",
        entity_type,
        entity_id,
    )
    .await?
    .map(|row| EntityWorkbenchDecision {
        surface: row.surface,
        mode: row.mode,
        provider: row.provider,
        model: row.model,
        recorded_at: row.created_at,
        answers: row.answers,
    });

    let threshold = core_db::config_identity::auto_merge_threshold(&mut tx, org_id).await?;
    let candidate_view = EntityCandidateView {
        primary_label: primary
            .contact
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_default(),
        candidate_label: candidate_side
            .contact
            .as_ref()
            .map(|c| c.name.clone())
            .or_else(|| candidate.vendor_label.clone()),
        candidate,
    };
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(EntityWorkbenchView {
        candidate: candidate_view,
        primary,
        candidate_side,
        fields,
        auto_merge_threshold: threshold,
        decision,
    }))
}

fn field_keys(side: &EntityWorkbenchSide) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for v in [&side.observed_fields, &side.curated_fields] {
        if let Some(obj) = v.as_object() {
            for k in obj.keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys
}

async fn side_for(
    tx: &mut sqlx::PgConnection,
    org_id: uuid::Uuid,
    candidate: &core_domain::entities::entity_dedupe::EntityMergeCandidate,
) -> EntityWorkbenchSide {
    match candidate.entity_kind {
        DedupeEntityKind::Contact => {
            let contact =
                core_db::clients::get_contact(&mut *tx, org_id, candidate.primary_entity_id)
                    .await
                    .ok();
            let links = core_db::entity_dedupe::list_entity_links(
                &mut *tx,
                org_id,
                DedupeEntityKind::Contact.as_str(),
                candidate.primary_entity_id,
            )
            .await
            .unwrap_or_default();
            EntityWorkbenchSide {
                observed_fields: contact
                    .as_ref()
                    .map(|c| c.observed_fields.clone())
                    .unwrap_or_default(),
                curated_fields: contact
                    .as_ref()
                    .map(|c| c.curated_fields.clone())
                    .unwrap_or_default(),
                field_sources: contact
                    .as_ref()
                    .map(|c| c.field_sources.clone())
                    .unwrap_or_default(),
                contact,
                source_links: links,
            }
        }
        DedupeEntityKind::Client => {
            let links = core_db::entity_dedupe::list_entity_links(
                &mut *tx,
                org_id,
                DedupeEntityKind::Client.as_str(),
                candidate.primary_entity_id,
            )
            .await
            .unwrap_or_default();
            EntityWorkbenchSide {
                observed_fields: serde_json::json!({}),
                curated_fields: serde_json::json!({}),
                field_sources: serde_json::json!({}),
                contact: None,
                source_links: links,
            }
        }
    }
}

async fn candidate_side_for(
    tx: &mut sqlx::PgConnection,
    org_id: uuid::Uuid,
    candidate: &core_domain::entities::entity_dedupe::EntityMergeCandidate,
) -> EntityWorkbenchSide {
    match (candidate.entity_kind, candidate.candidate_entity_id) {
        (DedupeEntityKind::Contact, Some(id)) => {
            let contact = core_db::clients::get_contact(&mut *tx, org_id, id)
                .await
                .ok();
            let links = core_db::entity_dedupe::list_entity_links(
                &mut *tx,
                org_id,
                DedupeEntityKind::Contact.as_str(),
                id,
            )
            .await
            .unwrap_or_default();
            EntityWorkbenchSide {
                observed_fields: contact
                    .as_ref()
                    .map(|c| c.observed_fields.clone())
                    .unwrap_or_default(),
                curated_fields: contact
                    .as_ref()
                    .map(|c| c.curated_fields.clone())
                    .unwrap_or_default(),
                field_sources: contact
                    .as_ref()
                    .map(|c| c.field_sources.clone())
                    .unwrap_or_default(),
                contact,
                source_links: links,
            }
        }
        // Client reconciliation: the candidate side IS the vendor org.
        _ => EntityWorkbenchSide {
            observed_fields: serde_json::json!({
                "vendor_org_id": candidate.vendor_org_id.clone().unwrap_or_default(),
                "vendor_label": candidate.vendor_label.clone().unwrap_or_default(),
            }),
            curated_fields: serde_json::json!({}),
            field_sources: serde_json::json!({}),
            contact: None,
            source_links: Vec::new(),
        },
    }
}

// ---- writes ------------------------------------------------------------------

/// POST /api/contacts/feed-upsert — the contact ingest seam (the CI2
/// pattern generalized): never blind-inserts; email/phone identity keys
/// resolve the deterministic case; ambiguity queues reviewable candidates;
/// the same org auto-merge threshold applies.
#[utoipa::path(operation_id = "contacts_feed_upsert", post, path = "/api/contacts/feed-upsert", tag = "entity-dedupe", security(("bearer" = [])),
    request_body = ContactFeedUpsertRequest,
    responses((status = 200, body = ContactFeedUpsertResponse),
              (status = 409, description = "idempotency key reused")))]
pub async fn contacts_feed_upsert(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ContactFeedUpsertRequest>,
) -> ApiResult<Json<ContactFeedUpsertResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let source_system = req.source_system.trim().to_ascii_lowercase();
    let external_id = req.external_id.trim().to_string();
    if source_system.is_empty() || external_id.is_empty() {
        return Err(ApiError::Validation(
            "source_system and external_id are required".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or_default();
    let key = req.instant.idempotency_key.clone();

    let input = ContactFeedUpsert {
        source_system: source_system.clone(),
        external_id: external_id.clone(),
        source_guid: req.source_guid.clone(),
        payload_ref: req.payload_ref.clone(),
        client_id: req.client_id,
        name: req.name.trim().to_string(),
        email: req.email.clone(),
        phone: req.phone.clone(),
        title: req.title.clone(),
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "contact",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }

        // Per-connector gate (Wave 15 lane 4): the org's `connectors.*`
        // toggles decide whether this source may write at all.
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

        let result: ContactFeedUpsertResult =
            core_db::entity_dedupe::contact_feed_upsert(&mut ctx.tx, org_id, &input).await?;
        let contact_id = result.contact.id;

        if result.created_contact {
            ctx.emit(
                event_types::types::CONTACT_CREATED,
                "contact",
                Some(contact_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::ContactCreated {
                    contact_id,
                    client_id: input.client_id,
                    name: input.name.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
        } else {
            ctx.emit(
                event_types::types::CONTACT_UPDATED,
                "contact",
                Some(contact_id.to_string().as_str()),
                &serde_json::json!({
                    "contact_id": contact_id,
                    "change": if result.resolved_existing { "adopted_existing" } else { "feed" },
                    "fields_changed": [],
                }),
            )
            .await?;
        }

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "contact",
                entity_id: contact_id,
                version: req.instant.version,
            },
        )
        .await?;
        let response_contact =
            core_db::clients::get_contact(&mut *ctx.tx, org_id, contact_id).await?;
        // The stored idempotency payload is the FULL typed response so a
        // replay deserializes byte-identical (IF1).
        let response_payload = ContactFeedUpsertResponse {
            contact: response_contact.clone(),
            source_link: result.link.clone(),
            created_contact: result.created_contact,
            resolved_existing: result.resolved_existing,
            auto_merged: result.auto_merged,
            queued_candidates: result.queued_candidates.clone(),
            contradictions: result.contradictions.clone(),
            // Placeholder meta — the REAL meta (written by `record`) is
            // spliced into the stored payload by the instant layer itself.
            mutation: instant::MutationMeta {
                seq: 0,
                version: 0,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::json!({}),
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "contact",
                entity_id: contact_id,
                op: "feed_upsert",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&response_contact).unwrap_or_default(),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &response_payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(ContactFeedUpsertResponse {
                contact: response_contact,
                source_link: result.link,
                created_contact: result.created_contact,
                resolved_existing: result.resolved_existing,
                auto_merged: result.auto_merged,
                queued_candidates: result.queued_candidates,
                contradictions: result.contradictions,
                mutation,
            }),
        ))
    })
    .await?;

    // decision_record_dedupe on the CONTACT line too (suggestion-grade;
    // merges stay human-approved; silent when unconfigured).
    if let instant::MutationOutcome::Applied(response) = &outcome {
        let candidates = response
            .queued_candidates
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            record_contact_dedupe_ask(&state, org_id, user_id, &candidates).await;
        }
    }

    Ok(Json(outcome.into_response()?))
}

async fn record_contact_dedupe_ask(
    state: &AppState,
    org_id: uuid::Uuid,
    user_id: uuid::Uuid,
    candidates: &[core_domain::entities::entity_dedupe::EntityMergeCandidate],
) {
    for candidate in candidates {
        let Some(right_id) = candidate.candidate_entity_id else {
            continue;
        };
        let mut tx = match crate::routes::ai_policy::org_bound_tx(state, org_id, user_id).await {
            Ok(tx) => tx,
            Err(err) => {
                tracing::debug!(%err, org = %org_id, "contact dedupe context read failed; skipping");
                continue;
            }
        };
        let left = match core_db::clients::get_contact(
            &mut *tx,
            org_id,
            candidate.primary_entity_id,
        )
        .await
        {
            Ok(c) => c,
            Err(err) => {
                tracing::debug!(%err, "contact dedupe primary read failed; skipping candidate");
                continue;
            }
        };
        let right = core_db::clients::get_contact(&mut *tx, org_id, right_id)
            .await
            .ok();
        tx.commit().await.ok();
        let ask = crate::routes::decision_edge::DecisionAsk {
            org_id,
            user_id,
            surface_key: "decision_record_dedupe",
            entity_type: "contact_merge_candidate",
            entity_id: Some(candidate.id),
            state: serde_json::json!({
                "left": {
                    "name": left.name,
                    "email": left.email.clone().unwrap_or_default(),
                },
                "right": {
                    "name": right.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
                    "email": right.and_then(|c| c.email).unwrap_or_default(),
                },
            }),
            request_id: format!("decision-dedupe-{}", candidate.id),
            inventories: Default::default(),
        };
        match crate::routes::decision_edge::decide(state, ask).await {
            Ok(Ok(_run)) => {
                tracing::info!(
                    org = %org_id,
                    candidate = %candidate.id,
                    "contact dedupe Noul recorded (suggestion-grade; merge stays human-approved)"
                );
            }
            Ok(Err(quiet)) => {
                tracing::debug!(org = %org_id, reason = %quiet.reason(), "contact dedupe quiet");
            }
            Err(err) => {
                tracing::debug!(org = %org_id, %err, "contact dedupe fan-out failed");
            }
        }
    }
}

/// POST /api/clients/org-reconcile — the connectors' org_map seam (public
/// API): report an unmapped vendor org; the durable map resolves it
/// instantly, otherwise deterministic label proposals queue reviewable
/// candidates. Never a guess, never a silent auto-map.
#[utoipa::path(operation_id = "clients_org_reconcile", post, path = "/api/clients/org-reconcile", tag = "entity-dedupe", security(("bearer" = [])),
    request_body = OrgReconcileRequest,
    responses((status = 200, body = OrgReconcileResponse),
              (status = 409, description = "idempotency key reused")))]
pub async fn org_reconcile(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<OrgReconcileRequest>,
) -> ApiResult<Json<OrgReconcileResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let source_system = req.source_system.trim().to_ascii_lowercase();
    let vendor_org_id = req.vendor_org_id.trim().to_string();
    if source_system.is_empty() || vendor_org_id.is_empty() {
        return Err(ApiError::Validation(
            "source_system and vendor_org_id are required".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or_default();
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "client_org_map",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }

        let queued = core_db::entity_dedupe::queue_client_reconciliation(
            &mut ctx.tx,
            org_id,
            &source_system,
            &vendor_org_id,
            req.vendor_label.as_deref(),
        )
        .await?;
        let already_mapped = core_db::entity_dedupe::get_org_map(
            &mut ctx.tx,
            org_id,
            &source_system,
            &vendor_org_id,
        )
        .await?
        .is_some();
        let mut views = Vec::with_capacity(queued.len());
        for c in queued {
            let primary_label =
                core_db::clients::get_client(&mut *ctx.tx, org_id, c.primary_entity_id)
                    .await
                    .map(|cl| cl.name)
                    .unwrap_or_default();
            views.push(EntityCandidateView {
                primary_label,
                candidate_label: c.vendor_label.clone(),
                candidate: c,
            });
        }
        // The stored idempotency payload is the FULL typed response so a
        // replay deserializes byte-identical (IF1).
        // The reconciliation op gets its own write-trail identity (the
        // per-org per-entity monotonic seq the instant layer keeps).
        let run_id = uuid::Uuid::now_v7();
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "client_org_map",
                entity_id: run_id,
                version: req.instant.version,
            },
        )
        .await?;
        let response = OrgReconcileResponse {
            queued_candidates: views.clone(),
            already_mapped,
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::json!({}),
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "client_org_map",
                entity_id: run_id,
                op: "org_reconcile",
                seq,
                before: None,
                after: serde_json::json!({
                    "queued": views.len(),
                    "source_system": source_system,
                    "vendor_org_id": vendor_org_id,
                }),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &response,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(OrgReconcileResponse {
                queued_candidates: response.queued_candidates,
                already_mapped: response.already_mapped,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/entity-dedupe/candidates/{id}/accept — contact: audited +
/// undoable merge; client: writes the durable operator map.
#[utoipa::path(operation_id = "entity_dedupe_accept_candidate", post, path = "/api/entity-dedupe/candidates/{id}/accept", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = core_domain::entities::entity_dedupe::EntityMerge),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate already decided")))]
pub async fn accept_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let candidate = core_db::entity_dedupe::get_entity_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        match candidate.entity_kind {
            DedupeEntityKind::Contact => {
                let merge = core_db::entity_dedupe::merge_contacts(
                    &mut ctx.tx,
                    org_id,
                    candidate.primary_entity_id,
                    candidate.candidate_entity_id.ok_or_else(|| {
                        ApiError::Validation("contact candidate carries no pair".into())
                    })?,
                    Some(candidate.id),
                    Some(candidate.confidence),
                    &candidate.reason,
                    &candidate.evidence,
                    Some(user_id),
                )
                .await?;
                core_db::entity_dedupe::decide_entity_candidate(
                    &mut ctx.tx,
                    org_id,
                    candidate.id,
                    EntityMergeStatus::Accepted.as_str(),
                    user_id,
                )
                .await?;
                core_db::audit::insert(
                    &mut *ctx.tx,
                    &core_domain::entities::AuditEvent {
                        id: uuid::Uuid::now_v7(),
                        org_id,
                        actor: core_domain::entities::Actor::Human(user_id),
                        action: "contact.merged".into(),
                        entity_type: Some("contact".into()),
                        entity_id: Some(candidate.primary_entity_id.to_string()),
                        detail: serde_json::json!({
                            "merged": candidate.candidate_entity_id,
                            "reason": candidate.reason,
                            "candidate": candidate.id,
                        }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
                Ok((ctx, serde_json::to_value(merge).unwrap_or_default()))
            }
            DedupeEntityKind::Client => {
                let (decided, map) = core_db::entity_dedupe::accept_client_reconciliation(
                    &mut ctx.tx,
                    org_id,
                    candidate.id,
                    user_id,
                )
                .await?;
                core_db::audit::insert(
                    &mut *ctx.tx,
                    &core_domain::entities::AuditEvent {
                        id: uuid::Uuid::now_v7(),
                        org_id,
                        actor: core_domain::entities::Actor::Human(user_id),
                        action: "client.org_map_recorded".into(),
                        entity_type: Some("client".into()),
                        entity_id: Some(candidate.primary_entity_id.to_string()),
                        detail: serde_json::json!({
                            "source_system": map.source_system,
                            "vendor_org_id": map.vendor_org_id,
                            "vendor_label": map.vendor_label,
                            "candidate": decided.id,
                        }),
                        ip: None,
                        occurred_at: chrono::Utc::now(),
                    },
                )
                .await?;
                Ok((
                    ctx,
                    serde_json::json!({ "candidate": decided, "org_map": map }),
                ))
            }
        }
    })
    .await?;

    Ok(Json(response))
}

/// POST /api/entity-dedupe/candidates/{id}/reject — keep both rows (or
/// keep the vendor org unmapped).
#[utoipa::path(operation_id = "entity_dedupe_reject_candidate", post, path = "/api/entity-dedupe/candidates/{id}/reject", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = core_domain::entities::entity_dedupe::EntityMergeCandidate),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate already decided")))]
pub async fn reject_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<core_domain::entities::entity_dedupe::EntityMergeCandidate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let candidate = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::entity_dedupe::get_entity_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let candidate = core_db::entity_dedupe::decide_entity_candidate(
            &mut ctx.tx,
            org_id,
            id,
            EntityMergeStatus::Rejected.as_str(),
            user_id,
        )
        .await?;
        Ok((ctx, candidate))
    })
    .await?;
    Ok(Json(candidate))
}

/// POST /api/entity-dedupe/candidates/{id}/defer — park without deciding.
#[utoipa::path(operation_id = "entity_dedupe_defer_candidate", post, path = "/api/entity-dedupe/candidates/{id}/defer", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = core_domain::entities::entity_dedupe::EntityMergeCandidate),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate already decided")))]
pub async fn defer_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<core_domain::entities::entity_dedupe::EntityMergeCandidate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let candidate = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::entity_dedupe::get_entity_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let candidate = core_db::entity_dedupe::decide_entity_candidate(
            &mut ctx.tx,
            org_id,
            id,
            EntityMergeStatus::Deferred.as_str(),
            user_id,
        )
        .await?;
        Ok((ctx, candidate))
    })
    .await?;
    Ok(Json(candidate))
}

/// POST /api/entity-dedupe/candidates/{id}/restore — un-defer.
#[utoipa::path(operation_id = "entity_dedupe_restore_candidate", post, path = "/api/entity-dedupe/candidates/{id}/restore", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = core_domain::entities::entity_dedupe::EntityMergeCandidate),
              (status = 404, description = "candidate not visible in scope"),
              (status = 409, description = "candidate is not deferred")))]
pub async fn restore_candidate(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<core_domain::entities::entity_dedupe::EntityMergeCandidate>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let candidate = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let candidate = core_db::entity_dedupe::restore_entity_candidate(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        Ok((ctx, candidate))
    })
    .await?;
    Ok(Json(candidate))
}

/// POST /api/entity-dedupe/candidates/batch — accept/reject/defer N.
#[utoipa::path(operation_id = "entity_dedupe_batch_candidates", post, path = "/api/entity-dedupe/candidates/batch", tag = "entity-dedupe", security(("bearer" = [])),
    request_body = EntityBatchRequest,
    responses((status = 200, body = EntityBatchResponse),
              (status = 422, description = "unknown batch action / empty id list")))]
pub async fn batch_candidates(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<EntityBatchRequest>,
) -> ApiResult<Json<EntityBatchResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    if req.ids.is_empty() {
        return Err(ApiError::Validation("ids must not be empty".into()));
    }
    let action = req.action.trim().to_ascii_lowercase();
    if !matches!(action.as_str(), "accept" | "reject" | "defer") {
        return Err(ApiError::Validation(
            "action must be one of accept | reject | defer".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut outcomes = Vec::with_capacity(req.ids.len());
    for id in &req.ids {
        let action = action.clone();
        let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
            let candidate = core_db::entity_dedupe::get_entity_candidate(&mut ctx.tx, org_id, *id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if candidate.status != EntityMergeStatus::Pending {
                return Ok((
                    ctx,
                    EntityBatchOutcome {
                        id: *id,
                        outcome: "skipped".into(),
                        merge: None,
                        detail: Some(format!("candidate is {}", candidate.status.as_str())),
                    },
                ));
            }
            match (action.as_str(), candidate.entity_kind) {
                ("accept", DedupeEntityKind::Contact) => {
                    let merged_id = candidate.candidate_entity_id.ok_or_else(|| {
                        ApiError::Validation("contact candidate carries no pair".into())
                    })?;
                    let merge = core_db::entity_dedupe::merge_contacts(
                        &mut ctx.tx,
                        org_id,
                        candidate.primary_entity_id,
                        merged_id,
                        Some(candidate.id),
                        Some(candidate.confidence),
                        &candidate.reason,
                        &candidate.evidence,
                        Some(user_id),
                    )
                    .await?;
                    core_db::entity_dedupe::decide_entity_candidate(
                        &mut ctx.tx,
                        org_id,
                        candidate.id,
                        EntityMergeStatus::Accepted.as_str(),
                        user_id,
                    )
                    .await?;
                    Ok((
                        ctx,
                        EntityBatchOutcome {
                            id: *id,
                            outcome: "applied".into(),
                            merge: Some(merge),
                            detail: None,
                        },
                    ))
                }
                // Client reconciliation is NEVER batch-accepted: a mapping
                // is a per-org operator decision (the single accept
                // endpoint exists for exactly that deliberateness).
                ("accept", DedupeEntityKind::Client) => Ok((
                    ctx,
                    EntityBatchOutcome {
                        id: *id,
                        outcome: "skipped".into(),
                        merge: None,
                        detail: Some("client reconciliation must be accepted individually".into()),
                    },
                )),
                ("reject", _) => {
                    core_db::entity_dedupe::decide_entity_candidate(
                        &mut ctx.tx,
                        org_id,
                        *id,
                        EntityMergeStatus::Rejected.as_str(),
                        user_id,
                    )
                    .await?;
                    Ok((
                        ctx,
                        EntityBatchOutcome {
                            id: *id,
                            outcome: "applied".into(),
                            merge: None,
                            detail: None,
                        },
                    ))
                }
                (_, _) => {
                    core_db::entity_dedupe::decide_entity_candidate(
                        &mut ctx.tx,
                        org_id,
                        *id,
                        EntityMergeStatus::Deferred.as_str(),
                        user_id,
                    )
                    .await?;
                    Ok((
                        ctx,
                        EntityBatchOutcome {
                            id: *id,
                            outcome: "applied".into(),
                            merge: None,
                            detail: None,
                        },
                    ))
                }
            }
        })
        .await;
        match outcome {
            Ok(o) => outcomes.push(o),
            Err(e) => outcomes.push(EntityBatchOutcome {
                id: *id,
                outcome: "failed".into(),
                merge: None,
                detail: Some(e.to_string()),
            }),
        }
    }

    let mut resp = EntityBatchResponse {
        accepted: 0,
        rejected: 0,
        deferred: 0,
        skipped: 0,
        failed: 0,
        outcomes,
    };
    for o in &resp.outcomes {
        match (action.as_str(), o.outcome.as_str()) {
            (_, "applied") if action == "accept" => resp.accepted += 1,
            (_, "applied") if action == "reject" => resp.rejected += 1,
            (_, "applied") => resp.deferred += 1,
            (_, "skipped") => resp.skipped += 1,
            _ => resp.failed += 1,
        }
    }
    Ok(Json(resp))
}

/// POST /api/entity-dedupe/merges/{id}/split — undo a contact merge (IF7).
#[utoipa::path(operation_id = "entity_dedupe_split_merge", post, path = "/api/entity-dedupe/merges/{id}/split", tag = "entity-dedupe", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = core_domain::entities::entity_dedupe::EntityMerge),
              (status = 404, description = "merge not visible in scope"),
              (status = 409, description = "merge already undone")))]
pub async fn split_merge(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<core_domain::entities::entity_dedupe::EntityMerge>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_edit_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let merge = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::entity_dedupe::get_entity_merge(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let merge =
            core_db::entity_dedupe::split_entity_merge(&mut ctx.tx, org_id, id, user_id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "contact.merge_undone".into(),
                entity_type: Some("contact".into()),
                entity_id: Some(merge.primary_entity_id.to_string()),
                detail: serde_json::json!({ "restored": merge.merged_entity_id }),
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

/// GET /api/clients/org-map — the durable operator mapping (settings view).
#[utoipa::path(operation_id = "clients_org_map_list", get, path = "/api/clients/org-map", tag = "entity-dedupe", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::entity_dedupe::ClientOrgMap])))]
pub async fn list_org_map(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::entity_dedupe::ClientOrgMap>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, clients_view_permission())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows: Vec<core_domain::entities::entity_dedupe::ClientOrgMap> = sqlx::query(
        r#"SELECT id, org_id, source_system, vendor_org_id, client_id, vendor_label,
                  created_by, created_at, updated_at
             FROM client_org_map WHERE org_id = $1 ORDER BY source_system, vendor_org_id"#,
    )
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| core_domain::entities::entity_dedupe::ClientOrgMap {
        id: r.get("id"),
        org_id: r.get("org_id"),
        source_system: r.get("source_system"),
        vendor_org_id: r.get("vendor_org_id"),
        client_id: r.get("client_id"),
        vendor_label: r.get("vendor_label"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
    .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

// ---- router -------------------------------------------------------------------

pub fn routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/entity-dedupe/candidates"),
            axum::routing::get(list_candidates),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/batch"),
            axum::routing::post(batch_candidates),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/{{id}}/accept"),
            axum::routing::post(accept_candidate),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/{{id}}/reject"),
            axum::routing::post(reject_candidate),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/{{id}}/defer"),
            axum::routing::post(defer_candidate),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/{{id}}/restore"),
            axum::routing::post(restore_candidate),
        )
        .route(
            &format!("{base}/entity-dedupe/candidates/{{id}}/workbench"),
            axum::routing::get(candidate_workbench),
        )
        .route(
            &format!("{base}/entity-dedupe/merges/{{id}}/split"),
            axum::routing::post(split_merge),
        )
        .route(
            &format!("{base}/contacts/feed-upsert"),
            axum::routing::post(contacts_feed_upsert),
        )
        .route(
            &format!("{base}/clients/org-reconcile"),
            axum::routing::post(org_reconcile),
        )
        .route(
            &format!("{base}/clients/org-map"),
            axum::routing::get(list_org_map),
        )
}
