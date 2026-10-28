//! One test binary for the projects, scheduling and dev-surface flows
//! (`test-binaries-rollout`, group 13 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! six `mod` lines below compile and link into one binary instead of six. Each
//! module is one former `tests/` target, moved with its tests unchanged: only
//! the duplicated harness at the top of each file now comes from `common`, so
//! `cargo test --test projects_misc <name>` selects the same functions it did
//! in the separate targets.
//!
//! `caller_fixture` keeps its own `#[path = "../support/caller.rs"] mod caller`
//! rather than minting from a shared module: its cases are the fixture's own
//! contract, including the one that asserts two back-to-back calls take
//! consecutive slots. Nothing else in this binary mints a caller, so its
//! counter has no other writer — the same isolation the target had alone, in
//! the same process that also runs the DB flows.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. The scheduling-link rate limiter is keyed per link token and the vault
//! budget per org, and every test mints its own.

mod common;

mod caller_fixture;
mod dev_surface;
mod entitlement_adopt_flow;
mod projects_flow;
mod scheduling_flow;
mod scheduling_links_flow;
