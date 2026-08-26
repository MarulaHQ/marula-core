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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        }
    }
}

/// A rule matches when all of its conditions hold (empty conditions = match
/// everything on the trigger event).
pub fn matches_all(conditions: &[AutomationCondition], snap: &TicketSnapshot) -> bool {
    conditions.iter().all(|c| c.holds(snap))
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
}
