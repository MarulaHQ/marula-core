//! SLA configuration surfaces: calendars, policies + escalation ladders,
//! and automation rules (TASKLIST 2.7).
//!
//! Purpose: admin-gated CRUD over the 0015 configuration tables. Writes
//! validate through core-domain types where shapes matter (calendars,
//! automation documents) and emit config-change events in the SAME
//! transaction, so downstream consumers see one consistent stream.
//!
//! Non-purpose: no clock math or rule matching (pure core-domain), no
//! evaluation loop (worker), no effect application (routes::internal).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::automations::{AutomationSubject, AutomationSubjectKind};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

// --- calendars ---------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCalendarRequest {
    pub name: String,
    /// Timezone/windows/holidays/maintenance document.
    pub calendar: core_domain::entities::BusinessCalendar,
}

/// GET /api/sla/calendars
#[utoipa::path(operation_id = "sla_calendars_list", get, path = "/api/sla/calendars", tag = "sla", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::SlaCalendar])))]
pub async fn list_calendars(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::SlaCalendar>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::sla::list_calendars(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/sla/calendars
#[utoipa::path(operation_id = "sla_calendars_create", post, path = "/api/sla/calendars", tag = "sla", security(("bearer" = [])),
    request_body = CreateCalendarRequest,
    responses((status = 200, body = core_domain::entities::SlaCalendar)))]
pub async fn create_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateCalendarRequest>,
) -> ApiResult<Json<core_domain::entities::SlaCalendar>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("calendar name is required".into()));
    }
    let mut cal = req.calendar;
    cal.name = name.clone();
    cal.validate()?;
    let org_id = authed.principal.user.org_id;

    let row = core_domain::entities::SlaCalendar {
        id: Uuid::now_v7(),
        org_id,
        name,
        calendar: cal,
    };
    let saved = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let saved = core_db::sla::insert_calendar(&mut *ctx.tx, &row).await?;
            ctx.emit(
                types::SLA_CALENDAR_UPDATED,
                "sla_calendar",
                Some(saved.id.to_string().as_str()),
                &serde_json::json!({ "calendar_id": saved.id, "name": saved.name }),
            )
            .await?;
            Ok((ctx, saved))
        },
    )
    .await?;
    Ok(Json(saved))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateCalendarRequest {
    pub name: Option<String>,
    pub timezone: Option<String>,
    pub windows: Option<serde_json::Value>,
    pub holidays: Option<serde_json::Value>,
    pub maintenance: Option<serde_json::Value>,
}

/// PATCH /api/sla/calendars/{id} — partial update; validates the merged
/// result by re-reading and running domain validation.
#[utoipa::path(operation_id = "sla_calendars_update", patch, path = "/api/sla/calendars/{id}", tag = "sla", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateCalendarRequest,
    responses((status = 200, body = core_domain::entities::SlaCalendar)))]
pub async fn update_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCalendarRequest>,
) -> ApiResult<Json<core_domain::entities::SlaCalendar>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;

    let updated = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let patched = core_db::sla::update_calendar(
                &mut *ctx.tx,
                org_id,
                id,
                &core_db::sla::CalendarPatch {
                    name: req.name.as_deref(),
                    timezone: req.timezone.as_deref(),
                    windows: req.windows.clone(),
                    holidays: req.holidays.clone(),
                    maintenance: req.maintenance.clone(),
                },
            )
            .await?;
            // Validate the MERGED document, not just the patch fragment.
            patched.calendar.validate().map_err(ApiError::from)?;
            ctx.emit(
                types::SLA_CALENDAR_UPDATED,
                "sla_calendar",
                Some(patched.id.to_string().as_str()),
                &serde_json::json!({ "calendar_id": patched.id }),
            )
            .await?;
            Ok((ctx, patched))
        },
    )
    .await?;
    Ok(Json(updated))
}

/// DELETE /api/sla/calendars/{id} — detaches policies (FK SET NULL); their
/// embedded calendars keep governing.
#[utoipa::path(operation_id = "sla_calendars_delete", delete, path = "/api/sla/calendars/{id}", tag = "sla", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = crate::routes::openapi::HealthResponse)))]
pub async fn delete_calendar(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            core_db::sla::delete_calendar(&mut *ctx.tx, org_id, id).await?;
            ctx.emit(
                types::SLA_CALENDAR_UPDATED,
                "sla_calendar",
                Some(id.to_string().as_str()),
                &serde_json::json!({ "deleted": true, "calendar_id": id }),
            )
            .await?;
            Ok((ctx, ()))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// --- policies + ladders --------------------------------------------------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PolicyView {
    pub id: Uuid,
    pub name: String,
    pub is_default: bool,
    /// Targets keyed by priority ("p1".."p4"), business minutes.
    pub targets: serde_json::Value,
    pub calendar_id: Option<Uuid>,
    /// True while the policy governs itself via its embedded calendar.
    pub uses_embedded_calendar: bool,
}

