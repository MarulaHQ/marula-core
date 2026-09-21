//! The decision edge (ADR 0010 §1, §5–§6 + ADR 0011 §1–§3): resolve →
//! enabled? granted/configured? caps OK? → compose/validate state →
//! dispatch the WHOLE typed question map in ONE speculative fan-out →
//! typed answers + thresholds → shadow store + usage ledger.
//!
//! Posture: degrade-never-5xx (a quiet decision edge never fails a
//! human workflow — consumers receive a typed [`DecisionQuiet`] reason),
//! deny-by-default (rides the shared AI admission pipeline including the
//! org master switch), audit everything (every run lands in
//! `decision_answers` AND `ai_usage_ledger`).
//!
//! Non-purpose: NO state composition from entity snapshots (consumer
//! lanes compose per the surface's `state_template`), NO UI, no PUBLIC
//! HTTP routes (internal edge, like the embeddings legs — the worker
//! fan-out seam rides the token-guarded /api/internal surface).

use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, AiAdmissionAsk, AiEdge};
use crate::state::AppState;
use core_decision::config::{
    validate_config, ChoiceOptions, QuestionConfig, QuestionKind, SurfaceConfig,
};
use core_decision::provider::{Answer, DecisionRequest, DecisionResponse, Question};
use core_decision::registry::{descriptor, make_provider};
use core_decision::starters::StarterSet;
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Org-keyed provider overrides — TEST support only, same doctrine as
/// chat_runner's `seed_transport_for_org` (deterministic scripted
/// answers, no provider keys or network in tests). Production builds
/// never touch this map; reaching it from prod code is a review red flag.
static PROVIDER_OVERRIDES: std::sync::LazyLock<
    Mutex<HashMap<Uuid, Arc<dyn core_decision::provider::DecisionProvider>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Seeds a deterministic decision provider for one org (test support only).
pub fn seed_provider_for_org(
    org_id: Uuid,
    provider: Arc<dyn core_decision::provider::DecisionProvider>,
) {
    PROVIDER_OVERRIDES
        .lock()
        .expect("provider overrides lock poisoned")
        .insert(org_id, provider);
}

/// Why a decision run went quiet. Consumers (suggestion surfaces, the
/// automation engine, the tool-routing gate) treat EVERY variant the
/// same way: the surface renders/acts exactly as it would without a
/// decision model — no cards, no matches, no shims.
#[derive(Debug, Clone, PartialEq)]
pub enum DecisionQuiet {
    /// Org master switch `ai.features_enabled` is OFF (default).
    FeaturesDisabled,
    /// No config row, the surface is disabled, the decision model is
    /// unpinned, or the pinned pair is not catalog-enabled with the
    /// `decision` capability. The honest "decision model not
    /// configured" posture (ADR 0011 §3).
    Unconfigured,
    /// Deny-by-default: no admin grant covers the caller on a native
    /// decision surface.
    NotGranted,
    /// Hard cap reached / no usable credential / unknown provider kind
    /// — caps bound dispatch with the same degrade semantics as the
    /// completion edge.
    Admission(String),
    /// The composed state exceeds the ~32k budget. Typed refusal, never
    /// truncation (ADR 0010 §2).
    StateBudgetExceeded { estimated_tokens: usize },
    /// Provider 429/529 after backoff, upstream failure, or a protocol
    /// violation. Logged, never surfaced as an error.
    ProviderDown(String),
    /// The org's stored config violates the class rules — defense in
    /// depth (validation runs at config time; stored sets are curated).
    ConfigRefused(String, String),
}

impl DecisionQuiet {
    pub fn reason(&self) -> String {
        match self {
            DecisionQuiet::FeaturesDisabled => "ai_features_disabled".into(),
            DecisionQuiet::Unconfigured => "ai_decision_unconfigured".into(),
            DecisionQuiet::NotGranted => "ai_surface_not_granted".into(),
            DecisionQuiet::Admission(r) => r.clone(),
            DecisionQuiet::StateBudgetExceeded { .. } => "ai_state_budget_exceeded".into(),
            DecisionQuiet::ProviderDown(_) => "ai_decision_provider_down".into(),
            DecisionQuiet::ConfigRefused(r, _) => r.clone(),
        }
    }
}

