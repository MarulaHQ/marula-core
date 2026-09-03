//! meridian-cli — the operator entrypoint for Meridian's background
//! services.
//!
//! Purpose: wrap the operational tasks self-hosters need — applying
//! database migrations and launching the worker / realtime services with
//! environment-driven config identical to their standalone binaries, the
//! upgrade preflight (U6), and the upgrade-fixture manifest emitter (U7).
//!
//! Non-purpose: no server runtime code of its own (it orchestrates, it does
//! not implement), no interactive setup wizard yet (later Stage 0 work),
//! and no `dev` harness command until apps/web exists to serve.

use clap::{Parser, Subcommand};
use tokio::sync::watch;

mod fixture;
mod upgrade;

/// Database URL default mirroring `core_api::config::Config` so bare
/// `meridian migrate` works against the dev compose stack.
const DEFAULT_DATABASE_URL: &str = "postgres://meridian_app:meridian_app@localhost:15432/meridian";

#[derive(Debug, Parser)]
#[command(name = "meridian", version, about = "Meridian PSA operations CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply pending database migrations, then exit.
    Migrate,
    /// Run the outbox→Redis relay worker with its /healthz endpoint.
    ServeWorker,
    /// Run the realtime WebSocket hub.
    ServeRealtime,
    /// Upgrade preflight + fixture tooling (policy §5/§8).
    Upgrade {
        /// Run the offline preflight: scan the customization manifest (and,
        /// when given, a database copy) for deprecated-construct touchpoints.
        #[arg(long)]
        check: bool,
        /// Path to the U5 customization manifest JSON to analyze.
        #[arg(long, requires = "check")]
        manifest: Option<std::path::PathBuf>,
        /// Path to the deprecation registry (defaults to ./deprecations.json).
        #[arg(long, requires = "check")]
        registry: Option<std::path::PathBuf>,
        /// Optional database copy URL for the second preflight leg.
        #[arg(long, requires = "check")]
        database_url: Option<String>,
    },
    /// Upgrade-fixture tooling (U7) — hidden support for fixture capture.
    #[command(hide = true)]
    Fixture {
        /// Emit the org's customization manifest (U5 envelope) as JSON.
        #[arg(long)]
        manifest: bool,
        /// Org to emit the manifest for.
        #[arg(long)]
        org_id: uuid::Uuid,
        /// Write to this path instead of stdout.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Seed the deterministic demo org (TASKLIST DX1). Dev-only: refuses
    /// unless MERIDIAN_DEV_TOOLS=1.
    SeedDemo {
        /// Wipe the demo org (if present) and reseed from scratch.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Held for the process lifetime; drop flushes/shuts down telemetry.
    let _telemetry = init_tracing();
    match Cli::parse().command {
        Command::Migrate => migrate().await,
        Command::ServeWorker => serve_worker().await,
        Command::ServeRealtime => serve_realtime().await,
        Command::Upgrade {
            check,
            manifest,
            registry,
            database_url,
        } => {
            anyhow::ensure!(check, "use `meridian upgrade --check` to run the preflight");
            upgrade_check(manifest, registry, database_url).await
        }
        Command::Fixture {
            manifest,
            org_id,
            out,
        } => {
            anyhow::ensure!(
                manifest,
                "use `meridian fixture --manifest` to emit a manifest"
            );
            let url =
                std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
            fixture::emit_manifest(&url, org_id, out).await
        }
        Command::SeedDemo { force } => seed_demo(force).await,
    }
}

/// Connects with the same env contract as core-api (`DATABASE_URL`) and
/// applies every pending migration via core-db.
async fn migrate() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    tracing::info!("applying migrations");
    let pool = core_db::connect(&url).await?;
    core_db::run_migrations(&pool).await?;
    tracing::info!("migrations applied");
    Ok(())
}

