//! Native window open/focus + live registry — the command surface the web
//! renderer drives (Wave 38 lane 2).
//!
//! Purpose: `open_native_window` is the desktop twin of lane 1's
//! `openPopout` — same route keys, same role names, but the window is a
//! real OS window whose frame persists per role. `focus_native_window`
//! realizes the open-or-FOCUS contract for deep links and dock clicks.
//! Geometry capture rides each window's own events, so quit-time state is
//! never special-cased.

use super::windows::{
    default_size, load_state, restore_frame, save_state, WindowGeometry, WindowStateFile,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Last geometry-event instant per window label — the persistence debounce:
/// a move/resize burst writes the file once, 400ms after the burst ends.
fn last_event_map() -> std::sync::MutexGuard<'static, HashMap<String, std::time::Instant>> {
    static CELL: std::sync::OnceLock<Mutex<HashMap<String, std::time::Instant>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("geometry event map")
}

/// Managed app state: the window-state file path + the live registry.
pub struct WindowRegistry {
    state_path: PathBuf,
    /// (role, instance_index) per live window label.
    pub live: Mutex<std::collections::HashMap<String, (String, usize)>>,
    in_memory: Mutex<WindowStateFile>,
}

impl WindowRegistry {
    pub fn new(state_path: PathBuf) -> Self {
        let in_memory = load_state(&state_path);
        Self {
            state_path,
            live: Mutex::new(std::collections::HashMap::new()),
            in_memory: Mutex::new(in_memory),
        }
    }
}

/// Persist one window's current geometry under its role.
pub fn capture_geometry(window: &tauri::WebviewWindow, role: &str, index: usize) {
    let registry = match window.app_handle().try_state::<WindowRegistry>() {
        Some(r) => r,
        None => return,
    };
    let Ok(pos) = window.outer_position() else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().map(|n| n.to_string()));
    let frame = WindowGeometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        monitor,
        index,
    };
    let mut state = registry.in_memory.lock().expect("window state mutex");
    let entry = state.roles.entry(role.to_string()).or_default();
    // Overwrite the saved frame for this instance index; grow if needed.
    match entry.windows.get_mut(index) {
        Some(saved) => *saved = frame,
        None => {
            while entry.windows.len() < index {
                // Fill gaps with the last-known/default frame so indices stay aligned.
                let last = entry.windows.last().cloned();
                entry.windows.push(last.unwrap_or(WindowGeometry {
                    x: 0,
                    y: 0,
                    width: 1100,
                    height: 820,
                    monitor: None,
                    index: entry.windows.len(),
                }));
            }
            entry.windows.push(frame);
        }
    }
    // Never persist empty windows lists: a role with all windows closed
    // keeps its frames (restore on relaunch uses last.quit sets), so this
    // capture only ever enriches.
    save_state(&registry.state_path, &state);
}

/// Drop registry entries whose windows no longer exist (Tauri destroys
/// webviews on close; the map only learns about it here — cheap, correct).
fn reconcile_live(app: &AppHandle) {
    let registry = match app.try_state::<WindowRegistry>() {
        Some(r) => r,
        None => return,
    };
    let mut live = registry.live.lock().expect("live registry mutex");
    live.retain(|label, _| app.get_webview_window(label).is_some());
}

/// Next free instance index for a role: reuse indices whose windows closed
/// (avoid unbounded growth when a user opens+closes repeatedly).
fn next_index(app: &AppHandle, registry: &WindowRegistry, role: &str) -> usize {
    reconcile_live(app);
    let live = registry.live.lock().expect("live registry mutex");
    let mut used: Vec<usize> = live
        .values()
        .filter(|(r, _)| r == role)
        .map(|(_, i)| *i)
        .collect();
    used.sort_unstable();
    let mut candidate = 0usize;
    for u in used {
        if u == candidate {
            candidate += 1;
        } else {
            break;
        }
    }
    candidate
}