/// One answered decision run (shadow or live). Answers carry full
/// normalized distributions; consumers gate against the effective
/// config's per-question thresholds themselves (act/review/ignore,
/// ADR 0010 §5).
#[derive(Debug, Clone)]
pub struct DecisionRun {
    pub surface_key: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    /// `shadow` (record only) or `live` (consumers may act above
    /// thresholds) — the org config's posture.
    pub mode: String,
    pub provider: String,
    pub model: String,
    pub response: DecisionResponse,
    /// The effective question map the provider answered (starter fork
    /// or org-customized) — thresholds live here.
    pub config: SurfaceConfig,
}

/// The edge's only entry point. Never panics, never 5xxs: the outer
/// `Err` is infrastructure failure while persisting the audit rows (the
/// run itself already degraded); the inner carries run-or-quiet.
pub struct DecisionAsk<'a> {
    pub org_id: Uuid,
    /// The principal the run is attributed to (bot principals ride the
    /// same ledger/user columns).
    pub user_id: Uuid,
    pub surface_key: &'a str,
    pub entity_type: &'a str,
    pub entity_id: Option<Uuid>,
    /// The composed state for THIS trigger (consumer lanes compose per
    /// the surface template).
    pub state: Value,
    /// Per-edge idempotence/trace id (rides the usage ledger).
    pub request_id: String,
    /// Dynamic Choice inventories by dot-path (org boards, dispatch
    /// queues, the granted tool inventory): path → labeled options.
    /// Tiered trees ride this too for their level-two pass.
    pub inventories: HashMap<String, Vec<String>>,
}

/// Resolve the effective question map for a surface: the org's forked
/// config when customized, else the pinned starter set version.
pub(crate) async fn effective_config(
    tx: &mut PgConnection,
    surface_key: &str,
    cfg: &core_db::decision::DecisionSurfaceConfig,
) -> Result<SurfaceConfig, DecisionQuiet> {
    let config: SurfaceConfig = if cfg.customized {
        let value = cfg.config.as_ref().ok_or(DecisionQuiet::Unconfigured)?;
        serde_json::from_value(value.clone())
            .map_err(|e| DecisionQuiet::ConfigRefused("invalid_config".into(), e.to_string()))?
    } else {
        let schema = core_db::decision::get_question_set(tx, surface_key, cfg.starter_version)
            .await
            .map_err(|e| DecisionQuiet::ProviderDown(e.to_string()))?
            .ok_or(DecisionQuiet::Unconfigured)?;
        let set: StarterSet = serde_json::from_value(schema)
            .map_err(|e| DecisionQuiet::ConfigRefused("invalid_config".into(), e.to_string()))?;
        set.config
    };
    Ok(config)
}

/// Materialize one configured question into the wire `Question`.
/// Dynamic inventories (`FromStatePath`) read the ask's inventory map;
/// tiered trees dispatch their LEVEL-ONE pass here — the second pass
/// (level-two item Choice over one category) is a fresh ~free call the
/// consumer runs when it needs it (ADR 0011 §4).
fn materialize_question(
    q: &QuestionConfig,
    inventories: &HashMap<String, Vec<String>>,
) -> Option<Question> {
    match q.kind {
        QuestionKind::Noul => Some(Question::Noul {
            id: q.id.clone(),
            instructions: q.instructions.clone(),
        }),
        QuestionKind::Score => Some(Question::Score {
            id: q.id.clone(),
            instructions: q.instructions.clone(),
            rubric: q.rubric.clone().unwrap_or_default(),
        }),
        QuestionKind::Choice => match q.options.as_ref()? {
            ChoiceOptions::Fixed { options } => Some(Question::Choice {
                id: q.id.clone(),
                instructions: q.instructions.clone(),
                options: options.clone(),
            }),
            ChoiceOptions::FromStatePath { path } => Some(Question::Choice {
                id: q.id.clone(),
                instructions: q.instructions.clone(),
                options: inventories.get(path)?.clone(),
            }),
            ChoiceOptions::Tiered { categories, .. } => Some(Question::Choice {
                id: format!("{}__level1", q.id),
                instructions: format!(
                    "{instructions} First pick the category.",
                    instructions = q.instructions
                ),
                options: categories.clone(),
            }),
        },
    }
}

