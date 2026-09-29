//! Meridian desktop test app (ADR 0005).
//!
//! One double-clickable .app for local testing: an embedded Postgres
//! (`postgresql_embedded`) + the existing core-api axum router mounted in
//! this process + the built web app as Tauri assets. No Docker, no
//! sidecar binaries — the API is a library call.
//!
//! Data lives in `~/Library/Application Support/Meridian` and persists
//! across rebuilds; migrations apply incrementally on startup.

use anyhow::{bail, Context};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Manager, RunEvent};
use tauri_plugin_updater::{Update, UpdaterExt};

mod os_integrations;
mod window_manager;
mod windows;

const PREFERRED_PG_PORT: u16 = 15434;
const PREFERRED_API_PORT: u16 = 15433;
const PREFERRED_WEB_PORT: u16 = 15435;
/// Raw-ZPL transport timeout (label printing must fail fast and honestly,
/// not hang the webview when the printer is off or the address is wrong).
const ZPL_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
/// Dev JWT secret — must match the compose/dev default so the demo seed's
/// at-rest seal key derives identically (crates/core-db/src/demo.rs).
const DEV_JWT_SECRET: &str = "dev-only-insecure-secret-change-me";
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
        PathBuf::from(home).join("Library/Application Support/Meridian")
    } else {
        PathBuf::from(home).join(".local/share/Meridian")
    }
}

/// First free TCP port at or after `start`, for Postgres and the API.
fn free_port(start: u16) -> u16 {
    for port in start..start + 100 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    start
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
                if bundled.join("server.js").exists() {
                    return bundled;
                }
            }
        }
    }
    PathBuf::from("../apps/web/.next/standalone")
}

