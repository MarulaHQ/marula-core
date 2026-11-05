//! The Marula desktop client: a shell around the product's own front end.
//!
//! It launches in one of two modes, decided before anything else starts
//! (`connection.rs`):
//!
//! - **connected** (ADR 0028) — a client of an instance that is already
//!   running, self-hosted or hosted. Nothing is booted locally; the webview
//!   loads that deployment's own URL.
//! - **embedded** (ADR 0005) — the local test app: an embedded Postgres
//!   (`postgresql_embedded`), the existing core-api axum router mounted in this
//!   process, and the built web app served on loopback. No Docker, no sidecar
//!   binaries — the API is a library call.
//!
//! A first run with neither chosen opens the connect screen, which asks for an
//! instance address or takes this machine. Data lives in
//! `~/Library/Application Support/Marula` and persists across rebuilds;
//! migrations apply incrementally on startup.

use anyhow::{bail, Context};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Manager, RunEvent};
use tauri_plugin_updater::{Update, UpdaterExt};

mod connection;
mod launch_secrets;
mod os_integrations;
mod printers;
mod window_manager;
mod windows;

const PREFERRED_PG_PORT: u16 = 15434;
const PREFERRED_API_PORT: u16 = 15433;
const PREFERRED_WEB_PORT: u16 = 15435;
/// Ports the bundled Next server may bind, and — exactly — the origins
/// `desktop/capabilities/desktop-web.json` grants IPC to. The webview's origin
/// is this server, so a wider grant is a wider IPC surface; a narrower one
/// silently denies every app command on the launch that lands on a port the
/// file does not list. The two move together, and the capability test in this
/// file fails if they drift.
const WEB_PORT_CANDIDATES: std::ops::RangeInclusive<u16> = PREFERRED_WEB_PORT..=15439;
/// Raw-ZPL transport timeout (label printing must fail fast and honestly,
/// not hang the webview when the printer is off or the address is wrong).
const ZPL_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
const DEMO_LOGINS: &str =
    "owner@demo.local / dispatcher@demo.local / tech@demo.local / bookkeeper@demo.local";

struct Backend {
    postgres: postgresql_embedded::PostgreSQL,
    next_server: std::sync::Mutex<tokio::process::Child>,
}

/// Pid of the spawned Next.js standalone server, for the SIGTERM handler
/// (see spawn_next_server). Local to this process by construction.
static NEXT_SERVER_PID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// The loopback URL the webviews are served from — managed state so the
/// window factory can build sibling webviews onto the SAME server (one
/// origin: session cookie, BroadcastChannel bus, and CORS all hold).
struct WebUrl(String);

fn data_root() -> PathBuf {
    // macOS-first (this lane targets a double-clickable .app); other
    // platforms fall back to a conventional user-data location.
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/Marula")
    } else {
        PathBuf::from(home).join(".local/share/Marula")
    }
}

/// First free TCP port at or after `start`, for Postgres and the API.
fn free_port(start: u16) -> u16 {
    first_free_port(start..=start.saturating_add(99)).unwrap_or(start)
}

/// First free port in `candidates`, or `None` when they are all taken.
fn first_free_port(candidates: std::ops::RangeInclusive<u16>) -> Option<u16> {
    candidates
        .into_iter()
        .find(|port| std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok())
}

/// Spawn the Next.js standalone server (bundled at Resources/resources/web
/// by scripts/build-desktop.sh) on a loopback port and wait until it
/// accepts connections. The machine's `node` is used — this is a local
/// TEST app, not a shippable bundle (documented deviation in docs/adr/0005).
async fn spawn_next_server(
    bundled: &Path,
    run_dir: &Path,
    port: u16,
) -> anyhow::Result<tokio::process::Child> {
    let server_js = bundled.join("server.js");
    anyhow::ensure!(
        server_js.exists(),
        "Next standalone server not found at {} — run scripts/build-desktop.sh",
        server_js.display()
    );
    // Stage a symlink farm so the server runs from a writable dir whose
    // `.next` is not hidden from whatever walking lists it: every top-level
    // entry of the bundled standalone tree is symlinked into `run_dir`.
    std::fs::create_dir_all(run_dir)?;
    for entry in std::fs::read_dir(bundled)? {
        let entry = entry?;
        let dest = run_dir.join(entry.file_name());
        let _ = std::fs::remove_file(&dest);
        #[cfg(target_family = "unix")]
        std::os::unix::fs::symlink(entry.path(), &dest)?;
    }
    // GUI-launched apps get a minimal PATH — probe the usual Homebrew
    // locations first, then trust PATH as a dev-run fallback.
    let node = ["/opt/homebrew/bin/node", "/usr/local/bin/node", "node"]
        .into_iter()
        .find(|n| *n == "node" || Path::new(n).exists())
        .context("node not found — required to run the bundled Next server")?;
    let mut child = tokio::process::Command::new(node)
        .arg("server.js")
        .current_dir(run_dir)
        .env("PORT", port.to_string())
        .env("HOSTNAME", "127.0.0.1")
        .env("NODE_ENV", "production")
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        // Belt-and-braces: record the pid so the SIGTERM handler (which
        // exits without unwinding, skipping kill_on_drop) can still reap
        // the server. A leaked next-server orphans on its preferred port
        // and breaks later launches (orphan-window/CORS bug).
        .inspect(|child| {
            if let Some(pid) = child.id() {
                let _ = NEXT_SERVER_PID.set(pid);
            }
        })
        .context("spawning the Next standalone server (is `node` on PATH?)")?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        if child.try_wait()?.is_some() {
            bail!("the Next standalone server exited during startup");
        }
        if tokio::time::Instant::now() > deadline {
            bail!("the Next standalone server did not become ready in 30s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(child)
}

/// The migration set requires the `vector` extension (migration 0062).
/// Stock Postgres binaries don't ship pgvector; copy the library AND the
/// extension control/SQL scripts from a Homebrew install into the
/// embedded installation before first use, matched to the PINNED major —
/// dylibs are per-major ABI, so a @17 copy into an 18 install hard-fails
/// at CREATE EXTENSION time (p36-infra-transition cut: two version dirs
/// now share installation_dir; directory order is arbitrary).
fn ensure_pgvector(installation_dir: &Path, pinned_major: &str) -> anyhow::Result<()> {
    // postgresql_embedded installs into a versioned subdirectory
    // (e.g. installation_dir/18.6.0) next to retained older majors (e.g.
    // installation_dir/17.11.0); resolve the pinned one explicitly.
    let versioned = std::fs::read_dir(installation_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(pinned_major))
                && p.join("bin/initdb").exists()
        })
        .with_context(|| {
            format!(
                "no Postgres installation root for major {pinned_major} with bin/initdb under {installation_dir:?}"
            )
        })?;
    let lib_dir = versioned.join("lib");
    let ext_dir = versioned.join("share/extension");
    let control = ext_dir.join("vector.control");
    // This build's module DLSUFFIX is .dylib (see the bundled contrib
    // modules); copy under both names so any DLSUFFIX resolves.
    let dests = [lib_dir.join("vector.dylib"), lib_dir.join("vector.so")];
    if control.exists() && dests.iter().all(|d| d.exists()) {
        return Ok(());
    }
    let brew = PathBuf::from("/opt/homebrew/opt/pgvector/lib");
    let brew_usr = PathBuf::from("/usr/local/opt/pgvector/lib");
    let lib_candidates = [
        brew.clone()
            .join(format!("postgresql@{pinned_major}/vector.dylib")),
        brew_usr
            .clone()
            .join(format!("postgresql@{pinned_major}/vector.dylib")),
        brew.join("vector.dylib"),
        brew_usr.join("vector.dylib"),
    ];
    let brew_share = PathBuf::from("/opt/homebrew/opt/pgvector/share");
    let brew_usr_share = PathBuf::from("/usr/local/opt/pgvector/share");
    let share_candidates = [
        brew_share
            .clone()
            .join(format!("postgresql@{pinned_major}/extension")),
        brew_usr_share
            .clone()
            .join(format!("postgresql@{pinned_major}/extension")),
        brew_share.join("postgresql@18/extension"),
        brew_share.clone().join("extension"),
        brew_usr_share.join("postgresql@18/extension"),
        brew_usr_share.join("extension"),
    ];
    // Explicit major preference only: a @17 library or control/SQL set would
    // install a 17-ABI extension into an 18 server — never silently shipped.
    let src_lib = lib_candidates
        .iter()
        .find(|p| p.exists())
        .with_context(|| {
            "pgvector library not found — run `brew install pgvector` (the migration \
         set requires the `vector` extension; see docs/adr/0005)"
        })?;
    let src_share = share_candidates
        .iter()
        .find(|p| p.exists())
        .with_context(|| {
            "pgvector extension scripts not found — reinstall pgvector (`brew reinstall pgvector`)"
        })?;
    // Clean up copies made by earlier builds that targeted the wrong root.
    let _ = std::fs::remove_file(installation_dir.join("lib/vector.so"));
    let _ = std::fs::remove_dir_all(installation_dir.join("share/postgresql"));
    std::fs::create_dir_all(&lib_dir)?;
    std::fs::create_dir_all(&ext_dir)?;
    for dest in &dests {
        // The source dylib may be read-only (Homebrew ships 444); remove
        // any stale copy before overwriting.
        let _ = std::fs::remove_file(dest);
        std::fs::copy(src_lib, dest)
            .with_context(|| format!("copying {src_lib:?} into {dest:?}"))?;
    }
    for entry in std::fs::read_dir(src_share)?.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("vector") {
            std::fs::copy(entry.path(), ext_dir.join(&name))?;
        }
    }
    tracing::info!("installed pgvector into the embedded Postgres ({src_lib:?})");
    Ok(())
}