/// Answers to the level-one tier question index into the original
/// question id namespace (strip the `__level1` suffix) so consumers
/// gate thresholds against the configured question.
fn rekey_tier_one(answers: Vec<Answer>) -> Vec<Answer> {
    answers
        .into_iter()
        .map(|a| match a {
            Answer::Choice { ref id, .. } if id.ends_with("__level1") => Answer::Choice {
                id: a.id().trim_end_matches("__level1").to_string(),
                picked: match &a {
                    Answer::Choice { picked, .. } => *picked,
                    _ => unreachable!(),
                },
                distribution: match &a {
                    Answer::Choice { distribution, .. } => distribution.clone(),
                    _ => unreachable!(),
                },
                confidence: match &a {
                    Answer::Choice { confidence, .. } => *confidence,
                    _ => unreachable!(),
                },
            },
            other => other,
        })
        .collect()
}

/// `typesafe` (the BYOK/catalog provider name) → `system_one` (the
/// provider-kind). New decision vendors add one mapping arm.
fn provider_kind(provider: &str) -> &'static str {
    match provider {
        "typesafe" => "system_one",
        _ => "unknown",
    }
}

/// Persistable audit shape: per-question full distribution + confidence +
/// the question kind + the resolved Choice option text for the picked
/// index (suggestion-grade readability — the workbench card renders the
/// label, never an index). No prose — the class produces no rationales.
pub fn answers_to_json(
    config: &SurfaceConfig,
    response: &DecisionResponse,
    inventories: &HashMap<String, Vec<String>>,
) -> Value {
    let mut out = serde_json::Map::new();
    for answer in &response.answers {
        let question = config.questions.iter().find(|q| q.id == answer.id());
        let kind = question
            .map(|q| format!("{:?}", q.kind).to_lowercase())
            .unwrap_or_default();
        // Resolve the Choice option labels the provider picked from
        // (fixed options or the inventory behind FromStatePath).
        let options: Option<Vec<String>> = match question.and_then(|q| q.options.as_ref()) {
            Some(ChoiceOptions::Fixed { options }) => Some(options.clone()),
            Some(ChoiceOptions::FromStatePath { path }) => inventories.get(path).cloned(),
            _ => None,
        };
        let entry = match answer {
            Answer::Choice {
                picked,
                distribution,
                confidence,
                ..
            } => {
                let picked_label = options
                    .as_ref()
                    .and_then(|opts| opts.get(*picked))
                    .cloned()
                    .unwrap_or_default();
                json!({
                    "kind": kind,
                    "picked": picked,
                    "picked_label": picked_label,
                    "distribution": distribution,
                    "confidence": confidence,
                })
            }
            Answer::Score {
                level,
                distribution,
                confidence,
                ..
            } => json!({
                "kind": kind,
                "level": level,
                "distribution": distribution,
                "confidence": confidence,
            }),
            Answer::Noul { probability, .. } => json!({
                "kind": kind,
                "probability": probability,
            }),
        };
        out.insert(answer.id().to_string(), entry);
    }
    Value::Object(out)
}

