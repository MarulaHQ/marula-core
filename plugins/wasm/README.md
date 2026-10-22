# wasm

First-party WASM plugins, built on `packages/plugin-kit` and installed through
the same signed lifecycle as every connector (ADR 0009). The guest ABI they
implement is ADR 0023's: a core WASM module with no WASI, three exports
(`marula_alloc`, `marula_entry`, `marula_free`) and one import namespace,
`marula`. It is not Extism — a component-model runtime cannot carry this
contract, and the ADR records why.

Every plugin here is written with `marula-pdk` (`packages/plugin-kit/pdk/rust`):

```rust
#![no_std]
extern crate alloc;

use alloc::{format, vec::Vec};
use marula_pdk::host;

fn handle(input: &[u8]) -> Vec<u8> { /* … */ }

marula_pdk::plugin!(handle);
```

The macro supplies the three exports, the panic handler (a trap, so a panic
burns no fuel) and the allocator; `host::*` supplies typed wrappers for the nine
host functions, each returning `Result<_, Refusal>` with the same reason strings
the audit row carries.

## Reference plugins (Wave 34 lane 4, §4.6)

Three real plugins prove the full chain — manifest declares a
`surface_profile` → consent renders the declarations → capability grant
(`ui.widgets` / `automation.actions` / `fields.validate`) registers the
entries → the core renders / executes them through its own components:

| Plugin | Contributes | Capability | Host calls |
| --- | --- | --- | --- |
| `escalation-widget/` | a read-only list widget on the ticket pane | `ui.widgets` | `log`, `now` |
| `triage-note-action/` | a typed automation action kind (internal-comment effect) | `automation.actions` | `log` |
| `asset-tag-validator/` | a JSON-Schema validator for custom fields | `fields.validate` | `log` |

They stay on `log` and `now` on purpose: both ship today, and the data-plane
calls (`read_model_query`, `emit_event`, `kv_*`, `http_request`) are typed
refusals until W53-5, W53-6 and W53-7 land. A reference plugin that called one
would fail at runtime, not document the ABI.

## Build

```sh
# 1. Build the WASM artifact (detached crate; wasm32-unknown-unknown target,
#    its own target dir so the shared cargo cache stays clean).
cargo build --manifest-path plugins/wasm/escalation-widget/Cargo.toml \
  --target wasm32-unknown-unknown --release --target-dir plugins/wasm/target
cp plugins/wasm/target/wasm32-unknown-unknown/release/escalation_widget.wasm \
  plugins/wasm/escalation-widget/plugin.wasm

# 2. Validate the manifest and the module's ABI surface, then bundle.
node packages/plugin-kit/bin/build.mjs plugins/wasm/escalation-widget
```

The kit validates the manifest against the SAME closed vocabularies the plugin
host enforces at ingest (capabilities, widget slots/kinds, action effects,
JSON-Schema subsets, egress scopes), refuses a module that imports anything but
`marula.*` or lacks a `marula_*` export, and prints the bundle's sha256 digest —
the value an operator pins in a registry entry.

## Install

Settings → Plugins → import (zip upload or registry entry). The staged review
shows the surface declarations verbatim; grant the named capability to register
the entries. Entries exist only while the install is enabled and the grant
stands.
