# plugin-kit

> **Purpose:** TS Plugin Development Kit + `marula dev` harness glue. Apache-2.0.
> **Non-purpose:** No host-side runtime code (that is crates/plugin-host).

## What it does today (Wave 34 lane 4)

`bin/build.mjs <plugin-dir>` is the PDK build step:

1. **Validates** `<plugin-dir>/manifest.json` against the manifest v2 shape —
   the SAME closed vocabularies the host enforces at ingest: the capability
   list, widget slots/kinds, action effects, id/label shapes, per-section
   size caps, and the JSON-Schema subsets (`props_schema`, `params_schema`,
   validator `schema`). Any violation fails the build (a bundle the host
   would refuse is never produced).
2. **Bundles** `manifest.json` + `plugin.wasm` into
   `dist/<slug>-<version>.zip` and prints the bundle sha256 — the digest an
   operator pins in a registry entry.

## Plugin layout

```
my-plugin/
  manifest.json    # v2 manifest; surface_profile declares what it contributes
  plugin.wasm      # built from your crate (wasm32-unknown-unknown)
  src/lib.rs       # the crate; see plugins/wasm/* for minimal examples
  Cargo.toml       # detached from the Marula workspace ([workspace] table)
```

## Walkthrough

See `plugins/wasm/README.md` — the three first-party reference plugins
(widget, automation action, field validator) build with exactly these two
commands and prove the install → consent → grant → render/execute chain.
