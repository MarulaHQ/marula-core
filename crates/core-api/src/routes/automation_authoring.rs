//! The rule builder's live draft summary (the private build tracker Wave 43 lane 17b,
//! `p43-automation-builder`, C1).
//!
//! One SSE endpoint that streams a one-sentence summary of the automation
//! rule a person is drafting, as the provider produces it. It is the AI leg
//! that makes the editor read like it understands the rule rather than like
//! it parsed it.
//!
//! # What this is, and what it is not
//!
//! - **The shared seam, not a second path.** The call rides
//!   `ai_policy::authorize_surface_call` with the completion edge — the same
//!   grants, allowlist, caps and sealed credential every other AI edge uses.
//!   That function is also where the org master switch is enforced
//!   (`ai.features_enabled`, ADR 0010 §4), so this module adds no second,
//!   parallel gate: a switch that is OFF refuses here, typed, before any
//!   stream begins.
//! - **A summary, never a write.** The endpoint persists no rule, no
//!   transcript and no draft. The usage ledger row and the provenance audit
//!   event are the only durable facts it produces, and they record the
//!   summary rather than describe the rule.
//! - **`general_chat`, not `automation_writing`.** The seeded
//!   `automation_writing` surface is a PLUGIN surface reserved for the
//!   premium plugin (docs/planning/16 §141: "with the core shipping only the
//!   registry seam"). The web sends `general_chat` because that is the
//!   native surface a core surface may call.
//! - **Degrades typed.** A transport failure after the stream has begun
//!   emits a `summary_failed` frame and closes; it never 5xxs mid-stream,
//!   and a refusal before the stream answers 403 with the machine reason.

use axum::{
    extract::State,
    http::Extensions,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{
    authorize_surface_call, org_bound_tx, AiAdmissionAsk, AiEdge, AuthorizedAiCall,
};
use crate::routes::chat_runner::{self, ChatModelTransport, LoopMessage, ModelCallRequest};
use crate::state::AppState;
use core_db::ai_policy::record_usage;
use core_db::ai_policy::AiUsageNew;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};

/// Admin-surface permission helper, matching the workflow document routes.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// SSE frame names. `summary_delta` carries `delta` — the fragment key the
/// chat runner's `assistant_delta` already uses — and `summary_completed`
/// carries `text`, the whole sentence.
pub const FRAME_SUMMARY_STARTED: &str = "summary_started";
pub const FRAME_SUMMARY_DELTA: &str = "summary_delta";
pub const FRAME_SUMMARY_COMPLETED: &str = "summary_completed";
pub const FRAME_SUMMARY_FAILED: &str = "summary_failed";

#[derive(Debug, Deserialize, ToSchema)]
pub struct AutomationDraftSummaryRequest {
    /// The AI surface the call is admitted on. The web sends `general_chat`,
    /// the native surface. NOT `automation_writing`: that key is a seeded
    /// PLUGIN surface reserved for the premium plugin, and core implements
    /// only the registry seam.
    pub surface_key: String,
    /// The rule's name as typed, when it has one.
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub name: Option<String>,
    pub trigger_event: String,
    /// `ticket` or `board`.
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub subject_kind: Option<String>,
    /// The board the rule is scoped to, when it is not org-wide.
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub scope: Option<String>,
    /// The operator's own read-back of the conditions, e.g. "it has no
    /// assignee and its priority is P1 — critical". The client renders this
    /// sentence from the same clauses its controls show, so the summary is
    /// written from what the person already sees rather than from a second
    /// renderer in Rust that could disagree with it.
    pub conditions_text: String,
    /// The operator's read-back of the actions, in order.
    #[serde(default)]
    pub actions_text: Vec<String>,
    /// How many condition rows are still incomplete. The summary must be
    /// able to say the rule is unfinished rather than describing it as
    /// though it were finished.
    #[serde(default)]
    pub incomplete_conditions: usize,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The system prompt: one sentence, no preamble, no markdown. Capped hard —
/// a summary that runs longer than the rule it summarises is a defect.
pub fn summary_system_prompt() -> String {
    "You summarise an automation rule a support-desk manager is drafting.\n\
     \n\
     Write ONE sentence of at most 35 words that says what the rule does and \
     why it matters to the person who will live with it. Plain English. No \
     preamble, no markdown, no quotes around the whole answer, no restating \
     the field names verbatim where a plainer word exists.\n\
     \n\
     If some conditions are marked incomplete, say plainly that the rule is \
     not finished rather than describing it as though it were.\n\
     \n\
     Output ONLY the sentence.\n"
        .to_string()
}

/// The user prompt carrying the draft. Both texts arrive already phrased,
/// so the model summarises rather than translating a JSON blob.
fn summary_user_prompt(req: &AutomationDraftSummaryRequest) -> String {
    let mut prompt = String::new();
    prompt.push_str(&format!("Trigger: {}\n", req.trigger_event.trim()));
    prompt.push_str(&format!(
        "Subject: {}\n",
        req.subject_kind.as_deref().unwrap_or("ticket")
    ));
    match req.scope.as_deref() {
        Some(scope) => prompt.push_str(&format!("Scope: the {scope} board\n")),
        None => prompt.push_str("Scope: every board\n"),
    }
    if let Some(name) = req.name.as_deref().filter(|n| !n.trim().is_empty()) {
        prompt.push_str(&format!("Rule name (the operator's own words): {name}\n"));
    }
    let conditions = if req.conditions_text.trim().is_empty() {
        String::from("no conditions — it matches every occurrence")
    } else if req.incomplete_conditions > 0 {
        format!(
            "{} (INCOMPLETE: {} condition row{} not finished yet)",
            req.conditions_text.trim(),
            req.incomplete_conditions,
            if req.incomplete_conditions == 1 {
                ""
            } else {
                "s"
            }
        )
    } else {
        req.conditions_text.trim().to_string()
    };
    prompt.push_str(&format!("Conditions: {conditions}\n"));
    if req.actions_text.is_empty() {
        prompt.push_str("Effects: none yet\n");
    } else {
        prompt.push_str("Effects, in order:\n");
        for action in &req.actions_text {
            prompt.push_str(&format!("  - {action}\n"));
        }
    }
    prompt
}

/// Bridges the tokio mpsc receiver into the SSE response's stream — the same
/// shape the chat runner uses, so the frame contract and its plumbing stay
/// one thing rather than two.
struct SummaryStream {
    receiver: tokio::sync::mpsc::Receiver<Result<Event, std::convert::Infallible>>,
}

impl futures_util::Stream for SummaryStream {
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.receiver).poll_recv(cx)
    }
}

