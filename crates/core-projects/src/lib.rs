//! core-projects — project phases/tasks/views/deps/templates + milestone
//! billing seam (the private build tracker Phase 3 projects line, migration 0030).
//!
//! Purpose: the project-domain logic that is neither pure entity shape
//! (core-domain `entities::projects`) nor SQL (core-db `projects`):
//!
//! * [`deps`] — finish-to-start dependency validation: same-project
//!   membership, self-dep rejection, and cycle detection over the whole
//!   task graph. The completion path (`/tasks/{id}/complete`) gates on it.
//! * [`template`] — the versioned template spec (additive-only JSON,
//!   customizations-as-data per policy §5) and the pure expansion plan the
//!   instantiate endpoint executes inside one transaction.
//! * [`schedule`] — working-calendar resolution: a project's own pattern wins
//!   over the organisation's, which is the rule every date calculation reads.
//! * [`capacity`] — how much planned work a person carries: a weekly figure
//!   against the calendar, never a second calendar.
//! * [`risk`] — the nine deterministic rules that turn stored plan facts into
//!   signals. Severity comes from the rule, never from a model.
//!
//! Milestone billing seam: completing a milestone phase/task emits
//! `project.milestone_completed` through the transactional outbox from the
//! core-api route layer. This crate deliberately knows nothing about
//! billing — the event envelope is the ONLY seam to core-billing (Wave 9
//! invoicing consumes it; no crate-level import in either direction).
//!
//! Non-purpose: no HTTP, no invoice rows, no labor-actuals rollups. Routes
//! live in core-api `routes/projects/`; persistence in core-db `projects`.

pub mod capacity;
pub mod deps;
pub mod risk;
pub mod schedule;
pub mod template;