/// Directory containing the bundled Next.js standalone server. Inside the
/// .app it lives under Contents/Resources (copied there by the bundler from
/// desktop/resources/web); under `cargo run` it resolves relative to the
/// repo's apps/web build output.
fn next_standalone_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut anc = exe.ancestors();
        if anc.any(|p| p.file_name().is_some_and(|n| n == "MacOS")) {
            if let Some(resources) = exe.ancestors().nth(2).map(|c| c.join("Resources")) {
                let bundled = resources.join("resources/web");
                if bundled.join("apps/web/server.js").exists() {
                    return bundled;
                }
            }
        }
    }
    PathBuf::from("../apps/web/.next/standalone/apps/web")
}

async fn role_exists(pool: &sqlx::PgPool, role: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query("SELECT 1 FROM pg_roles WHERE rolname = $1")
        .bind(role)
        .fetch_optional(pool)
        .await?
        .is_some())
}

/// A SQL string literal for `value`, quoted by the server: `CREATE ROLE …
/// PASSWORD` takes a literal, not a bind parameter, so Postgres quotes it
/// (`quote_literal`) and this process never splices a credential into SQL
/// text.
async fn quote_literal(pool: &sqlx::PgPool, value: &str) -> anyhow::Result<String> {
    sqlx::query_scalar("SELECT quote_literal($1::text)")
        .bind(value)
        .fetch_one(pool)
        .await
        .context("quoting a credential for a role statement")
}

/// Rotate a role's password. Same reason as [`quote_literal`]: the statement
/// text comes back from the server (`format` with `%I`/`%L`), so a credential
/// is never concatenated here.
async fn alter_role_password(
    pool: &sqlx::PgPool,
    role: &str,
    password: &str,
) -> anyhow::Result<()> {
    let statement: String =
        sqlx::query_scalar("SELECT format('ALTER ROLE %I PASSWORD %L', $1::text, $2::text)")
            .bind(role)
            .bind(password)
            .fetch_one(pool)
            .await
            .with_context(|| format!("building the password rotation for role {role}"))?;
    sqlx::query(&statement)
        .execute(pool)
        .await
        .with_context(|| format!("rotating the password of role {role}"))?;
    Ok(())
}

/// Connect as the bootstrap superuser.
///
/// A cluster an older build initialized carries the published default
/// password. A release build cannot connect with its own credential until the
/// cluster has been rotated, so the published one is accepted exactly once,
/// for the migration, and replaced before anything else runs. Debug builds
/// hold the published value already and never take that path.
async fn open_super_pool(
    postgresql: &postgresql_embedded::PostgreSQL,
    credentials: &launch_secrets::PgCredentials,
) -> anyhow::Result<sqlx::PgPool> {
    let url_with = |password: &str| {
        let mut settings = postgresql.settings().clone();
        settings.password = password.to_string();
        settings.url("postgres")
    };
    match sqlx::postgres::PgPool::connect(&url_with(&credentials.superuser)).await {
        Ok(pool) => Ok(pool),
        Err(current) => {
            if credentials.superuser == launch_secrets::LEGACY_SUPERUSER_PASSWORD {
                return Err(current).context(
                    "connecting as the embedded Postgres superuser with the published default \
                     password (a debug build). A data directory created by a release build \
                     carries a per-install password instead: run a release build again, or move \
                     that pgdata aside to start a fresh demo database",
                );
            }
            let legacy = sqlx::postgres::PgPool::connect(&url_with(
                launch_secrets::LEGACY_SUPERUSER_PASSWORD,
            ))
            .await
            .context(
                "connecting as the embedded Postgres superuser, both with this install's \
                 credential and with the published default an older cluster was created with",
            )?;
            tracing::warn!(
                "this data directory was created with the published Postgres passwords; \
                 rotating every role onto this install's credentials"
            );
            alter_role_password(&legacy, "postgres", &credentials.superuser).await?;
            legacy.close().await;
            sqlx::postgres::PgPool::connect(&url_with(&credentials.superuser))
                .await
                .context("connecting after rotating the superuser password")
        }
    }
}

/// Cluster major of an existing pgdata directory (PG_VERSION), if present.
fn pgdata_major(pgdata: &Path) -> Option<String> {
    std::fs::read_to_string(pgdata.join("PG_VERSION"))
        .ok()
        .map(|v| v.trim().to_string())
}

/// Binaries dir of an installed version under the installation dir matching
/// `major` (the crate layouts clusters as installation_dir/{version}/bin).
fn install_bin_dir(installation_dir: &Path, major: &str) -> Option<PathBuf> {
    std::fs::read_dir(installation_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(major))
                && p.join("bin").join("pg_ctl").exists()
        })
        .map(|p| p.join("bin"))
}

async fn run_checked(mut cmd: tokio::process::Command, what: &str) -> anyhow::Result<()> {
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "{what} exited with {}: {}{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    Ok(())
}

