//! marula-core-cli — the operator entrypoint for Marula's background
//! services.
//!
//! Purpose: wrap the operational tasks self-hosters need — applying
//! database migrations and launching the worker service with
//! environment-driven config identical to their standalone binaries, the
//! upgrade preflight (U6), and the upgrade-fixture manifest emitter (U7).
//!
//! Non-purpose: no server runtime code of its own (it orchestrates, it does
//! not implement), no interactive setup wizard yet (later Stage 0 work),
//! and no `dev` harness command until apps/web exists to serve.

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use marula_core_cli::{attachments, data, deploy, fixture, install, update, upgrade};
use tokio::sync::watch;

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
    /// Seed the deterministic demo org (the private build tracker DX1). Dev-only: refuses
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
    /// Deployment descriptor tooling (Wave 49 lane R7): the interface a
    /// commercial control plane consumes. Emits the machine-readable
    /// descriptor and keeps the checked-in profile files in step with it.
    Deploy {
        #[command(subcommand)]
        action: DeployAction,
    },
    /// Data lifecycle: the tenant erasure verb (docs/reference/data-lifecycle.md).
    /// There is no HTTP surface for this on purpose; an erasure is an operator
    /// decision with a receipt.
    Data {
        #[command(subcommand)]
        action: DataAction,
    },
    /// Stored-object maintenance. The verdict backfill is the pass an install
    /// runs BEFORE it turns a scanner on, so files stored without a scan are
    /// not locked out of every download door (Wave 55, follow-up F30).
    Attachments {
        #[command(subcommand)]
        action: AttachmentAction,
    },
    /// Install Marula on this machine (Wave 60 lane W60-2). Prints the plan by
    /// default; `--apply` writes the env file and starts the stack, and returns
    /// only when the profile's healthchecks pass.
    Install {
        /// Profile to install. Default: the shape this host fits.
        #[arg(long)]
        profile: Option<String>,
        /// Print the plan and write nothing (the default).
        #[arg(long)]
        dry_run: bool,
        /// Execute the plan.
        #[arg(long)]
        apply: bool,
        /// Where the deployment's files go.
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
        /// Override a lever: NAME=value (repeatable).
        #[arg(long = "set", value_name = "NAME=value")]
        set: Vec<String>,
        /// Compose file to run.
        #[arg(long, default_value = "deploy/docker-compose.yml")]
        compose_file: std::path::PathBuf,
        /// Declare this host's memory when the platform publishes no reading
        /// (the same value MARULA_HOST_MEMORY_MB carries).
        #[arg(long)]
        memory_mb: Option<u32>,
        /// Serve the wizard over loopback instead of printing the plan. This is
        /// the browser fallback; the native installer app is the primary, and
        /// both render the same front-end (ADR 0027).
        #[arg(long)]
        ui: bool,
        /// Port the wizard binds. `0` asks the OS for a free one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// The built front-end the wizard serves.
        #[arg(long, default_value = "apps/installer/dist")]
        ui_dir: std::path::PathBuf,
        /// Do not open a browser when `--ui` starts.
        #[arg(long)]
        no_open: bool,
    },
    /// Update Marula from a signed manifest (Wave 60 lane W60-5). `--check`
    /// prints whether a newer release exists and what it fixes; `--apply`
    /// backs the install up, applies through the same target, verifies, and
    /// rolls the files back if either step fails.
    Update {
        /// Ask what is available; change nothing.
        #[arg(long)]
        check: bool,
        /// Apply the update. Refused without a signature and a trusted key.
        #[arg(long)]
        apply: bool,
        /// The signed manifest, as published by the release.
        #[arg(long)]
        manifest: std::path::PathBuf,
        /// Detached signature over the manifest's bytes (hex).
        #[arg(long)]
        signature: Option<std::path::PathBuf>,
        /// Trust material: an `allowed_signers` file for an OpenSSH signature,
        /// or an ed25519 public key in hex (repeatable).
        #[arg(long = "key")]
        key: Vec<std::path::PathBuf>,
        /// The identity line an OpenSSH signature names.
        #[arg(long, default_value = "releases@getmarula.com")]
        identity: String,
        /// Directory holding the install.
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
        /// Compose file the install runs.
        #[arg(long, default_value = "deploy/docker-compose.yml")]
        compose_file: std::path::PathBuf,
    },
    /// Verify a release bundle before anything executes it (Wave 60 lane
    /// W60-5).
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },
    /// Check this machine and the install on it against the descriptor (Wave
    /// 60 lane W60-2). Exits non-zero when a check fails.
    Doctor {
        /// Profile the install claims to be. Default: the shape this host fits.
        #[arg(long)]
        profile: Option<String>,
        /// Directory holding `marula.env`.
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
        /// Compose file the install runs, for the running-services line.
        #[arg(long, default_value = "deploy/docker-compose.yml")]
        compose_file: std::path::PathBuf,
        /// Declare this host's memory when the platform publishes no reading.
        #[arg(long)]
        memory_mb: Option<u32>,
    },
}

