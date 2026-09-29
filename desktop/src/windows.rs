//! Native multi-window management (Wave 38 lane 2, `p38-desktop-multi-window`).
//!
//! Purpose: the desktop shell grows real OS windows. Every secondary window
//! is a `WebviewWindow` loading the SAME bundled Next server the main window
//! uses, carrying `?popout=<routeKey>` so the renderer's lane-1 compact
//! shell (PopoutShell, per-window-key layout persistence, the
//! `meridian-windows` BroadcastChannel bus) engages identically — one
//! renderer codebase, desktop adds native chrome.
//!
//! Window roles: the route key ("ticket-detail", "dispatch-board", …) is the
//! window ROLE. Label = `subwindow::<role>::<n>` so several windows of one
//! role coexist (three ticket windows is the point). Each role remembers its
//! own position/size/display in a JSON state file under the app data dir;
//! restore clamps into the currently-visible displays so a hotplugged or
//! re-arranged monitor can never strand a window off-screen.
//!
//! Non-purpose: no tabbing (macOS "Show Tab Bar" posture is tabs-OFF —
//! windows are the point, set in main.rs), no per-window processes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// The window roles the desktop shell can open natively. Mirrors the
/// renderer's POPOUT_ROUTES (apps/web/lib/popout/registry.ts) for the
/// FEEL (title + default footprint); the route key itself is passed
/// through uninterpreted so renderer registry changes never desync this
/// list's behavior — only unknown keys are rejected here.
pub const KNOWN_ROLES: &[&str] = &[
    "ticket-workspace",
    "ticket-detail",
    "dispatch-board",
    "stock-lookup",
    "decisions",
    "merge-workbench",
];

/// Default footprint per role (logical points), mirroring the renderer's
/// feature-string sizing so a native window opens at the size the web
/// popout would have.
pub fn default_size(role: &str) -> (f64, f64) {
    match role {
        "ticket-workspace" => (1280.0, 860.0),
        "ticket-detail" => (1100.0, 820.0),
        "dispatch-board" => (1440.0, 900.0),
        "stock-lookup" => (960.0, 720.0),
        "decisions" => (1100.0, 800.0),
        "merge-workbench" => (1200.0, 840.0),
        _ => (1100.0, 820.0),
    }
}

/// One window's saved geometry — physical pixels + the monitor name it sat
/// on, so restores can sanity-check the display still exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub monitor: Option<String>,
    /// Index among same-role windows (0 = first). Each instance remembers
    /// its own frame; a fresh instance count beyond saved entries cascades
    /// from the last one.
    pub index: usize,
}

/// Per-role persisted state: geometry per instance index + how many
/// instances existed at last quit (relaunch restores that many).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleState {
    pub windows: Vec<WindowGeometry>,
}

/// The whole window-state file. Keyed by role; counter keeps ids unique
/// across a session so closing window 0 then opening a new one doesn't
/// collide with a live label.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WindowStateFile {
    pub roles: HashMap<String, RoleState>,
    /// Monotonic counter for unique window labels.
    #[serde(default)]
    pub label_counter: u64,
}