/// Cut an existing embedded PG17 cluster to PG18 before the 18 setup() runs:
/// the 18 binaries refuse to start a 17 cluster's data directory. Strategy
/// (matches deploy/runbooks/pg17-to-pg18.md, dump/restore leg): start the
/// old cluster with ITS OWN retained binaries, pg_dumpall it, stop it, and
/// archive the pgdata directory for rollback — bootstrap then initializes a
/// fresh 18 cluster and replays the dump. Returns the dump path to replay
/// after the new cluster is up; Ok(None) means nothing to cut.
async fn cut_pg17_to_pg18(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let pgdata = root.join("pgdata");
    if pgdata_major(&pgdata).as_deref() != Some("17") {
        return Ok(None);
    }
    let old_bin = install_bin_dir(&root.join("pginstall"), "17").with_context(|| {
        "PG17 cluster found but the PG17 binaries are gone — if this data must be \
         kept, install the previous app build once (it re-downloads the 17 bitness) \
         or move pgdata aside manually"
    })?;
    let dump = root.join("pg17-restore.sql");
    let old_port = free_port(15499);

    let mut start = tokio::process::Command::new(old_bin.join("pg_ctl"));
    start
        .arg("-D")
        .arg(&pgdata)
        .arg("-l")
        .arg(root.join("pg17-cut.log"))
        .args(["-w", "start", "-o"])
        .arg(format!("-p {old_port} -c listen_addresses=127.0.0.1"));
    run_checked(start, "starting the PG17 cluster for its pre-cut dump").await?;

    let mut dumpall = tokio::process::Command::new(old_bin.join("pg_dumpall"));
    dumpall
        // A PG17 cluster predates this shell's credential split, so its
        // superuser carries the published password — that is the credential
        // the cut needs and the only place it is still used.
        .env("PGPASSWORD", launch_secrets::LEGACY_SUPERUSER_PASSWORD)
        .args(["-h", "127.0.0.1", "-U", "postgres"])
        .arg("-p")
        .arg(old_port.to_string())
        .arg("-f")
        .arg(&dump);
    run_checked(dumpall, "pg_dumpall of the PG17 cluster").await?;

    let mut stop = tokio::process::Command::new(old_bin.join("pg_ctl"));
    stop.arg("-D")
        .arg(&pgdata)
        .args(["-m", "fast", "-w", "stop"]);
    run_checked(stop, "stopping the PG17 cluster").await?;

    let archived = root.join("pgdata-pg17.bak");
    let _ = std::fs::remove_dir_all(&archived);
    std::fs::rename(&pgdata, &archived)
        .context("archiving the PG17 pgdata (rollback keeps the previous data)")?;
    tracing::info!(
        dir = %archived.display(),
        "PG17 cluster archived; the PG18 cluster will receive the dump replay"
    );
    Ok(Some(dump))
}

/// Replay a pg17 cut dump into the freshly initialized PG18 cluster. Runs
/// before the role/database bootstrap so an already-cut dump is authoritative.
/// The cluster is new, so it authenticates with the credential `initdb` was
/// given — this install's superuser password, never the published default.
async fn replay_pg17_dump(
    root: &Path,
    dump: &Path,
    pg_port: u16,
    superuser_password: &str,
) -> anyhow::Result<()> {
    let new_bin = install_bin_dir(&root.join("pginstall"), "18")
        .context("PG18 binaries missing right after setup — unexpected")?;
    let mut psql = tokio::process::Command::new(new_bin.join("psql"));
    psql.env("PGPASSWORD", superuser_password)
        .args(["-h", "127.0.0.1", "-U", "postgres", "-d", "postgres"])
        .arg("-p")
        .arg(pg_port.to_string())
        .args(["-v", "ON_ERROR_STOP=1", "-f"])
        .arg(dump);
    run_checked(psql, "replaying the PG17 dump into the PG18 cluster").await?;
    let _ = std::fs::remove_file(dump);
    tracing::info!("PG17 → PG18 cut complete (pg_dumpall replay)");
    Ok(())
}

/// Whether this launch seeds the demo org. The demo accounts are part of the
/// local TEST app, so the decision is the same one that mounts `/api/dev/*`
/// (ADR 0005, amendment 2026-10-25): a release build mounts no dev route and
/// seeds no demo account. [`core_api::config::Config::local_dev_tools`] is the
/// single source of truth, so the seed and the routes cannot disagree.
fn seeds_the_demo_org(release_build: bool) -> bool {
    core_api::config::Config::local_dev_tools(release_build)
}

