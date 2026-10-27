//! The deployment-target interface (Wave 60 lane W60-2, ADR 0026 §1).
//!
//! Four verbs and no more. A target detects what it can offer, plans against a
//! host, applies a plan, and reports what runs now. Compose is the first
//! implementation; a binary/systemd target lands behind the same trait without
//! the plan or the wizard changing.

use super::host::HostFacts;
use super::plan::{Plan, PlanError, Selection};

/// What an apply did, step by step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// One line per completed step, in order.
    pub done: Vec<String>,
    /// The step that stopped the run, when one did.
    pub failed: Option<String>,
    /// What the operator must still do.
    pub manual: Vec<String>,
}

/// What runs now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub target: &'static str,
    /// One line per service the target reports.
    pub services: Vec<String>,
    /// The sentence to print when the target cannot answer.
    pub error: Option<String>,
}

/// A place Marula can be installed.
pub trait DeploymentTarget {
    /// The target's name, as the plan and the status print it.
    fn name(&self) -> &'static str;

    /// Whether this host can run this target at all.
    fn available(&self, host: &HostFacts) -> Result<(), PlanError>;

    /// The plan for this host and selection. `--dry-run` prints it; `--apply`
    /// executes it.
    fn plan(&self, host: &HostFacts, selection: &Selection) -> Result<Plan, PlanError>;

    /// Executes a plan. Stops at the first failure and says which step.
    fn apply(&self, plan: &Plan) -> Applied;

    /// What is running now.
    fn status(&self) -> Status;
}
