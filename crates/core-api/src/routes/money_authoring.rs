//! "Describe it" → a quote or contract draft, from a sentence or pasted notes.
//!
//! Shape: one natural-language entry point per record kind, returning the SAME
//! fields the existing create dialogs already carry, so a described record is
//! never a privileged path around the schema. The endpoint persists NOTHING:
//! a person reviews the draft, edits it in the ordinary builder, and saves
//! through the existing create route, which re-validates.
//!
//! The model call rides the shared AI admission path
//! (`ai_policy::authorize_surface_call` + `chat_runner::resolve_transport`)
//! with the `quote_authoring` / `contract_authoring` native surfaces. Those
//! keys are migration-seeded and deny-by-default: no grant means the UI never
//! renders the door and the endpoint refuses with a typed reason.
//!
//! Non-purpose: no pricing invention. A line the notes never priced comes
//! back at zero with an assumption that says so, and the reviewer has to set
//! it. Nothing is written until the ordinary create route is called.

use axum::{extract::State, http::Extensions, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, org_bound_tx, AiAdmissionAsk, AiEdge};
use crate::routes::chat_runner::{self, LoopMessage, ModelCallRequest};
use crate::state::AppState;
use core_db::ai_policy::{record_usage, AiUsageNew};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};

/// The migration-seeded native surface keys. One per record kind, so an admin
/// can grant quotes without granting contracts.
pub const QUOTE_SURFACE: &str = "quote_authoring";
pub const CONTRACT_SURFACE: &str = "contract_authoring";

/// A pasted ledger of meeting notes is far larger than a sentence; the bound
/// keeps one request inside a model context and stops a paste accident from
/// becoming a bill.
const MAX_TRANSCRIPT_CHARS: usize = 12_000;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

