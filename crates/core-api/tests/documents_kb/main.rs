//! One test binary for the documents, KB and import flows
//! (`test-binaries-rollout`, group 11 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! six `mod` lines below compile and link into one binary instead of six. Each
//! module is one former `tests/` target, moved with its tests unchanged: only
//! the duplicated harness at the top of each file now comes from `common`, so
//! `cargo test --test documents_kb <name>` selects the same functions it did in
//! the separate targets.
//!
//! `imports_flow` is NOT here, and stays a top-level target: its boot drops and
//! recreates the `public` schema once per process (`static BOOTSTRAP`, then
//! `DROP SCHEMA public CASCADE`) before migrations. In a shared binary that
//! runs while sibling tests are registering orgs — the `two_weeks` failure from
//! the rollout's first four groups.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. None of these six asserts on a global the others write; every test
//! registers its own org and namespaces its rows.

mod common;

mod api_versioning;
mod documents_flow;
mod entity_dedupe_flow;
mod event_type_registry_flow;
mod import_async_flow;
mod kb_flow;
