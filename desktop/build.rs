fn main() {
    // Tauri's `tauri::generate_context!()` macro checks at compile time that
    // `build.frontendDist` (`../apps/web/out`) exists, even though the desktop
    // shell loads the bundled Next.js standalone server over loopback rather
    // than serving static assets (ADR 0005). Because `apps/web/next.config.mjs`
    // uses `output: "standalone"`, `apps/web/out` does not exist on a fresh
    // checkout until `scripts/build-desktop.sh` creates the placeholder.
    // Creating the gitignored placeholder directory here lets `cargo check
    // --workspace` and `cargo clippy --workspace` include `marula-desktop`
    // without requiring a prior desktop bundle build (Wave 43 lane 9,
    // `p43-repo-hygiene`).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let frontend_dist = std::path::Path::new(&manifest_dir).join("../apps/web/out");
        let index_html = frontend_dist.join("index.html");
        if !index_html.exists() {
            let _ = std::fs::create_dir_all(&frontend_dist);
            let _ = std::fs::write(
                &index_html,
                "The desktop app serves the bundled Next server, not these assets.\n",
            );
        }
    }

    // Explicit app ACL manifest (Wave 34 lane 6, updater frontend seam).
    //
    // Since tauri 2.11.1, IPC from a REMOTE origin (this shell loads the
    // web app from the local Next server, an external URL) is ACL-gated
    // even for custom commands — the pre-2.11 "custom commands are not
    // ACL-gated" note was the old behavior. Declaring the app commands
    // here generates the `allow-<command>` app permissions that the
    // `desktop-web` capability grants to the loopback origin, which is
    // what makes the app commands reachable from the web app at all.
    // Wave 38 lane 2 adds the native multi-window commands (open/focus
    // role windows, self-capture geometry, route marula:// links).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "print_raw_zpl",
            "check_desktop_update",
            "install_desktop_update",
            "open_native_window",
            "focus_native_window",
            "route_marula_link",
        ]),
    ))
    .expect("failed to run tauri-build");
}
