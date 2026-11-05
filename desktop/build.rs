fn main() {
    // `build.frontendDist` is `./connect`: the shell's own first-run screen.
    // The product's front end is not a Tauri asset at all — the webview loads
    // it from the instance it is connected to, or from the bundled Next.js
    // server in embedded mode (ADR 0005, ADR 0028). The connect screen is the
    // one page this crate serves itself, and it is tracked, so a fresh
    // checkout has nothing to generate and `cargo check --workspace` includes
    // `marula-desktop` without a prior desktop bundle build.

    // Explicit app ACL manifest (Wave 34 lane 6, updater frontend seam).
    //
    // Since tauri 2.11.1, IPC from a REMOTE origin (this shell loads the
    // web app from the local Next server or from an instance, an external
    // URL) is ACL-gated even for custom commands — the pre-2.11 "custom
    // commands are not ACL-gated" note was the old behavior. Declaring the app
    // commands here generates the `allow-<command>` app permissions that the
    // `desktop-web` capability grants to the loopback origin, which is what
    // makes the app commands reachable from the web app at all. Wave 38 lane 2
    // adds the native multi-window commands (open/focus role windows,
    // self-capture geometry, route marula:// links); the connection lane adds
    // `save_connection`, which only the connect screen's own capability
    // grants.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "print_raw_zpl",
            "check_desktop_update",
            "install_desktop_update",
            "open_native_window",
            "focus_native_window",
            "route_marula_link",
            "save_connection",
        ]),
    ))
    .expect("failed to run tauri-build");
}
