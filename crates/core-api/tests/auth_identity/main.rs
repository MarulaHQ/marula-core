//! One test binary for the auth / identity flows (`test-binaries-rollout`,
//! group 3 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! `mod` lines below compile and link into one binary instead of many. Each
//! module is one former `tests/` target, moved with its tests unchanged: only
//! the duplicated harness at the top of each file now comes from `common`, so
//! `cargo test --test auth_identity <name>` selects the same functions it did
//! in the separate targets.
//!
//! Three of this group's nine targets are **not** here, each because it owns
//! process-global state that a shared binary makes everyone else's:
//!
//! - `auth_hash_bound` and `mfa_hash_bound` both drive
//!   `core_api::auth::hash_gate::global`, a process-wide `OnceLock` whose first
//!   caller fixes the slot count for the process, and both drain every slot to
//!   force a 503. Measured with them in this binary: `mfa_hash_bound`'s
//!   "this case measures one slot" assertion read 20 slots instead of 1, and
//!   sibling registrations got 503 while a drain was held.
//! - `auth_timing` counts `core_api::auth::password::dummy_verify_count`, a
//!   process-wide counter, and asserts an exact delta across one request.
//!   Measured in this binary: 7 verifies where the case expects 6, because a
//!   sibling case verified a password inside the same window. Its timing
//!   ratios are no happier under a loaded binary.
//!
//! All three stay top-level targets, unchanged. `auth_identity` keeps six
//! modules.

mod common;

mod auth_hardening_flow;
mod directory_groups_flow;
mod mfa_enroll_gate_flow;
mod mfa_step_up_flow;
mod roles_flow;
mod session_revocation_flow;