/// The full URL for a role window: the lane-1 compact shell engages off
/// `?popout=<role>`; `&wid=<label>` gives the window a stable identity the
/// renderer can scope per-instance state to (windowScopedKey stays
/// role-keyed per lane 1's contract; wid disambiguates ephemeral things
/// like the bus window id).
///
/// One shape, two callers: a window being created, and a window a deep link
/// sends to another record. If they disagreed, following a link would move a
/// window off its own identity and the renderer's per-window state with it.
pub fn role_window_url(web_url: &str, role: &str, label: &str, path_suffix: &str) -> String {
    let base = web_url.trim_end_matches('/');
    let path = if path_suffix.contains('?') {
        format!("{path_suffix}&")
    } else {
        format!("{path_suffix}?")
    };
    format!("{base}/{path}popout={role}&wid={label}")
}

/// An in-app path as an absolute URL on the shell's own web origin — what the
/// main window loads (`"/tickets/<id>"` → `<web_url>/tickets/<id>`).
pub fn absolute_url(web_url: &str, path: &str) -> String {
    format!(
        "{}{}",
        web_url.trim_end_matches('/'),
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    )
}

/// Send a window to `url` unless it is already there. `Ok(false)` means the
/// window was left alone, which is the common case for a link that names what
/// the window already shows (reloading it would throw away unsaved work).
pub fn navigate_window(window: &tauri::WebviewWindow, url: &str) -> Result<bool, String> {
    let target: tauri::Url = url
        .parse()
        .map_err(|e| format!("invalid window URL {url}: {e}"))?;
    if let Ok(current) = window.url() {
        if current == target {
            return Ok(false);
        }
    }
    window
        .navigate(target)
        .map_err(|e| format!("navigating to {url}: {e}"))?;
    Ok(true)
}

/// Follow a deep link into the role window that already exists: focus it and
/// move it to the record the link names. Open-or-focus without this leaves the
/// window showing whatever it showed before, which reads as the wrong record.
pub fn navigate_role_window(
    app: &AppHandle,
    label: &str,
    role: &str,
    path_suffix: &str,
) -> Result<bool, String> {
    let Some(window) = app.get_webview_window(label) else {
        return Err(format!("window {label} is gone"));
    };
    let web_url = app
        .try_state::<crate::WebUrl>()
        .map(|w| w.0.clone())
        .ok_or_else(|| "shell web URL not initialized".to_string())?;
    let url = role_window_url(&web_url, role, label, path_suffix);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    navigate_window(&window, &url)
}

/// The fallback a deep link takes when its role window cannot be opened: the
/// main window, at the same record's in-app route (`DeepLinkTarget::main_path`).
pub fn navigate_main_window(app: &AppHandle, path: &str) -> Result<bool, String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(String::from("no main window"));
    };
    let web_url = app
        .try_state::<crate::WebUrl>()
        .map(|w| w.0.clone())
        .ok_or_else(|| "shell web URL not initialized".to_string())?;
    let url = absolute_url(&web_url, path);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    navigate_window(&window, &url)
}