/// The U6 preflight: registry + manifest are required; the database-copy
/// leg is optional and strictly read-only.
async fn upgrade_check(
    manifest: Option<std::path::PathBuf>,
    registry: Option<std::path::PathBuf>,
    database_url: Option<String>,
) -> anyhow::Result<()> {
    let manifest_path = manifest
        .ok_or_else(|| anyhow::anyhow!("--manifest <path> is required for upgrade --check"))?;
    let registry_path =
        registry.unwrap_or_else(|| std::path::PathBuf::from(upgrade::DEFAULT_REGISTRY_PATH));

    let reg = upgrade::load_registry(&registry_path)
        .map_err(|e| anyhow::anyhow!("loading registry: {e}"))?;
    let manifest_json = upgrade::load_manifest(&manifest_path)
        .map_err(|e| anyhow::anyhow!("loading manifest: {e}"))?;
    let findings = upgrade::scan_manifest(&reg, &manifest_json);

    println!(
        "meridian upgrade --check — registry v{} with {} registered deprecation(s)",
        reg.version,
        reg.deprecations.len()
    );
    println!("manifest: {}", manifest_path.display());
    if findings.is_empty() {
        println!("\nNo deprecated-construct touchpoints found in this org's customizations.");
    }
    let mut judgment_calls = 0usize;
    for f in &findings {
        println!(
            "\n[{}] {} (kind {}, deprecated in {}, removal target {})",
            match f.classification {
                upgrade::Classification::AutoMigratable => "auto-migratable",
                upgrade::Classification::JudgmentCall => "JUDGMENT CALL",
            },
            f.construct,
            f.kind,
            f.deprecated_in,
            f.removal_target
        );
        for tp in &f.touchpoints {
            match &tp.anchor {
                Some(anchor) => println!("  - {} = {:?}  ({anchor})", tp.path, tp.value),
                None => println!("  - {} = {:?}", tp.path, tp.value),
            }
        }
        println!("  {}", f.guidance);
        if let Some(doc) = &f.doc {
            println!("  note: {doc}");
        }
        if f.classification == upgrade::Classification::JudgmentCall {
            judgment_calls += 1;
        }
    }

    if let Some(url) = database_url {
        report_db_copy(&url).await;
    } else {
        println!("\n(Database-copy leg skipped — pass --database-url to include it.)");
    }

    if judgment_calls > 0 {
        println!("\nRESULT: {judgment_calls} judgment-call finding(s) — review before upgrading.");
        std::process::exit(1);
    }
    println!("\nRESULT: safe to proceed with the upgrade.");
    Ok(())
}

/// Read-only survey of a database copy: migration state plus per-surface
/// customization counts, so the operator sees what the upgrade will touch.
/// Never writes; failures degrade to a warning, not a crash.
async fn report_db_copy(url: &str) {
    println!("\nDatabase copy leg ({url}):");
    match core_db::connect(url).await {
        Ok(pool) => {
            let version: Option<(i64, String)> = sqlx::query_as(
                "SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 1",
            )
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
            match version {
                Some((v, desc)) => println!("  schema at migration {v} ({desc})"),
                None => println!("  schema version unknown (no _sqlx_migrations rows visible)"),
            }
            for (label, table) in [
                ("custom field defs", "custom_field_defs"),
                ("board statuses", "board_statuses"),
                ("automation rules", "automation_rules"),
                ("webhooks", "webhooks"),
                ("settings (org+user)", "settings"),
                ("SLA calendars", "sla_calendars"),
                ("SLA policies", "sla_policies"),
                ("report definitions", "report_definitions"),
            ] {
                let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);
                println!("  {label}: {count}");
            }
        }
        Err(e) => {
            println!("  warning: could not reach database copy ({e}); manifest analysis only")
        }
    }
}

/// Runtime gate for the dev-tool surface (hard rule: never default-on).
fn dev_tools_enabled() -> bool {
    std::env::var("MERIDIAN_DEV_TOOLS").as_deref() == Ok("1")
}