async fn bootstrap_backend() -> anyhow::Result<(Backend, String, String)> {
    let root = data_root();
    std::fs::create_dir_all(&root)?;
    // Which loopback credentials this build uses. A debug build keeps the
    // repository's published dev secret and Postgres passwords (demo-seed
    // parity, compose parity, existing data dirs); a release build mints a
    // per-install secret and derives the passwords from it. See
    // `launch_secrets` for why the split exists.
    let secrets = launch_secrets::for_this_build(&root)?;
    if secrets.publishes_dev_defaults {
        tracing::warn!(
            "DEBUG BUILD: the in-process API trusts the repository's published dev JWT \
             secret and the embedded Postgres keeps its published passwords — every local \
             process can forge an owner token or connect as superuser. A release build \
             mints a per-install secret instead; never ship this build."
        );
    }
    let pg_port = free_port(PREFERRED_PG_PORT);

    let mut settings = postgresql_embedded::Settings::default();
    // Pin PG18 for parity with deploy/docker-compose.yml (pgvector/pgvector:pg18).
    if let Ok(v) = postgresql_embedded::VersionReq::parse("18") {
        settings.version = v;
    }
    settings.username = "postgres".into();
    settings.password = secrets.pg.superuser.clone();
    settings.data_dir = root.join("pgdata");
    settings.installation_dir = root.join("pginstall");
    settings.temporary = false;
    settings.port = pg_port;
    // `initdb --pwfile` reads the bootstrap password from a file. The crate's
    // default path is a temp dir it keeps for the process lifetime; put the
    // file in the app data dir at 0600 and delete it once the cluster exists,
    // so the credential lives in the cluster (hashed by initdb) rather than in
    // a file on disk. An existing cluster never reads it.
    let initdb_password_file = root.join("pgpass-init");
    launch_secrets::write_owner_only(&initdb_password_file, &secrets.pg.superuser)
        .with_context(|| format!("writing {}", initdb_password_file.display()))?;
    settings.password_file = initdb_password_file.clone();

    // A pre-18 embedded cluster cannot boot under the 18 binaries — cut it
    // first (pg_dumpall replay; the old directory is archived for rollback).
    let cut_dump = cut_pg17_to_pg18(&root)
        .await
        .context("PG17 → PG18 cut preflight")?;

    let mut postgresql = postgresql_embedded::PostgreSQL::new(settings);
    postgresql
        .setup()
        .await
        .context("setting up the embedded Postgres (first run downloads the binaries)")?;
    // initdb has hashed the password into the cluster; the plaintext file has
    // done its job.
    let _ = std::fs::remove_file(&initdb_password_file);
    ensure_pgvector(&root.join("pginstall"), "18")?;
    postgresql
        .start()
        .await
        .context("starting the embedded Postgres")?;
    let pg_port = postgresql.settings().port;

    let super_pool = open_super_pool(&postgresql, &secrets.pg).await?;
    if let Some(dump) = cut_dump {
        replay_pg17_dump(&root, &dump, pg_port, &secrets.pg.superuser)
            .await
            .context("replaying the archived PG17 cluster into the PG18 cluster")?;
    }
    // Parity with deploy/docker-compose.yml: migrations and the worker run
    // as the TABLE OWNER `marula`; migration 0001 itself provisions the
    // RLS-constrained `marula_app` role the API connects as. Compose's
    // POSTGRES_USER is a bootstrap superuser, so `marula` gets the same
    // here (the idempotent ALTER also repairs data dirs created before
    // this was a superuser).
    if !role_exists(&super_pool, "marula").await? {
        let create = format!(
            "CREATE ROLE marula LOGIN SUPERUSER PASSWORD {}",
            quote_literal(&super_pool, &secrets.pg.owner).await?
        );
        sqlx::query(&create)
            .execute(&super_pool)
            .await
            .context("creating the marula owner role")?;
    } else {
        sqlx::query("ALTER ROLE marula SUPERUSER")
            .execute(&super_pool)
            .await
            .context("aligning the marula owner role with compose parity")?;
        alter_role_password(&super_pool, "marula", &secrets.pg.owner).await?;
    }
    let db_missing = sqlx::query("SELECT 1 FROM pg_database WHERE datname = 'marula'")
        .fetch_optional(&super_pool)
        .await?
        .is_none();
    if db_missing {
        sqlx::query("CREATE DATABASE marula OWNER marula")
            .execute(&super_pool)
            .await
            .context("creating the marula database")?;
    }
    super_pool.close().await;

    let owner_url = format!(
        "postgres://marula:{}@127.0.0.1:{pg_port}/marula",
        secrets.pg.owner
    );
    let owner_pool = core_db::connect(&owner_url)
        .await
        .context("connecting as the schema owner")?;
    core_db::run_migrations(&owner_pool)
        .await
        .context("running migrations (owner role)")?;
    // Migration 0114 creates `marula_app` with the published default
    // password; move it onto this install's credential before anything
    // connects as the app role.
    alter_role_password(&owner_pool, "marula_app", &secrets.pg.app).await?;
    core_db::assert_rls_coverage(&owner_pool)
        .await
        .context("RLS coverage check")?;

    // Seed the deterministic demo org (same path as `marula-core-cli
    // seed-demo`) — part of the local TEST app, so it follows the same
    // build-posture decision that mounts `/api/dev/*` (ADR 0005, amendment
    // 2026-10-25): a release build exposes no dev surface and seeds no demo
    // account. Idempotent — an existing seed is left untouched. The seal key
    // follows the effective JWT secret, which is what the API derives.
    let seal = secrets.seal_key();
    if seeds_the_demo_org(!secrets.publishes_dev_defaults) {
        // `object_store_dir: None` matches the pre-existing behaviour (the test
        // app seeds no attachment payloads); the field was added to the options
        // struct after this call site was written, leaving the workspace red on
        // base. Wave 43 lane 15 closes that inherited red. Point it at a real
        // directory if the demo ever needs attachment fixtures.
        let opts = core_db::demo::DemoSeedOptions {
            seal_key: seal,
            object_store_dir: None,
        };
        match core_db::demo::seed(&owner_pool, &opts).await {
            Ok(_) => tracing::info!("demo org seeded; logins: {DEMO_LOGINS}"),
            Err(core_db::demo::DemoError::AlreadySeeded(_)) => {
                tracing::info!("demo org already seeded")
            }
            Err(e) => return Err(anyhow::anyhow!("demo seed failed: {e}")),
        }
    } else {
        tracing::info!(
            "release build: no demo org seeded and no /api/dev/* routes mounted — \
             sign in with an account you created"
        );
    }
    owner_pool.close().await;

    let app_url = format!(
        "postgres://marula_app:{}@127.0.0.1:{pg_port}/marula",
        secrets.pg.app
    );
    let api_port = free_port(PREFERRED_API_PORT);
    let api_url = format!("http://127.0.0.1:{api_port}");
    let web_port = first_free_port(WEB_PORT_CANDIDATES).with_context(|| {
        format!(
            "no free web port in {WEB_PORT_CANDIDATES:?} — the capability file grants IPC to \
             exactly these origins, so the app cannot move to another port"
        )
    })?;
    let web_url = format!("http://127.0.0.1:{web_port}");
    let next_server =
        spawn_next_server(&next_standalone_dir(), &root.join("webrun"), web_port).await?;

    let mut config =
        core_api::config::Config::from_local(app_url.clone(), api_port, root.join("objects"));
    // `from_local` pins the repository's published dev JWT secret and derives
    // the at-rest seal key from it. Replace both with this launch's secret so
    // a release build trusts nothing that ships in the source.
    config.jwt_secret = secrets.jwt_secret.clone();
    config.secret_key = core_api::config::SecretKey(seal);
    let config = Arc::new(config);
    let pool = core_db::connect(&config.database_url).await?;
    let state = core_api::state::AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::clone(&config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        reporting: core_api::state::AppState::reporting_pool(&config.database_url).await,
        store: core_store::build(&config.object_store),
        routing: core_api::routing::build_registry(&config),
    };
    tokio::spawn(core_api::realtime::run_listener(
        config.database_url.clone(),
    ));

    // The webview is served by the Next standalone server on a loopback
    // port, so cross-origin API calls need that origin allowed.
    let app = core_api::router(state).layer(
        tower_http::cors::CorsLayer::new()
            .allow_origin([
                web_url.parse().expect("valid origin"),
                "tauri://localhost".parse().expect("valid origin"),
                "http://tauri.localhost".parse().expect("valid origin"),
            ])
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            // Credentialed mode: the web app's httpOnly refresh cookie rides
            // every app→API call, so wildcard headers and missing
            // allow-credentials fail the browser's preflight (matches
            // core-api main's CORS posture).
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::header::HeaderName::from_static("x-marula-csrf"),
            ])
            .allow_credentials(true),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", api_port)).await?;
    tracing::info!(%api_url, "core-api mounted for the desktop app");
    tokio::spawn(async move {
        // Connect info, matching the core-api binary: the real-client-IP
        // resolver (Wave 47 lane A10) keys off the direct peer, never off a
        // caller-supplied `X-Forwarded-For`. Loopback-only bind, so the
        // narrow default trust set is the right posture here.
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        {
            tracing::error!("api server stopped: {e}");
        }
    });

    Ok((
        Backend {
            postgres: postgresql,
            next_server: std::sync::Mutex::new(next_server),
        },
        api_url,
        web_url,
    ))
}

/// SIGTERM (e.g. `kill`, logout) bypasses Tauri's run loop, so stop the
/// embedded Postgres directly from the handler — async-signal-safety is
/// knowingly bent here; this is a local test app, not a service.
fn install_sigterm_handler(data_dir: PathBuf) {
    static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let _ = DATA_DIR.set(data_dir);
    extern "C" fn handler(sig: i32) {
        // Reap the bundled Next server first — a leaked next-server keeps
        // its port and breaks the next launch (orphan-window/CORS bug).
        if let Some(pid) = NEXT_SERVER_PID.get() {
            unsafe {
                libc::kill(*pid as libc::pid_t, libc::SIGKILL);
            }
        }
        // Best-effort fast shutdown of the embedded Postgres, then exit.
        if let Some(data_dir) = DATA_DIR.get() {
            let install_root = data_dir.parent().unwrap().join("pginstall");
            if let Ok(entries) = std::fs::read_dir(&install_root) {
                for e in entries.flatten() {
                    let bin = e.path().join("bin/pg_ctl");
                    if bin.exists() {
                        let _ = std::process::Command::new(bin)
                            .args(["stop", "-D"])
                            .arg(data_dir)
                            .args(["-m", "fast"])
                            .output();
                    }
                }
            }
        }
        std::process::exit(sig);
    }
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handler as extern "C" fn(i32) as libc::sighandler_t,
        );
    }
}

