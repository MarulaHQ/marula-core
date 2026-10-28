//! One test binary for the ticket-surface flows (`test-binaries-rollout`,
//! group 2 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! eight `mod` lines below compile and link into one binary instead of eight.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test tickets_core <name>` selects the same
//! functions it did in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one object store per boot and one set of process-global
//! statics. The tests themselves still run concurrently, exactly as they did
//! within each old binary.
//!
//! Every one of the eight carried its own `test_state`, `send`, `body_json`,
//! `connect` and register call; attachments additionally carried a second
//! constructor for its scripted scanner. Those are now `common`'s, with the
//! scanner, the `Config` tune-ups and the store handle kept as parameters.

mod common;

mod attachments_flow;
mod custom_fields_flow;
mod dispatch_flow;
mod dispatch_v2_flow;
mod labels_scan_flow;
mod merge_workbench_flow;
mod taxonomy_flow;
mod ticketless_effects_flow;
