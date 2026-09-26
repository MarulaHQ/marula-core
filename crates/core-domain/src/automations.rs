//! Event-triggered automation rules v1 — pure condition matching and
//! effect planning.
//!
//! Purpose: decide, for one domain event against one rule, whether the
//! rule matches (ALL conditions hold; board scope checked by the caller)
//! and what effects it asks core-api to apply. Everything here is pure:
//! the worker parses stored JSONB into these types, and a malformed rule
//! is REJECTED AT THE SERDE BOUNDARY (logged + skipped), never guessed at.
//!
//! Non-purpose: no stream consumption, no HTTP, no storage. Loop safety is
//! coordinated through [`VIA_AUTOMATION_KEY`]: every event emitted by an
//! automation-applied effect carries that marker and evaluators skip it.

use crate::entities::NotificationKind;
use crate::status::{Priority, TicketStatusCategory};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JSONB key marking payloads produced by automation effects. The worker's
/// evaluator drops envelopes carrying `"via_automation": true` so rules can
/// never trigger their own effects recursively.
pub const VIA_AUTOMATION_KEY: &str = "via_automation";

/// True when an event envelope payload was produced by automation effects
/// (loop guard). Unknown shapes are simply not automation echoes.
pub fn is_automation_echo(payload: &serde_json::Value) -> bool {
    payload.get(VIA_AUTOMATION_KEY).and_then(|v| v.as_bool()) == Some(true)
}

/// The immutable facts of a ticket an automation may match against.
#[derive(Debug, Clone, PartialEq)]
pub struct TicketSnapshot {
    pub ticket_id: Uuid,
    pub org_id: Uuid,
    pub board_id: Uuid,
    pub number: String,
    pub title: String,
    pub priority: Priority,
    pub category: TicketStatusCategory,
    pub assignee_id: Option<Uuid>,
}

/// One predicate; a rule matches only when EVERY condition holds.
///
/// Deterministic predicates evaluate against the ticket snapshot; the
/// [`AutomationCondition::AiDecision`] kind is pulled out by
/// [`split_conditions`] and evaluated by the worker against recorded
/// decision answers (ADR 0010 #2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationCondition {
    PriorityIs {
        priority: Priority,
    },
    CategoryIs {
        category: TicketStatusCategory,
    },
    TitleContains {
        needle: String,
    },
    /// Assignee is currently unset.
    Unassigned,
    /// CSAT surveys on resolution (Wave 34 lane 2): fires only on
    /// `csat.response_recorded` events whose score is BELOW the threshold
    /// (strictly less — threshold 3 fires on 1 and 2). Evaluated against
    /// the event payload score, not the ticket snapshot; on any other
    /// trigger event the condition is a no-match.
    CsatScoreBelow { threshold: i16 },
    /// Semantic condition answered by the decision model (ADR 0010 §3
    /// #2): references (surface, question, operator, threshold,
    /// confidence-path) and is evaluated by the worker AGAINST THE
    /// NEWEST RECORDED DECISION ANSWERS for the ticket — never by a
    /// live per-rule fan-out (the one-fan-out-per-trigger doctrine is
    /// owned by the worker decision subsystem). Missing/stale answers
    /// evaluate as NO-MATCH with a warn — the exact skip-with-warn
    /// posture a malformed deterministic rule gets today.
    AiDecision {
        surface: String,
        question: String,
        operator: AiDecisionOperator,
        threshold: f64,
        /// Dot-path into the answer entry overriding the default
        /// `"confidence"` key when the rule gates on a specific path.
        confidence_path: Option<String>,
    },
}

/// How an `ai_decision` condition judges one recorded answer entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AiDecisionOperator {
    /// Noul probability ≥ threshold.
    ProbabilityAbove,
    /// Score rubric level ≥ threshold.
    LevelAtLeast,
    /// Recorded confidence ≥ threshold (any answer kind).
    ConfidenceAbove,
}

impl AiDecisionOperator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProbabilityAbove => "probability_above",
            Self::LevelAtLeast => "level_at_least",
            Self::ConfidenceAbove => "confidence_above",
        }
    }
}

