fn main() {
    // Tauri's `tauri::generate_context!()` macro checks at compile time that
    // `build.frontendDist` (`../apps/installer/dist`) exists. That path is
    // vite's gitignored `outDir`, so a fresh checkout has none, and the macro
    // panics before a single line of Rust is typechecked — which meant a clean
    // clone could not run `cargo check`, `cargo clippy` or `cargo test` on the
    // workspace at all (CI's Rust job has no step that builds this front-end,
    // and neither does a developer's first checkout).
    //
    // Every real build populates the directory first: `release-installer.yml`
    // runs `pnpm --dir apps/installer build` before `cargo build`, and vite
    // empties `outDir` as it writes. So this guard only ever creates the
    // placeholder a typecheck needs. `desktop/build.rs` carried the same guard
    // for `apps/web/out` since Wave 43 lane 9, `p43-repo-hygiene`; that crate
    // now points `frontendDist` at its own tracked connect screen, so this is
    // the only guard left.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let frontend_dist = std::path::Path::new(&manifest_dir).join("../apps/installer/dist");
        let index_html = frontend_dist.join("index.html");
        if !index_html.exists() {
            let _ = std::fs::create_dir_all(&frontend_dist);
            let _ = std::fs::write(
                &index_html,
                "Placeholder: run `pnpm --dir apps/installer build` for the real bundle.\n",
            );
        }
    }

    tauri_build::build();
}