/// Artifact verification verbs.
#[derive(Debug, Subcommand)]
enum ArtifactAction {
    /// Verify a signed `SHA256SUMS` and every file it names.
    Verify {
        /// The checksum file shipped beside the bundle.
        #[arg(long)]
        sums: std::path::PathBuf,
        /// Detached signature over the sums file: an OpenSSH signature (as the
        /// release pipeline produces) or hex ed25519.
        #[arg(long)]
        signature: Option<std::path::PathBuf>,
        /// Trust material: an `allowed_signers` file for an OpenSSH signature,
        /// or an ed25519 public key in hex (repeatable).
        #[arg(long = "key")]
        key: Vec<std::path::PathBuf>,
        /// The identity line an OpenSSH signature names.
        #[arg(long, default_value = "releases@getmarula.com")]
        identity: String,
        /// Directory the checksum paths are relative to.
        #[arg(long, default_value = ".")]
        root: std::path::PathBuf,
    },
    /// Print the SHA-256 of one file, as `SHA256SUMS` writes it.
    Checksum { file: std::path::PathBuf },
}

/// Data-lifecycle verbs.
#[derive(Debug, Subcommand)]
enum DataAction {
    /// Erase one tenant: rows (via the org cascade), then objects, then a
    /// receipt. Prints the plan first; deletes nothing without --confirm.
    #[command(name = "purge-org")]
    PurgeOrg {
        /// The org to erase.
        #[arg(long)]
        org_id: uuid::Uuid,
        /// Who asked. Required to execute; recorded on the receipt.
        #[arg(long)]
        requested_by: Option<String>,
        /// Why, in your words. Recorded on the receipt.
        #[arg(long)]
        reason: Option<String>,
        /// The org's label, repeated back. Required to execute.
        #[arg(long)]
        confirm: Option<String>,
        /// Show what would be deleted and stop.
        #[arg(long)]
        dry_run: bool,
        /// Owner-role connection string (defaults to DATABASE_URL). The RLS app
        /// role cannot erase a tenant.
        #[arg(long)]
        database_url: Option<String>,
    },
}

/// Stored-object verbs.
#[derive(Debug, Subcommand)]
enum AttachmentAction {
    /// Give every stored object without an admission verdict one, by reading
    /// its bytes back and running them through the configured scanner. An
    /// object that already carries a verdict is never re-scanned, so a second
    /// run finds nothing. Run before enabling a scanner on an install that
    /// stored files without one (Wave 55, follow-up F30).
    #[command(name = "backfill-verdicts")]
    BackfillVerdicts {
        /// Report the plan and stop. Records nothing and scans nothing.
        #[arg(long)]
        dry_run: bool,
        /// The number of objects the dry run reported. Required to execute:
        /// the run re-counts first and refuses if the plan moved.
        #[arg(long)]
        confirm: Option<i64>,
        /// Restrict the walk to one org. The connection still has to be the
        /// deployment role: a narrowed walk is only a smaller plan.
        #[arg(long)]
        org_id: Option<uuid::Uuid>,
        /// Objects fetched per query batch.
        #[arg(long, default_value_t = core_api::scan_backfill::DEFAULT_BATCH)]
        batch_size: i64,
        /// Owner-role connection string (defaults to DATABASE_URL).
        #[arg(long)]
        database_url: Option<String>,
    },
}