/// The decision answers JSON an `ai_decision` condition reads: one entry
/// per question id, each `{ kind, picked|level|probability,
/// distribution, confidence }` (the audit store's shape, see 0090).
pub fn evaluate_ai_decision(
    answers: &serde_json::Value,
    _surface: &str,
    question: &str,
    operator: AiDecisionOperator,
    threshold: f64,
    confidence_path: Option<&str>,
) -> bool {
    use serde_json::Value as V;
    let Some(entry) = answers.get(question) else {
        return false;
    };
    let numeric = |key: &str| -> Option<f64> {
        match entry.get(key) {
            Some(V::Number(n)) => n.as_f64(),
            Some(V::Array(v)) => Some(v.iter().filter_map(|x| x.as_f64()).sum::<f64>()),
            _ => None,
        }
    };
    let Some(value) = (match operator {
        AiDecisionOperator::ProbabilityAbove => numeric("probability"),
        AiDecisionOperator::LevelAtLeast => numeric("level"),
        AiDecisionOperator::ConfidenceAbove => numeric("confidence"),
    }) else {
        return false;
    };
    if !value.is_finite() || value < threshold {
        return false;
    }
    // Confidence path (optional): when set, the referenced value gates
    // the match too — a confident pick can still be required to ride a
    // specific distribution slice.
    let Some(path) = confidence_path else {
        return true;
    };
    let Some(gate) = path
        .split('.')
        .try_fold(entry, |node, seg| {
            // Array segments index numerically ("distribution.0").
            match seg.parse::<usize>() {
                Ok(index) => node.get(index),
                Err(_) => node.get(seg),
            }
        })
        .and_then(|node| node.as_f64())
    else {
        return false;
    };
    gate.is_finite() && gate >= threshold
}

impl AutomationCondition {
    pub fn holds(&self, snap: &TicketSnapshot) -> bool {
        match self {
            Self::PriorityIs { priority } => *priority == snap.priority,
            Self::CategoryIs { category } => *category == snap.category,
            Self::TitleContains { needle } => {
                needle.is_empty() || snap.title.to_lowercase().contains(&needle.to_lowercase())
            }
            Self::Unassigned => snap.assignee_id.is_none(),
            // Deterministic snapshot matching never satisfies a semantic
            // condition — callers MUST split first (see split_conditions);
            // reaching here unsplit evaluates the AI condition as no-match.
            Self::AiDecision { .. } => false,
            // The CSAT score rides the event payload, not the snapshot —
            // callers MUST go through `csat_score_holds` (the worker does,
            // per its skip-with-warn posture for out-of-band predicates).
            Self::CsatScoreBelow { .. } => false,
        }
    }

    /// The condition's ai_decision spec, when it is one.
    pub fn as_ai_decision(&self) -> Option<AiDecisionSpec<'_>> {
        match self {
            Self::AiDecision {
                surface,
                question,
                operator,
                threshold,
                confidence_path,
            } => Some(AiDecisionSpec {
                surface: surface.as_str(),
                question: question.as_str(),
                operator: *operator,
                threshold: *threshold,
                confidence_path: confidence_path.as_deref(),
            }),
            _ => None,
        }
    }
}

/// One `ai_decision` condition's reference tuple: (surface, question,
/// operator, threshold, confidence-path) — the semantic condition shape
/// the worker evaluates against recorded decision answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiDecisionSpec<'a> {
    pub surface: &'a str,
    pub question: &'a str,
    pub operator: AiDecisionOperator,
    pub threshold: f64,
    pub confidence_path: Option<&'a str>,
}

/// The deterministic snapshot predicates (evaluated first, cheap and
/// pure) and the `ai_decision` specs the worker evaluates ONLY when
/// every deterministic condition already holds — the decision answers
/// read is the expensive leg.
#[derive(Debug, Clone)]
pub struct SplitConditions<'a> {
    pub deterministic: Vec<&'a AutomationCondition>,
    pub semantic: Vec<AiDecisionSpec<'a>>,
}

/// Partitions a rule's conditions: the deterministic snapshot predicates
/// (evaluated first, cheap and pure) and the `ai_decision` specs (worker
/// evaluates them ONLY when every deterministic condition already holds —
/// the decision answers read is the expensive leg).
pub fn split_conditions(conditions: &[AutomationCondition]) -> SplitConditions<'_> {
    let mut deterministic = Vec::new();
    let mut semantic = Vec::new();
    for condition in conditions {
        match condition.as_ai_decision() {
            Some(spec) => semantic.push(spec),
            None => deterministic.push(condition),
        }
    }
    SplitConditions {
        deterministic,
        semantic,
    }
}

/// A rule matches when all of its conditions hold (empty conditions = match
/// everything on the trigger event). Accepts owned slices or the split
/// deterministic part (`Vec<&AutomationCondition>`) from
/// [`split_conditions`].
pub fn matches_all<'a, I>(conditions: I, snap: &TicketSnapshot) -> bool
where
    I: IntoIterator<Item = &'a AutomationCondition>,
{
    conditions.into_iter().all(|c| c.holds(snap))
}

