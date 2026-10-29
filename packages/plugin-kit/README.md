# plugin-kit

> **Purpose:** Plugin Development Kit — the ABI bindings a plugin is written against, plus the build and signing tooling. Apache-2.0.
> **Non-purpose:** No host-side runtime code (that is crates/plugin-host).

## What a plugin is (ADR 0023)

A plugin is a core WASM module with **no WASI**, three exports and one import
namespace:

| Export | Signature | What it does |
| --- | --- | --- |
| `marula_alloc` | `(i32) -> i32` | reserve `len` guest bytes, return the pointer |
| `marula_entry` | `(i32, i32) -> i64` | run the handler on `(ptr, len)`, return packed `(ptr, len)` of the reply |
| `marula_free` | `(i32, i32) -> ()` | return a block to the guest free list |

The only import namespace is `marula`, with nine entries: `log`, `kv_get`,
`kv_put`, `kv_delete`, `kv_list`, `http_request`, `emit_event`,
`read_model_query`, `now`. The catalogue (names, signatures, gates, limits and
refusal codes) is **data** in [`abi/host-abi.v1.json`](abi/host-abi.v1.json),
and `crates/plugin-host` has a test that fails if that file and the host
registry disagree.

## Writing a plugin

Two options, both zero-dependency:

- **Rust:** `pdk/rust` is `marula-pdk`, a `no_std` PDK with the three exports, a
  panic handler, a per-invocation arena allocator and typed wrappers over the
  nine host functions. A plugin is a handler plus one macro call:

  ```rust
  #![no_std]
  extern crate alloc;
  use alloc::{format, vec::Vec};
  use marula_pdk::host;

  fn handle(input: &[u8]) -> Vec<u8> {
      host::log(host::Level::Info, "handling");
      format!("{{\"seen\":{}}}", host::now()).into_bytes()
  }

  marula_pdk::plugin!(handle);
  ```

- **Any other language:** export the three functions and import only `marula.*`.
  `pdk/marula-abi.mjs` (`inspectModule`, `validateModuleAbi`) is the surface
  check the build step runs, and you can call it from your own tooling.

## Build step

`bin/build.mjs <plugin-dir>`:

1. **Validates** `<plugin-dir>/manifest.json` against the manifest v2 shape —
   the SAME closed vocabularies the host enforces at ingest: the capability
   list, widget slots/kinds, action effects, id/label shapes, per-section size
   caps, the JSON-Schema subsets (`props_schema`, `params_schema`, validator
   `schema`), and the `connector_profile.outbound_scopes` egress allowlist
   (https, exact host, no credentials, no query). Any violation fails the build
   — a bundle the host would refuse is never produced.
2. **Checks** `plugin.wasm` against ABI v1: the magic, the four-MiB member cap,
   no WASI import, no unknown `marula` entry, no host entry with the wrong
   signature, and all three `marula_*` exports present with the right
   signatures.
3. **Bundles** `manifest.json` + `plugin.wasm` into
   `dist/<slug>-<version>.zip` and prints the bundle sha256 — the digest an
   operator pins in a registry entry.

`npm test` runs the kit's own node tests, including the ABI validator against
hand-built good and bad modules.

## Plugin layout

```
my-plugin/
  manifest.json    # v2 manifest; surface_profile declares what it contributes
  plugin.wasm      # built from your crate (wasm32-unknown-unknown, cdylib)
  src/lib.rs       # the crate; see plugins/wasm/* for minimal examples
  Cargo.toml       # detached from the Marula workspace ([workspace] table)
```

## Walkthrough

See `plugins/wasm/README.md` — the three first-party reference plugins
(widget, automation action, field validator) build with exactly these two
commands and prove the install → consent → grant → render/execute chain.
