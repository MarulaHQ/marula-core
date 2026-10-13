//! The workflow run ledger's vocabulary and its retry policy (TASKLIST
//! Wave 43 lane 14, `p43-workflow-run-ledger`).
//!
//! Purpose: the ONE place the run and step statuses, the trigger kinds and
//! the retry decision live, so the executor (lane 15), the operator
//! surface (lane 18) and the SQL CHECKs in migration 0129 cannot drift
//! apart. The statuses below are the closed vocabularies the database
//! CHECKs enforce; `RunStatus::parse` is the only way a stored string
//! becomes a value, and anything else is rejected rather than guessed at.
//!
//! # Why `waiting` is a status and not a comment on `running`
//!
//! A run is either EXECUTING or PARKED. Parking is what `flow.delay`,
//! `flow.waitUntil`, `flow.deadline`, `flow.waitFor`, `flow.waitForAll` and
//! `flow.awaitApproval` all do (lane 15's engine change), and it needs a
//! first-class status because three things have to be true of it at once:
//! it is not consuming a worker, it is due at a known time or on a known
//! event, and it can be RESUMED. `running` says none of that.
//!
//! # The chain rule (A2), stated as an invariant
//!
//! A `workflow_runs` row is ONE EXECUTION ATTEMPT of a workflow document.
//! A workflow's life is a chain of rows:
//!
//! * an envelope starts one (`trigger_kind = event`),
//! * a resume CONTINUES it as a NEW row pointing back (`resumed_from`),
//! * an operator retry RE-RUNS it as a NEW row pointing back (`retry_of`).
//!
//! The parked row is never rewritten into the continuation. Migration
//! 0129's `workflow_runs_ledger_guard` trigger makes that mechanical: the
//! identity, provenance and chain columns are immutable on UPDATE. This
//! module's job is to make sure the writers never TRY.
//!
//! # The write contract lane 15 consumes
//!
//! The executor is the first writer of these rows. In order:
//!
//! 1. `insert_run` — one row, `status = running`, `trigger_kind = event`
//!    (or `manual`), the rule snapshot, the catalogue release, and the
//!    `trigger_job_id` of the `jobs` row that woke it.
//! 2. `insert_step` — one row per step as it STARTS, `status = running`,
//!    with the step's input already written. Never a second row for a
//!    step that is running; a step retried inside the run appends a row
//!    with `attempt + 1` instead.
//! 3. `finish_step` — the step's outcome (succeeded / failed / abandoned /
//!    skipped) with its output and error. Guarded so a step cannot be
//!    finished twice.
//! 4. `park_run` — the run parks: `status = waiting`, the resume cursor,
//!    the wake-at (or the event type it waits on). The queue job that will
//!    wake it is `workflow.resume` ([`WORKFLOW_RESUME_JOB]) — recorded
//!    against the EXISTING queue at `crates/core-db/src/queue.rs`, never a
//!    fork of it.
//! 5. `begin_resumed` — the resume job creates the CONTINUATION: a NEW row,
//!    `status = running`, `resumed_from` = the parked row, cursor inherited.
//!    The parked row is not touched: it reads as the attempt that parked
//!    here, and a derived `superseded` flag (a child exists) is what the
//!    UI follows. Deriving it at read time is the point — a stored flag
//!    would be a mutation of the parked row wearing a different name.
//! 6. `finish_run` — the run's terminal state with its error.
//!
//! A parked run with no child yet is still OWED a wake. The wake ships as a
//! `workflow.resume` job with the right `run_after`, and
//! `idx_workflow_runs_wake` is the partial index a recovery pass reads when a
//! wake job is lost. The lane does not ship that recovery: without a link from
//! the parked row to its pending job, a sweep cannot tell a lost wake from a
//! live one, and resuming a run twice is worse than nudging it once.
//!
//! Non-purpose: no execution, no scheduling, no HTTP. The pure decisions
//! live here; the SQL lives in `core_db::workflow_runs`.

use crate::node_registry;
use serde::{Deserialize, Serialize};

