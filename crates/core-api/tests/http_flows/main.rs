//! One test binary for four HTTP-surface flows (pilot for consolidating the
//! `tests/*_flow.rs` targets).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! four `mod` lines below compile and link into one binary instead of four.
//! Each module is one former `tests/<name>_flow.rs` target, moved unchanged
//! apart from importing the harness from `common` — no test was renamed, so
//! `cargo test --test http_flows <name>` still selects the same functions.
//!
//! What a merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one connection to the test database per `test_state()`
//! call, and one set of process-global statics for the whole group. The four
//! flows here were picked because they touch none of those globals themselves
//! (no env overrides, no fixed ports, no `serial`). Test functions still run
//! concurrently within the binary, exactly as they did within each old one.
//!
//! The harness in `common/mod.rs` is a copy of the four identical copies that
//! lived in the separate targets, not a rewrite.

mod common;

mod change_order_cas_flow;
mod conditional_read_flow;
mod fx_flow;
mod scorecards_flow;