// ---------------------------------------------------------------------------
// Raw ZPL transport (Wave 32 lane 5, `p32-labels-scanning`)
// ---------------------------------------------------------------------------

/// Sends raw ZPL bytes to a networked Zebra printer over a TCP socket, on the
/// printer's raw port (9100 unless the configured printer says otherwise).
/// This is the desktop-only half of the label print contract: the web app
/// renders the label server-side and hands the exact ZPL string down; this
/// command transports those bytes and NOTHING else — it never parses,
/// rewrites, or "improves" the stream.
///
/// The destination arrives already resolved against configured state (see
/// [`printers`]); this function is the transport and does not decide where to
/// send.
async fn send_zpl_to(host: &str, port: u16, zpl: &[u8]) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    if host.trim().is_empty() {
        bail!("printer host is empty");
    }
    let addr = format!("{}:{}", host.trim(), port);
    let mut stream =
        tokio::time::timeout(ZPL_CONNECT_TIMEOUT, tokio::net::TcpStream::connect(&addr))
            .await
            .map_err(|_| anyhow::anyhow!("connecting to {addr} timed out"))?
            .with_context(|| format!("connecting to {addr}"))?;
    tokio::time::timeout(ZPL_CONNECT_TIMEOUT, stream.write_all(zpl))
        .await
        .map_err(|_| anyhow::anyhow!("writing to {addr} timed out"))?
        .with_context(|| format!("writing ZPL to {addr}"))?;
    tokio::time::timeout(ZPL_CONNECT_TIMEOUT, stream.shutdown())
        .await
        .map_err(|_| anyhow::anyhow!("closing {addr} timed out"))?
        .with_context(|| format!("closing {addr}"))?;
    Ok(())
}

/// Tauri command: the web app's typed raw-ZPL door. Errors come back as
/// strings the UI shows as-is ("connecting to 10.0.0.9:9100: Connection
/// refused", or "printer 10.0.0.9:9100 is not configured — …") — never a
/// silent success.
///
/// `host` and `port` are a LOOKUP KEY, not a destination. The renderer may
/// only name a printer this shell already has in `<app data>/printers.json`;
/// the socket goes to the configured address. Without that rule the webview
/// (and anything running in its origin) holds a TCP write primitive against
/// every host the workstation can reach.
#[tauri::command]
async fn print_raw_zpl(
    registry: tauri::State<'_, printers::Registry>,
    host: String,
    port: Option<u16>,
    zpl: String,
) -> Result<(), String> {
    let destination = registry.resolve(&host, port.unwrap_or(printers::DEFAULT_PORT))?;
    send_zpl_to(&destination.host, destination.port, zpl.as_bytes())
        .await
        .map_err(|e| format!("{e:#}"))
}

// ---------------------------------------------------------------------------
// Desktop updater (Wave 34 lane 6, `p34-open-gaps-collector`)
// ---------------------------------------------------------------------------

/// Update checked on the Wave 20 auto-update channel but not yet applied.
/// Managed in app state so the install command applies the EXACT update the
/// UI reported — one check, one apply, no second channel round-trip that
/// could resolve to a different release mid-flow.
#[derive(Default)]
struct PendingUpdate(std::sync::Mutex<Option<Update>>);

/// What the web app shows for an available update (camelCase wire shape).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopUpdateInfo {
    version: String,
    current_version: String,
    date: Option<String>,
    notes: Option<String>,
}

/// Tauri command: runs the real update check against the channel configured
/// in `tauri.conf.json` (endpoint + minisign pubkey; config is the only
/// source — the frontend cannot redirect the check). Returns `None` when the
/// running version is current; the typed error string is surfaced as-is
/// (e.g. the placeholder channel host is not deployed yet).
#[tauri::command]
async fn check_desktop_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<Option<DesktopUpdateInfo>, String> {
    let updater = app.updater().map_err(|e| format!("{e:#}"))?;
    let update = updater.check().await.map_err(|e| format!("{e:#}"))?;
    let info = update.as_ref().map(|u| DesktopUpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        date: u.date.map(|d| d.to_string()),
        notes: u.body.clone(),
    });
    *pending.0.lock().expect("pending update mutex") = update;
    Ok(info)
}

/// Tauri command: downloads + verifies (minisign against the pinned pubkey)
/// and installs the update checked by [`check_desktop_update`], then
/// relaunches into it. Refuses honestly when nothing was checked first. The
/// checked update stays in state until the restart, so a failed download
/// can be retried from the UI without another channel round-trip.
#[tauri::command]
async fn install_desktop_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<(), String> {
    let update = pending
        .0
        .lock()
        .expect("pending update mutex")
        .as_ref()
        .cloned()
        .ok_or_else(|| "no update has been checked".to_string())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("{e:#}"))?;
    // macOS/Linux need an explicit relaunch into the installed version.
    app.restart()
}

/// `generate_context!` can be expanded once per crate: it defines the embedded
/// Info.plist and asset symbols. Every mode builds the same app from it.
fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let root = data_root();
    match start_mode(&root) {
        Start::Ready(connection::Mode::Local) => {
            if cfg!(target_os = "macos") {
                run_embedded(root)
            } else {
                // The embedded app bundles a Postgres sidecar and the Next
                // standalone server, which this build does not ship. A stored
                // choice written on another machine must not reach the boot
                // path and fail halfway; ask again on the connect screen.
                run_connect(
                    root,
                    false,
                    Some(
                        "This build connects to a Marula instance. The embedded app is macOS-only."
                            .to_string(),
                    ),
                )
            }
        }
        Start::Ready(connection::Mode::Remote { server_url }) => run_connected(root, server_url),
        Start::FirstRun { embedded, notice } => run_connect(root, embedded, notice),
    }
}

/// What this launch is going to be.
enum Start {
    /// A mode was chosen: a flag, the environment, or `connection.json`.
    Ready(connection::Mode),
    /// Nothing has been chosen yet, so the shell opens the connect screen.
    FirstRun {
        embedded: bool,
        notice: Option<String>,
    },
}

const USAGE: &str = "\
Usage: marula-desktop [--server <url> | --local]

  --server <url>  Connect this client to a running instance, and remember it.
  --local         Run the embedded app on this machine, and remember it.
  --help          Print this.

With no flag the client uses the stored choice, and asks on the first run.
";

