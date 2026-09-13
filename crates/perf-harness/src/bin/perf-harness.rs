//! `perf-harness` CLI — Phase-4 perf program operator tool (Wave 13 lane 8).
//!
//! Subcommands:
//! - `seed`    — (re)create the reproducible perf org at a chosen scale
//! - `measure` — SQL-path ticket-list p95 against the seeded org
//! - `load`    — HTTP-path load run against a RUNNING core-api
//! - `smoke`   — seed (if needed) + measure + budget assertion, the
//!   same check CI runs via `scripts/perf-smoke.sh`
//!
//! Database selection mirrors the core-db test contract:
//! `MERIDIAN_TEST_DATABASE_URL` → `DATABASE_URL` → error (no container
//! auto-start in the bin; the smoke test inside `cargo test` does the
//! testcontainer dance). The pool must be a PRIVILEGED role (migrations +
//! seed run as the tables' owner).

use anyhow::Context;
use clap::{Parser, Subcommand};
use perf_harness::{load, measure, seed, SeedScale, PERF_ORG_ID};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "perf-harness",
    about = "Meridian perf program harness (test infra)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Seed the reproducible perf org (delete + re-insert, deterministic).
    Seed {
        /// Number of tickets in the org (default: the 50k budget scale).
        #[arg(long, default_value_t = 50_000)]
        tickets: i64,
        #[arg(long, default_value_t = 500)]
        clients: i64,
        #[arg(long, default_value_t = 10)]
        users: i64,
    },
    /// Measure ticket-list SQL-path latency (p50/p95/p99 per mix).
    Measure {
        #[arg(long, default_value_t = 5)]
        warmup: usize,
        #[arg(long, default_value_t = 40)]
        runs: usize,
        /// Fail (exit 1) when p95 exceeds this many ms.
        #[arg(long)]
        budget_ms: Option<f64>,
        /// Write the JSON report to this path.
        #[arg(long)]
        out: Option<String>,
    },
    /// Drive load against a RUNNING core-api (seed + boot first).
    Load {
        #[arg(long, default_value = "http://127.0.0.1:18080")]
        base_url: String,
        #[arg(long, default_value = "30s")]
        duration: String,
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Fail (exit 1) when p95 exceeds this many ms.
        #[arg(long, default_value_t = 150.0)]
        budget_ms: f64,
        #[arg(long)]
        out: Option<String>,
    },
    /// Seed (idempotent) + measure + assert the 150ms budget @50k.
    /// This is what CI runs; the testcontainer path is `scripts/perf-smoke.sh`
    /// or `cargo test -p perf-harness --test perf_smoke`.
    Smoke {
        #[arg(long, default_value_t = 150.0)]
        budget_ms: f64,
        #[arg(long, default_value_t = 50_000)]
        tickets: i64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Seed {
            tickets,
            clients,
            users,
        } => {
            let (_admin_url, admin) = connect().await?;
            core_db::run_migrations(&admin)
                .await
                .context("migrations")?;
            let report = seed::seed(
                &admin,
                &SeedScale {
                    tickets,
                    clients,
                    users,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Measure {
            warmup,
            runs,
            budget_ms,
            out,
        } => {
            let (admin_url, _admin) = connect().await?;
            let app = connect_app(&admin_url).await?;
            let report =
                measure::measure_ticket_list(&app, PERF_ORG_ID, seed::perf_user_id(), warmup, runs)
                    .await?;
            print_report(&report, budget_ms)?;
            write_out(out.as_deref(), &report)?;
            if let Some(budget) = budget_ms {
                measure::assert_budget(&report, budget)?;
            }
        }
        Command::Load {
            base_url,
            duration,
            concurrency,
            seed: mix_seed,
            budget_ms,
            out,
        } => {
            let config = load::LoadConfig {
                base_url,
                duration: parse_duration(&duration)?,
                concurrency,
                seed: mix_seed,
                budget_ms: Some(budget_ms),
            };
            let report = load::run(&config).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            write_out(out.as_deref(), &report)?;
            anyhow::ensure!(
                report.within_budget,
                "load run p95 {:.1}ms exceeded budget {:.1}ms",
                report.p95_ms,
                budget_ms
            );
        }
        Command::Smoke { budget_ms, tickets } => {
            let (admin_url, admin) = connect().await?;
            core_db::run_migrations(&admin)
                .await
                .context("migrations")?;
            seed::seed(
                &admin,
                &SeedScale {
                    tickets,
                    ..SeedScale::budget()
                },
            )
            .await?;
            let app = connect_app(&admin_url).await?;
            let report =
                measure::measure_ticket_list(&app, PERF_ORG_ID, seed::perf_user_id(), 5, 40)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            measure::assert_budget(&report, budget_ms)?;
            println!(
                "PERF SMOKE PASS: ticket-list p95 within {budget_ms}ms @ {tickets} tickets/org"
            );
        }
    }
    Ok(())
}

async fn connect() -> anyhow::Result<(String, sqlx::PgPool)> {
    let url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .context("set MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (privileged role)")?;
    let pool = core_db::connect(&url)
        .await
        .context("connect to postgres")?;
    Ok((url, pool))
}

/// The RLS-scoped pool measurement runs through (production parity).
async fn connect_app(admin_url: &str) -> anyhow::Result<sqlx::PgPool> {
    core_db::connect(&perf_harness::app_pool_url(admin_url))
        .await
        .context("connect as meridian_app (migrations must have run)")
}

fn print_report(report: &measure::MeasureReport, budget_ms: Option<f64>) -> anyhow::Result<()> {
    println!(
        "org {} @ {} tickets (warmup {}, timed {})",
        report.org_id, report.ticket_count, report.warmup_runs, report.timed_runs
    );
    println!(
        "{:<24} {:>9} {:>9} {:>9} {:>9}",
        "mix", "p50", "p95", "p99", "max"
    );
    for m in &report.mixes {
        println!(
            "{:<24} {:>7.1}ms {:>7.1}ms {:>7.1}ms {:>7.1}ms",
            m.name, m.p50_ms, m.p95_ms, m.p99_ms, m.max_ms
        );
    }
    if let Some(plan) = &report.explain_default {
        println!("--- EXPLAIN (ANALYZE, BUFFERS) page1_default ---\n{plan}");
    }
    if let Some(budget) = budget_ms {
        for m in &report.mixes {
            let status = if m.p95_ms <= budget {
                "OK"
            } else {
                "OVER BUDGET"
            };
            println!(
                "{:<24} p95 {:.1}ms / budget {:.1}ms — {status}",
                m.name, m.p95_ms, budget
            );
        }
    }
    Ok(())
}

fn write_out(path: Option<&str>, value: &impl serde::Serialize) -> anyhow::Result<()> {
    if let Some(path) = path {
        std::fs::write(path, serde_json::to_string_pretty(value)?)
            .with_context(|| format!("write {path}"))?;
    }
    Ok(())
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    let (value, unit) = s.split_at(s.len().saturating_sub(1));
    let value: u64 = value.parse().context("duration like 30s / 5m")?;
    match unit {
        "s" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(value * 60)),
        _ => anyhow::bail!("duration like 30s / 5m"),
    }
}