// --- wire types -----------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorMoneyRequest {
    /// The words to read: a sentence, a bullet list, or pasted meeting notes.
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteLineDraft {
    pub description: String,
    /// Quantity × 1000 (the product's milli-unit).
    pub quantity_milli: i64,
    /// Unit price in minor currency units (cents).
    pub unit_price_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QuoteDraft {
    pub title: String,
    pub currency: String,
    #[serde(default)]
    pub valid_for_days: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub lines: Vec<QuoteLineDraft>,
    #[serde(default)]
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractDraft {
    pub name: String,
    pub contract_type: String,
    #[serde(default)]
    pub billing_frequency: Option<String>,
    #[serde(default)]
    pub value_minor: Option<i64>,
    pub currency: String,
    #[serde(default)]
    pub term_months: Option<i64>,
    #[serde(default)]
    pub effective_from: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub clauses: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthoringIssue {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthorUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoneyAuthorModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorQuoteResponse {
    pub valid: bool,
    pub saved: bool,
    pub draft: Option<QuoteDraft>,
    pub assumptions: Vec<String>,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorContractResponse {
    pub valid: bool,
    pub saved: bool,
    pub draft: Option<ContractDraft>,
    pub assumptions: Vec<String>,
    pub issues: Vec<MoneyAuthoringIssue>,
    pub usage: MoneyAuthorUsage,
    pub model_info: MoneyAuthorModel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- prompts --------------------------------------------------------------

fn quote_system_prompt() -> String {
    r#"You turn a technician's own words about a job into a QUOTE DRAFT.

Return ONLY one JSON object with this shape:
{
  "title": "short title for the quote",
  "currency": "USD",
  "valid_for_days": 30,
  "notes": "anything the customer should read, or null",
  "lines": [
    {"description": "what is being sold", "quantity_milli": 1000, "unit_price_minor": 10000}
  ],
  "assumptions": ["facts you had to guess"]
}

Rules:
- quantity_milli is quantity x 1000: one unit is 1000, two and a half hours is 2500.
- unit_price_minor is the unit price in cents: $100.00 is 10000.
- NEVER invent a price that was not in the notes. Use 0 and add an assumption
  saying the price needs setting.
- Split distinct work into distinct lines. Keep descriptions short and concrete.
- If the notes name a currency, use it. Otherwise USD and say so in assumptions.
- If the notes are unusable, return {"lines": [], "assumptions": ["..."]}.
- No prose outside the JSON."#
        .to_string()
}

fn contract_system_prompt() -> String {
    r#"You turn a technician's own words about a client agreement into a CONTRACT DRAFT.

Return ONLY one JSON object with this shape:
{
  "name": "short name for the agreement",
  "contract_type": "recurring",
  "billing_frequency": "monthly",
  "value_minor": 250000,
  "currency": "USD",
  "term_months": 12,
  "effective_from": "2026-11-15",
  "notes": "what the agreement covers, or null",
  "clauses": ["short clause the agreement should carry"],
  "assumptions": ["facts you had to guess"]
}

Rules:
- contract_type is one of: recurring, time_and_materials, retainer, fixed_fee,
  per_unit, per_incident.
- billing_frequency is one of: weekly, monthly, quarterly, semiannual, annual,
  or null for a one-off.
- value_minor is the total contract value in cents: $2,500.00 is 250000.
- NEVER invent a number that was not in the notes. Use null and add an
  assumption saying it needs setting.
- effective_from is in the past or today, never a guess at the future.
- Clauses are plain sentences, never legalese you invent. Keep them short.
- No prose outside the JSON."#
        .to_string()
}

/// Pull a JSON object out of a model answer. Providers vary: some wrap it in a
/// fence, some prepend a sentence. The boundary is the outer braces and the
/// parse still has to succeed.
fn extract_json(content: &str) -> Option<Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&content[start..=end]).ok()
}

// --- validation -----------------------------------------------------------

fn validate_quote(draft: &mut QuoteDraft) -> Vec<MoneyAuthoringIssue> {
    let mut issues = Vec::new();
    if draft.title.trim().is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "title".into(),
            message: "the quote needs a title".into(),
        });
    }
    if draft.currency.len() != 3 {
        draft.currency = "USD".into();
    }
    draft.currency = draft.currency.to_uppercase();
    draft
        .lines
        .retain(|line| !line.description.trim().is_empty() && line.quantity_milli > 0);
    if draft.lines.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "lines".into(),
            message: "the notes did not name any billable work".into(),
        });
    }
    let mut extra_assumptions = Vec::new();
    for (index, line) in draft.lines.iter_mut().enumerate() {
        line.description = line.description.trim().to_string();
        if line.unit_price_minor < 0 {
            line.unit_price_minor = 0;
            extra_assumptions.push(format!(
                "Line {} had a negative price, so it is zero until you set it.",
                index + 1
            ));
        }
    }
    draft.assumptions.extend(extra_assumptions);
    if draft.valid_for_days.unwrap_or(0) <= 0 {
        draft.valid_for_days = Some(30);
    }
    issues
}

fn validate_contract(draft: &mut ContractDraft) -> Vec<MoneyAuthoringIssue> {
    const TYPES: [&str; 6] = [
        "recurring",
        "time_and_materials",
        "retainer",
        "fixed_fee",
        "per_unit",
        "per_incident",
    ];
    const FREQUENCIES: [&str; 5] = ["weekly", "monthly", "quarterly", "semiannual", "annual"];
    let mut issues = Vec::new();
    if draft.name.trim().is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "name".into(),
            message: "the agreement needs a name".into(),
        });
    }
    if !TYPES.contains(&draft.contract_type.as_str()) {
        draft.contract_type = "recurring".into();
    }
    if let Some(frequency) = draft.billing_frequency.as_deref() {
        if !FREQUENCIES.contains(&frequency) {
            draft.billing_frequency = Some("monthly".into());
        }
    }
    if draft.currency.len() != 3 {
        draft.currency = "USD".into();
    }
    draft.currency = draft.currency.to_uppercase();
    if draft.value_minor.is_some_and(|v| v < 0) {
        draft.value_minor = None;
        draft
            .assumptions
            .push("The notes gave a negative value, so it needs setting.".into());
    }
    issues
}

// --- one call -------------------------------------------------------------

struct CallOutcome {
    content: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    error: Option<String>,
}