/// Deployment-descriptor verbs.
#[derive(Debug, Subcommand)]
enum DeployAction {
    /// Emit the deploy descriptor: a human summary by default, the
    /// machine-readable JSON with `--json`.
    Describe {
        /// Emit the machine-readable JSON (the interface hq consumes).
        #[arg(long)]
        json: bool,
        /// Narrow the descriptor to one profile.
        #[arg(long)]
        profile: Option<String>,
        /// Probe this database for retrieval-mode satisfiability and the
        /// per-tenant embedding distribution. Defaults to `DATABASE_URL`;
        /// without either the descriptor reports `probe: not_run` instead of
        /// claiming the mode can run.
        #[arg(long)]
        database_url: Option<String>,
        /// Write to this path instead of stdout.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Regenerate `deploy/profiles/*.yaml` from the descriptor. A drift test
    /// in the crate fails when the checked-in copy falls behind.
    Profiles {
        #[arg(long, default_value = "deploy/profiles")]
        dir: std::path::PathBuf,
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
        Command::Deploy { action } => deploy_command(action).await,
        Command::Data { action } => data_command(action).await,
        Command::Attachments { action } => attachments_command(action).await,
        Command::Install {
            profile,
            dry_run,
            apply,
            dir,
            set,
            compose_file,
            memory_mb,
            ui,
            port,
            ui_dir,
            no_open,
        } => {
            install_command(
                profile,
                dry_run,
                apply,
                dir,
                set,
                compose_file,
                memory_mb,
                (ui, port, ui_dir, no_open),
            )
            .await
        }
        Command::Update {
            check,
            apply,
            manifest,
            signature,
            key,
            identity,
            dir,
            compose_file,
        } => {
            update_command(
                check,
                apply,
                manifest,
                signature,
                key,
                identity,
                dir,
                compose_file,
            )
            .await
        }
        Command::Artifact { action } => artifact_command(action),
        Command::Doctor {
            profile,
            dir,
            compose_file,
            memory_mb,
        } => doctor_command(profile, dir, compose_file, memory_mb),
    }
}

/// `marula update` (Wave 60 lane W60-5). Reads a signed manifest, says what is
/// available, and — only with a verified signature — backs the install up,
/// applies through the same target the installer used, verifies the result and
/// rolls the files back on failure.
#[allow(clippy::too_many_arguments)]
async fn update_command(
    check: bool,
    apply: bool,
    manifest: std::path::PathBuf,
    signature: Option<std::path::PathBuf>,
    keys: Vec<std::path::PathBuf>,
    identity: String,
    dir: std::path::PathBuf,
    compose_file: std::path::PathBuf,
) -> anyhow::Result<()> {
    use install::DeploymentTarget as _;

    anyhow::ensure!(
        !(check && apply),
        "--check asks what is available; --apply changes the install. Pick one."
    );
    anyhow::ensure!(check || apply, "pick --check or --apply.");

    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("reading the manifest at {}", manifest.display()))?;

    let verified = match &signature {
        Some(path) => {
            let detached = std::fs::read_to_string(path)
                .with_context(|| format!("reading the signature at {}", path.display()))?;
            // One rule for the whole command family: an OpenSSH signature is
            // verified by the tool that made it, a hex signature by the
            // ed25519 verifier. A manifest signed by the release pipeline is
            // the former, and refusing it as "not hex" was the W60-5 gap this
            // lane found.
            if update::artifact::is_ssh_signature(&detached) {
                let anchor = keys.first().ok_or_else(|| {
                    anyhow::anyhow!(
                        "an OpenSSH signature needs --key <allowed_signers>; trust is a file the \
                         operator names"
                    )
                })?;
                update::artifact::verify_ssh_signature(&manifest, path, anchor, &identity)?;
            } else {
                let trusted = keys
                    .iter()
                    .map(|path| update::read_public_key(path))
                    .collect::<Result<Vec<_>, _>>()?;
                update::verify_signature(&trusted, text.as_bytes(), &detached)?;
            }
            true
        }
        None => false,
    };

    if apply && !verified {
        anyhow::bail!(
            "--apply needs --signature and at least one --key. An unsigned manifest is a \
             suggestion, not an instruction."
        );
    }
    if !verified {
        eprintln!(
            "warning: no --signature/--key in this run, so the manifest is read but not trusted. \
             --apply refuses this."
        );
    }

