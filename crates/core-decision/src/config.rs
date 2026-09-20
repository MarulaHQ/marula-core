//! The decision surface config model: `state_template` + typed
//! questions + per-answer thresholds (ADR 0010 §2), with ADR 0011 §4's
//! class restrictions enforced at CONFIG TIME by [`validate_config`].
//! A config that violates the class is refused typed before it can be
//! stored, let alone dispatched.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The ~32k-token state + longest-question budget (ADR 0010 §2).
/// chars/4 is the estimate; the refusal is typed, truncation never
/// happens.
pub const STATE_BUDGET_TOKENS: usize = 32_768;
const CHARS_PER_TOKEN: f64 = 4.0;
/// Default per-field cap for template fields without an explicit
/// `max_chars` — keeps a template honest about its own budget.
const DEFAULT_FIELD_MAX_CHARS: usize = 512;

/// Typed config-time refusal reasons. These ride the stored/typed error
/// surface verbatim (no silent normalization, no silent truncation).
pub const CONFIG_REFUSAL_REASONS: &[&str] = &[
    "state_budget_exceeded",
    "raw_temporal_leaf",
    "arithmetic_criteria",
    "criteria_echoes_state",
    "composite_scoring_non_atomic",
    "choice_too_wide",
    "invalid_thresholds",
    "invalid_template_field",
    "empty_question_set",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceConfig {
    pub state_template: StateTemplate,
    pub questions: Vec<QuestionConfig>,
}

/// Dot-path composition of entity snapshots into the vendor's state
/// shape (the `shape_fields` mapping discipline), plus derived fields
/// composed IN CODE (ADR 0011 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateTemplate {
    /// Ordered field name → composition rule. BTreeMap for stable
    /// serialization (byte-stable audits, byte-stable budget checks).
    pub fields: BTreeMap<String, TemplateField>,
}

/// One template field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateField {
    /// Copy `path` (dot-path into the entity snapshot), truncated to
    /// `max_chars` (default 512). Paths naming temporal leaves are
    /// refused — temporal context must be a derived field.
    From {
        path: String,
        max_chars: Option<usize>,
    },
    /// Composed in harness code from the event envelope + org
    /// business-hours/SLA config. The class never sees raw timestamps,
    /// durations, or collections it would have to do math over.
    Derived { name: DerivedField },
    /// Admin-owned constant (string/number/bool only).
    Literal { value: LiteralValue },
}

/// The derived fields Meridian composes in code. Names are part of the
/// starter schemas' contract; temporal math lives here, never in a
/// question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedField {
    AfterHours,
    WithinSlaWindow,
    ElapsedSum,
    MinutesSinceCreated,
    OverageMinutesRemaining,
}

/// JSON-literal constraints for admin constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LiteralValue {
    Text(String),
    Number(f64),
    Flag(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    Choice,
    Score,
    Noul,
}

/// Choice option inventory: a fixed list (≤255), a dot-path to an
/// org-owned taxonomy (boards, tags), or a tiered tree for >255-item
/// inventories (level-one categories + level-two items).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ChoiceOptions {
    Fixed {
        options: Vec<String>,
    },
    FromStatePath {
        path: String,
    },
    Tiered {
        categories: Vec<String>,
        items_from: String,
    },
}

/// Ordered rubric levels (index = severity order, lowest first).
pub type Rubric = Vec<String>;

/// Per-answer thresholds: `act` (high confidence → act, subject to
/// mode), `review` (medium → hold as pending), below review → ignore
/// except as a recorded signal (ADR 0010 §5). Score thresholds index
/// the rubric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub act: f64,
    pub review: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionConfig {
    pub id: String,
    pub kind: QuestionKind,
    /// Admin-owned instruction text. NEVER interpolated with state.
    pub instructions: String,
    /// Admin-owned versioned criteria. NEVER composed from, appended
    /// to, or echoed into untrusted content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ChoiceOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric: Option<Rubric>,
    pub thresholds: Thresholds,
}

fn contains_temporal_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "created_at",
        "updated_at",
        "due_at",
        "started_at",
        "resolved_at",
        "closed_at",
        "first_response_at",
        "_timestamp",
        "timestamp",
        "duration",
        "elapsed",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