/// Create (or focus) the window for one role and instance, restoring the frame
/// that role and instance last had and capturing it from then on.
async fn create_role_window(
    app: &AppHandle,
    role: &str,
    instance: usize,
    api_url: &str,
    web_url: &str,
    path_suffix: &str,
) -> Result<String, String> {
    let label = format!("subwindow::{role}::{instance}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(label);
    }
    let url = role_window_url(web_url, role, &label, path_suffix);
    let (dw, dh) = default_size(role);
    let mut builder = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::External(
            url.parse()
                .map_err(|e| format!("invalid window URL: {e}"))?,
        ),
    )
    .title(role_title(role))
    .min_inner_size(640.0, 420.0)
    .inner_size(dw, dh)
    .initialization_script(format!("window.__MARULA_API_URL__ = '{api_url}';"));

    // Restore remembered geometry for THIS role+instance, clamped to the
    // monitors currently visible (hotplug: remember frame is always brought
    // back on-screen).
    let registry = app.try_state::<WindowRegistry>();
    if let Some(reg) = registry.as_ref() {
        let monitors: Vec<super::windows::MonitorBounds> = app
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let pos = m.position();
                let size = m.size();
                super::windows::MonitorBounds {
                    name: m.name().map(|n| n.to_string()),
                    x: pos.x,
                    y: pos.y,
                    width: size.width,
                    height: size.height,
                    scale_factor: m.scale_factor(),
                }
            })
            .collect();
        let state = reg.in_memory.lock().expect("window state mutex");
        if let Some((x, y, w, h)) = restore_frame(&state, role, instance, &monitors) {
            // The state file and the monitor bounds are both in PHYSICAL
            // pixels (outer_size/outer_size report physical; the builder's
            // position/inner_size want LOGICAL points). Converting here is
            // what keeps a restored window the size the user left it — on a
            // Retina display the raw values come back doubled.
            let saved = state
                .roles
                .get(role)
                .and_then(|r| r.windows.get(instance).or_else(|| r.windows.last()));
            let scale = saved.map_or(1.0, |frame| {
                super::windows::scale_for_frame(&monitors, frame)
            });
            builder = builder
                .position(x as f64 / scale, y as f64 / scale)
                .inner_size(w as f64 / scale, h as f64 / scale);
        }
    }

    let window = builder
        .build()
        .map_err(|e| format!("creating window for {role}: {e}"))?;

    if let Some(reg) = registry.as_ref() {
        reg.live
            .lock()
            .expect("live registry mutex")
            .insert(label.clone(), (role.to_string(), instance));
        capture_geometry(&window, role, instance);
    }

    // Persist geometry continuously: move/resize events debounce to the
    // state file, so a force-quit still leaves a restorable frame behind
    // (crash-honest persistence — no quit-time special casing).
    let event_app = app.clone();
    let event_role = role.to_string();
    let event_instance = instance;
    let event_label = label.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            let app = event_app.clone();
            let role = event_role.clone();
            let label = event_label.clone();
            let instance = event_instance;
            tauri::async_runtime::spawn(async move {
                last_event_map().insert(label.clone(), std::time::Instant::now());
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                // Only the LAST event of a burst survives the debounce gate.
                let still_last = last_event_map()
                    .get(&label)
                    .is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(350));
                if still_last {
                    last_event_map().remove(&label);
                    if let Some(w) = app.get_webview_window(&label) {
                        capture_geometry(&w, &role, instance);
                    }
                }
            });
        }
        _ => {}
    });

    Ok(label)
}

/// Display title for a role (native chrome shows OS fonts; the slim bar
/// inside keeps the renderer's own labels).
fn role_title(role: &str) -> &str {
    match role {
        "ticket-workspace" => "Ticket workspace — Marula",
        "ticket-detail" => "Ticket — Marula",
        "dispatch-board" => "Dispatch board — Marula",
        "stock-lookup" => "Stock lookup — Marula",
        "decisions" => "Decisions — Marula",
        "merge-workbench" => "Merge workbench — Marula",
        _ => "Marula",
    }
}

/// Find a live window whose role matches (reconciles first, so closed
/// windows never answer). Returns its label.
pub fn find_by_role(app: &AppHandle, role: &str) -> Option<String> {
    reconcile_live(app);
    let registry = app.try_state::<WindowRegistry>()?;
    let live = registry.live.lock().expect("live registry mutex");
    for (label, (r, _)) in live.iter() {
        if r == role {
            return Some(label.clone());
        }
    }
    None
}

/// Managed state carrying the API base URL the window init script injects
/// (same value main.rs hands the main webview — ADR 0005).
pub struct ApiUrl(pub String);

