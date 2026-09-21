//! The `decision_loop_termination` harness seam (Wave 23 lane 4, ADR 0011
//! §4 internal surface): the Noul a multi-step agent runner fires at each
//! STEP BOUNDARY — "has the task satisfied its goal criteria?" — enabling
//! early loop exit.
//!
//! Binding invariants (ADR 0011 §4, enforced here and documented for the
//! first multi-step agent execution work that wires this into a runner):
//!
//! 1. **Noul enables exit, never extends the cap.** The hard iteration
//!    cap lives in the runner's harness code and stays the actual
//!    infinite-loop bound. `termination_verdict` answers `CapExhausted`
//!    the moment the cap is reached — even a 0.99 probability cannot
//!    produce an exit verdict past the cap, because the loop is over
//!    regardless.
//! 2. **Threshold high; ≈0.5 routes to review, never acts.** Exit needs
//!    the `goal_satisfied` Noul at/above the effective config's act
//!    threshold (starter v1: act 0.9, review 0.7). Between review and act
//!    the verdict is `RouteToReview` (the runner surfaces the step for a
//!    human); below review it is `Continue`.
//! 3. **Shadow rows drive nothing.** A shadow-mode run records answers
//!    into the append-only store and answers `Continue` — calibration
//!    data, no control.
//! 4. **Unconfigured/downed → the loop runs exactly as it would without
//!    a decision model** (`Quiet`), typed, never an error (ADR 0011 §3).
//!
//! Non-purpose: this module does NOT wire into the chat runner (its loop
//! terminates on the model's own empty tool-call turn; ADR 0011 places
//! loop-termination wiring with the first multi-step agent execution
//! work in the runner). It ships the seam + invariants so that wiring is
//! a call at each step boundary, not a design decision.

use crate::routes::decision_edge::{self, DecisionAsk};
use crate::state::AppState;

/// What the runner does at this step boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopTermination {
    /// Goal criteria satisfied above the act threshold, live mode, cap
    /// not reached: the runner may exit the loop early.
    ExitEarly { probability: f64, act: f64 },
    /// Between the review and act thresholds (≈0.5 territory): surface
    /// the step for human review — never act on it.
    RouteToReview { probability: f64 },
    /// Below review, shadow mode, or quiet: keep iterating under the
    /// runner's own cap discipline.
    Continue,
    /// The hard iteration cap is reached. The loop ends HERE — harness
    /// code owns the bound; the Noul never extends it.
    CapExhausted,
}

/// The pure verdict over one boundary. `steps_completed` counts executed
/// steps BEFORE this boundary; `hard_iteration_cap` is the runner's own
/// bound. Unit-tested for every invariant combination.
pub fn termination_verdict(
    steps_completed: usize,
    hard_iteration_cap: usize,
    mode: &str,
    act: f64,
    review: f64,
    probability: f64,
) -> LoopTermination {
    if steps_completed >= hard_iteration_cap {
        return LoopTermination::CapExhausted;
    }
    if mode != "live" {
        return LoopTermination::Continue;
    }
    if probability >= act {
        LoopTermination::ExitEarly { probability, act }
    } else if probability >= review {
        LoopTermination::RouteToReview { probability }
    } else {
        LoopTermination::Continue
    }
}

/// One step-boundary ask. Composes the state per the surface's starter
/// template: goal.criteria + step.progress_excerpt + the derived
/// `minutes_since_created` (arithmetic happens HERE in code, ADR 0011 §4).
pub struct LoopTerminationAsk<'a> {
    pub org_id: Uuid,
    pub user_id: Uuid,
    /// The goal criteria the agent is executing against (admin/task
    /// owner-authored — criteria stay isolated from untrusted state).
    pub goal_criteria: &'a str,
    /// Excerpt of the progress so far (tool results, step transcript).
    pub progress_excerpt: &'a str,
    /// When the task started (for the derived minutes_since_created).
    pub task_created_at: chrono::DateTime<chrono::Utc>,
    /// How many steps the runner has executed so far.
    pub steps_completed: usize,
    /// The runner's own hard cap — copied into the verdict unchanged.
    pub hard_iteration_cap: usize,
    pub request_id: String,
}