async fn run_call(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    surface_key: &str,
    system: &str,
    transcript: &str,
) -> ApiResult<(CallOutcome, String, String)> {
    let mut tx = org_bound_tx(state, org_id, user_id).await?;
    let call = authorize_surface_call(
        state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key,
            provider_ask: None,
            model_ask: None,
            required_capabilities: &[AiEdge::Completion],
            internal_core_call: false,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let transport = chat_runner::resolve_transport(org_id, &call);
    let request = ModelCallRequest {
        model: Some(call.model.clone()),
        messages: vec![
            LoopMessage {
                role: "system".into(),
                content: system.to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            LoopMessage {
                role: "user".into(),
                content: transcript.to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ],
        tools: Vec::new(),
        response_format: Some(json!({ "type": "json_object" })),
    };
    let (content, prompt_tokens, completion_tokens, error) = match transport.call(&request).await {
        Ok(turn) => (
            turn.content,
            turn.prompt_tokens,
            turn.completion_tokens,
            None,
        ),
        Err(err) => (String::new(), 0, 0, Some(err.to_string())),
    };

    // Usage is recorded whether or not the transport returned content: the
    // provider may have billed a failed turn, and the cap has to see it.
    let mut usage_tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return Ok((
                CallOutcome {
                    content,
                    prompt_tokens,
                    completion_tokens,
                    error,
                },
                call.provider.clone(),
                call.model.clone(),
            ))
        }
    };
    if core_db::set_org_and_user(&mut usage_tx, org_id, user_id)
        .await
        .is_ok()
    {
        let _ = record_usage(
            &mut usage_tx,
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
                request_id: Some(format!("money_author:{}", Uuid::now_v7())),
            },
        )
        .await;
        let _ = core_db::audit::insert(
            &mut *usage_tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Agent(format!("money_authoring/user:{user_id}")),
                action: "ai.money_author".into(),
                entity_type: Some(surface_key.to_string()),
                entity_id: None,
                detail: json!({
                    "surface": call.surface_key,
                    "provider": call.provider,
                    "model": call.model,
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": completion_tokens,
                    "error": error,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await;
        let _ = usage_tx.commit().await;
    }

    Ok((
        CallOutcome {
            content,
            prompt_tokens,
            completion_tokens,
            error,
        },
        call.provider.clone(),
        call.model.clone(),
    ))
}

fn read_transcript(transcript: &str) -> ApiResult<String> {
    let text = transcript.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::Validation("transcript is required".into()));
    }
    if text.chars().count() > MAX_TRANSCRIPT_CHARS {
        return Err(ApiError::Validation(format!(
            "transcript must be at most {MAX_TRANSCRIPT_CHARS} characters"
        )));
    }
    Ok(text)
}

/// POST /api/quotes/author — draft a quote from notes. No write.
#[utoipa::path(operation_id = "quote_author", post, path = "/api/quotes/author",
    tag = "quotes", security(("bearer" = [])), request_body = AuthorMoneyRequest,
    responses((status = 200, body = AuthorQuoteResponse),
              (status = 403, description = "AI admission refused (typed reason in the envelope)")))]
pub async fn author_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorMoneyRequest>,
) -> ApiResult<Json<AuthorQuoteResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let transcript = read_transcript(&req.transcript)?;

    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        QUOTE_SURFACE,
        &quote_system_prompt(),
        &transcript,
    )
    .await?;

    let mut issues = Vec::new();
    let mut draft = extract_json(&outcome.content).and_then(|value| {
        match serde_json::from_value::<QuoteDraft>(value) {
            Ok(draft) => Some(draft),
            Err(error) => {
                issues.push(MoneyAuthoringIssue {
                    field: "draft".into(),
                    message: format!("the model's answer was not a quote draft: {error}"),
                });
                None
            }
        }
    });
    if let Some(draft) = draft.as_mut() {
        issues.extend(validate_quote(draft));
    }
    if draft.is_none() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "draft".into(),
            message: "the model returned no draft".into(),
        });
    }
    let assumptions = draft
        .as_ref()
        .map(|d| d.assumptions.clone())
        .unwrap_or_default();
    let valid = draft.is_some() && issues.is_empty();
    Ok(Json(AuthorQuoteResponse {
        valid,
        saved: false,
        draft,
        assumptions,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
    }))
}

/// POST /api/contracts/author — draft a contract from notes. No write.
#[utoipa::path(operation_id = "contract_author", post, path = "/api/contracts/author",
    tag = "contracts", security(("bearer" = [])), request_body = AuthorMoneyRequest,
    responses((status = 200, body = AuthorContractResponse),
              (status = 403, description = "AI admission refused (typed reason in the envelope)")))]