/// Evaluates the rule's `csat_score_below` conditions against the
/// `csat.response_recorded` payload score (`None` = not a CSAT event or a
/// malformed payload — no-match, the standard skip posture). All such
/// conditions must hold; rules without one are unaffected and stay
/// snapshot-driven.
pub fn csat_score_holds(conditions: &[AutomationCondition], score: Option<i16>) -> bool {
    let csat_rules: Vec<i16> = conditions
        .iter()
        .filter_map(|c| match c {
            AutomationCondition::CsatScoreBelow { threshold } => Some(*threshold),
            _ => None,
        })
        .collect();
    if csat_rules.is_empty() {
        return true;
    }
    match score {
        Some(score) => csat_rules.iter().all(|t| score < *t),
        None => false,
    }
}

/// Effects a rule may request. Every variant maps onto an existing core-api
/// write path — automations never gain private write routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationAction {
    AssignUser {
        user_id: Uuid,
    },
    SetPriority {
        priority: Priority,
    },
    AddInternalComment {
        body: String,
    },
    NotifyUsers {
        user_ids: Vec<Uuid>,
        kind: NotificationKind,
        title: String,
    },
}

/// A rule as stored (`automation_rules`) with its JSONB documents still
/// raw. Parsing is a separate, fallible step so one malformed rule can be
/// logged and skipped instead of poisoning a whole evaluation batch.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationRuleRow {
    pub id: Uuid,
    pub org_id: Uuid,
    /// `None` = every board in the org.
    pub board_id: Option<Uuid>,
    pub name: String,
    /// Domain event type from the envelope, e.g. `ticket.created`.
    pub trigger_event: String,
    pub conditions: serde_json::Value,
    pub actions: serde_json::Value,
    pub enabled: bool,
}

/// A validated rule with parsed condition/action documents.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationRule {
    pub id: Uuid,
    pub org_id: Uuid,
    pub board_id: Option<Uuid>,
    pub name: String,
    pub trigger_event: String,
    pub conditions: Vec<AutomationCondition>,
    pub actions: Vec<AutomationAction>,
    pub enabled: bool,
}

impl AutomationRule {
    /// Parses + validates the raw JSONB documents. `Err` carries a stable
    /// reason for the log line; callers MUST treat it as skip-not-fail.
    pub fn parse(row: &AutomationRuleRow) -> Result<Self, String> {
        let conditions: Vec<AutomationCondition> =
            serde_json::from_value(row.conditions.clone())
                .map_err(|e| format!("rule {}: bad conditions jsonb: {e}", row.id))?;
        let actions: Vec<AutomationAction> = serde_json::from_value(row.actions.clone())
            .map_err(|e| format!("rule {}: bad actions jsonb: {e}", row.id))?;
        validate_actions(&actions).map_err(|e| format!("rule {}: {e}", row.id))?;
        Ok(Self {
            id: row.id,
            org_id: row.org_id,
            board_id: row.board_id,
            name: row.name.clone(),
            trigger_event: row.trigger_event.clone(),
            conditions,
            actions,
            enabled: row.enabled,
        })
    }

    /// Board scope check: `None` rules see everything.
    pub fn applies_to_board(&self, board_id: Uuid) -> bool {
        self.board_id.is_none_or(|b| b == board_id)
    }
}

