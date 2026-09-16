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

const PREFERRED_PG_PORT: u16 = 15434;
const PREFERRED_API_PORT: u16 = 15433;
const PREFERRED_WEB_PORT: u16 = 15435;
/// Dev JWT secret — must match the compose/dev default so the demo seed's
/// at-rest seal key derives identically (crates/core-db/src/demo.rs).
const DEV_JWT_SECRET: &str = "dev-only-insecure-secret-change-me";
const DEMO_LOGINS: &str =
    "owner@demo.local / dispatcher@demo.local / tech@demo.local / bookkeeper@demo.local";

struct Backend {
    postgres: postgresql_embedded::PostgreSQL,
    next_server: std::sync::Mutex<tokio::process::Child>,
}

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
/// embedded installation before first use.
fn ensure_pgvector(installation_dir: &Path) -> anyhow::Result<()> {
    // postgresql_embedded installs into a versioned subdirectory
    // (e.g. installation_dir/17.11.0); find the actual root.
    let versioned = std::fs::read_dir(installation_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/initdb").exists())
        .with_context(|| {
            format!("no Postgres installation root with bin/initdb under {installation_dir:?}")
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
    let lib_candidates = [
        PathBuf::from("/opt/homebrew/opt/pgvector/lib/postgresql@17/vector.dylib"),
        PathBuf::from("/opt/homebrew/opt/pgvector/lib/postgresql@18/vector.dylib"),
        PathBuf::from("/opt/homebrew/opt/pgvector/lib/vector.dylib"),
        PathBuf::from("/opt/homebrew/opt/pgvector/lib/vector.so"),
        PathBuf::from("/usr/local/opt/pgvector/lib/vector.dylib"),
    ];
    let share_candidates = [
        PathBuf::from("/opt/homebrew/opt/pgvector/share/postgresql@17/extension"),
        PathBuf::from("/opt/homebrew/opt/pgvector/share/postgresql@18/extension"),
        PathBuf::from("/opt/homebrew/opt/pgvector/share/extension"),
        PathBuf::from("/usr/local/opt/pgvector/share/extension"),
    ];
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

async fn bootstrap_backend() -> anyhow::Result<(Backend, String, String)> {
    let root = data_root();
    std::fs::create_dir_all(&root)?;
    let pg_port = free_port(PREFERRED_PG_PORT);

    let mut settings = postgresql_embedded::Settings::default();
    // Pin PG17 for parity with deploy/docker-compose.yml (pgvector/pgvector:pg17).
    if let Ok(v) = postgresql_embedded::VersionReq::parse("17") {
        settings.version = v;
    }
    settings.username = "postgres".into();
    settings.password = "postgres".into();
    settings.data_dir = root.join("pgdata");
    settings.installation_dir = root.join("pginstall");
    settings.temporary = false;
    settings.port = pg_port;

    let mut postgresql = postgresql_embedded::PostgreSQL::new(settings);
    postgresql
        .setup()
        .await
        .context("setting up the embedded Postgres (first run downloads the binaries)")?;
    ensure_pgvector(&root.join("pginstall"))?;
    postgresql
        .start()
        .await
        .context("starting the embedded Postgres")?;
    let pg_port = postgresql.settings().port;

    let super_url = postgresql.settings().url("postgres");
    let super_pool = sqlx::postgres::PgPool::connect(&super_url).await?;
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
            ])
            .allow_headers(tower_http::cors::Any),
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

    tauri::Builder::default()
        .manage(backend)
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
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("building the Tauri app")
        .run(|app, event| {
            if let RunEvent::Exit = event {
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
        });
}
// touch 1789340445
