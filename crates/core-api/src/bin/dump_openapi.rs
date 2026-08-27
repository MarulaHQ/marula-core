//! Dev utility: dumps the OpenAPI document that core-api serves at
//! `/openapi.json` to stdout (or `--write <path>`), with NO database,
//! server, or network required — the document is derived purely from the
//! handler annotations plus the canonicalization applied by
//! [`core_api::routes::openapi::openapi_json`].
//!
//! Purpose: keeps `packages/sdk/openapi.json` regenerable hermetically
//! (`cargo run -p core-api --bin dump-openapi -- --write packages/sdk/openapi.json`
//! followed by `pnpm --filter @meridian/sdk generate`) so SDK snapshots
//! never require booting a stack. The old curl-the-running-server workflow
//! stays valid but is no longer necessary.
//!
//! Non-purpose: NOT a lint or drift gate. Snapshot freshness is a review
//! discipline; CI conformance-vs-spec enforcement is U8's job.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json = core_api::routes::openapi::openapi_json();
    let pretty = serde_json::to_string_pretty(&json).expect("serialize spec");
    if args.get(1).map(String::as_str) == Some("--write") {
        let path = args
            .get(2)
            .cloned()
            .expect("--write requires a file argument");
        std::fs::write(&path, pretty + "\n").expect("write spec snapshot");
        eprintln!("wrote {path}");
    } else {
        println!("{pretty}");
    }
}
