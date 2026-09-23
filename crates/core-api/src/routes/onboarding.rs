//! Wave 26 onboarding flows (org wizard + user-level first-run).
//!
//! Purpose: ONE typed summary seam for both flow scopes, server-gated
//! step completion, recorded skip/dismiss choices (never client-trusted,
//! never a silent re-nag), plus the re-run / re-trigger / org-disable
//! controls under Settings → Onboarding.
//!
//! Gate doctrine: `complete` verifies the underlying artifact server-side
//! (clients/boards/tickets/invites exist; a feeder link was written; the
//! AI master switch is on; the user's second factor is enrolled) BEFORE
//! the done flag is persisted. A plain "skip for now" persists NOTHING —
//! the step simply stays pending and resumable. Only `never: true`
//! ("never show this again") persists the dismissed flag. Recorded state
//! is the ONLY state, which is why a dismissal never resets on reload.
//! The legacy flat `onboarding.wizard_completed` key is honored as a
//! fallback (U6-safe supersede) when no per-step flags exist yet.
//!
//! Non-purpose: wizard step CONTENT (the web app owns the flows UX),
//! notification preference editing (dedicated surface), and member
//! onboarding state visibility for non-admin callers.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::settings::{self as registry, SettingValue};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

fn audit_event(
    org_id: Uuid,
    actor_id: Uuid,
    entity_id: String,
    detail: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(actor_id),
        action: "onboarding.updated".to_string(),
        entity_type: Some("onboarding".to_string()),
        entity_id: Some(entity_id),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

// --- wire views --------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingStepView {
    pub step: String,
    pub done: bool,
    /// "never show this again" was recorded for this step.
    pub dismissed: bool,
    /// Server-verified live gate state right now.
    pub satisfied: bool,
    pub resolved: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingScopeView {
    /// Org-wide master switch (Settings → Onboarding).
    pub disabled: bool,
    /// Legacy fallback: true while NO per-step flags exist yet but the old
    /// flat key did (U6-safe supersede).
    pub legacy_wizard_completed: bool,
    pub steps: Vec<OnboardingStepView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingMemberView {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub steps: Vec<OnboardingStepView>,
    /// Live: the member still has unresolved steps (drives re-trigger UX).
    pub unresolved: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingSummary {
    pub org: OnboardingScopeView,
    /// Populated for admin callers only.
    pub members: Vec<OnboardingMemberView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OnboardingSkipRequest {
    /// true = "never show this again" (persists the dismissed flag);
    /// false = plain "skip for now" (a no-op that intentionally records
    /// nothing — the step stays pending and resumable).
    #[serde(default)]
    pub never: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OnboardingTriggerRequest {
    /// The member whose first-run flow is re-triggered (admin calls);
    /// omitted means re-run the CALLER's own user onboarding.
    #[serde(default)]
    pub user_id: Option<Uuid>,
}

// --- step id + flag helpers --------------------------------------------------

fn org_step_keys(step: &str) -> ApiResult<(&'static str, String, String)> {
    registry::ONBOARDING_ORG_STEPS
        .iter()
        .copied()
        .find(|s| *s == step)
        .map(|s| {
            (
                s,
                registry::onboarding_org_step_key(s, "done"),
                registry::onboarding_org_step_key(s, "dismissed"),
            )
        })
        .ok_or_else(|| ApiError::Validation(format!("unknown org onboarding step '{step}'")))
}

fn user_step_keys(step: &str) -> ApiResult<(&'static str, String, String)> {
    registry::ONBOARDING_USER_STEPS
        .iter()
        .copied()
        .find(|s| *s == step)
        .map(|s| {
            (
                s,
                registry::onboarding_user_step_key(s, "done"),
                registry::onboarding_user_step_key(s, "dismissed"),
            )
        })
        .ok_or_else(|| ApiError::Validation(format!("unknown user onboarding step '{step}'")))
}

async fn write_org_flag(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    key: String,
    value: bool,
) -> ApiResult<()> {
    let decl = registry::declaration(&key)
        .ok_or_else(|| ApiError::Validation(format!("unknown setting key: {key}")))?;
    let backed = SettingValue::from_json(&serde_json::json!(value))
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    decl.validate(&backed)
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    if decl.scope != registry::SettingScope::Org {
        return Err(ApiError::Validation(format!(
            "{key} is not an org-scope flag"
        )));
    }
    crate::org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        core_db::settings::upsert_org_setting(
            &mut ctx.tx,
            org_id,
            &key,
            &backed.to_json(),
            user_id,
        )
        .await?;
        ctx.emit(
            types::SETTING_UPDATED,
            "setting",
            Some(&key),
            &serde_json::json!({ "scope": "org", "key": key, "value": backed.to_json() }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                user_id,
                format!("org:{key}"),
                serde_json::json!({ "key": key, "value": backed.to_json() }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await
}

async fn write_user_flag(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    key: String,
    value: bool,
) -> ApiResult<()> {
    let decl = registry::declaration(&key)
        .ok_or_else(|| ApiError::Validation(format!("unknown setting key: {key}")))?;
    let backed = SettingValue::from_json(&serde_json::json!(value))
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    decl.validate(&backed)
        .map_err(|e| ApiError::Validation(format!("{key}: {e}")))?;
    if decl.scope != registry::SettingScope::User {
        return Err(ApiError::Validation(format!(
            "{key} is not a user-scope flag"
        )));
    }
    crate::org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        core_db::settings::upsert_user_setting(&mut ctx.tx, org_id, user_id, &key, &backed.to_json())
            .await?;
        ctx.emit(
            types::SETTING_UPDATED,
            "setting",
            Some(&key),
            &serde_json::json!({ "scope": "user", "user_id": user_id, "key": key, "value": backed.to_json() }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                user_id,
                format!("user:{user_id}:{key}"),
                serde_json::json!({ "key": key, "value": backed.to_json() }),
            ),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await
}

fn flag_of(flagged: &[(String, bool)], key: &str) -> bool {
    flagged
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| *v)
        .unwrap_or(false)
}

fn org_step_views(
    flagged: &[(String, bool)],
    satisfied: &std::collections::HashMap<String, bool>,
) -> Vec<OnboardingStepView> {
    registry::ONBOARDING_ORG_STEPS
        .iter()
        .map(|step| {
            let done = flag_of(flagged, &registry::onboarding_org_step_key(step, "done"));
            let dismissed = flag_of(
                flagged,
                &registry::onboarding_org_step_key(step, "dismissed"),
            );
            let sat = satisfied.get(*step).copied().unwrap_or(false);
            OnboardingStepView {
                step: step.to_string(),
                done: done || sat,
                dismissed,
                satisfied: sat,
                resolved: done || sat || dismissed,
            }
        })
        .collect()
}

fn user_step_views(
    flagged: &[(String, bool)],
    satisfied: &std::collections::HashMap<String, bool>,
) -> Vec<OnboardingStepView> {
    registry::ONBOARDING_USER_STEPS
        .iter()
        .map(|step| {
            let done = flag_of(flagged, &registry::onboarding_user_step_key(step, "done"));
            let dismissed = flag_of(
                flagged,
                &registry::onboarding_user_step_key(step, "dismissed"),
            );
            let sat = satisfied.get(*step).copied().unwrap_or(false);
            OnboardingStepView {
                step: step.to_string(),
                done: done || sat,
                dismissed,
                satisfied: sat,
                resolved: done || sat || dismissed,
            }
        })
        .collect()
}

/// One summary read: org flow state plus (for admins) every active
/// member's user-flow state. Runs inside the caller's org RLS scope.
async fn summary(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    include_members: bool,
) -> ApiResult<OnboardingSummary> {
    let readiness = core_db::onboarding::org_step_readiness(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let satisfied_org: std::collections::HashMap<String, bool> = registry::ONBOARDING_ORG_STEPS
        .iter()
        .map(|s| (s.to_string(), readiness.satisfied(s)))
        .collect();
    let overrides = core_db::settings::list_org_overrides(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let org_flagged: Vec<(String, bool)> = overrides
        .iter()
        .filter(|o| {
            registry::is_onboarding_org_step_key(&o.key)
                || o.key == registry::ONBOARDING_FLOWS_DISABLED
        })
        .map(|o| (o.key.clone(), o.value.as_bool().unwrap_or(false)))
        .collect();
    let disabled = org_flagged
        .iter()
        .any(|(k, v)| *k == registry::ONBOARDING_FLOWS_DISABLED && *v);
    let has_flags = org_flagged
        .iter()
        .any(|(k, _)| registry::is_onboarding_org_step_key(k));
    let legacy = overrides
        .iter()
        .any(|o| o.key == registry::ONBOARDING_WIZARD_COMPLETED && o.value.as_bool() == Some(true));
    let legacy_completed = legacy && !has_flags;

    let mut org_steps = org_step_views(&org_flagged, &satisfied_org);
    if legacy_completed {
        for s in &mut org_steps {
            s.done = true;
            s.resolved = true;
            s.satisfied = true;
        }
    }

    let mut members = Vec::new();
    if include_members {
        let rows = sqlx::query!(
            r#"SELECT id, display_name, email FROM users
               WHERE org_id = $1 AND is_active = true
               ORDER BY display_name"#,
            org_id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(ApiError::from)?;
        for row in rows {
            let u_overrides = core_db::settings::list_user_overrides(&mut *tx, org_id, row.id)
                .await
                .map_err(ApiError::from)?;
            let user_flagged: Vec<(String, bool)> = u_overrides
                .into_iter()
                .filter(|o| registry::is_onboarding_user_step_key(&o.key))
                .map(|o| (o.key, o.value.as_bool().unwrap_or(false)))
                .collect();
            let mfa_ready = core_db::onboarding::user_mfa_ready(&mut *tx, row.id)
                .await
                .map_err(ApiError::from)?;
            let profile_ready = !row.display_name.is_empty();
            let satisfied_user: std::collections::HashMap<String, bool> = [
                ("profile", profile_ready),
                ("mfa", mfa_ready),
                ("orientation", false),
                ("defaults", false),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
            let steps = user_step_views(&user_flagged, &satisfied_user);
            let unresolved = steps.iter().any(|s| !s.resolved);
            members.push(OnboardingMemberView {
                user_id: row.id,
                display_name: row.display_name,
                email: row.email,
                steps,
                unresolved,
            });
        }
    }

    Ok(OnboardingSummary {
        org: OnboardingScopeView {
            disabled,
            legacy_wizard_completed: legacy_completed,
            steps: org_steps,
        },
        members,
    })
}

// --- endpoints ---------------------------------------------------------------

/// GET /api/onboarding — the caller's effective onboarding state: the org
/// wizard's per-step flags + live gate checks, and (admin callers only)
/// every active member's user-level onboarding state.
#[utoipa::path(operation_id = "onboarding_summary", get, path = "/api/onboarding",
    tag = "onboarding", security(("bearer" = [])),
    responses((status = 200, body = OnboardingSummary)))]
pub async fn get_summary(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<OnboardingSummary>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    let is_admin = require_permission(&authed.principal, admin(Action::Edit)).is_ok();
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let view = summary(&mut tx, org_id, is_admin).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// POST /api/onboarding/org/steps/{step}/complete — record server-gated
/// completion. Owner/admin only (the wizard is an owner/admin flow).
#[utoipa::path(operation_id = "onboarding_org_step_complete", post, path = "/api/onboarding/org/steps/{step}/complete",
    tag = "onboarding", security(("bearer" = [])),
    params(("step" = String, Path, description = "org wizard step id")),
    responses((status = 200, body = OnboardingStepView),
              (status = 403), (status = 409, description = "gate not yet satisfied")))]
pub async fn org_step_complete(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(step): axum::extract::Path<String>,
) -> ApiResult<Json<OnboardingStepView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let (step_id, done_key, _) = org_step_keys(&step)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let satisfied = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let r = core_db::onboarding::org_step_readiness(&mut *tx, org_id)
            .await
            .map_err(ApiError::from)?;
        tx.commit().await.map_err(ApiError::from)?;
        r.satisfied(step_id)
    };
    if !satisfied {
        return Err(ApiError::Conflict(format!(
            "onboarding gate for '{step_id}' is not satisfied yet — the artifact must exist server-side before the step can be completed"
        )));
    }
    write_org_flag(&state, org_id, user_id, done_key, true).await?;
    Ok(Json(OnboardingStepView {
        step: step_id.to_string(),
        done: true,
        dismissed: false,
        satisfied: true,
        resolved: true,
    }))
}

/// POST /api/onboarding/org/steps/{step}/skip — owner/admin records the
/// skip choice. `never: true` persists the dismissed flag ("never show
/// this again"); false is the "skip for now" no-op (the step stays pending
/// and resumable).
#[utoipa::path(operation_id = "onboarding_org_step_skip", post, path = "/api/onboarding/org/steps/{step}/skip",
    tag = "onboarding", security(("bearer" = [])), request_body = OnboardingSkipRequest,
    params(("step" = String, Path, description = "org wizard step id")),
    responses((status = 200, body = OnboardingStepView), (status = 403)))]
pub async fn org_step_skip(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(step): axum::extract::Path<String>,
    Json(req): Json<OnboardingSkipRequest>,
) -> ApiResult<Json<OnboardingStepView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let (step_id, _, dismissed_key) = org_step_keys(&step)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    if req.never {
        write_org_flag(&state, org_id, user_id, dismissed_key, true).await?;
    }
    Ok(Json(OnboardingStepView {
        step: step_id.to_string(),
        done: false,
        dismissed: req.never,
        satisfied: false,
        resolved: req.never,
    }))
}

/// POST /api/onboarding/user/steps/{step}/complete — the caller records
/// progress on their OWN first-run flow. Profile/mfa gates are
/// server-verified; orientation/defaults record on click (they are pure
/// UI milestones by design).
#[utoipa::path(operation_id = "onboarding_user_step_complete", post, path = "/api/onboarding/user/steps/{step}/complete",
    tag = "onboarding", security(("bearer" = [])),
    params(("step" = String, Path, description = "user onboarding step id")),
    responses((status = 200, body = OnboardingStepView),
              (status = 403), (status = 409, description = "gate not yet satisfied")))]
pub async fn user_step_complete(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(step): axum::extract::Path<String>,
) -> ApiResult<Json<OnboardingStepView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (step_id, done_key, _) = user_step_keys(&step)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    let satisfied = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        match step_id {
            "profile" => {
                let r = sqlx::query!(
                    r#"SELECT display_name <> '' AS ok FROM users WHERE id = $1"#,
                    user_id
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from)?;
                r.ok.unwrap_or(false)
            }
            "mfa" => core_db::onboarding::user_mfa_ready(&mut *tx, user_id)
                .await
                .map_err(ApiError::from)?,
            // Orientation + personal defaults are UI milestones: the
            // completion click IS the event (the content was shown).
            _ => true,
        }
    };
    if !satisfied {
        return Err(ApiError::Conflict(format!(
            "onboarding gate for '{step_id}' is not satisfied yet"
        )));
    }
    write_user_flag(&state, org_id, user_id, done_key, true).await?;
    Ok(Json(OnboardingStepView {
        step: step_id.to_string(),
        done: true,
        dismissed: false,
        satisfied: true,
        resolved: true,
    }))
}