fn policy_view(r: core_db::sla::SlaPolicyRecord) -> PolicyView {
    PolicyView {
        id: r.id,
        name: r.name,
        is_default: r.is_default,
        targets: r.targets,
        calendar_id: r.calendar_id,
        uses_embedded_calendar: r.calendar_id.is_none(),
    }
}

/// GET /api/sla/policies
#[utoipa::path(operation_id = "sla_policies_list", get, path = "/api/sla/policies", tag = "sla", security(("bearer" = [])),
    responses((status = 200, body = [PolicyView])))]
pub async fn list_policies(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PolicyView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::sla::list_policy_records(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(policy_view).collect()))
}

/// GET /api/sla/policies/{id}/ladder — enabled + disabled steps, ordered.
#[utoipa::path(operation_id = "sla_ladder_get", get, path = "/api/sla/policies/{id}/ladder", tag = "sla", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::EscalationStep])))]
pub async fn get_ladder(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(policy_id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::EscalationStep>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // Existence check stays org-scoped so foreign ids 404 instead of leaking.
    core_db::sla::get_policy_record(&mut *tx, org_id, policy_id).await?;
    let steps = core_db::sla::list_steps_for_policies(&mut *tx, &[policy_id]).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(steps))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LadderStepSpec {
    pub step_index: i32,
    /// 'risk' | 'breach' | 'escalate' (notification-center kinds).
    pub trigger_kind: String,
    /// Fires when consumed budget reaches this percent (1..=500).
    pub at_pct: i64,
    pub notify_user_ids: Vec<Uuid>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ReplaceLadderRequest {
    pub steps: Vec<LadderStepSpec>,
}

/// Validates one step spec against the engine's vocabulary.
fn validate_step(spec: &LadderStepSpec) -> ApiResult<()> {
    core_domain::sla_engine::StepTrigger::parse(&spec.trigger_kind).ok_or_else(|| {
        ApiError::Validation(format!("unknown trigger_kind {}", spec.trigger_kind))
    })?;
    if !(1..=500).contains(&spec.at_pct) {
        return Err(ApiError::Validation("at_pct must be within 1..=500".into()));
    }
    if spec.step_index < 0 {
        return Err(ApiError::Validation("step_index must be >= 0".into()));
    }
    Ok(())
}

/// PUT /api/sla/policies/{id}/ladder — replaces the whole ladder in one tx.
#[utoipa::path(operation_id = "sla_ladder_replace", put, path = "/api/sla/policies/{id}/ladder", tag = "sla", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ReplaceLadderRequest,
    responses((status = 200, body = [core_domain::entities::EscalationStep])))]
pub async fn replace_ladder(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(policy_id): Path<Uuid>,
    Json(req): Json<ReplaceLadderRequest>,
) -> ApiResult<Json<Vec<core_domain::entities::EscalationStep>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    for s in &req.steps {
        validate_step(s)?;
    }
    let mut indexes: Vec<i32> = req.steps.iter().map(|s| s.step_index).collect();
    indexes.sort_unstable();
    indexes.dedup();
    if indexes.len() != req.steps.len() {
        return Err(ApiError::Validation(
            "step_index values must be unique".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let steps = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::sla::get_policy_record(&mut *ctx.tx, org_id, policy_id).await?;
        let inputs: Vec<core_db::sla::LadderStepInput> = req
            .steps
            .iter()
            .map(|s| core_db::sla::LadderStepInput {
                step_index: s.step_index,
                trigger_kind: s.trigger_kind.clone(),
                at_pct: s.at_pct,
                notify_user_ids: s.notify_user_ids.clone(),
                enabled: s.enabled,
            })
            .collect();
        let saved = core_db::sla::replace_ladder(&mut ctx.tx, org_id, policy_id, &inputs).await?;
        ctx.emit(
            types::SLA_POLICY_UPDATED,
            "sla_policy",
            Some(policy_id.to_string().as_str()),
            &serde_json::json!({ "ladder_size": saved.len() }),
        )
        .await?;
        Ok((ctx, saved))
    })
    .await?;
    Ok(Json(steps))
}

// --- automation rules -------------------------------------------------------------

fn rule_row(rec: core_db::automations::AutomationRuleRecord) -> serde_json::Value {
    serde_json::json!({
        "id": rec.id,
        "org_id": rec.org_id,
        "board_id": rec.board_id,
        "subject_kind": rec.subject_kind.as_str(),
        "name": rec.name,
        "trigger_event": rec.trigger_event,
        "conditions": rec.conditions,
        "actions": rec.actions,
        "enabled": rec.enabled,
    })
}

