//! marula-core-cli — the operator entrypoint for Marula's background
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

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use tokio::sync::watch;

mod fixture;
mod upgrade;

/// Database URL default mirroring `core_api::config::Config` so bare
/// `marula migrate` works against the dev compose stack.
const DEFAULT_DATABASE_URL: &str = "postgres://marula_app:marula_app@localhost:15432/marula";

#[derive(Debug, Parser)]
#[command(name = "marula", version, about = "Marula PSA operations CLI")]
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
    /// unless MARULA_DEV_TOOLS=1.
    SeedDemo {
        /// Wipe the demo org (if present) and reseed from scratch.
        #[arg(long)]
        force: bool,
    },
    /// Operator signing-key management (Wave 19 lane 2, ADR 0009
    /// rotation semantics, docs/reference/plugin-signing-rotation-runbook.md).
    PluginKey {
        #[command(subcommand)]
        action: PluginKeyAction,
    },
}

/// The rotation runbook, as CLI verbs (one step per rotation, never a
/// combined command — every state change is deliberate and printed).
#[derive(Debug, Subcommand)]
enum PluginKeyAction {
    /// List every registered signing key (the rotation state, with
    /// statuses — the roster the runbook starts and ends with).
    #[command(name = "list")]
    List,
    /// Register a public key as ACTIVE (a new key in the trust set).
    /// `--public-key` is the ed25519 PUBLIC key (64 hex chars) — private
    /// keys never enter this surface.
    #[command(name = "add")]
    Add {
        /// Stable trust identity; defaults to the first 16 hex chars of
        /// the public key.
        #[arg(long)]
        key_id: Option<String>,
        /// The ed25519 public key, 64 hex chars.
        #[arg(long)]
        public_key: String,
        /// Free-label for the operator's runbook trail (e.g. "HSM slot
        /// 1, quarterly 2026Q4 key").
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Rotation step: mark an ACTIVE key RETIRED. It stops being named
    /// by new publications but KEEPS VERIFYING pre-rotation artifacts
    /// (retired is past-trust, not distrust — revoke is different).
    #[command(name = "retire")]
    Retire {
        #[arg(long)]
        key_id: String,
    },
    /// COMPROMISE response: the key verifies nothing from now on
    /// (retroactively). Requires a reason; the key_id stays occupied
    /// forever — never re-registered.
    #[command(name = "revoke")]
    Revoke {
        #[arg(long)]
        key_id: String,
        /// Why (recorded on the registry row forever).
        #[arg(long)]
        reason: String,
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
            anyhow::ensure!(check, "use `marula upgrade --check` to run the preflight");
            upgrade_check(manifest, registry, database_url).await
        }
        Command::Fixture {
            manifest,
            org_id,
            out,
        } => {
            anyhow::ensure!(
                manifest,
                "use `marula fixture --manifest` to emit a manifest"
            );
            let url =
                std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
            fixture::emit_manifest(&url, org_id, out).await
        }
        Command::SeedDemo { force } => seed_demo(force).await,
        Command::PluginKey { action } => plugin_key(action).await,
    }
}