    let manifest = update::Manifest::parse(&text)?;
    let (os, arch) = update::platform();
    let installed = update::installed_version(&dir);
    let report = update::check(&manifest, &installed, os, arch);
    println!("{}", report.sentence());
    for advisory in &report.advisories {
        println!(
            "  {} [{}] {} (fixed in {})",
            advisory.id,
            advisory.severity.word(),
            advisory.summary,
            advisory.fixed_in
        );
    }

    if check {
        return Ok(());
    }
    anyhow::ensure!(
        report.availability == update::Availability::Available,
        "nothing to apply: {} is current.",
        installed
    );

    let env_text = std::fs::read_to_string(dir.join("marula.env")).ok();
    let env = env_text
        .map(|text| install::doctor::parse_env_file(&text))
        .unwrap_or_default();
    let profile = match env.get("MARULA_ALL_IN_ONE").map(String::as_str) {
        Some("1") | Some("true") => "all-in-one".to_string(),
        _ => "split".to_string(),
    };

    let session = install::Session::new(dir.clone(), compose_file.clone(), None);
    let target = install::ComposeTarget::new(deploy::descriptor(), compose_file.clone())
        .with_env_file(dir.join("marula.env"));
    let facts = install::HostFacts::detect(&dir);
    let selection = install::Selection::new(profile.clone(), &dir);
    let plan = target.plan(&facts, &selection)?;
    let backup = update::backup(&dir, &installed)?;
    println!(
        "backed up {} file(s) to {}",
        backup.files().len(),
        backup.dir.display()
    );
    let outcome = update::run(&target, &plan, &backup, || match session.doctor(&profile) {
        Ok(verdict) if verdict.ok => Ok(()),
        Ok(verdict) => Err(format!(
            "the doctor reports the install is not healthy after the update ({} checks failed)",
            verdict.checks.iter().filter(|check| !check.ok).count()
        )),
        Err(reason) => Err(reason),
    });

    for line in &outcome.applied.done {
        println!("  ok  {line}");
    }
    if let Some(reason) = &outcome.rolled_back {
        eprintln!(
            "update rolled back: {reason}\nthe files were restored from {}: {}",
            backup.dir.display(),
            outcome.restored.join(", ")
        );
        anyhow::bail!("the update did not take");
    }
    println!(
        "updated to {} ({} steps). Still yours to do: {}",
        report.latest.as_deref().unwrap_or("the new release"),
        outcome.applied.done.len(),
        outcome.manual.join("; ")
    );
    Ok(())
}

/// `marula artifact verify|checksum` (Wave 60 lane W60-5). The verification a
/// release bootstrap runs before it executes anything.
fn artifact_command(action: ArtifactAction) -> anyhow::Result<()> {
    match action {
        ArtifactAction::Checksum { file } => {
            println!("{}  {}", update::checksum_file(&file)?, file.display());
            Ok(())
        }
        ArtifactAction::Verify {
            sums,
            signature,
            key,
            identity,
            root,
        } => {
            let bytes =
                std::fs::read(&sums).with_context(|| format!("reading {}", sums.display()))?;
            match signature {
                Some(path) => {
                    let detached = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading the signature at {}", path.display()))?;
                    if update::artifact::is_ssh_signature(&detached) {
                        let anchor = key.first().ok_or_else(|| {
                            anyhow::anyhow!(
                                "an OpenSSH signature needs --key <allowed_signers>. Trust is a \
                                 file the operator names, not a key this binary carries."
                            )
                        })?;
                        update::artifact::verify_ssh_signature(&sums, &path, anchor, &identity)?;
                        println!(
                            "OpenSSH signature verified for {identity} against {}",
                            anchor.display()
                        );
                    } else {
                        let keys = key
                            .iter()
                            .map(|key| update::read_public_key(key))
                            .collect::<Result<Vec<_>, _>>()?;
                        update::verify_signature(&keys, &bytes, &detached)?;
                        println!("ed25519 signature verified against {} key(s)", keys.len());
                    }
                }
                None => eprintln!(
                    "warning: no --signature/--key, so only the checksums are verified. A checksum \
                     proves the bytes arrived intact, not who published them."
                ),
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| anyhow::anyhow!("{} is not UTF-8: {error}", sums.display()))?;
            let count = update::verify_sums(text, &root)?;
            println!("verified {count} file(s) under {}", root.display());
            Ok(())
        }
    }
}