/// POST /api/automation-rules/{id}/test-run — dry-run one rule against the
/// org's most recent tickets (board-scoped when the rule binds a board),
/// PROPOSAL-shaped: what WOULD match and what the actions would do. The
/// rule editor renders the result; nothing is ever applied and no event is
/// emitted — a test-run writes nothing (Wave 17 lane 13 contract: the rule
/// editor never pretends to execute).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutomationTestRunMatch {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub number: String,
    pub title: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_id: Option<Uuid>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutomationTestRunResponse {
    #[schema(value_type = uuid::Uuid)]
    pub rule_id: Uuid,
    /// What the run sampled (the org's recent tickets, board-filtered
    /// when the rule binds a board).
    pub sampled_count: usize,
    pub matched_count: usize,
    pub matches: Vec<AutomationTestRunMatch>,
    /// The rule's CURRENT action documents, returned verbatim — the
    /// editor renders them as the proposal preview (never re-derived).
    pub actions_preview: serde_json::Value,
}

/// How many recent tickets one test run samples.
const TEST_RUN_SAMPLE: i64 = 50;

#[utoipa::path(operation_id = "automation_rules_test_run", post, path = "/api/automation-rules/{id}/test-run", tag = "automations", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AutomationTestRunResponse),
              (status = 404, description = "rule not visible in scope"),
              (status = 422, description = "stored rule document is malformed")))]
pub async fn test_run_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AutomationTestRunResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let record = core_db::automations::get(&mut *tx, org_id, id).await?;
    // Parse through core-domain — the SAME parse path the worker's matcher
    // runs, so a preview never disagrees with a live evaluation. A rule
    // the runner would SKIP (malformed, out-of-band-written) reports as
    // typed validation instead of faking zero matches.
    let row = core_domain::automations::AutomationRuleRow {
        id: record.id,
        org_id: record.org_id,
        board_id: record.board_id,
        subject_kind: record.subject_kind,
        name: record.name.clone(),
        trigger_event: record.trigger_event.clone(),
        conditions: record.conditions.clone(),
        actions: record.actions.clone(),
        enabled: record.enabled,
    };
    let parsed = core_domain::automations::AutomationRule::parse(&row)
        .map_err(|e| ApiError::Validation(format!("stored rule does not parse: {e}")))?;
    // Sample the org's recent tickets bounded by the sample cap; when the
    // rule binds a board, the sample IS board-live (matching the live
    // matcher's board gate) instead of padding the sample off-board.
    // A board-subject rule has no ticket to sample, so the dry-run reads
    // zero rows rather than pretending a ticket preview (Wave 43 lane 3) —
    // matching nothing here is the honest answer for a ticket-less rule.
    let rows = if parsed.subject_kind.requires_ticket() {
        core_db::tickets::list_tickets(
            &mut *tx,
            org_id,
            core_db::tickets::TicketFilter {
                board_id: record.board_id,
                assignee_id: None,
                client_id: None,
                category: None,
                priority: None,
                tag_ids: None,
                limit: TEST_RUN_SAMPLE,
                offset: 0,
                cursor: None,
            },
        )
        .await?
    } else {
        Vec::new()
    };
    tx.commit().await.map_err(ApiError::from)?;

    let previews: Vec<AutomationTestRunMatch> = rows
        .iter()
        .filter(|t| {
            core_domain::automations::matches_all(
                &parsed.conditions,
                AutomationSubject::Ticket(&core_domain::automations::TicketSnapshot {
                    ticket_id: t.id,
                    org_id: t.org_id,
                    board_id: t.board_id,
                    number: t.number.0.clone(),
                    title: t.title.clone(),
                    priority: t.priority,
                    category: t.category,
                    assignee_id: t.assignee_id,
                }),
            )
        })
        .map(|t| AutomationTestRunMatch {
            ticket_id: t.id,
            number: t.number.0.clone(),
            title: t.title.clone(),
            assignee_id: t.assignee_id,
        })
        .collect();
    Ok(Json(AutomationTestRunResponse {
        rule_id: id,
        sampled_count: rows.len(),
        matched_count: previews.len(),
        matches: previews,
        actions_preview: record.actions,
    }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRuleRequest {
    pub name: String,
    pub board_id: Option<Uuid>,
    /// What the rule's subject is: `ticket` (default, historical) or
    /// `board` — a rule that fires on ticket-less envelopes (Wave 43
    /// lane 3).
    pub subject_kind: Option<String>,
    /// Domain event type, e.g. "ticket.created".
    pub trigger_event: String,
    /// Condition documents (see AutomationCondition schema).
    pub conditions: serde_json::Value,
    /// Action documents (see AutomationAction schema).
    pub actions: serde_json::Value,
}

/// GET /api/automation-rules
#[utoipa::path(operation_id = "automation_rules_list", get, path = "/api/automation-rules", tag = "automations", security(("bearer" = [])),
    responses((status = 200, body = [Object])))]