/// Decide the mode before anything starts, because the answer decides whether
/// this process boots a database at all.
///
/// A flag is an instruction, not a hint: it is written to `connection.json`, so
/// an install script that copies the bundle into place can settle the choice in
/// the same step. Deep links arrive in argv as well, so an argument that is not
/// one of ours is left alone rather than refused.
fn start_mode(root: &Path) -> Start {
    let mut choice = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--local" => {
                if cfg!(target_os = "macos") {
                    choice = Some(connection::Mode::Local);
                } else {
                    eprintln!(
                        "--local runs the embedded app, which is macOS-only. Use --server <url>."
                    );
                    std::process::exit(2);
                }
            }
            "--server" => {
                let Some(raw) = args.next() else {
                    eprintln!("--server needs an address, for example https://psa.example.com");
                    std::process::exit(2);
                };
                match connection::parse_server_url(&raw) {
                    Ok(server_url) => choice = Some(connection::Mode::Remote { server_url }),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(2);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(mode) = choice {
        if let Err(e) = connection::save(root, &mode) {
            tracing::warn!("could not remember the chosen connection: {e}");
        }
        return Start::Ready(mode);
    }
    match connection::resolve(root) {
        Ok(Some(mode)) => Start::Ready(mode),
        Ok(None) => Start::FirstRun {
            embedded: embedded_available(),
            notice: None,
        },
        Err(e) => {
            // A file we cannot read is a first run with an explanation, not a
            // silent reset into a mode the person did not pick.
            tracing::warn!("{e}");
            Start::FirstRun {
                embedded: embedded_available(),
                notice: Some(e),
            }
        }
    }
}

/// Is the embedded app (ADR 0005) present in this build? The standalone Next.js
/// server is the marker: without it there is nothing to run on this machine,
/// and the connect screen offers the instance only.
fn embedded_available() -> bool {
    // The embedded app is macOS-only until the per-OS Postgres sidecar and the
    // Next standalone tree are staged for Linux and Windows (ADR 0031). The
    // marker alone is not enough: a resources directory copied from a macOS
    // build must not make a Linux client offer a mode it cannot run.
    cfg!(target_os = "macos") && next_standalone_dir().join("server.js").exists()
}

/// The part of the app every mode shares: the update channel, single instance,
/// the typed command surface, and the shell's own files.
fn base_builder(
    window_state_path: PathBuf,
    printer_registry: printers::Registry,
) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        // Auto-update channel (GA packaging): the updater reads its endpoint
        // and pubkey from tauri.conf.json `plugins.updater`. The check is
        // exposed to the web app as the typed `check_desktop_update` /
        // `install_desktop_update` commands (Wave 34 lane 6) — the plugin's own
        // JS commands are deliberately NOT granted; the UI rides the app ACL
        // only. Release workflow lives in docs/desktop-release.md.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Wave 38 lane 2: `marula://` deep links open-or-focus the right
        // window; single instance makes a second launch focus the running app
        // (and hand it the deep link it was opened with). Deep-link OS
        // registration is the bundle Info.plist (build-desktop.sh); runtime
        // delivery is RunEvent::Opened (macOS warm launch), argv on first
        // launch, and this hook's argv for Linux/Windows re-launches.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(link) = argv.iter().find(|a| a.starts_with("marula://")) {
                let app = app.clone();
                let link = link.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = os_integrations::route_deep_link(&app, &link).await {
                        tracing::warn!("deep link routing failed: {e}");
                    }
                });
            } else if let Some(window) = app
                .get_webview_window("main")
                .or_else(|| app.get_webview_window("connect"))
            {
                // A second launch before a mode is chosen focuses the connect
                // window rather than opening a second one.
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // Wave 32 lane 5: typed raw-ZPL door for label printing. Custom app
        // commands are ACL-gated for this shell's remote web origin (tauri
        // >= 2.11); the grants live in desktop/capabilities/ and are generated
        // from desktop/build.rs.
        .invoke_handler(tauri::generate_handler![
            print_raw_zpl,
            check_desktop_update,
            install_desktop_update,
            window_manager::open_native_window,
            window_manager::focus_native_window,
            os_integrations::route_marula_link,
            save_connection
        ])
        .manage(PendingUpdate::default())
        .manage(window_manager::WindowRegistry::new(window_state_path))
        .manage(printer_registry)
}

/// The main window: the product's own front end, with the API base injected
/// before its scripts run.
///
/// In connected mode `web_url` is the instance itself, so the page and the API
/// share an origin and the session cookie is first-party. That is also why a
/// connected page cannot call the app commands: `desktop/capabilities/desktop-web.json`
/// grants them to the loopback origins this shell serves, and an origin the
/// person typed is not one of those. The shell's own paths still reach them —
/// the window, the menu, the tray, deep links and the update channel start in
/// Rust, not in the page.
fn open_main_window(
    app: &tauri::AppHandle,
    web_url: &str,
    api_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let injected = serde_json::to_string(api_url).expect("a string serializes");
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(web_url.parse()?))
        .title("Marula")
        .inner_size(1440.0, 900.0)
        .min_inner_size(900.0, 640.0)
        .initialization_script(format!("window.__MARULA_API_URL__ = {injected};"))
        // Windows are the point: native tabbing stays OFF. Without an explicit
        // tabbing identifier macOS never auto-groups this window into tabs, and
        // the Window menu lists windows rather than merging them.
        .build()?;
    Ok(())
}

/// Native chrome: the menu bar (Window menu with the tabs-off posture) and the
/// tray quick actions. Failures are honest but non-fatal — a tray-less launch
/// beats a dead app.
fn chrome(app: &tauri::AppHandle) {
    if let Err(e) = os_integrations::build_menu(app) {
        tracing::warn!("menu bar unavailable: {e}");
    }
    if let Err(e) = os_integrations::build_tray(app) {
        tracing::warn!("tray unavailable: {e}");
    }
}

/// Any deep link this FIRST launch was opened with (macOS hands warm launches
/// over through RunEvent::Opened instead).
fn first_launch_deep_links(app: &tauri::AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        for arg in std::env::args().skip(1) {
            if arg.starts_with("marula://") {
                let app = handle.clone();
                tauri::async_runtime::block_on(async move {
                    if let Err(e) = os_integrations::route_deep_link(&app, &arg).await {
                        tracing::warn!("launch deep link failed: {e}");
                    }
                });
            }
        }
    });
}

/// The event loop every mode ends in.
fn run_event_loop(app: tauri::App<tauri::Wry>) {
    app.run(|app, event| match event {
        // macOS: warm-launch deep links (dock right-click "Open URL", OS open
        // with the app running) arrive here.
        RunEvent::Opened { urls } => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                for url in urls {
                    let link = url.to_string();
                    if let Err(e) = os_integrations::route_deep_link(&app, &link).await {
                        tracing::warn!("deep link {link} failed: {e}");
                    }
                }
            });
        }
        RunEvent::Exit => {
            // Graceful shutdown: stop the embedded Postgres so the data dir is
            // left consistent for the next launch. A connected client has no
            // backend state, so this is a no-op there.
            if let Some(b) = app.try_state::<Backend>() {
                if let Err(e) = tauri::async_runtime::block_on(b.postgres.stop()) {
                    eprintln!("stopping embedded Postgres failed: {e}");
                }
                // kill_on_drop also covers this; explicit kill keeps the
                // shutdown order deterministic (web, then PG).
                let _ = b.next_server.lock().map(|mut c| c.start_kill());
            }
        }
        _ => {}
    });
}