/// `marula data purge-org` (Wave 52 lane R6). The store is resolved from the
/// process environment exactly as core-api resolves it, so the verb deletes the
/// bytes the API wrote.
async fn data_command(action: DataAction) -> anyhow::Result<()> {
    match action {
        DataAction::PurgeOrg {
            org_id,
            requested_by,
            reason,
            confirm,
            dry_run,
            database_url,
        } => {
            let url = database_url
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());
            if !dry_run && std::env::var("OBJECT_STORE").is_err() {
                eprintln!(
                    "warning: OBJECT_STORE is unset, so the object leg targets the default local \
                     directory. Point the store variables at what core-api writes, or the receipt \
                     will say objects_incomplete."
                );
            }
            let store = core_store::build(&core_store::StoreChoice::from_env());
            let args = data::PurgeArgs {
                org_id,
                requested_by,
                reason,
                confirm,
                dry_run,
            };
            data::purge_org(&url, store.as_ref(), &args).await
        }
    }
}

/// `marula attachments backfill-verdicts` (Wave 55, follow-up F30). The store
/// and the scanner are resolved from the process environment exactly as
/// core-api resolves them, so the walk reads the bytes the API wrote and asks
/// the scanner the API asks.
async fn attachments_command(action: AttachmentAction) -> anyhow::Result<()> {
    match action {
        AttachmentAction::BackfillVerdicts {
            dry_run,
            confirm,
            org_id,
            batch_size,
            database_url,
        } => {
            let url = database_url
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());
            if std::env::var("OBJECT_STORE").is_err() {
                eprintln!(
                    "warning: OBJECT_STORE is unset, so the walk reads the default local \
                     directory. Point the store variables at what core-api writes, or every object \
                     reports unreadable."
                );
            }
            anyhow::ensure!(
                batch_size > 0,
                "--batch-size must be at least 1 (0 would fetch nothing and report success)"
            );
            let args = attachments::BackfillArgs {
                org_id,
                batch_size,
                dry_run,
                confirm,
            };
            attachments::backfill_verdicts(&url, &args).await
        }
    }
}

/// `marula deploy describe|profiles` (Wave 49 lane R7). Pure and synchronous:
/// it reads and writes files, never the network or the database.
async fn deploy_command(action: DeployAction) -> anyhow::Result<()> {
    match action {
        DeployAction::Describe {
            json,
            profile,
            database_url,
            out,
        } => {
            let database_url = database_url.or_else(|| std::env::var("DATABASE_URL").ok());
            let text = if json {
                deploy::describe_json_probed(profile.as_deref(), database_url.as_deref()).await?
            } else {
                deploy::describe_summary_probed(database_url.as_deref()).await
            };
            match out {
                Some(path) => {
                    std::fs::write(&path, text)
                        .with_context(|| format!("write {}", path.display()))?;
                    println!("wrote {}", path.display());
                }
                None => println!("{text}"),
            }
        }
        DeployAction::Profiles { dir } => {
            for path in deploy::write_profiles(&dir)? {
                println!("wrote {}", path.display());
            }
        }
    }
    Ok(())
}