/// The job kind a parked or retried run is woken by. Recorded against the
/// EXISTING queue (`crates/core-db/src/queue.rs:20`) rather than a fork of
/// it, which means the queue runner's `settle_job` must learn this kind or
/// the job dead-letters as "unknown job kind" and parked runs sleep
/// forever. The executor lane owns adding it there; the constant is
/// declared here so both sides spell it the same way.
pub const WORKFLOW_RESUME_JOB: &str = "workflow.resume";

/// The setting key holding the run-retention window (declared in the typed
/// settings registry; see [`RETENTION_DEFAULT_DAYS`] for the default).
pub const RUN_RETENTION_KEY: &str = "workflow.run_retention_days";

/// How long a finished run is kept before the purge drops it. Runs are the
/// highest-volume rows this engine produces, so the window is a SETTING
/// (org-scope, admin-editable) rather than an unbounded table, and 30 days
/// is the default an untouched org gets. Long enough to inspect a failure
/// a week later, short enough that the ledger stays a working surface
/// rather than an archive nobody can page through.
pub const RETENTION_DEFAULT_DAYS: i64 = 30;

/// The retention bounds the setting's declaration enforces: a week keeps a
/// ledger too short to answer "what happened last month", and a decade is
/// an unbounded table with extra steps.
pub const RETENTION_MIN_DAYS: i64 = 7;
pub const RETENTION_MAX_DAYS: i64 = 3650;

/// Resolves the effective retention window for one org: the org's override
/// when it set one, the declared default otherwise. Out-of-range values
/// fall back to the default rather than extending the table unbounded.
pub fn retention_days(override_days: Option<i64>) -> i64 {
    match override_days {
        Some(days) if (RETENTION_MIN_DAYS..=RETENTION_MAX_DAYS).contains(&days) => days,
        _ => RETENTION_DEFAULT_DAYS,
    }
}

/// What one execution attempt is doing. `Waiting` is the parked state and
/// is as terminal-as-nothing: it is waiting for a clock or an event, not
/// for a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(as = WorkflowRunStatus))]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Executing right now.
    Running,
    /// Parked: the run carries a resume cursor and a wake-at (or the event
    /// type it waits on).
    Waiting,
    /// Finished clean.
    Succeeded,
    /// Broke. The per-step error says where; this says that the run did.
    Failed,
    /// The workflow chose to stop (`flow.abandon`) — a decision, not a
    /// break. Distinct from `Failed` so "we chose not to dispatch" never
    /// reads like "the dispatch broke".
    Abandoned,
}

impl RunStatus {
    /// The closed vocabulary, as the SQL CHECK spells it.
    pub const ALL: [RunStatus; 5] = [
        RunStatus::Running,
        RunStatus::Waiting,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Abandoned,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Waiting => "waiting",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Abandoned => "abandoned",
        }
    }

    /// Parses a stored status. Unknown values are `None`, never a guess:
    /// a status this build does not know is a build drift, and reading it
    /// as "running" would hide a run.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// True for the three states a run stops moving in.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Abandoned
        )
    }

    /// True when an operator may retry the run: it STOPPED and it did not
    /// stop clean. A `running` run has nothing to retry (it is going), a
    /// `waiting` run should be waited for rather than duplicated, and a
    /// `succeeded` run is not a failure to re-run.
    pub const fn is_retryable(self) -> bool {
        matches!(self, RunStatus::Failed | RunStatus::Abandoned)
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one step attempt is doing. Wider than the run's because a step can
/// be SKIPPED: a branch not taken is a recorded fact, not a step that
/// never happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(as = WorkflowStepStatus))]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Running,
    /// Parked inside the run (`flow.waitFor` as a step): the run it belongs
    /// to is `waiting` too, but the step's own state reads the same way.
    Waiting,
    Succeeded,
    Failed,
    Abandoned,
    /// Not executed: the branch was not taken. Terminal, and honest.
    Skipped,
}