/// The embedded app from ADR 0005: Postgres, core-api and the bundled Next.js
/// server, all in this process, on loopback ports.
fn run_embedded(root: PathBuf) {
    let (backend, api_url, web_url) = match tauri::async_runtime::block_on(bootstrap_backend()) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("Marula desktop failed to start: {e:#}");
            std::process::exit(1);
        }
    };
    install_sigterm_handler(root.join("pgdata"));

    // Configured printers (raw-ZPL destination check): the shell owns the
    // list, the renderer only names an entry. The template is written on first
    // launch so the file the refusal message names actually exists.
    if let Err(e) = printers::ensure_config(&root) {
        tracing::warn!("could not write the printer config template: {e}");
    }
    let printer_registry = printers::Registry::load(&root);

    let app = base_builder(root.join("window-state.json"), printer_registry)
        .manage(backend)
        .manage(WebUrl(web_url.clone()))
        .manage(window_manager::ApiUrl(api_url.clone()))
        .setup(move |app| {
            open_main_window(app.handle(), &web_url, &api_url)?;
            chrome(app.handle());
            first_launch_deep_links(app.handle());
            Ok(())
        })
        .build(app_context())
        .expect("building the Tauri app");
    run_event_loop(app);
}

/// A client of an instance that is already running. Nothing is booted: no
/// database, no second copy of the API, no bundled web server. The window loads
/// the instance's own deployment, which is the same front end a browser gets.
fn run_connected(root: PathBuf, server_url: String) {
    let printer_registry = printers::Registry::load(&root);
    let app = base_builder(root.join("window-state.json"), printer_registry)
        .manage(WebUrl(server_url.clone()))
        .manage(window_manager::ApiUrl(server_url.clone()))
        .setup(move |app| {
            open_main_window(app.handle(), &server_url, &server_url)?;
            chrome(app.handle());
            first_launch_deep_links(app.handle());
            Ok(())
        })
        .build(app_context())
        .expect("building the Tauri app");
    run_event_loop(app);
}

/// The first run: one screen, one decision, then the window it implies.
fn run_connect(root: PathBuf, embedded: bool, notice: Option<String>) {
    let printer_registry = printers::Registry::load(&root);
    let state = serde_json::json!({ "embedded": embedded, "notice": notice });
    let injected = serde_json::to_string(&state).expect("a JSON value serializes");
    let app = base_builder(root.join("window-state.json"), printer_registry)
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "connect",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Connect to Marula")
            .inner_size(760.0, 620.0)
            .min_inner_size(560.0, 540.0)
            .center()
            .initialization_script(format!("window.__MARULA_CONNECT__ = {injected};"))
            .build()?;
            // This window is the whole app until a mode is chosen, and macOS
            // keeps a windowless app alive by convention. Closing it ends the
            // launch rather than leaving a dock icon with no way back: there is
            // no tray yet, and no main window to reopen.
            let handle = app.handle().clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    handle.exit(0);
                }
            });
            Ok(())
        })
        .build(app_context())
        .expect("building the Tauri app");
    run_event_loop(app);
}

/// Tauri command: the connect screen's one decision.
///
/// `mode` says which decision it is, so an empty address can never be read as
/// "run it here" — the local path has to be asked for. The choice is written
/// before the session starts, so a failure between the two lands on the mode the
/// person chose instead of asking again.
#[tauri::command]
async fn save_connection(
    app: tauri::AppHandle,
    mode: String,
    server_url: Option<String>,
) -> Result<(), String> {
    let mode = match mode.as_str() {
        "local" if cfg!(target_os = "macos") => connection::Mode::Local,
        "local" => {
            return Err(
                "This build connects to a Marula instance. The embedded app is macOS-only."
                    .to_string(),
            )
        }
        "remote" => connection::Mode::Remote {
            server_url: connection::parse_server_url(server_url.as_deref().unwrap_or_default())?,
        },
        other => return Err(format!("unknown connection mode: {other}")),
    };
    connection::save(&data_root(), &mode)
        .map_err(|e| format!("The choice could not be saved: {e}"))?;
    begin_session(&app, mode).await
}