/// `marula install` (Wave 60 lane W60-2): the recommendation, the plan, and,
/// with `--apply`, the execution. ADR 0026: one target per host, and `--apply`
/// runs exactly the steps the dry run printed.
#[allow(clippy::too_many_arguments)]
async fn install_command(
    profile: Option<String>,
    dry_run: bool,
    apply: bool,
    dir: std::path::PathBuf,
    set: Vec<String>,
    compose_file: std::path::PathBuf,
    memory_mb: Option<u32>,
    wizard: (bool, u16, std::path::PathBuf, bool),
) -> anyhow::Result<()> {
    use install::DeploymentTarget as _;

    anyhow::ensure!(
        !(dry_run && apply),
        "--dry-run prints the plan; --apply executes it. Pick one."
    );

    let (ui, port, ui_dir, no_open) = wizard;
    if ui {
        anyhow::ensure!(
            !dry_run && !apply && set.is_empty() && profile.is_none(),
            "--ui serves the wizard, which plans and applies on its own. Drop --dry-run, \
             --apply, --set and --profile; the wizard asks for the same things."
        );
        return install::server::serve(install::server::Options {
            dir,
            compose_file,
            memory_mb,
            port,
            ui_dir,
            open: !no_open,
        })
        .await;
    }

    let descriptor = deploy::descriptor();
    let facts = install::HostFacts::detect(&dir).with_declared_memory(memory_mb);
    let recommendation = install::recommend(&descriptor, &facts);

    let chosen = match profile.clone().or_else(|| recommendation.profile.clone()) {
        Some(name) => name,
        None => {
            let mut out =
                String::from("no shipped profile fits this host. What each one needs beyond it:\n");
            for shipped in &descriptor.profiles {
                let missing = install::plan::missing_for(shipped, &facts);
                if !missing.is_empty() {
                    out.push_str(&format!("  {}: {}\n", shipped.name, missing.join("; ")));
                }
            }
            if facts.ram_mb.is_none() {
                out.push_str(
                    "\nThis platform publishes no memory reading, so every profile was \
                     refused on memory. Declare it with --memory-mb or \
                     MARULA_HOST_MEMORY_MB, and the same host can be scored again.\n",
                );
            }
            out.push_str(
                "A smaller box can still run `all-in-one` with cloud embeddings: set \
                 MARULA_EMBEDDING_PROVIDER=openai and MARULA_EMBEDDING_API_KEY, then \
                 name a host that meets its cores and disk floors.",
            );
            anyhow::bail!("{out}");
        }
    };

    let mut selection = install::Selection::new(chosen.clone(), &dir);
    for assignment in &set {
        selection.set(assignment)?;
    }

    let target = install::ComposeTarget::new(descriptor, compose_file);
    let plan = target.plan(&facts, &selection)?;
    println!("{}", plan.render());

    if profile.is_none() {
        println!("recommendation for this host:");
        for line in &recommendation.why {
            println!("  - {line}");
        }
        if let Some(shortfall) = &recommendation.shortfall {
            println!(
                "  - {} would also need: {}",
                shortfall.profile,
                shortfall.needs.join("; ")
            );
        }
    }
    if !recommendation.embedding_fits {
        println!(
            "\nthe profile's local embedding model does not fit this host. Set \
             MARULA_EMBEDDING_PROVIDER=openai and MARULA_EMBEDDING_API_KEY, or the \
             process will refuse to boot (R14: cloud is the default when no local \
             model fits)."
        );
    }

    if !apply {
        println!(
            "\nnothing was written. Run again with --apply to execute these steps, \
             or keep this output as the plan."
        );
        return Ok(());
    }

    target.available(&facts)?;
    let applied = target.apply(&plan);
    for line in &applied.done {
        println!("  + {line}");
    }
    if let Some(failed) = &applied.failed {
        for item in &applied.manual {
            println!("  manual: {item}");
        }
        anyhow::bail!("install stopped: {failed}");
    }
    println!("\ninstall applied. Still yours to do:");
    for item in &applied.manual {
        println!("  - {item}");
    }
    println!("\nRun `marula doctor` to check the result.");
    Ok(())
}

