//! One test binary for the portal, share-link and client-vault flows
//! (`test-binaries-rollout`, group 9 of 13).
//!
//! Cargo treats `tests/<dir>/main.rs` as a single target named `<dir>`, so the
//! four `mod` lines below compile and link into one binary instead of four.
//! Each module is one former `tests/` target, moved with its tests unchanged:
//! only the duplicated harness at the top of each file now comes from
//! `common`, so `cargo test --test portal <name>` selects the same functions it
//! did in the separate targets.
//!
//! The caller-identity fixture is declared ONCE here, at the crate root, and
//! both portal flows mint from it. As separate targets each held its own copy
//! with its own address base, which is the cross-process collision the fixture
//! documents; one copy per process is strictly closer to its contract.
//!
//! `portal_hash_bound` is NOT here, and stays a top-level target: its cases
//! drive `auth::hash_gate::global`, a process-wide `OnceLock` whose first
//! caller fixes the slot count, and each case is `serial`. Sharing a process
//! with flows that sign in would let a one-slot gate drain a sibling — the
//! measured `auth_hash_bound` failure from the rollout's first four groups.
//!
//! What the merged binary shares that otherwise separate ones did not: one
//! process, one tracing subscriber, one set of process-global statics and one
//! pool per boot. The portal sign-in gate is durable and per-IP, so every
//! portal request carries a minted caller rather than the shared `unknown`
//! bucket.

#[path = "../support/caller.rs"]
mod caller;

mod common;

mod client_vault_record_scope;
mod portal_flow;
mod portal_v2_flow;
mod share_links_flow;