pub async fn author_contract(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<AuthorMoneyRequest>,
) -> ApiResult<Json<AuthorContractResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let transcript = read_transcript(&req.transcript)?;

    let (outcome, provider, model) = run_call(
        &state,
        org_id,
        user_id,
        CONTRACT_SURFACE,
        &contract_system_prompt(),
        &transcript,
    )
    .await?;

    let mut issues = Vec::new();
    let mut draft = extract_json(&outcome.content).and_then(|value| {
        match serde_json::from_value::<ContractDraft>(value) {
            Ok(draft) => Some(draft),
            Err(error) => {
                issues.push(MoneyAuthoringIssue {
                    field: "draft".into(),
                    message: format!("the model's answer was not a contract draft: {error}"),
                });
                None
            }
        }
    });
    if let Some(draft) = draft.as_mut() {
        issues.extend(validate_contract(draft));
    }
    if draft.is_none() && issues.is_empty() {
        issues.push(MoneyAuthoringIssue {
            field: "draft".into(),
            message: "the model returned no draft".into(),
        });
    }
    let assumptions = draft
        .as_ref()
        .map(|d| d.assumptions.clone())
        .unwrap_or_default();
    let valid = draft.is_some() && issues.is_empty();
    Ok(Json(AuthorContractResponse {
        valid,
        saved: false,
        draft,
        assumptions,
        issues,
        usage: MoneyAuthorUsage {
            prompt_tokens: outcome.prompt_tokens,
            completion_tokens: outcome.completion_tokens,
        },
        model_info: MoneyAuthorModel { provider, model },
        error: outcome.error,
    }))
}

/// The two authoring legs share the quotes and contracts tags, so they mount
/// from one merge line beside those modules.
pub fn routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/quotes/author"),
            axum::routing::post(author_quote),
        )
        .route(
            &format!("{base}/contracts/author"),
            axum::routing::post(author_contract),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fenced_answer_still_parses() {
        let value = extract_json("```json\n{\"title\":\"x\"}\n```").expect("parse");
        assert_eq!(value["title"], "x");
    }

    #[test]
    fn an_unpriced_line_is_kept_at_zero_with_an_assumption() {
        let mut draft = QuoteDraft {
            title: "Onsite visit".into(),
            currency: "usd".into(),
            valid_for_days: Some(30),
            notes: None,
            lines: vec![QuoteLineDraft {
                description: "  Replace switch  ".into(),
                quantity_milli: 1000,
                unit_price_minor: -5,
            }],
            assumptions: Vec::new(),
        };
        let issues = validate_quote(&mut draft);
        assert!(issues.is_empty());
        assert_eq!(draft.currency, "USD");
        assert_eq!(draft.lines[0].description, "Replace switch");
        assert_eq!(draft.lines[0].unit_price_minor, 0);
        assert_eq!(draft.assumptions.len(), 1);
    }

    #[test]
    fn a_quote_with_no_work_is_an_issue_not_a_draft() {
        let mut draft = QuoteDraft {
            title: "Empty".into(),
            currency: "USD".into(),
            valid_for_days: Some(30),
            notes: None,
            lines: Vec::new(),
            assumptions: Vec::new(),
        };
        let issues = validate_quote(&mut draft);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "lines");
    }

    #[test]
    fn an_unknown_contract_type_falls_back_and_says_nothing_it_does_not_know() {
        let mut draft = ContractDraft {
            name: "Managed".into(),
            contract_type: "gold".into(),
            billing_frequency: Some("fortnightly".into()),
            value_minor: None,
            currency: "usd".into(),
            term_months: Some(12),
            effective_from: None,
            notes: None,
            clauses: Vec::new(),
            assumptions: Vec::new(),
        };
        let issues = validate_contract(&mut draft);
        assert!(issues.is_empty());
        assert_eq!(draft.contract_type, "recurring");
        assert_eq!(draft.billing_frequency.as_deref(), Some("monthly"));
        assert_eq!(draft.currency, "USD");
        assert!(draft.value_minor.is_none());
    }
}