/// `marula doctor` (Wave 60 lane W60-2): the planner's facts, read back as a
/// verdict. Reads `<dir>/marula.env` when it exists, and the process
/// environment otherwise, so it answers for an install it did not create.
fn doctor_command(
    profile: Option<String>,
    dir: std::path::PathBuf,
    compose_file: std::path::PathBuf,
    memory_mb: Option<u32>,
) -> anyhow::Result<()> {
    let descriptor = deploy::descriptor();
    let facts = install::HostFacts::detect(&dir).with_declared_memory(memory_mb);
    let recommendation = install::recommend(&descriptor, &facts);
    let name = profile
        .or_else(|| recommendation.profile.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no shipped profile fits this host; name one with --profile to check it anyway"
            )
        })?;
    let chosen = descriptor
        .profiles
        .iter()
        .find(|candidate| candidate.name == name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown profile {name:?} — known: {}",
                descriptor
                    .profiles
                    .iter()
                    .map(|profile| profile.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    let env_path = dir.join("marula.env");
    let (env, source) = match std::fs::read_to_string(&env_path) {
        Ok(text) => (
            install::doctor::parse_env_file(&text),
            env_path.display().to_string(),
        ),
        Err(_) => (
            std::env::vars().collect(),
            "the process environment (no marula.env here)".to_string(),
        ),
    };

    // The compose status answers first: it decides whether the port question is
    // "is this free" (no install running) or "does this install own it".
    use install::DeploymentTarget as _;
    let target = install::ComposeTarget::new(deploy::descriptor(), compose_file)
        .with_env_file(dir.join("marula.env"));
    let status = target.status();
    let running = !status.services.is_empty();

    println!("marula doctor — profile {name}, values from {source}\n");
    let checks = install::doctor::run_default(&descriptor, chosen, &facts, &env, !running);
    for check in &checks {
        println!("{}", check.render());
    }
    println!("\nservices ({} target):", status.target);
    match status.error {
        Some(error) => println!("  {error}"),
        None if status.services.is_empty() => println!("  nothing running"),
        None => {
            for service in &status.services {
                println!("  {service}");
            }
        }
    }

    if install::doctor::all_ok(&checks) {
        println!("\nall {} checks passed", checks.len());
        return Ok(());
    }
    let failed = checks.iter().filter(|check| !check.ok).count();
    anyhow::bail!("{failed} of {} checks failed", checks.len());
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
    // The retrieval index is DDL and this is the owner path, so a deployment
    // that selected `iterative` or `index` builds its index here. `exact`
    // (the default) builds nothing: an index costs build time and memory that
    // the many-small-tenants shape should not pay (Wave 52 lane R15).
    let mode = core_domain::retrieval::RetrievalMode::from_lookup(&|key| std::env::var(key).ok())
        .context("MARULA_RETRIEVAL_MODE names no retrieval mode")?;
    if mode != core_domain::retrieval::RetrievalMode::Exact {
        let report = core_db::retrieval::ensure_index(&pool, mode)
            .await
            .context("building the retrieval index")?;
        tracing::info!(
            mode = %mode,
            index = ?report.index,
            present = report.present,
            detail = %report.detail,
            "retrieval index"
        );
    }
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
    // Demo attachments are stored beside the ones a real upload would use,
    // so the download endpoint finds them. Same env var core-api's config
    // reads for the local object store; unset leaves the seeded rows
    // metadata-only, which the route reports as a missing object.
    let object_store_dir = std::env::var("OBJECT_STORE_LOCAL_DIR")
        .ok()
        .filter(|d| !d.trim().is_empty())
        .map(std::path::PathBuf::from);
    Ok(core_db::demo::DemoSeedOptions {
        seal_key: key,
        object_store_dir,
    })
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
        (
            "portal logins (one per client)",
            report.portal_users.to_string(),
        ),
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
        (
            "vendors / orders / client loans",
            format!(
                "{} / {} / {}",
                report.vendors, report.purchase_orders, report.client_loans
            ),
        ),
        (
            "receipts / bills / vendor contacts",
            format!(
                "{} / {} / {}",
                report.purchase_receipts, report.purchase_bills, report.vendors
            ),
        ),
        (
            "attachments (ticket / document)",
            format!(
                "{} / {} ({:.1} KiB stored)",
                report.attachments,
                report.document_attachments,
                report.attachment_store_bytes as f64 / 1024.0
            ),
        ),
        (
            "email (messages / outbound)",
            format!("{} / {}", report.email_messages, report.email_outbound),
        ),
        (
            "csat / secrets / change orders",
            format!(
                "{} / {} / {}",
                report.csat_surveys, report.secret_records, report.contract_change_orders
            ),
        ),
        ("time categories", report.time_categories.to_string()),
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
    // Portal seats are printed too: the portal is the half of the demo a
    // prospect's own staff touches, and an operator has to be able to hand
    // over a client login without digging through the database.
    println!();
    println!("portal logins (client view, same password):");
    for (email, client_name) in &report.portal_logins {
        println!("  {email:<26} {client_name}");
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

/// A watch channel that flips true on Ctrl-C; the worker uses it for
/// graceful shutdown.
fn shutdown_watch() -> watch::Receiver<bool> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });
    shutdown_rx
}

