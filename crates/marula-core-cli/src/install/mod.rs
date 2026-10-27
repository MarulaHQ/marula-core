//! The installer (Wave 60 lane W60-2).
//!
//! Purpose: take a machine that has nothing but Docker and produce a working
//! Marula, with every decision explained in the operator's words. The
//! explanations are not written here — they come from the deploy descriptor
//! (`crate::deploy`), so a lever's consequence is stated once and the wizard,
//! this plan and the docs all read the same sentence.
//!
//! Shape, per ADR 0026:
//!
//! * [`host::HostFacts`] is what the machine reports: cores, memory, free disk,
//!   whether Docker answers, whether a GPU is visible.
//! * [`plan::Plan`] is data — file writes and command steps, each with the
//!   reason it exists and the descriptor field that justifies it. `--dry-run`
//!   prints it and writes nothing; `--apply` executes the same object, so a dry
//!   run cannot describe something the apply would not do.
//! * [`target::DeploymentTarget`] is the interface. Compose is the first
//!   implementation; a binary/systemd target is the second and lands behind the
//!   same trait.
//! * [`doctor`] answers "is this install healthy" from the same facts.
//!
//! Non-purpose: no UI (W60-3), no artifact packaging (W60-4), no updates
//! (W60-5). This module produces a plan and applies it, and says what it could
//! not do.

pub mod compose;
pub mod doctor;
pub mod host;
pub mod plan;
pub mod target;

pub use compose::ComposeTarget;
pub use host::HostFacts;
pub use plan::{recommend, Selection};
pub use target::DeploymentTarget;