/// POST /api/onboarding/user/steps/{step}/skip — self-scoped skip recording
/// (same never/skip-for-now semantics as the org steps).
#[utoipa::path(operation_id = "onboarding_user_step_skip", post, path = "/api/onboarding/user/steps/{step}/skip",
    tag = "onboarding", security(("bearer" = [])), request_body = OnboardingSkipRequest,
    params(("step" = String, Path, description = "user onboarding step id")),
    responses((status = 200, body = OnboardingStepView)))]
pub async fn user_step_skip(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(step): axum::extract::Path<String>,
    Json(req): Json<OnboardingSkipRequest>,
) -> ApiResult<Json<OnboardingStepView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (step_id, _, dismissed_key) = user_step_keys(&step)?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    if req.never {
        write_user_flag(&state, org_id, user_id, dismissed_key, true).await?;
    }
    Ok(Json(OnboardingStepView {
        step: step_id.to_string(),
        done: false,
        dismissed: req.never,
        satisfied: false,
        resolved: req.never,
    }))
}

/// POST /api/onboarding/reset/org — admin re-run of the org wizard: clears
/// every per-step flag (the flows_disabled switch is untouched on purpose;
/// re-running a disabled family stays silent until re-enabled).
#[utoipa::path(operation_id = "onboarding_org_reset", post, path = "/api/onboarding/reset/org",
    tag = "onboarding", security(("bearer" = [])),
    responses((status = 200), (status = 403)))]
