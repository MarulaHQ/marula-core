//! OS integration: menu bar, tray quick actions, deep links, single
//! instance (Wave 38 lane 2).
//!
//! Purpose: the native chrome around the window fleet.
//! - macOS Window menu with the standard Minimize/Zoom items; native
//!   tabbing stays OFF (windows are the point: the Window menu lists
//!   windows, never merges them).
//! - Tray menu with the two quick actions the scope names ("New ticket
//!   window", "Open dispatch board") plus stock lookup and Quit.
//! - `marula://` deep links open-or-focus the right window, using the
//!   same small grammar the renderer ships (native-side mirror so a link
//!   works even before a webview exists).
//! - Single instance: a second launch focuses the running app and hands it
//!   any deep link it was opened with.

use tauri::{
    image::Image,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

// ---------------------------------------------------------------------------
// Deep-link grammar — native mirror of the renderer's lanes
// ---------------------------------------------------------------------------

/// What a `marula://` link resolves to natively. Mirrors the renderer
/// grammar (native/src/core/links.ts parseMarulaLink + lane 1's
/// popoutKeyForDeepLinkKind) so both shells agree without a webview.
#[derive(Debug, PartialEq)]
pub enum DeepLinkTarget {
    /// ticket/<uuid>
    Ticket(String),
    /// notifications
    Notifications,
    /// stock-item/<uuid>, stock-location/<uuid>, stock-serial/<uuid>,
    /// device-asset/<uuid>, tool-kit/<uuid>
    Entity(String, String),
    /// board/<uuid>
    Board(String),
    Unrecognized(String),
}

/// Parse a `marula://…` URL into a target. Lenient on whitespace and
/// case of the scheme; the host/path are the same shape either way.
pub fn parse_deep_link(raw: &str) -> DeepLinkTarget {
    let trimmed = raw.trim();
    let Some(rest) = trimmed
        .strip_prefix("marula://")
        .or_else(|| trimmed.strip_prefix("MARULA://"))
    else {
        return DeepLinkTarget::Unrecognized(raw.to_string());
    };
    // Drop any query; take the path segments.
    let path = rest.split('?').next().unwrap_or("");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Shape check only (version nibble included in the hex range): the id is
    // pasted into a path, so anything that is not hex and dashes is refused
    // rather than carried into a window URL.
    let uuid = |s: &str| {
        s.len() == 36
            && s.chars().filter(|c| *c == '-').count() == 4
            && s.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
    };
    match segments.as_slice() {
        ["ticket", id] if uuid(id) => DeepLinkTarget::Ticket((*id).to_string()),
        ["notifications"] => DeepLinkTarget::Notifications,
        ["board", id] if uuid(id) => DeepLinkTarget::Board((*id).to_string()),
        ["stock-item", id]
        | ["stock-location", id]
        | ["stock-serial", id]
        | ["device-asset", id]
        | ["tool-kit", id]
            if uuid(id) =>
        {
            DeepLinkTarget::Entity(segments[0].to_string(), (*id).to_string())
        }
        _ => DeepLinkTarget::Unrecognized(raw.to_string()),
    }
}

impl DeepLinkTarget {
    /// (role, path suffix) the target opens-or-focuses. None = nothing to
    /// open (unrecognized links route to the main window's path, below).
    pub fn window_target(&self) -> Option<(String, String)> {
        match self {
            DeepLinkTarget::Ticket(id) => {
                Some((String::from("ticket-detail"), format!("tickets/{id}")))
            }
            DeepLinkTarget::Board(_) => {
                Some((String::from("dispatch-board"), String::from("dispatch")))
            }
            DeepLinkTarget::Notifications => {
                // The web app has no dedicated notifications page — the
                // dock routes by ticket. Notifications links surface the
                // main window; nothing is invented.
                None
            }
            DeepLinkTarget::Entity(_, id) => {
                // Stock entities (the only QR-printable kinds today) land on
                // the stock lookup window; the window's page can filter by
                // the id param when the renderer picks it up.
                Some((
                    String::from("stock-lookup"),
                    format!("stock?highlight={id}"),
                ))
            }
            DeepLinkTarget::Unrecognized(_) => None,
        }
    }

    /// Where the MAIN window routes for this link (always Some: unknown
    /// links still surface somewhere honest; a role window that cannot be
    /// opened falls back here).
    pub fn main_path(&self) -> String {
        match self {
            DeepLinkTarget::Ticket(id) => format!("/tickets/{id}"),
            DeepLinkTarget::Board(_) => String::from("/dispatch"),
            DeepLinkTarget::Entity(kind, id) => match kind.as_str() {
                "stock-location" => format!("/stock/locations/{id}"),
                "stock-serial" => format!("/stock/{id}"),
                "device-asset" => format!("/assets/{id}"),
                "tool-kit" => format!("/tools/{id}"),
                _ => format!("/stock?highlight={id}"),
            },
            DeepLinkTarget::Unrecognized(_) => String::from("/"),
            // No dedicated notifications page exists: land on home and let
            // the dock (bottom-right bell) show the items.
            DeepLinkTarget::Notifications => String::from("/"),
        }
    }
}

/// Open-or-focus the right window for a parsed deep link. Returns the
/// window label acted on.
pub async fn route_deep_link(app: &AppHandle, raw: &str) -> Result<String, String> {
    let target = parse_deep_link(raw);
    match target.window_target() {
        Some((role, path)) => {
            // Open-or-focus, then FOLLOW the link. A window of the right role
            // showing the previous record is the wrong answer to
            // `marula://ticket/<new id>`; `navigate_role_window` moves it and
            // leaves it alone when it already shows what the link names.
            if let Some(label) = super::window_manager::find_by_role(app, &role) {
                if app.get_webview_window(&label).is_some() {
                    if let Err(e) =
                        super::window_manager::navigate_role_window(app, &label, &role, &path)
                    {
                        tracing::warn!("deep link could not move {label}: {e}");
                    }
                    return Ok(label);
                }
            }
            match super::window_manager::open_native_window(app.clone(), role, path).await {
                Ok(label) => Ok(label),
                Err(open_error) => {
                    // A role window that cannot open still leaves a route for
                    // this record: the main window, at the same in-app path.
                    match super::window_manager::navigate_main_window(app, &target.main_path()) {
                        Ok(_) => {
                            tracing::warn!("{open_error}; routed the link to the main window");
                            Ok(String::from("main"))
                        }
                        Err(fallback_error) => Err(format!(
                            "{open_error}; main window fallback failed: {fallback_error}"
                        )),
                    }
                }
            }
        }
        None => {
            // At minimum surface the main window.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.unminimize();
                let _ = main.set_focus();
                Ok(String::from("main"))
            } else {
                Err(format!("no main window to route {raw} to and role unknown"))
            }
        }
    }
}