pub async fn decide(
    state: &AppState,
    ask: DecisionAsk<'_>,
) -> ApiResult<Result<DecisionRun, DecisionQuiet>> {
    let DecisionAsk {
        org_id,
        user_id,
        surface_key,
        entity_type,
        entity_id,
        state: composed_state,
        request_id,
        inventories,
    } = ask;

    // --- Phase 1: admission under RLS (no HTTP in this tx) -----------
    let mut tx = crate::routes::ai_policy::org_bound_tx(state, org_id, user_id).await?;

    // 1. Org master switch (also enforced inside the shared pipeline —
    //    this early exit keeps unconfigured orgs entirely silent).
    if !core_db::ai_policy::features_enabled(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(Err(DecisionQuiet::FeaturesDisabled));
    }

    // 2. Org config: enabled surface + pinned decision model.
    let cfg_row = core_db::decision::get_surface_config(&mut *tx, org_id, surface_key)
        .await
        .map_err(ApiError::from)?;
    let Some(cfg_row) = cfg_row else {
        return Ok(Err(DecisionQuiet::Unconfigured));
    };
    if !cfg_row.enabled {
        return Ok(Err(DecisionQuiet::Unconfigured));
    }
    let (Some(model_provider), Some(model_id)) =
        (cfg_row.model_provider.clone(), cfg_row.model_id.clone())
    else {
        return Ok(Err(DecisionQuiet::Unconfigured));
    };
    let mode = cfg_row.mode.clone();

    let config = match effective_config(&mut tx, surface_key, &cfg_row).await {
        Ok(config) => config,
        Err(quiet) => return Ok(Err(quiet)),
    };

    // 3. Class rules re-check (defense in depth) + state budget on the
    //    COMPOSED state — the config-time estimate cannot see content.
    if let Err((reason, detail)) = validate_config(&config) {
        return Ok(Err(DecisionQuiet::ConfigRefused(reason, detail)));
    }
    let state_chars = match serde_json::to_string(&composed_state) {
        Ok(s) => s.len(),
        Err(error) => {
            return Ok(Err(DecisionQuiet::ConfigRefused(
                "invalid_config".into(),
                error.to_string(),
            )))
        }
    };
    let estimated_tokens = state_chars.div_ceil(4);
    if estimated_tokens > core_decision::STATE_BUDGET_TOKENS {
        return Ok(Err(DecisionQuiet::StateBudgetExceeded { estimated_tokens }));
    }

    // 4. Shared admission: catalog (decision capability), caps, sealed
    //    credential — the full pipeline; internal decision surfaces are
    //    core-called (no user-facing grant exists for them).
    let internal = matches!(
        surface_key,
        "decision_search_rerank"
            | "decision_llm_verify"
            | "decision_tool_routing"
            | "decision_loop_termination"
    );
    let call = match authorize_surface_call(
        state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key,
            provider_ask: Some(&model_provider),
            model_ask: Some(&model_id),
            required_capabilities: &[AiEdge::Decision],
            internal_core_call: internal,
        },
    )
    .await
    {
        Ok(call) => call,
        Err(error) => {
            let quiet = match &error {
                ApiError::AiCallRefused { reason, .. } => match *reason {
                    "ai_surface_not_granted" => DecisionQuiet::NotGranted,
                    other => DecisionQuiet::Admission(other.to_string()),
                },
                _ => DecisionQuiet::Admission("ai_admission_failed".into()),
            };
            return Ok(Err(quiet));
        }
    };
    tx.commit().await.map_err(ApiError::from)?;

    // --- Phase 2: the speculative fan-out (NO tx held) ---------------
    let descriptor = descriptor(provider_kind(&model_provider));
    let Some(descriptor) = descriptor else {
        return Ok(Err(DecisionQuiet::Admission(
            "ai_provider_kind_unknown".into(),
        )));
    };
    // Test seam first (org-keyed override), then the real provider.
    let provider: Arc<dyn core_decision::provider::DecisionProvider> = if let Some(seeded) =
        PROVIDER_OVERRIDES
            .lock()
            .expect("provider overrides lock poisoned")
            .get(&org_id)
    {
        Arc::clone(seeded)
    } else {
        let Some(provider) = make_provider(
            descriptor.kind,
            descriptor.default_base_url,
            std::str::from_utf8(&call.plaintext_secret).unwrap_or(""),
            reqwest::Client::new(),
        ) else {
            return Ok(Err(DecisionQuiet::Admission(
                "ai_provider_kind_unknown".into(),
            )));
        };
        Arc::from(provider)
    };

    let questions: Vec<Question> = config
        .questions
        .iter()
        .filter_map(|q| materialize_question(q, &inventories))
        .collect();
    if questions.is_empty() {
        return Ok(Err(DecisionQuiet::ConfigRefused(
            "empty_question_set".into(),
            "no question materialized from the effective config (check inventory paths)".into(),
        )));
    }

    let request = DecisionRequest {
        model: call.model.clone(),
        state: composed_state,
        questions,
    };
    let response = match provider.answer(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                org = %org_id,
                surface = %surface_key,
                %error,
                "decision edge went quiet on provider failure (degrade-never-5xx)"
            );
            return Ok(Err(DecisionQuiet::ProviderDown(error.to_string())));
        }
    };

    // --- Phase 3: audit (usage ledger + shadow store) ----------------
    let mut tx = crate::routes::ai_policy::org_bound_tx(state, org_id, user_id).await?;
    core_db::ai_policy::record_usage(
        &mut tx,
        &core_db::ai_policy::AiUsageNew {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            surface: surface_key.to_string(),
            provider: call.provider.clone(),
            model: call.model.clone(),
            prompt_tokens: response.usage.prompt_tokens as i32,
            completion_tokens: response.usage.completion_tokens as i32,
            cost_weight: call.cost_weight,
            request_id: Some(request_id.clone()),
        },
    )
    .await
    .map_err(ApiError::from)?;
    let answers_json = answers_to_json(&config, &response, &inventories);
    core_db::decision::record_decision_answers(
        &mut tx,
        &core_db::decision::DecisionAnswersNew {
            id: Uuid::now_v7(),
            org_id,
            surface: surface_key.to_string(),
            entity_type: entity_type.to_string(),
            entity_id,
            mode: mode.clone(),
            provider: call.provider.clone(),
            model: call.model.clone(),
            answers: answers_json,
            prompt_tokens: response.usage.prompt_tokens as i32,
            completion_tokens: response.usage.completion_tokens as i32,
            request_id: Some(request_id),
            created_by: Some(user_id),
        },
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Ok(DecisionRun {
        surface_key: surface_key.to_string(),
        entity_type: entity_type.to_string(),
        entity_id,
        mode,
        provider: call.provider,
        model: call.model,
        response: DecisionResponse {
            answers: rekey_tier_one(response.answers),
            usage: response.usage,
        },
        config,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_maps_typesafe_only() {
        assert_eq!(provider_kind("typesafe"), "system_one");
        assert_eq!(provider_kind("openai"), "unknown");
    }

    #[test]
    fn tier_one_answers_rekey_to_the_configured_question() {
        let answers = vec![Answer::Choice {
            id: "tool__level1".into(),
            picked: 1,
            distribution: vec![0.3, 0.7],
            confidence: 0.9,
        }];
        let rekeyed = rekey_tier_one(answers);
        match &rekeyed[0] {
            Answer::Choice { id, .. } => assert_eq!(id, "tool"),
            other => panic!("expected choice, got {other:?}"),
        }
    }

    #[test]
    fn answers_json_carries_full_distributions() {
        let config = SurfaceConfig {
            state_template: core_decision::config::StateTemplate {
                fields: Default::default(),
            },
            questions: vec![QuestionConfig {
                id: "actionable".into(),
                kind: QuestionKind::Noul,
                instructions: "irrelevant".into(),
                criteria: vec![],
                options: None,
                rubric: None,
                thresholds: core_decision::config::Thresholds {
                    act: 0.8,
                    review: 0.5,
                },
            }],
        };
        let response = DecisionResponse {
            answers: vec![Answer::Noul {
                id: "actionable".into(),
                probability: 0.83,
            }],
            usage: Default::default(),
        };
        let json = answers_to_json(&config, &response, &HashMap::new());
        let entry = &json["actionable"];
        assert_eq!(entry["kind"], "noul");
        assert_eq!(entry["probability"], 0.83);
        assert!(entry.get("distribution").is_none());
    }

    #[test]
    fn materialization_uses_fixed_options_and_inventories() {
        use core_decision::config::Thresholds;
        let mut inventories = HashMap::new();
        inventories.insert(
            "org.boards".to_string(),
            vec!["helpdesk".to_string(), "projects".to_string()],
        );
        let fixed = QuestionConfig {
            id: "coverage".into(),
            kind: QuestionKind::Choice,
            instructions: "classify".into(),
            criteria: vec![],
            options: Some(ChoiceOptions::Fixed {
                options: vec!["a".into(), "b".into(), "c".into()],
            }),
            rubric: None,
            thresholds: Thresholds {
                act: 0.8,
                review: 0.5,
            },
        };
        let dynamic = QuestionConfig {
            options: Some(ChoiceOptions::FromStatePath {
                path: "org.boards".into(),
            }),
            ..fixed.clone()
        };
        let tiered = QuestionConfig {
            options: Some(ChoiceOptions::Tiered {
                categories: vec!["x".into(), "y".into()],
                items_from: "org.boards".into(),
            }),
            ..fixed.clone()
        };
        let missing = QuestionConfig {
            options: Some(ChoiceOptions::FromStatePath {
                path: "org.missing".into(),
            }),
            ..fixed.clone()
        };

        assert!(materialize_question(&fixed, &inventories).is_some());
        assert!(materialize_question(&dynamic, &inventories).is_some());
        assert!(materialize_question(&missing, &inventories).is_none());
        let tier = materialize_question(&tiered, &inventories).unwrap();
        match tier {
            Question::Choice { id, options, .. } => {
                assert_eq!(id, "coverage__level1");
                assert_eq!(options, vec!["x".to_string(), "y".to_string()]);
            }
            other => panic!("expected tier-one choice, got {other:?}"),
        }
    }
}
