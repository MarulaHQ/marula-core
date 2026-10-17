//! Phase-4 perf program test infrastructure (Wave 13 lane 8 `p13-perf-baseline`).
//!
//! Three deliverables, test-infra only (no product code, no migrations):
//!
//! 1. **Query budget** — ticket-list p95 < 150ms @ 50k tickets/org
//!    (`measure::measure_ticket_list`); measured first, then enforced by
//!    the CI perf smoke.
//! 2. **Load-test harness** — reproducible dataset generator
//!    (`seed::seed`) + HTTP load runner (`load::run`) at 100k tickets/org.
//! 3. **Perf smoke** — fast, CI-runnable budget check
//!    (`tests/perf_smoke.rs`, env-gated so the plain workspace sweep skips it).
//! 4. **Multi-endpoint journey** (Wave 49 lane R5) — `journey::run` drives a
//!    realistic agent session (board, ticket, thread, status, comment) paced
//!    by a think-time, and reports per-endpoint latency plus the per-user
//!    request rate and the Little's-law users-per-instance figure. This is
//!    the workload a users-per-instance number has to be measured on.
//! 5. **Capacity model** (Wave 49 lane R8) — `capacity::plan` turns a journey
//!    report into a sizing plan: the users one instance holds and how many
//!    instances a target user count wants. The prose model is
//!    `docs/capacity-model.md`.
//!
//! Reproducibility contract: every measured entity (tickets, clients,
//! sites, contacts, users, custom fields) gets a deterministic id derived
//! from `md5('<tag>')::uuid`, so a re-seed of the same scale produces a
//! byte-identical dataset shape. Only infrastructure rows whose ids never
//! enter the measured queries (roles) use time-based uuids.
//!
//! This crate is AGPL like the rest of the core (workspace license); it is
//! a developer/test tool and is not part of the shipped API surface.

pub mod capacity;
pub mod journey;
pub mod load;
pub mod measure;
pub mod seed;

/// Fixed org id for the perf dataset. Deleting this org cascades the whole
/// dataset, so seeding is idempotent (delete-then-insert in one tx).
pub const PERF_ORG_ID: uuid::Uuid = uuid::uuid!("9e2f0000-0000-4000-8000-00000000d001");

/// Login email of the seeded harness user (index 1). The load runner logs
/// this user in through the real `POST /api/auth/login` flow.
pub const PERF_USER_EMAIL: &str = "perf-user-1@perf.local";
/// Shared password for all seeded harness users.
pub const PERF_PASSWORD: &str = "perf-harness-password";

/// Dataset shape knobs. Defaults match the Wave-13 budgets: 50k for the
/// smoke/budget run, 100k for load tests.
#[derive(Debug, Clone, Copy)]
pub struct SeedScale {
    pub tickets: i64,
    pub clients: i64,
    pub users: i64,
}

impl SeedScale {
    /// The budget scale: ticket-list p95 budget is defined @ 50k.
    #[must_use]
    pub const fn budget() -> Self {
        Self {
            tickets: 50_000,
            clients: 500,
            users: 10,
        }
    }

    /// The load-test scale: harness runs at 100k tickets/org.
    #[must_use]
    pub const fn load() -> Self {
        Self {
            tickets: 100_000,
            clients: 1_000,
            users: 25,
        }
    }
}

/// Derives the `marula_app` connection URL from a privileged admin URL.
/// The role is created by migration 0001 with the fixed dev password — the
/// same derivation the core-db test harness performs. Measurement MUST run
/// through this pool (RLS + org GUC = production traffic parity); the
/// privileged pool is only for migrations and fixture seeding.
#[must_use]
pub fn app_pool_url(admin_url: &str) -> String {
    // Replace everything between "scheme://" and "@" with the marula_app
    // credentials (fixed dev password from migration 0001). Works whether
    // the admin URL carries a password or not.
    let (scheme, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    let host_part = match rest.split_once('@') {
        Some((_, host)) => host,
        None => rest,
    };
    format!("{scheme}://marula_app:marula_app@{host_part}")
}
