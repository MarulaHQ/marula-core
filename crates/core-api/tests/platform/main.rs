//! One test binary for the platform flows (`test-binaries-rollout`, group 12
//! of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! seven `mod` lines below compile and link into one binary instead of seven.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test platform <name>` selects the same functions
//! it did in the separate targets.
//!
//! `reports_flow` is NOT here, and stays a top-level target: its boot drops
//! and recreates the `public` schema once per process (`static RESET`, then
//! `DROP SCHEMA public CASCADE`) before migrations, which in a shared binary
//! runs while sibling tests are registering orgs — the `two_weeks` failure.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. The one global reachable from here is the vault write budget, keyed
//! per org; each test registers its own org, and `secrets_flow` is the only
//! module that drives it.

mod common;

mod config_items_flow;
mod instant_first_flow;
mod quarantine_admin_flow;
mod report_defs_flow;
mod report_schedules_flow;
mod secrets_flow;
mod webhook_receiver_flow;