/// Tauri command: open (or focus) a native window for a route key. Returns
/// the window label on success. `path_suffix` is the in-app path AFTER the
/// leading "/" e.g. "tickets/<id>" or "dispatch".
#[tauri::command]
pub async fn open_native_window(
    app: AppHandle,
    role: String,
    path_suffix: String,
) -> Result<String, String> {
    if !super::windows::KNOWN_ROLES.contains(&role.as_str()) {
        return Err(format!("unknown window role: {role}"));
    }
    let api_url = app
        .try_state::<ApiUrl>()
        .map(|a| a.0.clone())
        .ok_or_else(|| "shell API URL not initialized".to_string())?;
    let web_url = app
        .try_state::<crate::WebUrl>()
        .map(|w| w.0.clone())
        .ok_or_else(|| "shell web URL not initialized".to_string())?;
    let registry = app
        .try_state::<WindowRegistry>()
        .ok_or_else(|| "window registry not initialized".to_string())?;
    let instance = next_index(&app, &registry, &role);
    create_role_window(&app, &role, instance, &api_url, &web_url, &path_suffix).await
}

/// Tauri command: focus an existing window of a role (open-or-focus
/// contract). Ok(None) means "no such window" — the caller decides whether
/// to open one.
#[tauri::command]
pub async fn focus_native_window(
    app: AppHandle,
    role: String,
    label: Option<String>,
) -> Result<Option<String>, String> {
    let target = match label {
        Some(l) => {
            if app.get_webview_window(&l).is_none() {
                return Ok(None);
            }
            l
        }
        None => match find_by_role(&app, &role) {
            Some(l) => l,
            None => return Ok(None),
        },
    };
    if let Some(w) = app.get_webview_window(&target) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(Some(target));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_window_url_carries_role_and_identity() {
        assert_eq!(
            role_window_url(
                "http://127.0.0.1:15435/",
                "ticket-detail",
                "subwindow::ticket-detail::0",
                "tickets/abc",
            ),
            "http://127.0.0.1:15435/tickets/abc?popout=ticket-detail&wid=subwindow::ticket-detail::0"
        );
    }

    #[test]
    fn role_window_url_keeps_an_existing_query() {
        assert_eq!(
            role_window_url(
                "http://127.0.0.1:15435",
                "stock-lookup",
                "subwindow::stock-lookup::0",
                "stock?highlight=abc",
            ),
            "http://127.0.0.1:15435/stock?highlight=abc&popout=stock-lookup&wid=subwindow::stock-lookup::0"
        );
    }

    /// The point of the deep-link fix: the URL a link computes for a second
    /// record differs from the one the window shows, and the window's own
    /// identity survives the move.
    #[test]
    fn another_record_moves_the_window_without_losing_its_identity() {
        let first = role_window_url(
            "http://127.0.0.1:15435",
            "ticket-detail",
            "subwindow::ticket-detail::0",
            "tickets/aaa",
        );
        let second = role_window_url(
            "http://127.0.0.1:15435",
            "ticket-detail",
            "subwindow::ticket-detail::0",
            "tickets/bbb",
        );
        assert_ne!(first, second);
        assert!(second.ends_with("?popout=ticket-detail&wid=subwindow::ticket-detail::0"));
    }

    #[test]
    fn main_window_routes_are_absolute_on_one_origin() {
        assert_eq!(
            absolute_url("http://127.0.0.1:15435", "/tickets/x"),
            "http://127.0.0.1:15435/tickets/x"
        );
        assert_eq!(
            absolute_url("https://psa.example.com/", "dispatch"),
            "https://psa.example.com/dispatch"
        );
    }

    #[test]
    fn state_file_round_trip_and_defaults() {
        let dir = std::env::temp_dir().join("marula-window-registry-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("windows.json");
        let registry = WindowRegistry::new(path.clone());
        // Seed two live entries with a hole at index 1.
        {
            let mut live = registry.live.lock().unwrap();
            live.insert(
                String::from("subwindow::ticket-detail::0"),
                (String::from("ticket-detail"), 0),
            );
            live.insert(
                String::from("subwindow::ticket-detail::2"),
                (String::from("ticket-detail"), 2),
            );
        }
        assert!(load_state(&path).roles.is_empty());
        // A fresh registry over the same file loads nothing bad.
        let again = WindowRegistry::new(path.clone());
        assert!(again.live.lock().unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
