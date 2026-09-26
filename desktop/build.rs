fn main() {
    // Explicit app ACL manifest (Wave 34 lane 6, updater frontend seam).
    //
    // Since tauri 2.11.1, IPC from a REMOTE origin (this shell loads the
    // web app from the local Next server, an external URL) is ACL-gated
    // even for custom commands — the pre-2.11 "custom commands are not
    // ACL-gated" note was the old behavior. Declaring the app commands
    // here generates the `allow-<command>` app permissions that the
    // `desktop-web` capability grants to the loopback origin, which is
    // what makes `print_raw_zpl`, `check_desktop_update`, and
    // `install_desktop_update` reachable from the web app at all.
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "print_raw_zpl",
                "check_desktop_update",
                "install_desktop_update",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
