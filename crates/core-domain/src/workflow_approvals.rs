//! The workflow approval gate's decision rule (Wave 55 lane
//! `p55-workflow-approval-binding`).
//!
//! `flow.awaitApproval` is the one parking node that carries IDENTITY: it
//! holds a run until a named person releases it. The gate's pending approval
//! is one `agent_proposals` row, because the queue already carries a decision
//! route, an audit trail and a surface — a second queue would be a second
//! place to look. So the engine has to bind three things together that were
//! previously only assumed:
//!
//! * the run's parked gate and the approval row it filed (`proposal_id`),
//! * the decision event that names that row,
//! * the principal who decided it, against the gate's own approver list.
//!
//! This module is that rule as data and as one predicate. It is pure: the
//! worker evaluates it before it continues a run, and the decision route
//! evaluates the same predicate before it records a decision, so the two
//! cannot drift apart.
//!
//! # An approver entry
//!
//! `flow.awaitApproval.approvers` is an array of strings, each one of:
//!
//! * a user id (UUID) — that member, and only that member;
//! * a builtin role key (`owner`, `admin`, `dispatcher`, `senior_tech`,
//!   `technician`, `bookkeeper`, `auditor`, `read_only`);
//! * a role NAME in the org's role directory (compared case-insensitively, so
//!   a forked custom role works without a second vocabulary).
//!
//! An entry that matches nothing entitles nobody. A gate whose list is empty
//! is refused at run time rather than parked: a run nobody can release is the
//! failure mode this node exists to prevent.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::events::types;

/// The `agent_proposals.entity` value that marks a workflow gate decision.
///
/// A distinct value, not `"workflow_run"`: the queue's other rows are writes
/// an agent proposed, and a gate is not a write — nothing is applied when it
/// is approved. The run the gate belongs to rides `entity_id`.
pub const GATE_ENTITY: &str = "workflow_approval";

/// The lifecycle events that can decide a gate, as one `wake_on` list.
///
/// Built from the event constants rather than spelled out, so the parked
/// row's wake and the emitted event cannot disagree — the drift that made
/// this gate inert in the first place.
#[must_use]
pub fn decision_wake() -> String {
    format!(
        "{},{}",
        types::AGENT_PROPOSAL_APPROVED,
        types::AGENT_PROPOSAL_DECLINED
    )
}

/// True for the two events that can decide a gate.
#[must_use]
pub fn is_decision(event_type: &str) -> bool {
    event_type == types::AGENT_PROPOSAL_APPROVED || event_type == types::AGENT_PROPOSAL_DECLINED
}

/// The gate request an approval row carries in `proposed_value`.
///
/// The approver reads this: who was asked, which gate of which document node,
/// what the run is waiting to do, and what happens at expiry. The engine reads
/// it back instead of trusting the document again, because the row is the
/// record of what the approver actually saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateRequest {
    /// The document node whose gate this approval satisfies.
    pub gate: String,
    /// Who may decide it: user ids and/or role keys or names.
    pub approvers: Vec<String>,
    /// The author's prompt for the approver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The authored expiry. Carried for the approver's benefit; the engine
    /// reads the node's own copy when it computes the wake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// What the run does when the expiry passes: `fail`, `continue`, `route`.
    #[serde(default = "default_on_expiry")]
    pub on_expiry: String,
}

fn default_on_expiry() -> String {
    "continue".to_string()
}

impl GateRequest {
    /// Reads a gate node's params into the shape the approval row carries.
    ///
    /// A missing or empty `approvers` is a typed refusal: the caller fails the
    /// step instead of parking a run no one can release.
    pub fn from_params(gate: &str, params: &Value) -> Result<Self, String> {
        let approvers = approver_list(params.get("approvers"))?;
        Ok(Self {
            gate: gate.to_string(),
            approvers,
            prompt: text(params.get("prompt")),
            timeout: text(params.get("timeout")),
            on_expiry: text(params.get("onExpiry")).unwrap_or_else(default_on_expiry),
        })
    }

    /// Reads the request back from an approval row's `proposed_value`. A row
    /// this build cannot read entitles nobody, and the gate refuses to resume
    /// on it rather than guessing.
    #[must_use]
    pub fn from_row(proposed_value: &Value) -> Option<Self> {
        serde_json::from_value(proposed_value.clone()).ok()
    }

    /// The approvers an approval row names, or an empty list when the row is
    /// not a readable gate request.
    #[must_use]
    pub fn approvers_of(proposed_value: &Value) -> Vec<String> {
        Self::from_row(proposed_value)
            .map(|gate| gate.approvers)
            .unwrap_or_default()
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn approver_list(value: Option<&Value>) -> Result<Vec<String>, String> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| "`approvers` must be an array of user ids or role keys".to_string())?;
    let approvers: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect();
    if approvers.is_empty() {
        return Err("`approvers` names no principal, so no one could decide this gate".to_string());
    }
    Ok(approvers)
}

