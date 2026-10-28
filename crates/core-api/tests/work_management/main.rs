//! One test binary for the work-management flows (`test-binaries-rollout`,
//! group 1 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! seven `mod` lines below compile and link into one binary instead of seven.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test work_management <name>` selects the same
//! functions it did in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one database
//! connection pool per `test_state()` call. The tests themselves still run
//! concurrently, exactly as they did within each old binary.
//!
//! `two_weeks` was in this group and is not: its `tests/support/` harness drops
//! and re-creates the `public` schema once per process, which empties the
//! database under every other test in a shared binary. It stays its own target;
//! the run that proved it is recorded in the lane report.

mod common;

mod alerts_flow;
mod approvals_flow;
mod backend_gaps_flow;
mod csat_flow;
mod time_discipline;
mod undo_flow;
mod workbar_flow;