pub async fn org_reset(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    cleared_org(&state, org_id, user_id).await
}

#[allow(clippy::needless_range_loop)]
async fn cleared_org(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Json<serde_json::Value>> {
    let removed = crate::org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        let n = core_db::onboarding::reset_org_flags(&mut *ctx.tx, org_id)
            .await
            .map_err(ApiError::from)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                user_id,
                "org:reset".into(),
                serde_json::json!({ "removed": n }),
            ),
        )
        .await?;
        Ok((ctx, n))
    })
    .await?;
    Ok(Json(serde_json::json!({ "cleared": removed })))
}

/// POST /api/onboarding/reset/user — re-run one member's first-run flow
/// (admin re-trigger) or the CALLER's own. Clears that user's onboarding
/// flags so every step resumes pending.
#[utoipa::path(operation_id = "onboarding_user_reset", post, path = "/api/onboarding/reset/user",
    tag = "onboarding", security(("bearer" = [])), request_body = OnboardingTriggerRequest,
    responses((status = 200), (status = 403)))]
pub async fn user_reset(
    State(state): State<AppState>,
    extensions: Extensions,
    body: Option<Json<OnboardingTriggerRequest>>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let (org_id, caller_id) = (authed.principal.user.org_id, authed.principal.user.id);
    let target = body
        .map(|Json(req)| req)
        .and_then(|req| req.user_id)
        .unwrap_or(caller_id);
    let self_reset = target == caller_id;
    if !self_reset {
        require_permission(&authed.principal, admin(Action::Edit))?;
    }
    let removed = crate::org_tx::with_org_tx(&state, org_id, caller_id, |mut ctx| async move {
        let n = core_db::onboarding::reset_user_flags(&mut *ctx.tx, org_id, target)
            .await
            .map_err(ApiError::from)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &audit_event(
                org_id,
                caller_id,
                format!("user:{target}:reset"),
                serde_json::json!({ "target": target, "removed": n, "self": self_reset }),
            ),
        )
        .await?;
        Ok((ctx, n))
    })
    .await?;
    Ok(Json(
        serde_json::json!({ "cleared": removed, "user_id": target }),
    ))
}

/// POST /api/onboarding/disable — org-wide master switch (Settings →
/// Onboarding). True disables every onboarding flow org-wide (both the
/// wizard banner and member first-run); false re-enables — state is kept,
/// so flows resume exactly where they were left.
#[utoipa::path(operation_id = "onboarding_disable", post, path = "/api/onboarding/disable",
    tag = "onboarding", security(("bearer" = [])), request_body = OnboardingDisableRequest,
    responses((status = 200, body = OnboardingDisableResponse), (status = 403)))]
pub async fn disable(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<OnboardingDisableRequest>,
) -> ApiResult<Json<OnboardingDisableResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let (org_id, user_id) = (authed.principal.user.org_id, authed.principal.user.id);
    write_org_flag(
        &state,
        org_id,
        user_id,
        registry::ONBOARDING_FLOWS_DISABLED.to_string(),
        req.disabled,
    )
    .await?;
    Ok(Json(OnboardingDisableResponse {
        disabled: req.disabled,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct OnboardingDisableRequest {
    pub disabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OnboardingDisableResponse {
    pub disabled: bool,
}