async fn role_exists(pool: &sqlx::PgPool, role: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query("SELECT 1 FROM pg_roles WHERE rolname = $1")
        .bind(role)
        .fetch_optional(pool)
        .await?
        .is_some())
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
        .env("PGPASSWORD", "postgres")
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
async fn replay_pg17_dump(root: &Path, dump: &Path, pg_port: u16) -> anyhow::Result<()> {
    let new_bin = install_bin_dir(&root.join("pginstall"), "18")
        .context("PG18 binaries missing right after setup — unexpected")?;
    let mut psql = tokio::process::Command::new(new_bin.join("psql"));
    psql.env("PGPASSWORD", "postgres")
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

async fn bootstrap_backend() -> anyhow::Result<(Backend, String, String)> {
    let root = data_root();
    std::fs::create_dir_all(&root)?;
    let pg_port = free_port(PREFERRED_PG_PORT);

    let mut settings = postgresql_embedded::Settings::default();
    // Pin PG18 for parity with deploy/docker-compose.yml (pgvector/pgvector:pg18).
    if let Ok(v) = postgresql_embedded::VersionReq::parse("18") {
        settings.version = v;
    }
    settings.username = "postgres".into();
    settings.password = "postgres".into();
    settings.data_dir = root.join("pgdata");
    settings.installation_dir = root.join("pginstall");
    settings.temporary = false;
    settings.port = pg_port;

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
    ensure_pgvector(&root.join("pginstall"), "18")?;
    postgresql
        .start()
        .await
        .context("starting the embedded Postgres")?;
    let pg_port = postgresql.settings().port;

    let super_url = postgresql.settings().url("postgres");
    let super_pool = sqlx::postgres::PgPool::connect(&super_url).await?;
    if let Some(dump) = cut_dump {
        replay_pg17_dump(&root, &dump, pg_port)
            .await
            .context("replaying the archived PG17 cluster into the PG18 cluster")?;
    }
    // Parity with deploy/docker-compose.yml: migrations and the worker run
    // as the TABLE OWNER `meridian`; migration 0001 itself provisions the
    // RLS-constrained `meridian_app` role the API connects as. Compose's
    // POSTGRES_USER is a bootstrap superuser, so `meridian` gets the same
    // here (the idempotent ALTER also repairs data dirs created before
    // this was a superuser).
    if !role_exists(&super_pool, "meridian").await? {
        sqlx::query("CREATE ROLE meridian LOGIN SUPERUSER PASSWORD 'meridian'")
            .execute(&super_pool)
            .await
            .context("creating the meridian owner role")?;
    } else {
        sqlx::query("ALTER ROLE meridian SUPERUSER")
            .execute(&super_pool)
            .await
            .context("aligning the meridian owner role with compose parity")?;
    }
    let db_missing = sqlx::query("SELECT 1 FROM pg_database WHERE datname = 'meridian'")
        .fetch_optional(&super_pool)
        .await?
        .is_none();
    if db_missing {
        sqlx::query("CREATE DATABASE meridian OWNER meridian")
            .execute(&super_pool)
            .await
            .context("creating the meridian database")?;
    }
    super_pool.close().await;

    let owner_url = format!("postgres://meridian:meridian@127.0.0.1:{pg_port}/meridian");
    let owner_pool = core_db::connect(&owner_url)
        .await
        .context("connecting as the schema owner")?;
    core_db::run_migrations(&owner_pool)
        .await
        .context("running migrations (owner role)")?;
    core_db::assert_rls_coverage(&owner_pool)
        .await
        .context("RLS coverage check")?;

    // Seed the deterministic demo org (same path as `meridian-cli
    // seed-demo`); idempotent — an existing seed is left untouched.
    let seal = core_db::demo::derive_dev_seal_key(DEV_JWT_SECRET);
    let opts = core_db::demo::DemoSeedOptions { seal_key: seal };
    match core_db::demo::seed(&owner_pool, &opts).await {
        Ok(_) => tracing::info!("demo org seeded; logins: {DEMO_LOGINS}"),
        Err(core_db::demo::DemoError::AlreadySeeded(_)) => {
            tracing::info!("demo org already seeded")
        }
        Err(e) => return Err(anyhow::anyhow!("demo seed failed: {e}")),
    }
    owner_pool.close().await;

    let app_url = format!("postgres://meridian_app:meridian_app@127.0.0.1:{pg_port}/meridian");
    let api_port = free_port(PREFERRED_API_PORT);
    let api_url = format!("http://127.0.0.1:{api_port}");
    let web_port = free_port(PREFERRED_WEB_PORT);
    let web_url = format!("http://127.0.0.1:{web_port}");
    let next_server =
        spawn_next_server(&next_standalone_dir(), &root.join("webrun"), web_port).await?;

    let config = Arc::new(core_api::config::Config::from_local(
        app_url.clone(),
        api_port,
        root.join("objects"),
    ));
    let pool = core_db::connect(&config.database_url).await?;
    let state = core_api::state::AppState {
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
                axum::http::header::HeaderName::from_static("x-meridian-csrf"),
            ])
            .allow_credentials(true),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", api_port)).await?;
    tracing::info!(%api_url, "core-api mounted for the desktop app");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
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

/// Sends raw ZPL bytes to a networked Zebra printer over a TCP socket — the
/// printer's standard raw port (9100 unless the caller says otherwise).
/// This is the desktop-only half of the label print contract: the web app
/// renders the label server-side and hands the exact ZPL string down; this
/// command transports those bytes and NOTHING else — it never parses,
/// rewrites, or "improves" the stream.
async fn send_zpl_to(host: &str, port: u16, zpl: &[u8]) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    if host.trim().is_empty() {
        bail!("printer host is empty");
    }
    let addr = format!("{}:{}", host.trim(), port);
    let mut stream = tokio::time::timeout(ZPL_CONNECT_TIMEOUT, tokio::net::TcpStream::connect(&addr))
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
/// refused") — never a silent success.
#[tauri::command]
async fn print_raw_zpl(host: String, port: Option<u16>, zpl: String) -> Result<(), String> {
    send_zpl_to(&host, port.unwrap_or(9100), zpl.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// ACL wiring contract (Wave 34 lane 6): the web app loads from a
    /// REMOTE loopback origin, and since tauri 2.11 remote origins need an
    /// explicit `remote` capability even for custom commands. This pins
    /// the capability file that grants the three app commands — losing any
    /// of them silently disables the ZPL transport or the update UI in the
    /// real shell while every browser test stays green.
    #[test]
    fn loopback_web_capability_grants_the_three_app_commands() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capabilities/desktop-web.json"
        ))
        .expect("capability file exists");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("capability parses");
        assert_eq!(v["windows"][0], "main");
        assert_eq!(v["remote"]["urls"][0], "http://127.0.0.1:*");
        let permissions = v["permissions"].as_array().expect("permissions array");
        for expected in [
            "allow-print-raw-zpl",
            "allow-check-desktop-update",
            "allow-install-desktop-update",
        ] {
            assert!(
                permissions.contains(&serde_json::json!(expected)),
                "capability must grant {expected}"
            );
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    // The backend must be up before the webview loads: the web app reads
    // the injected API base URL before its own scripts run, and login has
    // nothing to talk to until the API listens. First run may take a
    // while (binary download + full migrations); later starts are quick.
    let (backend, api_url, web_url) = match tauri::async_runtime::block_on(bootstrap_backend()) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("Meridian desktop failed to start: {e:#}");
            std::process::exit(1);
        }
    };
    install_sigterm_handler(data_root().join("pgdata"));

    // Window-geometry state for the native fleet (Wave 38 lane 2): one
    // JSON file under the app data dir, roles keyed by route key.
    let window_state_path = data_root().join("window-state.json");

    tauri::Builder::default()
        // Auto-update channel (GA packaging): the updater reads its
        // endpoint + pubkey from tauri.conf.json `plugins.updater`. The
        // check is exposed to the web app as the typed
        // `check_desktop_update` / `install_desktop_update` commands (Wave
        // 34 lane 6) — the plugin's own JS commands are deliberately NOT
        // granted; the UI rides the app ACL only. Release workflow lives in
        // docs/desktop-release.md.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Wave 38 lane 2: `meridian://` deep links open-or-focus the right
        // window; single instance makes a second launch focus the running
        // app (and hand it the deep link it was opened with). Deep-link OS
        // registration is the bundle Info.plist (build-desktop.sh); runtime
        // delivery is RunEvent::Opened (macOS warm launch), argv on first
        // launch, and this hook's argv for Linux/Windows re-launches.
        .plugin(tauri_plugin_single_instance::init(|_app, argv, _cwd| {
            // Second-launch focus: bring the fleet forward. Deep links
            // arriving via argv are handled by the normal event path on
            // macOS RunEvent::Opened; on Linux/Windows argv carries them.
            if let Some(link) = argv.iter().find(|a| a.starts_with("meridian://")) {
                let app = _app.clone();
                let link = link.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = os_integrations::route_deep_link(&app, &link).await {
                        tracing::warn!("deep link routing failed: {e}");
                    }
                });
            } else if let Some(main) = _app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.unminimize();
                let _ = main.set_focus();
            }
        }))
        // Wave 32 lane 5: typed raw-ZPL door for label printing. Custom app
        // commands are ACL-gated for this shell's remote web origin (tauri
        // >= 2.11); the grants live in desktop/capabilities/desktop-web.json
        // and are generated from desktop/build.rs.
        .invoke_handler(tauri::generate_handler![
            print_raw_zpl,
            check_desktop_update,
            install_desktop_update,
            window_manager::open_native_window,
            window_manager::focus_native_window,
            os_integrations::route_meridian_link
        ])
        .manage(backend)
        .manage(PendingUpdate::default())
        .manage(window_manager::WindowRegistry::new(window_state_path))
        .manage(WebUrl(web_url.clone()))
        .manage(window_manager::ApiUrl(api_url.clone()))
        .setup(move |app| {
            // Rust-built window so the initialization script (API base
            // URL injection, ADR 0005) runs before the page's scripts.
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(web_url.parse().expect("valid web URL")),
            )
            .title("Meridian")
            .inner_size(1440.0, 900.0)
            .min_inner_size(900.0, 640.0)
            .initialization_script(format!("window.__MERIDIAN_API_URL__ = '{api_url}';"))
            // Windows are the point: native tabbing stays OFF. Without an
            // explicit tabbing identifier macOS never auto-groups this
            // window into tabs, and the Window menu below lists windows
            // rather than merging them.
            .build()?;

            // Native chrome: menu bar (Window menu with tabs-off posture)
            // + tray quick actions. Failures are honest but non-fatal — a
            // tray-less launch beats a dead app.
            if let Err(e) = os_integrations::build_menu(app.handle()) {
                tracing::warn!("menu bar unavailable: {e}");
            }
            if let Err(e) = os_integrations::build_tray(app.handle()) {
                tracing::warn!("tray unavailable: {e}");
            }

            // Any deep link this FIRST launch was opened with (macOS hands
            // it over via RunEvent::Opened below for warm launches).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                for arg in std::env::args().skip(1) {
                    if arg.starts_with("meridian://") {
                        let app = handle.clone();
                        let arg = arg.clone();
                        tauri::async_runtime::block_on(async move {
                            if let Err(e) = os_integrations::route_deep_link(&app, &arg).await {
                                tracing::warn!("launch deep link failed: {e}");
                            }
                        });
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("building the Tauri app")
        .run(|app, event| {
            match event {
                // macOS: warm-launch deep links (dock right-click "Open
                // URL", OS open with app running) arrive here.
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
                    // Graceful shutdown: stop the embedded Postgres so the
                    // data dir is left consistent for the next launch.
                    if let Some(b) = app.try_state::<Backend>() {
                        if let Err(e) = tauri::async_runtime::block_on(b.postgres.stop()) {
                            eprintln!("stopping embedded Postgres failed: {e}");
                        }
                        // kill_on_drop also covers this; explicit kill keeps
                        // the shutdown order deterministic (web, then PG).
                        let _ = b.next_server.lock().map(|mut c| c.start_kill());
                    }
                }
                _ => {}
            }
        });
}