async fn emit(
    sender: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    event: &'static str,
    data: serde_json::Value,
) {
    let _ = sender
        .send(Ok(Event::default().event(event).data(data.to_string())))
        .await;
}

/// POST /api/v1/automation-rules/draft-summary — stream a one-sentence
/// summary of the rule draft.
///
/// Admin-gated (Edit). NOT a write: it returns an SSE stream of frames and
/// persists nothing. Refusals answer typed BEFORE any stream begins, so the
/// client branches on the status code and renders a not-available state
/// instead of a dead spinner.
#[utoipa::path(
    operation_id = "automation_draft_summary",
    post,
    path = "/api/automation-rules/draft-summary",
    tag = "automations",
    security(("bearer" = [])),
    request_body = AutomationDraftSummaryRequest,
    responses(
        (status = 200, description = "SSE stream of summary frames (text/event-stream)"),
        (status = 403, description = "typed refusal: ai_surface_not_granted | ai_surface_disabled | ai_model_not_allowed | ai_no_models_configured | ai_hard_cap_reached | ai_no_credential (the org master switch among them)"),
        (status = 400, description = "validation")
    )
)]
pub async fn draft_summary(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AutomationDraftSummaryRequest>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }
    let trigger_event = req.trigger_event.trim().to_string();
    if trigger_event.is_empty() {
        return Err(ApiError::Validation("trigger_event is required".into()));
    }

    // --- admission, on the shared path -------------------------------------
    // `authorize_surface_call` enforces the org master switch
    // (`ai.features_enabled`) first, then grant → model → caps → sealed
    // credential. One gate, typed, before anything streams.
    let mut tx = org_bound_tx(&state, org_id, user_id).await?;
    let call = authorize_surface_call(
        &state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key: &surface_key,
            provider_ask: req.provider.as_deref(),
            model_ask: req.model.as_deref(),
            required_capabilities: &[AiEdge::Completion],
            internal_core_call: false,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    // --- the stream ---------------------------------------------------------
    let transport = chat_runner::resolve_transport(org_id, &call);
    let messages = vec![
        LoopMessage {
            role: "system".into(),
            content: summary_system_prompt(),
            tool_calls: None,
            tool_call_id: None,
        },
        LoopMessage {
            role: "user".into(),
            content: summary_user_prompt(&req),
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let (sender, receiver) =
        tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    tokio::spawn(run_summary_stream(
        state, call, transport, messages, org_id, user_id, sender,
    ));

    let response = Sse::new(SummaryStream { receiver })
        .keep_alive(KeepAlive::default())
        .into_response();
    Ok(response)
}

/// The spawned half: one streamed completion, its frames, and the ledger
/// rows that record it. Transport provenance rides `summary_started` so the
/// client can attribute the summary it just read.
async fn run_summary_stream(
    state: AppState,
    call: AuthorizedAiCall,
    transport: std::sync::Arc<dyn ChatModelTransport>,
    messages: Vec<LoopMessage>,
    org_id: Uuid,
    user_id: Uuid,
    sender: tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
) {
    emit(
        &sender,
        FRAME_SUMMARY_STARTED,
        json!({
            "provider": call.provider,
            "model": call.model,
            "resolution": call.resolution,
            "warnings": call.warnings,
        }),
    )
    .await;

    // The transport's sink is synchronous and the channel send is async, so
    // fragments are collected here and crossed over one at a time — a slow
    // client still sees them arrive in order rather than as one burst.
    let ask = ModelCallRequest {
        model: None,
        messages,
        tools: Vec::new(),
        response_format: None,
    };
    let mut fragments: Vec<String> = Vec::new();
    let sink = &mut |fragment: String| fragments.push(fragment);
    let outcome = transport.stream(&ask, sink).await;

    match outcome {
        Ok(turn) => {
            for fragment in fragments {
                emit(&sender, FRAME_SUMMARY_DELTA, json!({ "delta": fragment })).await;
            }
            emit(
                &sender,
                FRAME_SUMMARY_COMPLETED,
                json!({
                    "text": turn.content,
                    "usage": {
                        "prompt_tokens": turn.prompt_tokens,
                        "completion_tokens": turn.completion_tokens,
                    },
                }),
            )
            .await;
            record_summary_usage(
                &state,
                org_id,
                user_id,
                &call,
                turn.prompt_tokens,
                turn.completion_tokens,
            )
            .await;
        }
        Err(error) => {
            emit(
                &sender,
                FRAME_SUMMARY_FAILED,
                json!({ "reason": error.to_string() }),
            )
            .await;
        }
    }
}

/// Usage ledger + provenance audit for the summary. Same table, same
/// columns, one completion and no tool rows — a summary is a read-shaped
/// call even though the surface is an editor's. A ledger failure must not
/// take the summary down with it: the row is bookkeeping, the sentence is
/// the product.
async fn record_summary_usage(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    call: &AuthorizedAiCall,
    prompt_tokens: u32,
    completion_tokens: u32,
) {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(%error, "draft summary usage ledger begin failed");
            return;
        }
    };
    if core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .is_err()
    {
        return;
    }
    let _ = record_usage(
        &mut tx,
        &AiUsageNew {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            surface: call.surface_key.clone(),
            provider: call.provider.clone(),
            model: call.model.clone(),
            prompt_tokens: prompt_tokens as i32,
            completion_tokens: completion_tokens as i32,
            cost_weight: call.cost_weight,
            request_id: Some(format!("automation_draft_summary:{}", Uuid::now_v7())),
        },
    )
    .await;
    let _ = core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Agent(format!("automation_draft_summary/user:{user_id}")),
            action: "ai.automation_draft_summary".into(),
            entity_type: Some("automation_rule".into()),
            entity_id: None,
            detail: json!({
                "surface": call.surface_key,
                "provider": call.provider,
                "model": call.model,
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await;
    let _ = tx.commit().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_prompt_stays_one_sentence_and_honest_about_incompleteness() {
        let prompt = summary_system_prompt();
        assert!(prompt.contains("ONE sentence"));
        assert!(
            prompt.contains("not finished"),
            "incomplete drafts must not be described as done"
        );

        let req = AutomationDraftSummaryRequest {
            surface_key: "general_chat".into(),
            name: None,
            trigger_event: "ticket.created".into(),
            subject_kind: Some("ticket".into()),
            scope: None,
            conditions_text: "it has no assignee".into(),
            actions_text: vec!["assign to Alice".into()],
            incomplete_conditions: 1,
            provider: None,
            model: None,
        };
        let user = summary_user_prompt(&req);
        assert!(user.contains("INCOMPLETE: 1 condition row not finished yet"));
        assert!(
            user.contains("Scope: every board"),
            "no scope means org-wide, said so"
        );
        assert!(user.contains("- assign to Alice"), "effects ride in order");
        assert!(
            !user.contains("Effects: none yet"),
            "a draft with an effect is not summarised as one that does nothing"
        );
    }

    #[test]
    fn an_incomplete_flag_of_zero_reads_the_conditions_as_they_are() {
        let req = AutomationDraftSummaryRequest {
            surface_key: "general_chat".into(),
            name: Some("Route unassigned work".into()),
            trigger_event: "ticket.created".into(),
            subject_kind: None,
            scope: Some("Support".into()),
            conditions_text: "it has no assignee".into(),
            actions_text: vec![],
            incomplete_conditions: 0,
            provider: None,
            model: None,
        };
        let user = summary_user_prompt(&req);
        assert!(
            !user.contains("INCOMPLETE"),
            "a complete draft is not marked incomplete"
        );
        assert!(user.contains("the Support board"));
        assert!(user.contains("Rule name (the operator's own words): Route unassigned work"));
        assert!(
            user.contains("Effects: none yet"),
            "a draft with no action is summarised as a rule that does nothing yet"
        );
    }
}
