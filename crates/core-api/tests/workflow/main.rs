//! One test binary for the workflow and seam flows
//! (`test-binaries-rollout`, group 10 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! seven `mod` lines below compile and link into one binary instead of seven.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test workflow <name>` selects the same functions
//! it did in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. None of these seven asserts on a global the others write: every test
//! registers its own org, the run ledger and audit rows are per org, and the
//! two internal seams are keyed by the org id in the body they post.

mod common;

mod error_envelope_flow;
mod internal_effects_principal_flow;
mod provenance_flow;
mod workflow_authoring_flow;
mod workflow_node_authority_flow;
mod workflow_overrides_flow;
mod workflow_run_ledger_flow;