pub async fn list_rules(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::automations::list(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(rule_row).collect()))
}

/// POST /api/automation-rules — documents are validated through core-domain
/// BEFORE storage, so stored rules are always well-formed (malformed rules
/// can only arrive out-of-band and are skipped at evaluation time).
#[utoipa::path(operation_id = "automation_rules_create", post, path = "/api/automation-rules", tag = "automations", security(("bearer" = [])),
    request_body = CreateRuleRequest,
    responses((status = 200, body = Object)))]
pub async fn create_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateRuleRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("rule name is required".into()));
    }
    if req.trigger_event.trim().is_empty() {
        return Err(ApiError::Validation("trigger_event is required".into()));
    }
    serde_json::from_value::<Vec<core_domain::automations::AutomationCondition>>(
        req.conditions.clone(),
    )
    .map_err(|e| ApiError::Validation(format!("bad conditions: {e}")))?;
    let actions: Vec<core_domain::automations::AutomationAction> =
        serde_json::from_value(req.actions.clone())
            .map_err(|e| ApiError::Validation(format!("bad actions: {e}")))?;
    core_domain::automations::validate_actions(&actions)?;

    let org_id = authed.principal.user.org_id;
    let subject_kind = AutomationSubjectKind::from_db(req.subject_kind.as_deref());
    let id = Uuid::now_v7();
    let saved = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let rec = core_db::automations::insert(
                &mut *ctx.tx,
                id,
                org_id,
                req.board_id,
                subject_kind,
                &name,
                req.trigger_event.trim(),
                req.conditions.clone(),
                req.actions.clone(),
            )
            .await?;
            ctx.emit(
                types::AUTOMATION_RULE_UPDATED,
                "automation_rule",
                Some(rec.id.to_string().as_str()),
                &serde_json::json!({ "rule_id": rec.id, "trigger_event": rec.trigger_event }),
            )
            .await?;
            Ok((ctx, rec))
        },
    )
    .await?;
    Ok(Json(rule_row(saved)))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRuleRequest {
    pub name: Option<String>,
    /// `Some(Some(id))` scopes; `Some(None)` clears to all boards.
    pub board_id: Option<Option<Uuid>>,
    /// Replaces the subject declaration (Wave 43 lane 3).
    pub subject_kind: Option<String>,
    pub trigger_event: Option<String>,
    pub conditions: Option<serde_json::Value>,
    pub actions: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

/// PATCH /api/automation-rules/{id}
#[utoipa::path(operation_id = "automation_rules_update", patch, path = "/api/automation-rules/{id}", tag = "automations", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateRuleRequest,
    responses((status = 200, body = Object)))]
pub async fn update_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRuleRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    // Validate replacement documents when present.
    if let Some(c) = &req.conditions {
        serde_json::from_value::<Vec<core_domain::automations::AutomationCondition>>(c.clone())
            .map_err(|e| ApiError::Validation(format!("bad conditions: {e}")))?;
    }
    if let Some(a) = &req.actions {
        let actions: Vec<core_domain::automations::AutomationAction> =
            serde_json::from_value(a.clone())
                .map_err(|e| ApiError::Validation(format!("bad actions: {e}")))?;
        core_domain::automations::validate_actions(&actions)?;
    }
    let org_id = authed.principal.user.org_id;
    let saved = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let rec = core_db::automations::update(
                &mut *ctx.tx,
                org_id,
                id,
                &core_db::automations::RulePatch {
                    name: req.name.clone(),
                    board_id: req.board_id,
                    subject_kind: req
                        .subject_kind
                        .as_deref()
                        .map(|value| AutomationSubjectKind::from_db(Some(value))),
                    trigger_event: req.trigger_event.as_ref().map(|s| s.trim().to_string()),
                    conditions: req.conditions.clone(),
                    actions: req.actions.clone(),
                    enabled: req.enabled,
                },
            )
            .await?;
            ctx.emit(
                types::AUTOMATION_RULE_UPDATED,
                "automation_rule",
                Some(rec.id.to_string().as_str()),
                &serde_json::json!({ "rule_id": rec.id }),
            )
            .await?;
            Ok((ctx, rec))
        },
    )
    .await?;
    Ok(Json(rule_row(saved)))
}

/// DELETE /api/automation-rules/{id}
#[utoipa::path(operation_id = "automation_rules_delete", delete, path = "/api/automation-rules/{id}", tag = "automations", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object)))]
pub async fn delete_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            core_db::automations::delete(&mut *ctx.tx, org_id, id).await?;
            ctx.emit(
                types::AUTOMATION_RULE_UPDATED,
                "automation_rule",
                Some(id.to_string().as_str()),
                &serde_json::json!({ "deleted": true }),
            )
            .await?;
            Ok((ctx, ()))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