/// Fires the surface at a step boundary. Degrade-never-5xx: every quiet
/// variant maps to [`LoopTermination::Continue`] with the reason logged.
pub async fn check(
    state: &AppState,
    ask: LoopTerminationAsk<'_>,
) -> Result<LoopTermination, crate::error::ApiError> {
    let minutes_since_created = (chrono::Utc::now() - ask.task_created_at).num_minutes();
    let decision_ask = DecisionAsk {
        org_id: ask.org_id,
        user_id: ask.user_id,
        surface_key: "decision_loop_termination",
        entity_type: "agent_step",
        entity_id: None,
        state: serde_json::json!({
            "goal": { "criteria": ask.goal_criteria },
            "step": { "progress_excerpt": ask.progress_excerpt },
            "minutes_since_created": minutes_since_created,
        }),
        request_id: ask.request_id,
        inventories: Default::default(),
    };
    let run = match decision_edge::decide(state, decision_ask).await {
        Ok(Ok(run)) => run,
        Ok(Err(quiet)) => {
            tracing::debug!(
                org = %ask.org_id,
                reason = %quiet.reason(),
                "loop termination quiet — the loop runs on under its own cap"
            );
            return Ok(LoopTermination::Continue);
        }
        Err(err) => {
            tracing::debug!(org = %ask.org_id, %err, "loop termination failed");
            return Ok(LoopTermination::Continue);
        }
    };
    let probability = run.response.answers.iter().find_map(|a| match a {
        core_decision::provider::Answer::Noul { id, probability } if id == "goal_satisfied" => {
            Some(*probability)
        }
        _ => None,
    });
    let question = run
        .config
        .questions
        .iter()
        .find(|q| q.id == "goal_satisfied");
    let Some((probability, question)) = probability.zip(question) else {
        return Ok(LoopTermination::Continue);
    };
    Ok(termination_verdict(
        ask.steps_completed,
        ask.hard_iteration_cap,
        &run.mode,
        question.thresholds.act,
        question.thresholds.review,
        probability,
    ))
}

use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_half_routes_to_review_never_acts() {
        // Starter thresholds: act 0.9, review 0.7. 0.5-ish answers sit
        // below review → Continue; the review band starts above it.
        assert_eq!(
            termination_verdict(2, 8, "live", 0.9, 0.7, 0.5),
            LoopTermination::Continue
        );
        assert_eq!(
            termination_verdict(2, 8, "live", 0.9, 0.7, 0.75),
            LoopTermination::RouteToReview { probability: 0.75 }
        );
    }

    #[test]
    fn exit_requires_live_and_act() {
        assert_eq!(
            termination_verdict(2, 8, "live", 0.9, 0.7, 0.93),
            LoopTermination::ExitEarly {
                probability: 0.93,
                act: 0.9
            }
        );
        // Shadow records the answer but drives nothing.
        assert_eq!(
            termination_verdict(2, 8, "shadow", 0.9, 0.7, 0.99),
            LoopTermination::Continue
        );
    }

    #[test]
    fn the_noul_never_extends_the_hard_cap() {
        // Cap reached: even a certain Noul cannot mint an exit verdict.
        assert_eq!(
            termination_verdict(8, 8, "live", 0.9, 0.7, 0.99),
            LoopTermination::CapExhausted
        );
        assert_eq!(
            termination_verdict(9, 8, "live", 0.9, 0.7, 0.99),
            LoopTermination::CapExhausted
        );
        // One step under the cap still allows the early exit.
        assert!(matches!(
            termination_verdict(7, 8, "live", 0.9, 0.7, 0.95),
            LoopTermination::ExitEarly { .. }
        ));
    }
}