/// Start the session the choice implies, then replace the connect window with
/// the main one.
async fn begin_session(app: &tauri::AppHandle, mode: connection::Mode) -> Result<(), String> {
    let (web_url, api_url) = match mode {
        connection::Mode::Remote { server_url } => (server_url.clone(), server_url),
        connection::Mode::Local => {
            let root = data_root();
            install_sigterm_handler(root.join("pgdata"));
            let (backend, api_url, web_url) = bootstrap_backend()
                .await
                .map_err(|e| format!("Marula could not start on this machine: {e:#}"))?;
            app.manage(backend);
            (web_url, api_url)
        }
    };
    app.manage(WebUrl(web_url.clone()));
    app.manage(window_manager::ApiUrl(api_url.clone()));
    open_main_window(app, &web_url, &api_url).map_err(|e| e.to_string())?;
    chrome(app);
    if let Some(connect) = app.get_webview_window("connect") {
        let _ = connect.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release build ships neither the `/api/dev/*` routes nor the demo
    /// accounts they exist for (ADR 0005, amendment 2026-10-25). The seed
    /// decision is the same one `Config::from_local` makes, so the two cannot
    /// drift apart: a build that mounts no dev route must seed nothing.
    #[test]
    fn a_release_build_seeds_no_demo_org() {
        assert!(
            seeds_the_demo_org(false),
            "a debug build keeps the demo logins the ADR exists for"
        );
        assert!(
            !seeds_the_demo_org(true),
            "a release build must seed no demo account"
        );
        for release_build in [false, true] {
            assert_eq!(
                seeds_the_demo_org(release_build),
                core_api::config::Config::local_dev_tools(release_build),
                "the seed follows the dev-surface decision for every posture"
            );
        }
    }

    /// Command-level check without a physical printer: a local listener
    /// stands in for the Zebra and must receive the EXACT ZPL bytes.
    #[tokio::test]
    async fn print_raw_zpl_delivers_exact_bytes_to_a_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let printer = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut sock, &mut buf)
                .await
                .expect("read");
            buf
        });
        let zpl = "^XA\n^CI28\n^FO12,12\n^A0N,26,16\n^FH^FDHello^FS\n^XZ\n";
        send_zpl_to("127.0.0.1", addr.port(), zpl.as_bytes())
            .await
            .expect("send");
        let received = printer.await.expect("printer task");
        assert_eq!(
            String::from_utf8(received).expect("utf8"),
            zpl,
            "the printer receives the exact ZPL bytes the core rendered"
        );
    }

    #[tokio::test]
    async fn empty_host_is_refused_honestly() {
        let err = send_zpl_to("   ", 9100, b"^XA^XZ\n").await.unwrap_err();
        assert!(format!("{err:#}").contains("empty"), "{err:#}");
    }

    /// The ZPL door end to end, without a printer: a listener is configured in
    /// the registry, the command's resolve step is what turns the caller's
    /// string into the address, and the bytes arrive. A second call to a
    /// printer that is NOT configured is refused before a socket exists — a
    /// listener that counts connections on that port proves it.
    #[tokio::test]
    async fn a_configured_printer_is_reached_and_an_unconfigured_one_is_refused() {
        let configured = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind configured printer");
        let configured_addr = configured.local_addr().expect("addr");
        let configured_printer = tokio::spawn(async move {
            let (mut sock, _) = configured.accept().await.expect("accept");
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut sock, &mut buf)
                .await
                .expect("read");
            buf
        });

        let unconfigured = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind unconfigured printer");
        let unconfigured_addr = unconfigured.local_addr().expect("addr");
        let refused_connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&refused_connections);
        tokio::spawn(async move {
            while unconfigured.accept().await.is_ok() {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            root.path().join(printers::CONFIG_FILE),
            format!(
                r#"{{"printers":[{{"name":"configured","host":"127.0.0.1","port":{}}}]}}"#,
                configured_addr.port()
            ),
        )
        .expect("write printer config");
        let registry = printers::Registry::load(root.path());

        let destination = registry
            .resolve("127.0.0.1", configured_addr.port())
            .expect("the configured printer resolves");
        send_zpl_to(&destination.host, destination.port, b"^XA^XZ\n")
            .await
            .expect("send to the configured printer");
        assert_eq!(configured_printer.await.expect("printer task"), b"^XA^XZ\n");

        let refusal = registry
            .resolve("127.0.0.1", unconfigured_addr.port())
            .expect_err("an unconfigured printer must be refused");
        assert!(refusal.contains("not configured"), "{refusal}");
        // Give the listener a moment to feel a connection that must not exist.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            refused_connections.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a refused destination must never open a socket"
        );
    }

    /// ACL wiring contract (Wave 34 lane 6, narrowed in Wave 54): the web app
    /// loads from a REMOTE loopback origin, and since tauri 2.11 remote origins
    /// need an explicit `remote` capability even for custom commands. This pins
    /// two things at once: the app commands the shell exposes, and the exact
    /// origins that may invoke them. Losing a permission silently disables the
    /// ZPL transport or the update UI in the real shell while every browser
    /// test stays green; widening the URLs hands those commands to any local
    /// HTTP server.
    #[test]
    fn the_capability_grants_the_app_commands_to_exactly_the_shells_web_origins() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capabilities/desktop-web.json"
        ))
        .expect("capability file exists");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("capability parses");

        assert_eq!(v["windows"][0], "main");
        assert_eq!(v["windows"][1], "subwindow:*");

        let listed: Vec<String> = v["remote"]["urls"]
            .as_array()
            .expect("remote urls array")
            .iter()
            .map(|url| url.as_str().expect("url is a string").to_string())
            .collect();
        let expected: Vec<String> = WEB_PORT_CANDIDATES
            .map(|port| format!("http://127.0.0.1:{port}"))
            .collect();
        assert_eq!(
            listed, expected,
            "the capability must grant exactly the ports the shell may bind \
             (WEB_PORT_CANDIDATES in src/main.rs)"
        );
        assert!(
            listed.iter().all(|url| !url.contains('*')),
            "a wildcard origin would grant these commands to any local server: {listed:?}"
        );

        let permissions = v["permissions"].as_array().expect("permissions array");
        for expected in [
            "allow-print-raw-zpl",
            "allow-check-desktop-update",
            "allow-install-desktop-update",
            "allow-open-native-window",
            "allow-focus-native-window",
            "allow-route-marula-link",
        ] {
            assert!(
                permissions.contains(&serde_json::json!(expected)),
                "capability must grant {expected}"
            );
        }
    }

    /// The port candidates are a closed set, and the fallback stays inside it:
    /// `first_free_port` is what bootstrap asks for, and a launch may not land
    /// on an origin the capability does not grant.
    #[test]
    fn the_web_port_candidates_are_bounded_and_held_to_the_capability() {
        let ports: Vec<u16> = WEB_PORT_CANDIDATES.collect();
        assert_eq!(ports.first(), Some(&PREFERRED_WEB_PORT));
        assert!(ports.len() >= 2, "one candidate leaves no fallback");
        assert!(
            ports.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "the range must be contiguous: {ports:?}"
        );
        // Every candidate is either bindable here or already taken by another
        // process; either way the answer stays inside the granted set.
        assert!(first_free_port(WEB_PORT_CANDIDATES)
            .is_none_or(|port| WEB_PORT_CANDIDATES.contains(&port)));
    }

    /// The narrowing, proven against tauri's own URLPattern matcher instead of
    /// by eye: every origin the shell may serve matches, and a neighbouring
    /// port, another host spelling and another scheme do not. This is the test
    /// that would fail if someone widened the file back to a wildcard — and the
    /// one that keeps the grant honest if the port candidates ever move.
    #[test]
    fn the_capability_origins_match_the_webview_origin_and_nothing_else() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capabilities/desktop-web.json"
        ))
        .expect("capability file exists");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("capability parses");
        let patterns: Vec<tauri::utils::acl::RemoteUrlPattern> = v["remote"]["urls"]
            .as_array()
            .expect("remote urls array")
            .iter()
            .map(|url| {
                url.as_str()
                    .expect("url is a string")
                    .parse()
                    .expect("a parseable URL pattern")
            })
            .collect();

        for port in WEB_PORT_CANDIDATES {
            let served = tauri::Url::parse(&format!("http://127.0.0.1:{port}/tickets/42"))
                .expect("parse served origin");
            assert!(
                patterns.iter().any(|pattern| pattern.test(&served)),
                "the webview's own origin {served} must be granted IPC"
            );
        }

        for refused in [
            // One past the candidate set: the shell may not serve here.
            "http://127.0.0.1:15440/tickets/42",
            // The API and Postgres ports are not webview origins.
            "http://127.0.0.1:15433/login",
            "http://127.0.0.1:15434/login",
            // Another spelling of loopback, another scheme, another host.
            "http://localhost:15435/login",
            "https://127.0.0.1:15435/login",
            "http://192.168.1.20:15435/login",
        ] {
            let url = tauri::Url::parse(refused).expect("parse refused origin");
            assert!(
                !patterns.iter().any(|pattern| pattern.test(&url)),
                "{refused} must not match a granted origin"
            );
        }
    }

    /// The connect screen holds exactly one grant. A capability that grew a
    /// second permission would be a first-run window able to do more than
    /// choose where the product lives, and nobody would see it happen.
    #[test]
    fn the_connect_capability_grants_only_the_connection_decision() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capabilities/connect.json"
        ))
        .expect("connect capability exists");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("capability parses");
        assert_eq!(v["windows"], serde_json::json!(["connect"]));
        assert_eq!(
            v["permissions"],
            serde_json::json!(["core:default", "allow-save-connection"])
        );
        assert!(
            v.get("remote").is_none(),
            "the connect screen is served from the bundle, so it needs no remote grant"
        );
    }

    /// `build.frontendDist` is `./connect`, and a missing asset there is a
    /// first-run window with no styles or no script — visible only on a machine
    /// that has never run the app, which is the worst place to find it.
    #[test]
    fn the_connect_screen_ships_with_the_crate() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("tauri.conf.json")).expect("config exists"),
        )
        .expect("config parses");
        assert_eq!(conf["build"]["frontendDist"], "./connect");
        let index = std::fs::read_to_string(dir.join("connect/index.html")).expect("index.html");
        for asset in ["connect.css", "connect.js"] {
            assert!(
                dir.join("connect").join(asset).exists(),
                "{asset} is missing from the connect screen"
            );
            assert!(index.contains(asset), "index.html does not load {asset}");
        }
        let css = std::fs::read_to_string(dir.join("connect/connect.css")).expect("connect.css");
        assert!(
            dir.join("connect/figtree-variable-latin.woff2").exists(),
            "the brand face is missing from the connect screen"
        );
        assert!(
            css.contains("figtree-variable-latin.woff2"),
            "connect.css does not load the brand face"
        );
    }
}