/// Load the state file (missing/corrupt = empty — a bad file never blocks
/// window creation, it just forgets remembered frames).
pub fn load_state(path: &PathBuf) -> WindowStateFile {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(path: &PathBuf, state: &WindowStateFile) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Atomic replace so a crash mid-write can't truncate the file.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Plain bounds snapshot of one monitor (physical pixels) — testable
/// without Tauri runtime internals.
#[derive(Debug, Clone)]
pub struct MonitorBounds {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Clamp a remembered frame into the union of the currently visible
/// monitors (physical pixels): windows come back on-screen after
/// display changes, never stranded on a hotplugged monitor.
fn clamp_to_visible(
    frame: &WindowGeometry,
    monitors: &[MonitorBounds],
) -> (i32, i32, u32, u32) {
    // Fallback: if no monitors report (paranoid), trust the saved frame.
    if monitors.is_empty() {
        return (frame.x, frame.y, frame.width, frame.height);
    }
    // Union bounds of all monitors.
    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = monitors
        .iter()
        .map(|m| m.x + m.width as i32)
        .max()
        .unwrap_or(i32::MAX);
    let max_y = monitors
        .iter()
        .map(|m| m.y + m.height as i32)
        .max()
        .unwrap_or(i32::MAX);
    // Awarded size: at most the largest single monitor (a window can't
    // meaningfully exceed its display; keeps a stray giant remembered
    // frame from covering the union of two).
    let max_w = monitors.iter().map(|m| m.width).max().unwrap_or(frame.width);
    let max_h = monitors.iter().map(|m| m.height).max().unwrap_or(frame.height);
    let width = frame.width.min(max_w).max(400);
    let height = frame.height.min(max_h).max(300);
    // Keep at least a 60px sliver visible horizontally and the title bar
    // reachable vertically.
    let x = frame.x.clamp(min_x, (max_x - 60).max(min_x));
    let y = frame.y.clamp(min_y - 0, (max_y - 60).max(min_y));
    (x, y, width, height)
}

/// Compute the restore frame for a role from saved state, clamped to the
/// current monitors. Returns (x, y, w, h) or None when nothing saved — the
/// caller then uses the role default + OS cascade position.
pub fn restore_frame(
    state: &WindowStateFile,
    role: &str,
    index: usize,
    monitors: &[MonitorBounds],
) -> Option<(i32, i32, u32, u32)> {
    let role_state = state.roles.get(role)?;
    let frame = role_state.windows.get(index).or_else(|| role_state.windows.last())?;
    Some(clamp_to_visible(frame, monitors))
}

/// True when the label pattern `subwindow::<role>::<n>` names a lane window.
pub fn is_lane_window_label(label: &str) -> bool {
    label.starts_with("subwindow::")
}

/// Parse a role out of a lane window label, for per-role bookkeeping.
pub fn role_of_label(label: &str) -> Option<&str> {
    let rest = label.strip_prefix("subwindow::")?;
    rest.split("::").next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: i32, y: i32, w: u32, h: u32) -> MonitorBounds {
        MonitorBounds {
            name: Some(String::from("m")),
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn clamp_pulls_offscreen_frame_back_into_union() {
        let monitors = vec![monitor(0, 0, 1920, 1080), monitor(1920, 0, 1920, 1080)];
        let saved = WindowGeometry {
            x: 5000,
            y: 4000,
            width: 1200,
            height: 800,
            monitor: Some(String::from("gone")),
            index: 0,
        };
        let (x, y, w, h) = clamp_to_visible(&saved, &monitors);
        // x clamps so ≥60px remains visible before the union's right edge.
        assert!(x >= 0 && x <= 1920 + 1920 - 60);
        assert!(y >= 0 && y <= 1080 - 60);
        assert_eq!(w, 1200);
        assert_eq!(h, 800);
    }

    #[test]
    fn clamp_caps_size_at_largest_monitor() {
        let monitors = vec![monitor(0, 0, 1440, 900), monitor(1440, 0, 1440, 900)];
        let saved = WindowGeometry {
            x: 0,
            y: 0,
            width: 4000,
            height: 3000,
            monitor: None,
            index: 0,
        };
        let (x, _, w, h) = clamp_to_visible(&saved, &monitors);
        assert_eq!(x, 0);
        assert_eq!(w, 1440);
        assert_eq!(h, 900);
    }

    #[test]
    fn clamp_handles_no_monitors_honestly() {
        let saved = WindowGeometry {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
            monitor: None,
            index: 0,
        };
        let (x, y, w, h) = clamp_to_visible(&saved, &[]);
        assert_eq!((x, y, w, h), (100, 100, 800, 600));
    }

    #[test]
    fn round_trips_through_json() {
        let mut state = WindowStateFile::default();
        state.roles.insert(
            String::from("dispatch-board"),
            RoleState {
                windows: vec![WindowGeometry {
                    x: 10,
                    y: 20,
                    width: 1440,
                    height: 900,
                    monitor: Some(String::from("Built-in")),
                    index: 0,
                }],
            },
        );
        state.label_counter = 7;
        let dir = std::env::temp_dir().join("meridian-window-state-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("windows.json");
        save_state(&path, &state);
        let loaded = load_state(&path);
        assert_eq!(loaded.label_counter, 7);
        assert_eq!(loaded.roles["dispatch-board"].windows[0].width, 1440);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_state_file_loses_to_default() {
        let dir = std::env::temp_dir().join("meridian-window-state-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("broken.json");
        std::fs::write(&path, "{not json").unwrap();
        let loaded = load_state(&path);
        assert!(loaded.roles.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn label_parsing_round_trips() {
        assert!(is_lane_window_label("subwindow::ticket-detail::1"));
        assert_eq!(
            role_of_label("subwindow::ticket-detail::1"),
            Some("ticket-detail")
        );
        assert!(!is_lane_window_label("main"));
        assert_eq!(role_of_label("main"), None);
    }
}