/// Structural validation beyond serde: rejects empty effect payloads that
/// would silently do nothing (or worse, notify nobody while claiming to).
pub fn validate_actions(actions: &[AutomationAction]) -> Result<(), crate::DomainError> {
    use crate::DomainError;
    if actions.is_empty() {
        return Err(DomainError::Validation(
            "automation rule must declare at least one action".into(),
        ));
    }
    for action in actions {
        match action {
            AutomationAction::AddInternalComment { body } if body.trim().is_empty() => {
                return Err(DomainError::Validation(
                    "automation comment body must not be empty".into(),
                ));
            }
            AutomationAction::NotifyUsers {
                user_ids, title, ..
            } => {
                if user_ids.is_empty() {
                    return Err(DomainError::Validation(
                        "automation notification needs at least one recipient".into(),
                    ));
                }
                if title.trim().is_empty() {
                    return Err(DomainError::Validation(
                        "automation notification title must not be empty".into(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> TicketSnapshot {
        TicketSnapshot {
            ticket_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            number: "MER-42".into(),
            title: "Mail server down".into(),
            priority: Priority::P1,
            category: TicketStatusCategory::Open,
            assignee_id: None,
        }
    }

    #[test]
    fn empty_conditions_match_everything() {
        assert!(matches_all(&[], &snapshot()));
    }

    #[test]
    fn conjunction_semantics_hold() {
        let snap = snapshot();
        let both = vec![
            AutomationCondition::PriorityIs {
                priority: Priority::P1,
            },
            AutomationCondition::Unassigned,
        ];
        assert!(matches_all(&both, &snap));
        let one_misses = vec![
            AutomationCondition::PriorityIs {
                priority: Priority::P1,
            },
            AutomationCondition::CategoryIs {
                category: TicketStatusCategory::Closed,
            },
        ];
        assert!(!matches_all(&one_misses, &snap));
    }

    #[test]
    fn title_contains_is_case_insensitive_and_empty_needle_wildcards() {
        let snap = snapshot();
        let c = vec![AutomationCondition::TitleContains {
            needle: "SERVER Down".into(),
        }];
        assert!(matches_all(&c, &snap));
        let wild = vec![AutomationCondition::TitleContains {
            needle: String::new(),
        }];
        assert!(matches_all(&wild, &snap));
    }

    #[test]
    fn conditions_round_trip_through_jsonb_shape() {
        let raw = serde_json::json!([
            { "type": "priority_is", "priority": "p2" },
            { "type": "category_is", "category": "waiting" },
            { "type": "unassigned" },
            { "type": "title_contains", "needle": "vpn" }
        ]);
        let parsed: Vec<AutomationCondition> =
            serde_json::from_value(raw).expect("canonical documents parse");
        assert_eq!(parsed.len(), 4);
    }

    #[test]
    fn malformed_conditions_are_rejected_not_panicked() {
        for garbage in [
            serde_json::json!([{ "type": "teleport", "to": "moon" }]),
            serde_json::json!([{ "type": "priority_is", "priority": "p9" }]),
            serde_json::json!("just a string"),
            serde_json::json!([{ "priority_is": true }]),
        ] {
            assert!(
                serde_json::from_value::<Vec<AutomationCondition>>(garbage).is_err(),
                "malformed condition document must be rejected"
            );
        }
    }

    #[test]
    fn actions_reject_empty_effects() {
        let ok = vec![AutomationAction::SetPriority {
            priority: Priority::P2,
        }];
        assert!(validate_actions(&ok).is_ok());
        assert!(validate_actions(&[]).is_err());
        let empty_comment = vec![AutomationAction::AddInternalComment { body: "  ".into() }];
        assert!(validate_actions(&empty_comment).is_err());
        let nobody = vec![AutomationAction::NotifyUsers {
            user_ids: vec![],
            kind: NotificationKind::TicketUpdate,
            title: "x".into(),
        }];
        assert!(validate_actions(&nobody).is_err());
    }

    #[test]
    fn echo_detection_ignores_unknown_payloads() {
        assert!(is_automation_echo(
            &serde_json::json!({ "via_automation": true })
        ));
        assert!(!is_automation_echo(
            &serde_json::json!({ "via_automation": false })
        ));
        assert!(!is_automation_echo(
            &serde_json::json!({ "ticket_id": "x" })
        ));
        assert!(!is_automation_echo(&serde_json::json!(null)));
    }

    #[test]
    fn rule_parse_rejects_malformed_documents_as_skip_not_fail() {
        let good = AutomationRuleRow {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            board_id: None,
            name: "p1 triage".into(),
            trigger_event: "ticket.created".into(),
            conditions: serde_json::json!([{ "type": "priority_is", "priority": "p1" }]),
            actions: serde_json::json!([
                { "type": "set_priority", "priority": "p2" },
                { "type": "notify_users", "user_ids": ["01890a5d-ac96-774b-bcce-b302099a8057"],
                  "kind": "ticket_update", "title": "routed" }
            ]),
            enabled: true,
        };
        let rule = AutomationRule::parse(&good).expect("valid rule parses");
        assert!(rule.applies_to_board(Uuid::now_v7()));
        let board = Uuid::now_v7();
        let scoped = AutomationRuleRow {
            board_id: Some(board),
            ..good.clone()
        };
        let scoped = AutomationRule::parse(&scoped).expect("scoped rule parses");
        assert!(scoped.applies_to_board(board));
        assert!(!scoped.applies_to_board(Uuid::now_v7()));

        for (label, row) in [
            (
                "conditions garbage",
                AutomationRuleRow {
                    conditions: serde_json::json!("nope"),
                    ..good.clone()
                },
            ),
            (
                "actions garbage",
                AutomationRuleRow {
                    actions: serde_json::json!([{ "type": "detonate" }]),
                    ..good.clone()
                },
            ),
            (
                "empty action list",
                AutomationRuleRow {
                    actions: serde_json::json!([]),
                    ..good
                },
            ),
        ] {
            let err = AutomationRule::parse(&row).expect_err(label);
            assert!(!err.is_empty());
        }
    }

    #[test]
    fn ai_decision_condition_round_trips_through_serde() {
        let raw = serde_json::json!({
            "type": "ai_decision",
            "surface": "decision_ticket_triage",
            "question": "actionable",
            "operator": "probability_above",
            "threshold": 0.8,
            "confidence_path": null
        });
        let condition: AutomationCondition = serde_json::from_value(raw.clone()).unwrap();
        match &condition {
            AutomationCondition::AiDecision {
                surface,
                question,
                operator,
                threshold,
                confidence_path,
            } => {
                assert_eq!(surface, "decision_ticket_triage");
                assert_eq!(question, "actionable");
                assert_eq!(*operator, AiDecisionOperator::ProbabilityAbove);
                assert_eq!(*threshold, 0.8);
                assert!(confidence_path.is_none());
            }
            other => panic!("expected AiDecision, got {other:?}"),
        }
        // Unknown operator is refused at the serde boundary (skip-with-warn).
        let mut bad = raw.clone();
        bad["operator"] = serde_json::json!("vibes_above");
        assert!(serde_json::from_value::<AutomationCondition>(bad).is_err());
        // Round trip.
        assert_eq!(
            serde_json::to_value(&condition).unwrap(),
            serde_json::json!({
                "type": "ai_decision",
                "surface": "decision_ticket_triage",
                "question": "actionable",
                "operator": "probability_above",
                "threshold": 0.8,
                "confidence_path": null
            })
        );
    }

    #[test]
    fn split_conditions_partitions_deterministic_from_semantic() {
        let conditions = vec![
            AutomationCondition::PriorityIs {
                priority: Priority::P1,
            },
            AutomationCondition::AiDecision {
                surface: "decision_ticket_triage".into(),
                question: "actionable".into(),
                operator: AiDecisionOperator::ProbabilityAbove,
                threshold: 0.8,
                confidence_path: None,
            },
        ];
        let split = split_conditions(&conditions);
        assert_eq!(split.deterministic.len(), 1);
        assert_eq!(split.semantic.len(), 1);
        assert!(matches_all(
            split.deterministic.iter().copied(),
            &snapshot()
        ));
    }

    #[test]
    fn evaluate_ai_decision_gates_probability_level_and_confidence() {
        use AiDecisionOperator as Op;
        let answers = serde_json::json!({
            "actionable": { "kind": "noul", "probability": 0.9 },
            "urgency": { "kind": "score", "level": 4 },
            "board": { "kind": "choice", "picked": 0, "confidence": 0.55, "distribution": [0.8, 0.2] }
        });
        assert!(evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "actionable",
            Op::ProbabilityAbove,
            0.8,
            None
        ));
        assert!(!evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "actionable",
            Op::ProbabilityAbove,
            0.95,
            None
        ));
        assert!(evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "urgency",
            Op::LevelAtLeast,
            4.0,
            None
        ));
        assert!(!evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "urgency",
            Op::LevelAtLeast,
            5.0,
            None
        ));
        assert!(evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "board",
            Op::ConfidenceAbove,
            0.5,
            None
        ));
        // Missing question ⇒ no-match (skip-with-warn posture upstream).
        assert!(!evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "missing",
            Op::ProbabilityAbove,
            0.1,
            None
        ));
        // Confidence path gate: exactly at threshold holds.
        assert!(evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "board",
            Op::ConfidenceAbove,
            0.5,
            Some("distribution.0")
        ));
        // Malformed confidence path ⇒ no-match.
        assert!(!evaluate_ai_decision(
            &answers,
            "decision_ticket_triage",
            "board",
            Op::ConfidenceAbove,
            0.5,
            Some("nope.deep")
        ));
    }
}

#[cfg(test)]
mod csat_tests {
    use super::*;

    #[test]
    fn csat_score_below_matches_payload_scores_strictly() {
        let conditions = vec![AutomationCondition::CsatScoreBelow { threshold: 3 }];
        assert!(csat_score_holds(&conditions, Some(1)));
        assert!(csat_score_holds(&conditions, Some(2)));
        assert!(!csat_score_holds(&conditions, Some(3)));
        assert!(!csat_score_holds(&conditions, Some(5)));
        // Malformed payload: no-match, never a false fire.
        assert!(!csat_score_holds(&conditions, None));
        // Rules without a CSAT condition are unaffected.
        assert!(csat_score_holds(&[], None));
        assert!(csat_score_holds(&[], Some(1)));
    }
}