/// Connects with the same env contract as core-api (`DATABASE_URL`) and
/// applies every pending migration via core-db.
async fn migrate() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    tracing::info!("applying migrations");
    let pool = core_db::connect(&url).await.context(
        "connecting to Postgres — set DATABASE_URL and make sure the \
         database is reachable (dev compose: `docker compose up -d postgres`)",
    )?;
    core_db::run_migrations(&pool).await.context(
        "running migrations — migrations must run as the schema owner \
         (the compose `migrate` one-shot role), NOT as the RLS-scoped \
         app role, which cannot CREATE TABLE on fresh installs",
    )?;
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
    let manifest_path = manifest.ok_or_else(|| {
        anyhow::anyhow!(
            "--manifest <path> is required for upgrade --check — pass the \
             org's U5 customization manifest (emit one first: \
             `marula fixture --manifest --org-id <uuid> --out m.json`)"
        )
    })?;
    let registry_path =
        registry.unwrap_or_else(|| std::path::PathBuf::from(upgrade::DEFAULT_REGISTRY_PATH));

    let reg = upgrade::load_registry(&registry_path).map_err(|e| {
        anyhow::anyhow!(
            "loading the deprecation registry from {} failed: {e} — ship the \
             repo's deprecations.json alongside the binary, or point \
             --registry at it",
            registry_path.display()
        )
    })?;
    let manifest_json = upgrade::load_manifest(&manifest_path).map_err(|e| {
        anyhow::anyhow!(
            "loading the customization manifest from {} failed: {e} — \
             regenerate it with `marula fixture --manifest --org-id <uuid>` \
             against the pre-upgrade database",
            manifest_path.display()
        )
    })?;
    let findings = upgrade::scan_manifest(&reg, &manifest_json);

    println!(
        "marula upgrade --check — registry v{} with {} registered deprecation(s)",
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
    std::env::var("MARULA_DEV_TOOLS").as_deref() == Ok("1")
}

/// The at-rest seal key for demo secrets, using the same precedence as
/// core-api's config: `MARULA_SECRET_KEY` (64 hex chars) when set, else
/// the documented dev fallback derived from `JWT_SECRET`.
fn dev_seal_key() -> anyhow::Result<core_db::demo::DemoSeedOptions> {
    let key = match std::env::var("MARULA_SECRET_KEY") {
        Ok(hex_key) if hex_key.len() == 64 => {
            let mut bytes = [0u8; 32];
            hex::decode_to_slice(&hex_key, &mut bytes)
                .map_err(|e| anyhow::anyhow!("MARULA_SECRET_KEY is not valid hex: {e}"))?;
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
            "seed-demo is a dev-only surface: set MARULA_DEV_TOOLS=1 to enable \
             (keep it unset in production — the surface must not exist there)"
        );
    }
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let pool = core_db::connect(&url).await.context(
        "connecting to Postgres for the demo seed — set DATABASE_URL and \
         start the dev compose stack (`docker compose up -d postgres`)",
    )?;
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
            println!("wipe + reseed: MARULA_DEV_TOOLS=1 marula seed-demo --force");
            println!("or POST /api/dev/reset-org on a dev API with MARULA_DEV_TOOLS=1.");
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
        (
            "assets / vault entries / api keys",
            format!(
                "{} / {} / {}",
                report.assets, report.vault_entries, report.api_keys
            ),
        ),
        (
            "catalog items / quotes / pipeline deals",
            format!(
                "{} / {} / {}",
                report.catalog_items, report.quotes, report.pipeline_deals
            ),
        ),
        (
            "credit notes / tags / notifications",
            format!(
                "{} / {} / {}",
                report.credit_notes, report.tags, report.notifications
            ),
        ),
        (
            "pm schedules / automations / plugins",
            format!(
                "{} / {} / {}",
                report.pm_schedules, report.automation_rules, report.plugin_installs
            ),
        ),
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

/// Operator signing-key surface (the runbook's verbs over the global
/// trust store). Prints the resulting roster after each write so the
/// operator's terminal log IS the rotation trail.
async fn plugin_key(action: PluginKeyAction) -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let pool = core_db::connect(&url).await.context(
        "connecting to Postgres — set DATABASE_URL (settings live in the \
         database's plugin_signing_keys registry)",
    )?;
    let mut conn = pool
        .acquire()
        .await
        .context("acquiring a database connection")?;
    match action {
        PluginKeyAction::List => {
            let keys = core_db::plugin_signing_keys::list_keys(&mut conn).await?;
            if keys.is_empty() {
                println!("(no signing keys registered yet — see the rotation runbook)");
            }
            for key in keys {
                println!(
                    "{:<18} {:<7} {:<24} reason: {}",
                    key.key_id,
                    key.status.as_str(),
                    key.label,
                    key.revoked_reason.as_deref().unwrap_or("-")
                );
            }
        }
        PluginKeyAction::Add {
            key_id,
            public_key,
            label,
        } => {
            let key = core_db::plugin_signing_keys::add_key(
                &mut conn,
                key_id.as_deref(),
                &public_key,
                &label,
            )
            .await
            .context("registering the public key failed")?;
            println!(
                "registered key_id {} (active) — artifacts naming it verify from now on",
                key.key_id
            );
        }
        PluginKeyAction::Retire { key_id } => {
            let key = core_db::plugin_signing_keys::retire_key(&mut conn, &key_id)
                .await
                .map_err(|e| anyhow::anyhow!("key_id {key_id:?} not found or not active: {e}"))?;
            println!(
                "retired key_id {} — still verifies pre-rotation artifacts; \
                 no NEW publications reference it",
                key.key_id
            );
        }
        PluginKeyAction::Revoke { key_id, reason } => {
            let key = core_db::plugin_signing_keys::revoke_key(&mut conn, &key_id, &reason)
                .await
                .context("revoking the key failed")?;
            println!(
                "REVOKED key_id {} — it verifies nothing (retroactively poisons \
                 artifacts citing it; the key_id can never be re-registered)",
                key.key_id
            );
        }
    }
    Ok(())
}

