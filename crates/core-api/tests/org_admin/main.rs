//! One test binary for the org-admin flows (`test-binaries-rollout`, group 4
//! of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! eight `mod` lines below compile and link into one binary instead of eight.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test org_admin <name>` selects the same
//! functions it did in the separate targets.
//!
//! What the merged binary shares that separate ones did not: one process, one
//! tracing subscriber, one set of process-global statics and one pool per
//! `test_state()` call. `org_custom_domain` installs a process-wide TXT
//! resolver (`dns_verify::install_txt_resolver`) — it is the only module here
//! that verifies DNS, so the install is not visible to the rest.

mod common;

mod byok_flow;
mod onboarding_flow;
mod org_custom_domain_flow;
mod org_suspension_flow;
mod org_usage_flow;
mod org_users_flow;
mod settings_flow;
mod settings_gaps_flow;