/// The at-rest seal key for demo secrets, using the same precedence as
/// core-api's config: `MERIDIAN_SECRET_KEY` (64 hex chars) when set, else
/// the documented dev fallback derived from `JWT_SECRET`.
fn dev_seal_key() -> anyhow::Result<core_db::demo::DemoSeedOptions> {
    let key = match std::env::var("MERIDIAN_SECRET_KEY") {
        Ok(hex_key) if hex_key.len() == 64 => {
            let mut bytes = [0u8; 32];
            hex::decode_to_slice(&hex_key, &mut bytes)
                .map_err(|e| anyhow::anyhow!("MERIDIAN_SECRET_KEY is not valid hex: {e}"))?;
            bytes
        }
        _ => {
            let jwt = std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-only-insecure-secret-change-me".into());
            core_db::demo::derive_dev_seal_key(&jwt)
        }
    };
    Ok(core_db::demo::DemoSeedOptions { seal_key: key })
}

/// Seeds (or reseeds, with `--force`) the demo org and prints the summary
/// table + login hint. Same key derivation as core-api so the seeded
/// webhook/alert-source secrets are openable by the running stack.
async fn seed_demo(force: bool) -> anyhow::Result<()> {
    if !dev_tools_enabled() {
        anyhow::bail!(
            "seed-demo is a dev-only surface: set MERIDIAN_DEV_TOOLS=1 to enable \
             (keep it unset in production — the surface must not exist there)"
        );
    }
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let pool = core_db::connect(&url).await?;
    core_db::run_migrations(&pool).await?;
    let opts = dev_seal_key()?;

    let outcome = if force {
        core_db::demo::reset(&pool, &opts).await.map(Err)
    } else {
        core_db::demo::seed(&pool, &opts).await.map(Ok)
    };
    match outcome {
        Ok(Err(report)) => print_seed_summary(&report),
        // --force path: always fresh.
        Ok(Ok(report)) => print_seed_summary(&report),
        Err(core_db::demo::DemoError::AlreadySeeded(org_id)) => {
            println!("demo org already seeded (org_id {org_id}) — nothing written.");
            println!("wipe + reseed: MERIDIAN_DEV_TOOLS=1 meridian seed-demo --force");
            println!("or POST /api/dev/reset-org on a dev API with MERIDIAN_DEV_TOOLS=1.");
            println!();
            println!("logins: owner@demo.local / dispatcher@demo.local / tech@demo.local / bookkeeper@demo.local");
            println!("password: {}", core_db::demo::DEMO_PASSWORD);
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// The end-of-seed summary: counts table, login hint, and the dev secrets
/// (alert HMAC + webhook signing secret) printed ONCE, per the DX1 contract.
fn print_seed_summary(report: &core_db::demo::DemoSeedReport) {
    let rows: &[(&str, String)] = &[
        ("org", format!("{} ({})", report.org_name, report.slug)),
        ("users", report.users.to_string()),
        ("clients (sites/contacts)", report.clients.to_string()),
        ("boards", report.boards.to_string()),
        ("tickets", report.tickets.to_string()),
        (
            "comments / checklist items",
            format!("{} / {}", report.comments, report.checklist_items),
        ),
        (
            "timer sessions / time entries",
            format!("{} / {}", report.timer_sessions, report.time_entries),
        ),
        (
            "contracts / invoices",
            format!("{} / {}", report.contracts, report.invoices),
        ),
        ("kb articles", report.kb_articles.to_string()),
        ("documents", report.documents.to_string()),
        ("projects", report.projects.to_string()),
        ("appointments", report.appointments.to_string()),
        (
            "alert sources / alerts / incidents",
            format!(
                "{} / {} / {}",
                report.alert_sources, report.alerts, report.alert_incidents
            ),
        ),
        ("report schedules", report.report_schedules.to_string()),
        ("webhooks", report.webhooks.to_string()),
    ];
    println!();
    println!("demo org seeded");
    println!("---------------");
    for (label, value) in rows {
        println!("{label:<34} {value}");
    }
    println!();
    println!("logins (password: {}):", report.password);
    for (email, role) in &report.logins {
        println!("  {email:<26} {role}");
    }
    println!();
    println!(
        "alert HMAC secret (printed once): {}",
        report.alert_hmac_secret
    );
    println!(
        "webhook signing secret (printed once): {}",
        report.webhook_signing_secret
    );
    println!();
    println!(
        "sign in at http://localhost:3000 with owner@demo.local / {}",
        report.password
    );
}

async fn serve_worker() -> anyhow::Result<()> {
    worker::run(worker::config::WorkerConfig::from_env(), shutdown_watch()).await
}

async fn serve_realtime() -> anyhow::Result<()> {
    realtime::run(realtime::config::RealtimeConfig::from_env()).await
}

/// A watch channel that flips true on Ctrl-C; both services use it for
/// graceful shutdown.
fn shutdown_watch() -> watch::Receiver<bool> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });
    shutdown_rx
}