/// Does this principal satisfy the gate's approver list?
///
/// One predicate, two call sites: the decision route passes the caller's
/// principal, the executor passes the deciding user's effective roles read
/// from the org's role directory. A gate names who may open it, so an
/// owner/admin the gate did not name is NOT entitled — the list is the
/// authority, not the proposal-owner fallback the rest of the queue uses.
#[must_use]
pub fn entitled(
    approvers: &[String],
    user_id: Uuid,
    role_keys: &[String],
    role_names: &[String],
) -> bool {
    approvers.iter().any(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        if let Ok(id) = Uuid::parse_str(entry) {
            return id == user_id;
        }
        role_keys
            .iter()
            .chain(role_names.iter())
            .any(|role| entry.eq_ignore_ascii_case(role))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gate_user() -> Uuid {
        Uuid::from_u128(0xA11C_E000_0000_0000_0000_0000_0000_0001)
    }

    fn gate_role_user() -> Uuid {
        Uuid::from_u128(0xA11C_E000_0000_0000_0000_0000_0000_0002)
    }

    #[test]
    fn the_wake_list_is_built_from_the_lifecycle_constants() {
        let wake = decision_wake();
        assert_eq!(
            wake,
            format!(
                "{},{}",
                types::AGENT_PROPOSAL_APPROVED,
                types::AGENT_PROPOSAL_DECLINED
            )
        );
        assert!(is_decision(types::AGENT_PROPOSAL_APPROVED));
        assert!(is_decision(types::AGENT_PROPOSAL_DECLINED));
        assert!(!is_decision(types::AGENT_PROPOSAL_CREATED));
        assert!(!is_decision(types::AGENT_PROPOSAL_WITHDRAWN));
    }

    #[test]
    fn a_gate_request_round_trips_through_the_row_that_carries_it() {
        let params = json!({
            "approvers": ["technician", format!("{}", gate_user())],
            "timeout": "4h",
            "onExpiry": "fail",
            "prompt": "Release the refund?"
        });
        let gate = GateRequest::from_params("gate-1", &params).expect("gate");
        assert_eq!(gate.gate, "gate-1");
        assert_eq!(gate.approvers.len(), 2);
        assert_eq!(gate.on_expiry, "fail");

        let value = serde_json::to_value(&gate).expect("serialize");
        assert_eq!(value["onExpiry"], json!("fail"));
        assert_eq!(GateRequest::from_row(&value), Some(gate));
        assert_eq!(
            GateRequest::approvers_of(&value),
            vec!["technician".to_string(), gate_user().to_string()]
        );
    }

    #[test]
    fn a_gate_naming_no_one_is_refused_instead_of_parked() {
        let empty = json!({ "approvers": [], "timeout": "1h", "onExpiry": "fail" });
        assert!(GateRequest::from_params("gate-1", &empty).is_err());
        let absent = json!({ "timeout": "1h", "onExpiry": "fail" });
        assert!(GateRequest::from_params("gate-1", &absent).is_err());
        // Blank entries do not count as a principal.
        let blank = json!({ "approvers": ["  "], "timeout": "1h" });
        assert!(GateRequest::from_params("gate-1", &blank).is_err());
    }

    #[test]
    fn a_named_user_entitles_that_user_and_no_one_else() {
        let approvers = vec![gate_user().to_string()];
        assert!(entitled(&approvers, gate_user(), &[], &[]));
        assert!(!entitled(&approvers, gate_role_user(), &[], &[]));
        // A near-miss id is not a match.
        let other = Uuid::from_u128(0xA11C_E000_0000_0000_0000_0000_0000_000F);
        assert!(!entitled(&approvers, other, &[], &[]));
    }

    #[test]
    fn a_named_role_entitles_its_holders_by_key_or_by_directory_name() {
        let by_key = vec!["senior_tech".to_string()];
        // The key is what a builtin role reports; a different key does not count.
        assert!(!entitled(
            &by_key,
            gate_role_user(),
            &["technician".to_string()],
            &[]
        ));
        assert!(entitled(
            &by_key,
            gate_role_user(),
            &["senior_tech".to_string()],
            &[]
        ));
        // A forked role's display name works too, case-insensitively.
        let by_name = vec!["Service Lead".to_string()];
        assert!(entitled(
            &by_name,
            gate_role_user(),
            &[],
            &["service lead".to_string()]
        ));
        // A role the gate did not name does not entitle.
        assert!(!entitled(
            &by_key,
            gate_role_user(),
            &["technician".to_string()],
            &["Technician".to_string()]
        ));
    }

    #[test]
    fn an_unreadable_approver_entry_entitles_nobody() {
        let approvers = vec!["not-a-user-and-not-a-role".to_string()];
        assert!(!entitled(
            &approvers,
            gate_user(),
            &["technician".to_string()],
            &["Technician".to_string()]
        ));
    }
}