impl StepStatus {
    pub const ALL: [StepStatus; 6] = [
        StepStatus::Running,
        StepStatus::Waiting,
        StepStatus::Succeeded,
        StepStatus::Failed,
        StepStatus::Abandoned,
        StepStatus::Skipped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Running => "running",
            StepStatus::Waiting => "waiting",
            StepStatus::Succeeded => "succeeded",
            StepStatus::Failed => "failed",
            StepStatus::Abandoned => "abandoned",
            StepStatus::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// True when the step finished clean.
    pub const fn succeeded(self) -> bool {
        matches!(self, StepStatus::Succeeded)
    }

    /// True when running the step AGAIN may apply something twice.
    ///
    /// This is the question the retry gate actually asks, and it is wider
    /// than "did it succeed": a step that FAILED may have applied its
    /// action and lost the response (an email sent, a provider that never
    /// answered), and a step still marked `running` is one the executor
    /// died in the middle of. Both repeat the same way.
    ///
    /// The two states that do NOT count are the two where the action
    /// demonstrably did not happen: `skipped` is a branch not taken, and
    /// `waiting` is a step the continuation RESUMES rather than repeats.
    pub const fn may_have_applied(self) -> bool {
        !matches!(self, StepStatus::Waiting | StepStatus::Skipped)
    }
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a run row exists. The two chain kinds are pinned to their pointer
/// in the database (`workflow_runs_trigger_shape`), so a row can never
/// claim to be a resume without naming the run it continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(as = WorkflowRunTrigger))]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// An envelope matched the rule.
    Event,
    /// A parked run woke and continued as this row.
    Resume,
    /// An operator re-ran a failed run as this row.
    OperatorRetry,
    /// An operator ran the document directly.
    Manual,
}

impl RunTrigger {
    pub const ALL: [RunTrigger; 4] = [
        RunTrigger::Event,
        RunTrigger::Resume,
        RunTrigger::OperatorRetry,
        RunTrigger::Manual,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RunTrigger::Event => "event",
            RunTrigger::Resume => "resume",
            RunTrigger::OperatorRetry => "operator_retry",
            RunTrigger::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == value)
    }
}

/// How far back a retry re-executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(as = WorkflowRetryMode))]
#[serde(rename_all = "snake_case")]
pub enum RetryMode {
    /// Re-execute from the first step that did not succeed. The steps
    /// before it are not run again, so what already applied stays applied.
    #[default]
    Resume,
    /// Re-execute the whole document from the top. Only legal when every
    /// step that already succeeded is safe to repeat.
    FromStart,
}

impl RetryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RetryMode::Resume => "resume",
            RetryMode::FromStart => "from_start",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "resume" => Some(RetryMode::Resume),
            "from_start" => Some(RetryMode::FromStart),
            _ => None,
        }
    }
}

/// The step facts the retry policy needs. Everything else about a step is
/// the executor's business; the decision only cares about what it would
/// have to run again.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRetryInput {
    pub step_index: i32,
    pub node_id: String,
    /// The step's input — the params `retry_refusal` resolves the node's
    /// idempotence against.
    pub input: serde_json::Value,
    pub status: StepStatus,
}

/// Why a retry was refused. Typed so the HTTP surface answers with a
/// machine-readable reason (IF3) instead of prose a client has to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunLedgerError {
    /// The run is not in a state an operator may re-run: it is still going,
    /// it is parked, or it already succeeded.
    NotRetryable { status: RunStatus },
    /// Re-running would repeat a step that declares itself unsafe to
    /// repeat. `nodes` names every offending node so the refusal is
    /// actionable rather than a shrug.
    RetryRefused { nodes: Vec<String> },
}

impl std::fmt::Display for RunLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunLedgerError::NotRetryable { status } => write!(
                f,
                "a {status} run cannot be retried; only a failed or abandoned run can"
            ),
            RunLedgerError::RetryRefused { nodes } => write!(
                f,
                "retrying would repeat steps that declare idempotent=false: {}",
                nodes.join(", ")
            ),
        }
    }
}