/// Tauri command: the web renderer's deep-link door (a renderer can ALSO
/// route links — e.g. a QR hit inside a ticket window asks the shell to
/// open-or-focus). Same grammar, same open-or-focus contract.
#[tauri::command]
pub async fn route_marula_link(app: AppHandle, url: String) -> Result<String, String> {
    route_deep_link(&app, &url).await
}

// ---------------------------------------------------------------------------
// Menu bar — macOS Window menu, tabs OFF
// ---------------------------------------------------------------------------

/// Build the app menu. App/GEdit/View follow the platform defaults; the
/// Window submenu is explicit so the fleet pattern (Minimize / Zoom /
/// Bring All to Front + per-window entries the OS fills) is guaranteed on
/// every platform we ship.
pub fn build_menu(app: &AppHandle) -> tauri::Result<()> {
    // License and name are the About box's job, and until now it carried the
    // bundle's version only. `copyright` and `credits` are what macOS shows;
    // `license` and `website` are what Windows and Linux show, so both are
    // filled rather than one platform's half.
    let about = {
        let version = app.package_info().version.to_string();
        AboutMetadata {
            name: Some("Marula".into()),
            version: Some(version.clone()),
            short_version: Some(version),
            copyright: Some("AGPL-3.0 — Marula contributors".into()),
            license: Some("AGPL-3.0 (GNU Affero General Public License v3.0)".into()),
            credits: Some(
                "Free software under the AGPL-3.0. Third-party notices sit beside the app in \
                 ACKNOWLEDGMENTS.md and LEGAL/."
                    .into(),
            ),
            ..Default::default()
        }
    };
    let app_menu = Submenu::with_id_and_items(
        app,
        "marula-app",
        "Marula",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Marula"), Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let edit_menu = Submenu::with_id_and_items(
        app,
        "marula-edit",
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view_menu = Submenu::with_id_and_items(
        app,
        "marula-view",
        "View",
        true,
        &[
            // Full screen is system-zoom truth; tabs never appear.
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::separator(app)?,
        ],
    )?;

    let window_menu = Submenu::with_id_and_items(
        app,
        "marula-window",
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            // Zoom = the macOS zoom toggle (not "maximize"); Front brings
            // the whole fleet forward.
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::bring_all_to_front(app, None)?,
        ],
    )?;

    let menu = Menu::with_items(app, &[&app_menu, &edit_menu, &view_menu, &window_menu])?;
    app.set_menu(menu)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tray — quick actions
// ---------------------------------------------------------------------------

// Brand-kit tray glyphs, baked into the binary (`include_image!` embeds the
// pixels at compile time). macOS gets the template image: the ink glyph on a
// transparent field, which AppKit re-tints to match a light or dark menu bar,
// so one asset serves both appearances. Windows and Linux taskbars do not
// recolour template images, so those platforms keep the full-colour glyph the
// kit ships alongside it. tray-icon normalises whatever it is handed to an
// 18pt-high status item, so the @2x files (44px) are the source — crisp on a
// Retina bar rather than an upscaled 22px.
#[cfg(target_os = "macos")]
const TRAY_ICON: Image<'static> = tauri::include_image!("icons/tray/tray-iconTemplate@2x.png");
#[cfg(not(target_os = "macos"))]
const TRAY_ICON: Image<'static> = tauri::include_image!("icons/tray/tray-color-44.png");

/// Show the tray with quick actions. Menu ids are dispatched in
/// `on_tray_event` in main.rs; keeping ids stringly makes the mapping
/// reviewable next to the actions themselves.
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let new_ticket = MenuItem::with_id(
        app,
        "tray-new-ticket",
        "New ticket window",
        true,
        None::<&str>,
    )?;
    let dispatch = MenuItem::with_id(
        app,
        "tray-dispatch",
        "Open dispatch board",
        true,
        None::<&str>,
    )?;
    let stock = MenuItem::with_id(app, "tray-stock", "Open stock lookup", true, None::<&str>)?;
    let show_main = MenuItem::with_id(app, "tray-show-main", "Show Marula", true, None::<&str>)?;
    let sep = tauri::menu::PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Marula", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show_main,
            &sep,
            &new_ticket,
            &dispatch,
            &stock,
            &sep,
            &quit,
        ],
    )?;
    let builder = TrayIconBuilder::with_id("marula-tray")
        .icon(TRAY_ICON)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Marula")
        .on_menu_event(|app, event| {
            dispatch_tray_action(app, event.id().as_ref());
        });

    // AppKit paints a template image in the menu bar's own colour; off macOS
    // the flag is ignored (the kit's colour glyph already carries the brand).
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);

    builder.build(app)?;
    Ok(())
}

