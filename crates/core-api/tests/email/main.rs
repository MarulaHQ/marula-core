//! One test binary for the email flows (`test-binaries-rollout`, group 6 of
//! 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! six `mod` lines below compile and link into one binary instead of six. Each
//! module is one former `tests/` target, moved with its tests unchanged: only
//! the duplicated harness at the top of each file now comes from `common`, so
//! `cargo test --test email <name>` selects the same functions it did in the
//! separate targets.
//!
//! `email_attachments` is NOT here, and stays a top-level target: its harness
//! is `tests/support`, whose bootstrap drops and recreates the `public` schema
//! once per process. In a shared binary that runs while sibling tests are
//! registering orgs, which is the failure that pulled `two_weeks` back out of
//! `work_management`. It is also container-marked.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! boot. None of these six reads a global the others write: each test
//! registers its own org, and the outbound ledger and inbound addresses are
//! per-org or per-run.

mod common;

mod email_signature_prefs_flow;
mod inbound_sender_binding;
mod mail_admin_flow;
mod reply_snippets_flow;
mod templates_flow;
mod ticket_email_gaps_flow;