/// What a retry decided, so the writer does not have to work it out again.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPlan {
    pub mode: RetryMode,
    /// The step position the retry starts at. Everything before it already
    /// succeeded and is NOT re-executed.
    pub start_at_step: i32,
    /// The steps the retry will re-execute, in order.
    pub reexecuted: Vec<i32>,
}

/// Decides whether a failed run may be retried and where the retry starts.
///
/// The rule that makes this worth a function instead of a flag: **re-running
/// a failed run must not silently double-apply the steps that already
/// succeeded.** Two different things can go wrong, and both are checked
/// here against lane 13's per-node `idempotent` flag:
///
/// 1. the FAILED step itself may have half-applied — an email sent and a
///    provider that never answered are the same email twice — so repeating
///    it is exactly the double-apply a non-idempotent node forbids;
/// 2. the steps AFTER the failure may have applied too: a sibling branch
///    can finish while another one is failing, so re-running from the
///    failure point re-executes them as well.
///
/// Both cases reduce to one question: does the re-execution set contain a
/// step that MAY HAVE APPLIED something and whose node declares
/// `idempotent=false`? (`StepStatus::may_have_applied` is deliberately
/// wider than "succeeded" — see its note.) If it does, the retry is
/// REFUSED and the offending nodes are named. A node this build cannot
/// vouch for — an unknown id, or a generic `domain.update` whose
/// `operation` nobody resolved — is treated as NOT safe to repeat, which is
/// the same decision migration 0128 made: an undeclared action's
/// retry-safety is unknown, so it must say so rather than assume. The
/// generic node is not simply refused, though: lane 13 resolves it through
/// the operation its params name, so it is repeatable as a state setter and
/// not as a creation, and fails closed only when the operation is missing.
pub fn retry_plan(
    run_status: RunStatus,
    steps: &[StepRetryInput],
    mode: RetryMode,
) -> Result<RetryPlan, RunLedgerError> {
    if !run_status.is_retryable() {
        return Err(RunLedgerError::NotRetryable { status: run_status });
    }

    // Steps in execution order: the policy never trusts the caller's
    // ordering, because "which step came first" IS the decision.
    let mut ordered: Vec<&StepRetryInput> = steps.iter().collect();
    ordered.sort_by_key(|s| s.step_index);

    let start_at_step = match mode {
        RetryMode::Resume => match ordered.iter().find(|s| !s.status.succeeded()) {
            Some(step) => step.step_index,
            // Everything succeeded: nothing to resume from. Re-running the
            // whole document is the only honest reading of "retry".
            None => ordered.first().map(|s| s.step_index).unwrap_or(0),
        },
        RetryMode::FromStart => ordered.first().map(|s| s.step_index).unwrap_or(0),
    };

    let reexecuted: Vec<i32> = ordered
        .iter()
        .filter(|s| s.step_index >= start_at_step)
        .map(|s| s.step_index)
        .collect();

    // The double-apply gate, over every step the retry would run again
    // whose action may already have been applied — succeeded, failed, or
    // left mid-flight. All three repeat the same way.
    let mut refused: Vec<String> = Vec::new();
    for step in ordered
        .iter()
        .filter(|s| s.step_index >= start_at_step && s.status.may_have_applied())
    {
        if !is_safe_to_repeat(&step.node_id, &step.input) {
            refused.push(step.node_id.clone());
        }
    }

    if refused.is_empty() {
        Ok(RetryPlan {
            mode,
            start_at_step,
            reexecuted,
        })
    } else {
        refused.sort();
        refused.dedup();
        Err(RunLedgerError::RetryRefused { nodes: refused })
    }
}

/// True when repeating this node's action is safe. Lane 13's
/// `retry_refusal` answers for every declared node: the Tier A wrappers
/// through their own flag, the generic `domain.update` through the
/// `operation` its params name, and anything unknown or unresolved by
/// failing closed — because an action nobody declared is one nobody proved
/// repeatable, and an unresolvable one is the same thing.
fn is_safe_to_repeat(node_id: &str, params: &serde_json::Value) -> bool {
    node_registry::retry_refusal(node_id, params).is_ok()
}

