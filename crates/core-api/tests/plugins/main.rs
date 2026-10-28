//! One test binary for the plugin, manifest and integration flows
//! (`test-binaries-rollout`, group 7 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! six `mod` lines below compile and link into one binary instead of six. Each
//! module is one former `tests/` target, moved with its tests unchanged: only
//! the duplicated harness at the top of each file now comes from `common`, so
//! `cargo test --test plugins <name>` selects the same functions it did in the
//! separate targets.
//!
//! `connector_conformance` is NOT here, and stays a top-level target: its boot
//! drops and recreates the `public` schema once per process
//! (`static BOOTSTRAP: OnceCell`, then `DROP SCHEMA public CASCADE`), which in
//! a shared binary runs while sibling tests are registering orgs. That is the
//! failure that pulled `two_weeks` back out of `work_management`.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. `plugin_http` keeps a process-wide client cache and the connector
//! routes keep a per-org rate window; no test here asserts on either, and each
//! test registers its own org.

mod common;

mod integrations_flow;
mod manifest_flow;
mod plugin_consent_flow;
mod plugin_surface_flow;
mod plugins_flow;
mod rag_vector_namespace_flow;
