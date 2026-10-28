//! One test binary for the billing, purchasing and asset flows
//! (`test-binaries-rollout`, group 5 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! seven `mod` lines below compile and link into one binary instead of seven.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test billing_assets <name>` selects the same
//! functions it did in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! `test_state()` call. None of these seven touches a process-global the
//! others read: the vault budget is keyed per org and each test registers its
//! own, and the rate limiters these routes hold are keyed per link/connector,
//! not per process.

mod common;

mod assets_flow;
mod billing_verification_flow;
mod export_flow;
mod purchasing_flow;
mod quote_catalog_flow;
mod stock_flow;
mod tools_loans_flow;