/// The cursor a retry writes into the run it creates: the documented shape
/// the executor reads to know where to start. Kept here so the writer and
/// the reader agree on the field names without a second contract.
pub fn retry_cursor(plan: &RetryPlan) -> serde_json::Value {
    serde_json::json!({
        "start_at_step": plan.start_at_step,
        "mode": plan.mode.as_str(),
        "steps": plan.reexecuted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(index: i32, node_id: &str, status: StepStatus) -> StepRetryInput {
        StepRetryInput {
            step_index: index,
            node_id: node_id.to_string(),
            input: json!({}),
            status,
        }
    }

    #[test]
    fn statuses_round_trip_through_their_storage_strings() {
        for status in RunStatus::ALL {
            assert_eq!(RunStatus::parse(status.as_str()), Some(status), "{status}");
            assert_eq!(status.to_string(), status.as_str());
        }
        for status in StepStatus::ALL {
            assert_eq!(StepStatus::parse(status.as_str()), Some(status), "{status}");
        }
        for trigger in RunTrigger::ALL {
            assert_eq!(RunTrigger::parse(trigger.as_str()), Some(trigger));
        }
        // An unknown status is a build drift, never a guess.
        assert_eq!(RunStatus::parse("parked"), None);
        assert_eq!(StepStatus::parse("queued"), None);
        assert_eq!(RunTrigger::parse("cron"), None);
    }

    #[test]
    fn only_a_stopped_and_unclean_run_is_retryable() {
        assert!(RunStatus::Failed.is_retryable());
        assert!(RunStatus::Abandoned.is_retryable());
        assert!(!RunStatus::Running.is_retryable());
        assert!(!RunStatus::Waiting.is_retryable());
        assert!(!RunStatus::Succeeded.is_retryable());
        assert!(RunStatus::Succeeded.is_terminal());
        assert!(!RunStatus::Waiting.is_terminal());
    }

    #[test]
    fn a_run_still_going_is_not_retryable() {
        let steps = vec![step(0, "ticket.create", StepStatus::Failed)];
        let refusal = retry_plan(RunStatus::Running, &steps, RetryMode::Resume)
            .expect_err("a running run has nothing to re-run");
        assert_eq!(
            refusal,
            RunLedgerError::NotRetryable {
                status: RunStatus::Running
            }
        );
        let parked = retry_plan(RunStatus::Waiting, &steps, RetryMode::FromStart)
            .expect_err("a parked run should be waited for, not duplicated");
        assert_eq!(
            parked,
            RunLedgerError::NotRetryable {
                status: RunStatus::Waiting
            }
        );
    }

    #[test]
    fn a_resume_retry_starts_at_the_first_step_that_did_not_succeed() {
        // `ticket.update` is a first-party STATE SETTER the registry
        // declares idempotent-safe, so this workflow retries cleanly.
        let steps = vec![
            step(0, "ticket.create", StepStatus::Succeeded),
            step(1, "ticket.update", StepStatus::Failed),
            step(2, "ticket.update", StepStatus::Running),
        ];
        let plan =
            retry_plan(RunStatus::Failed, &steps, RetryMode::Resume).expect("a safe retry plans");
        assert_eq!(plan.start_at_step, 1);
        assert_eq!(plan.reexecuted, vec![1, 2]);
        // The cursor names where to start, in the shape the executor reads.
        assert_eq!(retry_cursor(&plan)["start_at_step"], json!(1));
        assert_eq!(retry_cursor(&plan)["mode"], json!("resume"));
    }

    #[test]
    fn a_retry_is_refused_when_it_would_repeat_a_non_idempotent_step() {
        // `ticket.create` is a CREATION: the registry declares it
        // idempotent=false, so a from-start retry would mint a second
        // ticket. `ticket.update` is safe to repeat, so the refusal names
        // exactly the offending node and nothing else.
        let steps = vec![
            step(0, "ticket.create", StepStatus::Succeeded),
            step(1, "ticket.update", StepStatus::Failed),
        ];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::FromStart)
            .expect_err("re-running a creation double-applies it");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["ticket.create".to_string()]
            }
        );
        // The same run resumes cleanly: the creation is NOT re-executed,
        // and the failed update is safe to repeat.
        let plan = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect("resuming past the creation is safe");
        assert_eq!(plan.reexecuted, vec![1]);
    }

    #[test]
    fn a_failed_step_that_may_have_half_applied_blocks_the_resume() {
        // The step that failed is itself non-idempotent: an email send
        // whose response was lost may already have sent. Re-running it
        // without a human looking would be a second email.
        let steps = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "email.send", StepStatus::Failed),
        ];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect_err("a failed send may already have sent");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["email.send".to_string()]
            }
        );
    }

    #[test]
    fn a_succeeded_step_after_the_failure_point_also_blocks_the_resume() {
        // The branch that failed took a sibling branch down with it: step
        // 2 already applied while step 1 was failing, so resuming at 1
        // re-runs it. That is the double-apply a plain "restart from the
        // failure" retry would hide.
        let steps = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "ticket.update", StepStatus::Failed),
            step(2, "email.send", StepStatus::Succeeded),
        ];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect_err("the succeeded send would be repeated");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["email.send".to_string()]
            }
        );
    }

    #[test]
    fn a_second_failed_step_in_the_reexecution_set_is_gated_too() {
        // Two sends both failed. Resuming at the first would re-run the
        // second, which may already have applied — so the refusal names
        // BOTH, not just the step the run stopped at. a plain "restart
        // from the failure" retry would have re-run the second one silently.
        let steps = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "email.send", StepStatus::Failed),
            step(2, "email.send", StepStatus::Failed),
        ];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect_err("both failed sends may have half-applied");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["email.send".to_string()]
            },
            "the refusal names the offending node once, having found it twice"
        );
    }

    #[test]
    fn a_step_left_running_is_gated_like_a_failed_one() {
        // A crashed executor leaves its step marked `running`. That step may
        // have applied and lost its response, so it is gated exactly like a
        // failed one rather than treated as "not yet started".
        let steps = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "email.send", StepStatus::Running),
        ];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect_err("a step interrupted mid-flight may have half-applied");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["email.send".to_string()]
            }
        );
    }

    #[test]
    fn a_skipped_or_waiting_step_is_not_gated() {
        // A branch not taken never applied anything, and a parked step is
        // CONTINUED by the cursor rather than repeated — neither is a
        // double-apply risk, so neither blocks the resume.
        let steps = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "email.send", StepStatus::Skipped),
            step(2, "notification.send", StepStatus::Waiting),
            step(3, "ticket.update", StepStatus::Failed),
        ];
        let plan = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect("neither the skipped nor the parked step is a repeat risk");
        // The resume point is the first step that did not SUCCEED — the
        // skipped one — and the parked step the continuation continues is
        // inside the set but is not gated by it.
        assert_eq!(plan.start_at_step, 1);
        assert_eq!(plan.reexecuted, vec![1, 2, 3]);
    }

    #[test]
    fn an_unknown_node_is_not_declared_safe_to_repeat() {
        let steps = vec![step(0, "plugin.unknown_thing", StepStatus::Failed)];
        let refusal = retry_plan(RunStatus::Failed, &steps, RetryMode::Resume)
            .expect_err("an action nobody declared is not one nobody proved repeatable");
        assert_eq!(
            refusal,
            RunLedgerError::RetryRefused {
                nodes: vec!["plugin.unknown_thing".to_string()]
            }
        );
    }

    #[test]
    fn an_action_node_resolves_its_idempotence_and_a_generic_one_fails_closed() {
        // Lane 13's flag is PER NODE for the Tier A wrappers, so each
        // answers for itself: a state setter repeats, a creation does not.
        let setter = StepRetryInput {
            step_index: 0,
            node_id: "ticket.update".to_string(),
            input: json!({"ticketId": "t", "fields": {"title": "x"}}),
            status: StepStatus::Failed,
        };
        let creation = StepRetryInput {
            step_index: 1,
            node_id: "ticket.create".to_string(),
            input: json!({"boardId": "b", "title": "x"}),
            status: StepStatus::Failed,
        };
        assert!(retry_plan(RunStatus::Failed, &[setter], RetryMode::Resume).is_ok());
        assert!(retry_plan(RunStatus::Failed, &[creation], RetryMode::Resume).is_err());

        // The GENERIC `domain.update` carries no per-node flag, because its
        // safety depends on the `operation` it was given. Lane 13 resolves
        // it through that operation, so the same node id is repeatable for a
        // state setter and not for a creation. Measured, not assumed:
        // `retry_refusal("domain.update", {"operation": "ticket.update"})`
        // is Ok, and `{"operation": "ticket.create"}` is refused.
        let safe_generic = StepRetryInput {
            step_index: 0,
            node_id: "domain.update".to_string(),
            input: json!({"operation": "ticket.update"}),
            status: StepStatus::Failed,
        };
        let unsafe_generic = StepRetryInput {
            step_index: 1,
            node_id: "domain.update".to_string(),
            input: json!({"operation": "ticket.create"}),
            status: StepStatus::Failed,
        };
        assert!(retry_plan(RunStatus::Failed, &[safe_generic], RetryMode::Resume).is_ok());
        assert!(retry_plan(RunStatus::Failed, &[unsafe_generic], RetryMode::Resume).is_err());

        // And it fails CLOSED when the operation is missing or unparseable:
        // a generic node nobody resolved is not one nobody proved
        // repeatable.
        for unparseable in [json!([]), json!({}), json!({"operation": "nope.create"})] {
            let step = StepRetryInput {
                step_index: 0,
                node_id: "domain.update".to_string(),
                input: unparseable,
                status: StepStatus::Failed,
            };
            let refusal = retry_plan(RunStatus::Failed, &[step], RetryMode::Resume)
                .expect_err("an unresolved generic node is not an assumed-safe one");
            assert_eq!(
                refusal,
                RunLedgerError::RetryRefused {
                    nodes: vec!["domain.update".to_string()]
                }
            );
        }
    }

    #[test]
    fn the_step_order_never_trusts_the_caller() {
        // The same three steps, submitted out of order, plan identically:
        // "which step came first" is the decision, so the policy sorts.
        let ordered = vec![
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "ticket.update", StepStatus::Failed),
            step(2, "ticket.update", StepStatus::Running),
        ];
        let shuffled = vec![
            step(2, "ticket.update", StepStatus::Running),
            step(0, "ticket.update", StepStatus::Succeeded),
            step(1, "ticket.update", StepStatus::Failed),
        ];
        let a = retry_plan(RunStatus::Failed, &ordered, RetryMode::Resume).expect("plan");
        let b = retry_plan(RunStatus::Failed, &shuffled, RetryMode::Resume).expect("plan");
        assert_eq!(a, b);
        assert_eq!(a.start_at_step, 1);
    }

    #[test]
    fn the_retention_window_defaults_and_clamps() {
        assert_eq!(retention_days(None), RETENTION_DEFAULT_DAYS);
        assert_eq!(retention_days(Some(90)), 90);
        // Below the floor and above the ceiling fall back to the default
        // rather than silently trimming or extending the table.
        assert_eq!(retention_days(Some(0)), RETENTION_DEFAULT_DAYS);
        assert_eq!(retention_days(Some(-1)), RETENTION_DEFAULT_DAYS);
        assert_eq!(retention_days(Some(99_999)), RETENTION_DEFAULT_DAYS);
        assert_eq!(retention_days(Some(RETENTION_MIN_DAYS)), RETENTION_MIN_DAYS);
        assert_eq!(retention_days(Some(RETENTION_MAX_DAYS)), RETENTION_MAX_DAYS);
    }

    #[test]
    fn retry_modes_round_trip() {
        assert_eq!(RetryMode::default(), RetryMode::Resume);
        assert_eq!(RetryMode::parse("from_start"), Some(RetryMode::FromStart));
        assert_eq!(RetryMode::parse("everything"), None);
    }
}