/// Shared telemetry bootstrap (the private build tracker 2.18): none-mode unless
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
            parse(&["seed-demo"]),
            Ok(Command::SeedDemo { force: false })
        ));
        assert!(matches!(
            parse(&["seed-demo", "--force"]),
            Ok(Command::SeedDemo { force: true })
        ));
        // The deploy descriptor verbs parse (Wave 49 lane R7).
        assert!(matches!(
            parse(&["deploy", "describe", "--json"]),
            Ok(Command::Deploy {
                action: DeployAction::Describe { json: true, .. }
            })
        ));
        assert!(matches!(
            parse(&["deploy", "profiles"]),
            Ok(Command::Deploy {
                action: DeployAction::Profiles { .. }
            })
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

    /// The erasure verb has to be reachable and its flags have to mean what the
    /// doc says: a dry run needs no confirmation, and executing carries the
    /// tenant label and the operator identity.
    #[test]
    fn parses_data_purge_org() {
        let cmd = parse(&[
            "data",
            "purge-org",
            "--org-id",
            "00000000-0000-4000-8000-000000000001",
            "--dry-run",
        ])
        .expect("parses");
        match cmd {
            Command::Data {
                action:
                    DataAction::PurgeOrg {
                        org_id,
                        dry_run,
                        confirm,
                        requested_by,
                        ..
                    },
            } => {
                assert_eq!(org_id.to_string(), "00000000-0000-4000-8000-000000000001");
                assert!(dry_run);
                assert!(confirm.is_none());
                assert!(requested_by.is_none());
            }
            _ => panic!("expected data purge-org"),
        }

        let cmd = parse(&[
            "data",
            "purge-org",
            "--org-id",
            "00000000-0000-4000-8000-000000000001",
            "--confirm",
            "acme",
            "--requested-by",
            "ops@example.test",
            "--reason",
            "customer request 1234",
        ])
        .expect("parses");
        assert!(matches!(
            cmd,
            Command::Data {
                action: DataAction::PurgeOrg { dry_run: false, .. }
            }
        ));

        // The org id is not optional: without it the verb would have to guess
        // which tenant to erase.
        assert!(parse(&["data", "purge-org", "--dry-run"]).is_err());
    }

    /// The verdict backfill's flags have to mean what the doc says: a dry run
    /// needs no confirmation, the org filter is optional (absent walks all of
    /// them), and the batch default is the shared constant.
    #[test]
    fn parses_attachments_backfill_verdicts() {
        let cmd = parse(&["attachments", "backfill-verdicts", "--dry-run"]).expect("parses");
        match cmd {
            Command::Attachments {
                action:
                    AttachmentAction::BackfillVerdicts {
                        dry_run,
                        confirm,
                        org_id,
                        batch_size,
                        ..
                    },
            } => {
                assert!(dry_run);
                assert!(confirm.is_none(), "a dry run confirms nothing");
                assert!(org_id.is_none(), "absent means every org");
                assert_eq!(batch_size, core_api::scan_backfill::DEFAULT_BATCH);
            }
            _ => panic!("expected attachments backfill-verdicts"),
        }

        let cmd = parse(&[
            "attachments",
            "backfill-verdicts",
            "--confirm",
            "37",
            "--org-id",
            "00000000-0000-4000-8000-000000000001",
            "--batch-size",
            "50",
        ])
        .expect("parses");
        match cmd {
            Command::Attachments {
                action:
                    AttachmentAction::BackfillVerdicts {
                        dry_run,
                        confirm,
                        org_id,
                        batch_size,
                        ..
                    },
            } => {
                assert!(!dry_run);
                assert_eq!(confirm, Some(37));
                assert_eq!(
                    org_id.unwrap().to_string(),
                    "00000000-0000-4000-8000-000000000001"
                );
                assert_eq!(batch_size, 50);
            }
            _ => panic!("expected attachments backfill-verdicts"),
        }
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
