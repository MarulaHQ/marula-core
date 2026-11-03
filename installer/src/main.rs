//! The native installer shell (Wave 60 lane W60-3, ADR 0027).
//!
//! Purpose: put the wizard on a desktop where a browser is not the assumed
//! entry point. The window loads the same front-end the loopback fallback
//! serves (`apps/installer/dist`), and every screen's data comes from a Tauri
//! command that calls `install::Session` — the same functions `marula install
//! --ui` calls. There is no second engine and no HTTP server in this host: the
//! webview and the engine speak over IPC.
//!
//! Non-purpose: no packaging or signing policy (W60-4 owns per-OS artifacts,
//! notarisation and the offline bundle), no update engine (W60-5). This crate
//! is the window, the IPC surface, and the one thing a webview must not be
//! trusted with: deciding which URL the OS may open.
//!
//! Trust boundary: the front-end is local code, but it renders text the
//! descriptor and the machine supply, so the IPC surface is treated as an API.
//! Each command takes typed input, the engine refuses what it does not publish,
//! and `installer_open` accepts only `http`/`https` — a shell that hands the OS
//! an arbitrary scheme is how a local page opens a local file handler.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::path::PathBuf;

use marula_core_cli::install::view::{ApplyView, DoctorView, PlanView, Session, StateView};
use serde::Deserialize;
use tauri::Manager as _;

/// The engine, bound to one install directory for the lifetime of the window.
struct AppState {
    session: Session,
}

#[derive(Debug, Deserialize)]
struct PlanRequest {
    profile: String,
    #[serde(default)]
    overrides: BTreeMap<String, String>,
    #[serde(default)]
    images: Option<String>,
    #[serde(default)]
    restart: Option<String>,
    /// Developer mode may plan a shape this machine cannot carry.
    #[serde(default)]
    allow_unfit: bool,
}

impl PlanRequest {
    fn shaped(&self, session: Session) -> Result<Session, String> {
        let images = match self.images.as_deref() {
            None => marula_core_cli::install::plan::ImageSource::Build,
            Some(value) => marula_core_cli::install::plan::ImageSource::parse(value)
                .ok_or_else(|| format!("images takes build or registry, not {value}"))?,
        };
        let restart = match self.restart.as_deref() {
            None => marula_core_cli::install::plan::RestartPolicy::UnlessStopped,
            Some(value) => {
                marula_core_cli::install::plan::RestartPolicy::parse(value).ok_or_else(|| {
                    format!("restart takes unless-stopped, always or no, not {value}")
                })?
            }
        };
        Ok(session.with_shape(images, restart, self.allow_unfit))
    }
}

#[tauri::command]
async fn installer_state(state: tauri::State<'_, AppState>) -> Result<StateView, String> {
    let session = state.session.clone();
    blocking(move || session.state()).await
}

#[tauri::command]
async fn installer_plan(
    state: tauri::State<'_, AppState>,
    request: PlanRequest,
) -> Result<PlanView, String> {
    let session = request.shaped(state.session.clone())?;
    blocking(move || session.plan(&request.profile, &request.overrides))
        .await
        .and_then(|result| result)
}

#[tauri::command]
async fn installer_apply(
    state: tauri::State<'_, AppState>,
    request: PlanRequest,
) -> Result<(), String> {
    let session = request.shaped(state.session.clone())?;
    blocking(move || session.start_apply(&request.profile, &request.overrides))
        .await
        .and_then(|result| result)
}

#[tauri::command]
async fn installer_apply_state(state: tauri::State<'_, AppState>) -> Result<ApplyView, String> {
    let session = state.session.clone();
    blocking(move || session.apply_state()).await
}

#[tauri::command]
async fn installer_doctor(
    state: tauri::State<'_, AppState>,
    profile: String,
) -> Result<DoctorView, String> {
    let session = state.session.clone();
    blocking(move || session.doctor(&profile))
        .await
        .and_then(|result| result)
}

/// Opens the installed product. Only a web URL, and only one this file names.
#[tauri::command]
fn installer_open(url: String) -> Result<(), String> {
    if !openable(&url) {
        return Err("the installer opens http and https URLs only".to_string());
    }
    launch(&url).map_err(|error| format!("could not open {url}: {error}"))
}

fn openable(url: &str) -> bool {
    if url.len() > 2048 || url.contains(char::is_whitespace) {
        return false;
    }
    url.starts_with("http://") || url.starts_with("https://")
}

/// Runs a blocking engine call off the UI thread.
///
/// The engine dials Docker and reads the filesystem; a command that did that on
/// the main thread would freeze the window exactly at the moment it says
/// "starting the stack".
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("the engine stopped unexpectedly: {error}"))
}

fn launch(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        // `cmd /C start "" <url>`: the empty title keeps Windows from treating
        // the URL as a window title.
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };

    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Where this install writes, and which compose file it runs.
///
/// A double-clicked app has no arguments, so the defaults have to be somewhere a
/// person can find: `~/marula` for the files, and the compose file the release
/// bundle ships beside the binary. Both are overridable for a packager or a
/// test, and both are named in the window before anything is written.
fn locations() -> (PathBuf, PathBuf) {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    let dir = std::env::var_os("MARULA_INSTALL_DIR")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join("marula")))
        .unwrap_or_else(|| PathBuf::from("marula"));

    let compose = std::env::var_os("MARULA_COMPOSE_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            let beside = std::env::current_exe().ok().and_then(|exe| {
                exe.parent()
                    .map(|parent| parent.join("deploy/docker-compose.yml"))
            });
            beside.filter(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("deploy/docker-compose.yml"));

    (dir, compose)
}

fn main() {
    let (dir, compose) = locations();
    let session = Session::new(dir, compose, None);

    tauri::Builder::default()
        .manage(AppState { session })
        .invoke_handler(tauri::generate_handler![
            installer_state,
            installer_plan,
            installer_apply,
            installer_apply_state,
            installer_doctor,
            installer_open,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Marula Installer");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the installer window could not start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_are_handed_to_the_operating_system() {
        assert!(openable("http://localhost:8080"));
        assert!(openable("https://marula.example.com"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("x-apple-systempreferences:"));
        assert!(!openable("http://example.com/a b"));
        assert!(!openable(&format!(
            "https://example.com/{}",
            "a".repeat(2100)
        )));
    }
}