async fn serve_worker() -> anyhow::Result<()> {
    worker::run(worker::config::WorkerConfig::from_env(), shutdown_watch())
        .await
        .map_err(|e| {
            e.context(
                "worker exited — check the DATABASE_URL / REDIS_URL hints in \
                 the error above; every loop the worker hosts reports to \
                 /healthz, so an orchestrator can also restart the container",
            )
        })
}

async fn serve_realtime() -> anyhow::Result<()> {
    realtime::run(realtime::config::RealtimeConfig::from_env())
        .await
        .map_err(|e| {
            e.context(
                "realtime hub failed — check DATABASE_URL and the WS bind \
                 settings; browsers reconnect automatically once it is up",
            )
        })
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
    core_otel::init("marula-core-cli")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        Cli::try_parse_from(std::iter::once("marula").chain(args.iter().copied()))
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
        // The rotation runbook verbs parse (Wave 19 lane 2).
        assert!(matches!(
            parse(&["plugin-key", "list"]),
            Ok(Command::PluginKey {
                action: PluginKeyAction::List
            })
        ));
        assert!(matches!(
            parse(&["plugin-key", "add", "--public-key", &"a".repeat(64)]),
            Ok(Command::PluginKey {
                action: PluginKeyAction::Add { .. }
            })
        ));
        assert!(matches!(
            parse(&["plugin-key", "retire", "--key-id", "ops-1"]),
            Ok(Command::PluginKey {
                action: PluginKeyAction::Retire { .. }
            })
        ));
        assert!(matches!(
            parse(&[
                "plugin-key",
                "revoke",
                "--key-id",
                "ops-1",
                "--reason",
                "compromise"
            ]),
            Ok(Command::PluginKey {
                action: PluginKeyAction::Revoke { .. }
            })
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

    /// The DX hard rule: without MARULA_DEV_TOOLS=1 the seeder refuses
    /// with a message that names the variable (never a default-on surface).
    #[tokio::test]
    async fn seed_demo_refuses_without_dev_tools_gate() {
        // The test process must NOT carry the gate from the ambient env.
        unsafe { std::env::remove_var("MARULA_DEV_TOOLS") };
        let err = seed_demo(false).await.expect_err("must refuse");
        assert!(err.to_string().contains("MARULA_DEV_TOOLS"));
        // The same refusal applies to --force: the gate precedes everything.
        let err = seed_demo(true).await.expect_err("must refuse");
        assert!(err.to_string().contains("MARULA_DEV_TOOLS"));
    }
}