/// Arithmetic / multi-step-logic phrasings that a question must not
/// demand (sums, counts, comparisons, elapsed-time or ordering
/// judgments happen in code — ADR 0011 §4).
fn demands_arithmetic(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "sum ",
        "sums",
        "count ",
        "counts",
        "how many",
        "how much",
        "how long",
        "average",
        "total up",
        "add up",
        "subtract",
        "multiply",
        "divide",
        "calculate",
        "compute",
        "arithmetic",
        " sort ",
        "sorted by",
        "ordering of",
        "compare each",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

/// Criteria must be isolated from untrusted state: no interpolation
/// markers, no template field references, no dot-paths — criteria are
/// admin-owned static text only.
fn echoes_state(text: &str, template_field_names: &[&str]) -> bool {
    text.contains("{{")
        || text.contains("}}")
        || text.contains('{')
        || text.contains("state.")
        || template_field_names.iter().any(|name| {
            // A dot-path reference into the composed template
            // (e.g. `subject.body`) — criteria never read state.
            text.contains(&format!("{name}."))
        })
}

/// Config-time enforcement of the ADR 0011 §4 restrictions + the
/// ~32k budget. Returns the typed refusal reason + human detail.
pub fn validate_config(config: &SurfaceConfig) -> Result<(), (String, String)> {
    if config.questions.is_empty() {
        return Err((
            "empty_question_set".into(),
            "a surface config must carry at least one question".into(),
        ));
    }

    // Budget: estimate from the template's own declared caps (never the
    // content — that check repeats at dispatch on the composed state).
    let mut estimated_chars = 0usize;
    for (name, field) in &config.state_template.fields {
        match field {
            TemplateField::From { path, max_chars } => {
                // Non-derived temporal leaves are refused: the class
                // treats dates as plain strings, so temporal context
                // must arrive pre-computed.
                if contains_temporal_marker(name) || contains_temporal_marker(path) {
                    return Err((
                        "raw_temporal_leaf".into(),
                        format!(
                            "template field '{name}' (path '{path}') looks temporal; compose a \
                         derived field (after_hours, within_sla_window, elapsed_sum, \
                         minutes_since_created, overage_minutes_remaining) instead"
                        ),
                    ));
                }
                let cap = max_chars.unwrap_or(DEFAULT_FIELD_MAX_CHARS);
                estimated_chars += name.len() + cap;
            }
            TemplateField::Derived { name: derived } => {
                let _ = derived;
                estimated_chars += name.len() + 16;
            }
            TemplateField::Literal { value } => match value {
                LiteralValue::Text(text) => {
                    if contains_temporal_marker(name) && looks_like_timestamp(text) {
                        return Err((
                            "raw_temporal_leaf".into(),
                            format!(
                                "literal for '{name}' is a timestamp; temporal context must be \
                             pre-computed as a derived field"
                            ),
                        ));
                    }
                    estimated_chars += name.len() + text.len();
                }
                LiteralValue::Number(_) | LiteralValue::Flag(_) => {
                    estimated_chars += name.len() + 8;
                }
            },
        }
    }
    let estimated_tokens = (estimated_chars as f64 / CHARS_PER_TOKEN).ceil() as usize;
    if estimated_tokens > STATE_BUDGET_TOKENS {
        return Err((
            "state_budget_exceeded".into(),
            format!(
                "template's declared caps estimate ~{estimated_tokens} tokens, over the \
                 ~{STATE_BUDGET_TOKENS} state budget — shrink field caps or drop fields \
                 (refusal, never truncation)"
            ),
        ));
    }

    let field_names: Vec<&str> = config
        .state_template
        .fields
        .keys()
        .map(String::as_str)
        .collect();

    for q in &config.questions {
        if q.id.trim().is_empty() {
            return Err(("empty_question_set".into(), "question id is empty".into()));
        }
        // Criteria/instructions hygiene: static admin text only.
        for text in std::iter::once(&q.instructions).chain(q.criteria.iter()) {
            if echoes_state(text, &field_names) {
                return Err((
                    "criteria_echoes_state".into(),
                    format!(
                        "question '{}' carries interpolation markers or state references in \
                         admin-owned text; untrusted content rides state only",
                        q.id
                    ),
                ));
            }
            if demands_arithmetic(text) {
                return Err((
                    "arithmetic_criteria".into(),
                    format!(
                        "question '{}' demands arithmetic/multi-step reasoning; pre-compute \
                         the result in code as an explicit state field",
                        q.id
                    ),
                ));
            }
        }

        let (upper, review, act) = match q.kind {
            QuestionKind::Noul | QuestionKind::Choice => {
                (1.0_f64, q.thresholds.review, q.thresholds.act)
            }
            QuestionKind::Score => {
                let rubric = q.rubric.as_ref().ok_or_else(|| {
                    (
                        "invalid_thresholds".into(),
                        format!("score question '{}' needs a rubric", q.id),
                    )
                })?;
                if rubric.len() < 2 {
                    return Err((
                        "composite_scoring_non_atomic".into(),
                        format!(
                            "score question '{}' has a rubric with fewer than two levels",
                            q.id
                        ),
                    ));
                }
                (
                    rubric.len() as f64 - 1.0,
                    q.thresholds.review,
                    q.thresholds.act,
                )
            }
        };
        if !(0.0..=upper).contains(&review) || !(0.0..=upper).contains(&act) || act < review {
            return Err((
                "invalid_thresholds".into(),
                format!(
                    "question '{}': thresholds (review {review}, act {act}) must satisfy \
                     0 ≤ review ≤ act ≤ {upper}",
                    q.id
                ),
            ));
        }

        match q.kind {
            QuestionKind::Choice => {
                let options = q.options.as_ref().ok_or_else(|| {
                    (
                        "invalid_thresholds".into(),
                        format!("choice question '{}' needs options", q.id),
                    )
                })?;
                match options {
                    ChoiceOptions::Fixed { options } => {
                        if options.len() > 255 {
                            return Err((
                                "choice_too_wide".into(),
                                format!(
                                    "question '{}' carries {} fixed options; Choice >255 must \
                                     be a tiered hierarchical tree (level-one category + \
                                     level-two item)",
                                    q.id,
                                    options.len()
                                ),
                            ));
                        }
                        if options.iter().any(|o| o.trim().is_empty()) {
                            return Err((
                                "invalid_template_field".into(),
                                format!("question '{}' has an empty option label", q.id),
                            ));
                        }
                    }
                    ChoiceOptions::FromStatePath { path } => {
                        // Dynamic inventories CAN exceed 255 at runtime;
                        // tiering is enforced at dispatch — config just
                        // requires the path to name an org-owned list.
                        if path.trim().is_empty() {
                            return Err((
                                "invalid_template_field".into(),
                                format!("question '{}' has an empty options path", q.id),
                            ));
                        }
                    }
                    ChoiceOptions::Tiered {
                        categories,
                        items_from,
                    } => {
                        if categories.is_empty() || items_from.trim().is_empty() {
                            return Err((
                                "invalid_template_field".into(),
                                format!(
                                    "question '{}' tiered tree needs level-one categories and \
                                     an items path",
                                    q.id
                                ),
                            ));
                        }
                    }
                }
            }
            QuestionKind::Score => {
                // Atomic composite scoring: no synthesized
                // "overall severity" style questions (ADR 0011 §4).
                let text = format!("{} {}", q.id, q.instructions).to_lowercase();
                if text.contains("overall") || text.contains("general severity") {
                    return Err((
                        "composite_scoring_non_atomic".into(),
                        format!(
                            "question '{}' looks like a synthesized composite (overall \
                             severity); decompose into atomic questions combined in code",
                            q.id
                        ),
                    ));
                }
            }
            QuestionKind::Noul => {}
        }
    }
    Ok(())
}

fn looks_like_timestamp(text: &str) -> bool {
    // ISO-8601-ish: 2026-09-19T… or 2026-09-19 …
    text.len() >= 10
        && text.as_bytes().get(4) == Some(&b'-')
        && text.as_bytes().get(7) == Some(&b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> StateTemplate {
        StateTemplate {
            fields: [
                (
                    "subject".into(),
                    TemplateField::From {
                        path: "ticket.subject".into(),
                        max_chars: Some(200),
                    },
                ),
                (
                    "after_hours".into(),
                    TemplateField::Derived {
                        name: DerivedField::AfterHours,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn noul_question() -> QuestionConfig {
        QuestionConfig {
            id: "actionable".into(),
            kind: QuestionKind::Noul,
            instructions: "Is this ticket actionable by a technician?".into(),
            criteria: vec!["Marketing spam or out-of-scope requests are not actionable.".into()],
            options: None,
            rubric: None,
            thresholds: Thresholds {
                act: 0.8,
                review: 0.5,
            },
        }
    }

    fn config(questions: Vec<QuestionConfig>) -> SurfaceConfig {
        SurfaceConfig {
            state_template: template(),
            questions,
        }
    }

    #[test]
    fn valid_config_passes() {
        validate_config(&config(vec![noul_question()])).unwrap();
    }

    #[test]
    fn raw_temporal_leaves_are_refused() {
        let mut cfg = config(vec![noul_question()]);
        cfg.state_template.fields.insert(
            "created".into(),
            TemplateField::From {
                path: "ticket.created_at".into(),
                max_chars: None,
            },
        );
        let (reason, _) = validate_config(&cfg).unwrap_err();
        assert_eq!(reason, "raw_temporal_leaf");
    }

    #[test]
    fn budget_overflow_refuses_typed() {
        let mut cfg = config(vec![noul_question()]);
        cfg.state_template.fields.insert(
            "body".into(),
            TemplateField::From {
                path: "ticket.body".into(),
                max_chars: Some(200_000),
            },
        );
        let (reason, detail) = validate_config(&cfg).unwrap_err();
        assert_eq!(reason, "state_budget_exceeded");
        assert!(detail.contains("never truncation"));
    }

    #[test]
    fn arithmetic_criteria_are_refused() {
        let mut q = noul_question();
        q.instructions = "Count how many messages are in the thread and say if actionable.".into();
        let (reason, _) = validate_config(&config(vec![q])).unwrap_err();
        assert_eq!(reason, "arithmetic_criteria");
    }

    #[test]
    fn criteria_echoing_state_is_refused() {
        let mut q = noul_question();
        q.instructions = "If state.subject mentions an outage, answer yes.".into();
        let (reason, _) = validate_config(&config(vec![q])).unwrap_err();
        assert_eq!(reason, "criteria_echoes_state");
    }

    #[test]
    fn choice_over_255_requires_tiering() {
        let q = QuestionConfig {
            id: "board".into(),
            kind: QuestionKind::Choice,
            instructions: "Pick the board".into(),
            criteria: vec![],
            options: Some(ChoiceOptions::Fixed {
                options: (0..256).map(|i| format!("board-{i}")).collect(),
            }),
            rubric: None,
            thresholds: Thresholds {
                act: 0.7,
                review: 0.4,
            },
        };
        let (reason, _) = validate_config(&config(vec![q])).unwrap_err();
        assert_eq!(reason, "choice_too_wide");

        let tiered = QuestionConfig {
            id: "board".into(),
            kind: QuestionKind::Choice,
            instructions: "Pick the board".into(),
            criteria: vec![],
            options: Some(ChoiceOptions::Tiered {
                categories: vec!["internal".into(), "client".into()],
                items_from: "org.boards".into(),
            }),
            rubric: None,
            thresholds: Thresholds {
                act: 0.7,
                review: 0.4,
            },
        };
        validate_config(&config(vec![tiered])).unwrap();
    }

    #[test]
    fn composite_overall_severity_is_refused() {
        let q = QuestionConfig {
            id: "severity".into(),
            kind: QuestionKind::Score,
            instructions: "Give the overall severity of this ticket.".into(),
            criteria: vec![],
            options: None,
            rubric: Some(vec![
                "low".into(),
                "medium".into(),
                "high".into(),
                "critical".into(),
            ]),
            thresholds: Thresholds {
                act: 3.0,
                review: 2.0,
            },
        };
        let (reason, _) = validate_config(&config(vec![q])).unwrap_err();
        assert_eq!(reason, "composite_scoring_non_atomic");
    }

    #[test]
    fn thresholds_must_order() {
        let mut q = noul_question();
        q.thresholds = Thresholds {
            act: 0.3,
            review: 0.6,
        };
        let (reason, _) = validate_config(&config(vec![q])).unwrap_err();
        assert_eq!(reason, "invalid_thresholds");
    }

    #[test]
    fn empty_question_set_is_refused() {
        let (reason, _) = validate_config(&config(vec![])).unwrap_err();
        assert_eq!(reason, "empty_question_set");
    }
}
