//! One test binary for the AI, decision and agent flows
//! (`test-binaries-rollout`, group 8 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! seven `mod` lines below compile and link into one binary instead of seven.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test ai <name>` selects the same functions it did
//! in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. Three of those globals are reachable from here and none is asserted
//! on by another module: the decision provider overrides are keyed per org and
//! each test seeds its own, the chat runner's router slot is re-stamped by
//! every `build_router` (all of them point at the same database), and the
//! vault budget is per org.

mod common;

mod agent_platform_flow;
mod ai_policy_flow;
mod automation_draft_summary_flow;
mod chat_runner_flow;
mod decision_auxiliary_flow;
mod decision_triage_flow;
mod secret_grant_seeding_flow;