/// One mapping: tray menu id → fleet action. The quick actions the scope
/// names ("New ticket window", "Open dispatch board") live here.
pub fn dispatch_tray_action(app: &AppHandle, id: &str) {
    let action: Option<(&str, &str)> = match id {
        "tray-new-ticket" => Some(("ticket-detail", "tickets")),
        "tray-dispatch" => Some(("dispatch-board", "dispatch")),
        "tray-stock" => Some(("stock-lookup", "stock")),
        _ => None,
    };
    if let Some((role, path)) = action {
        let app = app.clone();
        let role = role.to_string();
        let path = path.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = super::window_manager::open_native_window(app.clone(), role, path).await
            {
                tracing::warn!("tray window action failed: {e}");
            }
        });
        return;
    }
    match id {
        "tray-show-main" => {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.unminimize();
                let _ = main.set_focus();
            }
        }
        "tray-quit" => app.exit(0),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ticket_link() {
        let id = "0198c7a2-7b1c-7000-8000-000000000001";
        match parse_deep_link(&format!("marula://ticket/{id}")) {
            DeepLinkTarget::Ticket(got) => assert_eq!(got, id),
            other => panic!("expected ticket, got {other:?}"),
        }
    }

    #[test]
    fn parses_stock_item_link_to_entity() {
        let id = "0198c7a2-7b1c-7000-8000-000000000002";
        match parse_deep_link(&format!("marula://stock-item/{id}?x=1")) {
            DeepLinkTarget::Entity(kind, got) => {
                assert_eq!(kind, "stock-item");
                assert_eq!(got, id);
            }
            other => panic!("expected entity, got {other:?}"),
        }
    }

    #[test]
    fn notifications_link_parses() {
        assert!(matches!(
            parse_deep_link("marula://notifications"),
            DeepLinkTarget::Notifications
        ));
    }

    #[test]
    fn non_uuid_ticket_is_unrecognized() {
        assert!(matches!(
            parse_deep_link("marula://ticket/not-a-uuid"),
            DeepLinkTarget::Unrecognized(_)
        ));
    }

    /// A 36-character string with four dashes is not an id. An id that is not
    /// hex stays out of the window URL it would otherwise be pasted into.
    #[test]
    fn a_36_character_non_hex_id_is_unrecognized() {
        assert!(matches!(
            parse_deep_link("marula://ticket/zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz"),
            DeepLinkTarget::Unrecognized(_)
        ));
    }

    #[test]
    fn foreign_scheme_never_parses() {
        assert!(matches!(
            parse_deep_link("https://marula.example/ticket/abc"),
            DeepLinkTarget::Unrecognized(_)
        ));
    }

    #[test]
    fn window_targets_match_lane_grammar() {
        let id = "0198c7a2-7b1c-7000-8000-000000000003";
        let t = parse_deep_link(&format!("marula://ticket/{id}"));
        let (role, path) = t.window_target().expect("window target");
        assert_eq!(role, "ticket-detail");
        assert_eq!(path, format!("tickets/{id}"));

        let b = parse_deep_link(&format!("marula://board/{id}"));
        let (role, _) = b.window_target().expect("board target");
        assert_eq!(role, "dispatch-board");

        let s = parse_deep_link(&format!("marula://stock-item/{id}"));
        let (role, _) = s.window_target().expect("stock target");
        assert_eq!(role, "stock-lookup");
    }

    #[test]
    fn main_paths_route_entities() {
        let id = "0198c7a2-7b1c-7000-8000-000000000004";
        assert_eq!(
            DeepLinkTarget::Ticket(id.to_string()).main_path(),
            format!("/tickets/{id}")
        );
        assert_eq!(
            DeepLinkTarget::Entity(String::from("device-asset"), id.to_string()).main_path(),
            format!("/assets/{id}")
        );
        // Notifications route to home (no dedicated page; the dock owns them).
        assert_eq!(DeepLinkTarget::Notifications.main_path(), "/");
        assert!(DeepLinkTarget::Notifications.window_target().is_none());
    }

    /// The two places a link can land (its role window, and the main window
    /// when that role window cannot open) must name the same record, or the
    /// fallback shows someone a different ticket than the link did.
    #[test]
    fn a_link_lands_on_the_same_record_in_a_window_and_in_the_main_view() {
        let id = "0198c7a2-7b1c-7000-8000-000000000005";
        let link = parse_deep_link(&format!("marula://ticket/{id}"));
        let (role, window_path) = link.window_target().expect("ticket has a window target");
        assert_eq!(role, "ticket-detail");
        assert!(
            window_path.ends_with(id),
            "window path {window_path} should name the ticket"
        );
        assert!(
            link.main_path().ends_with(id),
            "main path {} should name the same ticket",
            link.main_path()
        );
    }
}