/// Shared telemetry bootstrap (TASKLIST 2.18): none-mode unless
/// OTEL_EXPORTER_OTLP_ENDPOINT is set; the guard lives for `main`.
fn init_tracing() -> core_otel::TelemetryGuard {
    core_otel::init("meridian-cli")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        Cli::try_parse_from(std::iter::once("meridian").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    #[test]
    fn parses_each_subcommand() {
        assert!(matches!(parse(&["migrate"]), Ok(Command::Migrate)));
        assert!(matches!(parse(&["serve-worker"]), Ok(Command::ServeWorker)));
        assert!(matches!(
            parse(&["serve-realtime"]),
            Ok(Command::ServeRealtime)
        ));
        assert!(matches!(
            parse(&["seed-demo"]),
            Ok(Command::SeedDemo { force: false })
        ));
        assert!(matches!(
            parse(&["seed-demo", "--force"]),
            Ok(Command::SeedDemo { force: true })
        ));
    }

    #[test]
    fn parses_upgrade_check_with_args() {
        let cmd = parse(&[
            "upgrade",
            "--check",
            "--manifest",
            "m.json",
            "--database-url",
            "postgres://example/db",
        ])
        .expect("parses");
        match cmd {
            Command::Upgrade {
                check,
                manifest,
                database_url,
                ..
            } => {
                assert!(check);
                assert_eq!(manifest.as_deref(), Some(std::path::Path::new("m.json")));
                assert_eq!(database_url.as_deref(), Some("postgres://example/db"));
            }
            _ => panic!("expected upgrade"),
        }
    }

    #[test]
    fn upgrade_requires_check_flag() {
        assert!(
            parse(&["upgrade"]).is_err()
                || matches!(
                    parse(&["upgrade"]),
                    Ok(Command::Upgrade { check: false, .. })
                )
        );
    }

    #[test]
    fn check_flags_require_check() {
        // --manifest without --check must be rejected by clap.
        assert!(parse(&["upgrade", "--manifest", "m.json"]).is_err());
    }

    #[test]
    fn parses_fixture_manifest() {
        let cmd = parse(&[
            "fixture",
            "--manifest",
            "--org-id",
            "00000000-0000-4000-8000-000000000001",
            "--out",
            "m.json",
        ])
        .expect("parses");
        assert!(matches!(cmd, Command::Fixture { manifest: true, .. }));
    }

    #[test]
    fn rejects_unknown_subcommands() {
        assert!(parse(&["bogus"]).is_err());
        assert!(parse(&[]).is_err()); // a subcommand is required
    }

    /// Mechanical CI guard: the derive stays valid even if fields are
    /// added later.
    #[test]
    fn cli_definition_is_debug_valid() {
        Cli::command().debug_assert();
    }

    /// The DX hard rule: without MERIDIAN_DEV_TOOLS=1 the seeder refuses
    /// with a message that names the variable (never a default-on surface).
    #[tokio::test]
    async fn seed_demo_refuses_without_dev_tools_gate() {
        // The test process must NOT carry the gate from the ambient env.
        unsafe { std::env::remove_var("MERIDIAN_DEV_TOOLS") };
        let err = seed_demo(false).await.expect_err("must refuse");
        assert!(err.to_string().contains("MERIDIAN_DEV_TOOLS"));
        // The same refusal applies to --force: the gate precedes everything.
        let err = seed_demo(true).await.expect_err("must refuse");
        assert!(err.to_string().contains("MERIDIAN_DEV_TOOLS"));
    }
}
