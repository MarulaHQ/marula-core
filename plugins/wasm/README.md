# wasm

First-party Extism/WASM plugins, built on `packages/plugin-kit` and installed
through the same signed lifecycle as every connector (ADR 0009).

## Reference plugins (Wave 34 lane 4, §4.6)

Three real plugins prove the full chain — manifest declares a
`surface_profile` → consent renders the declarations → capability grant
(`ui.widgets` / `automation.actions` / `fields.validate`) registers the
entries → the core renders / executes them through its own components:

| Plugin | Contributes | Capability |
| --- | --- | --- |
| `escalation-widget/` | a read-only list widget on the ticket pane | `ui.widgets` |
| `triage-note-action/` | a typed automation action kind (internal-comment effect) | `automation.actions` |
| `asset-tag-validator/` | a JSON-Schema validator for custom fields | `fields.validate` |

## Build (walk the PDK story)

```sh
# 1. Build the WASM artifact (detached crate; wasm32-unknown-unknown target).
cargo build --manifest-path plugins/wasm/escalation-widget/Cargo.toml \
  --target wasm32-unknown-unknown --release --target-dir plugins/wasm/target
cp plugins/wasm/target/wasm32-unknown-unknown/release/escalation_widget.wasm \
  plugins/wasm/escalation-widget/plugin.wasm

# 2. Validate the manifest + bundle manifest.json and plugin.wasm.
node packages/plugin-kit/bin/build.mjs plugins/wasm/escalation-widget
```

The kit validates the manifest against the SAME closed vocabularies the
plugin host enforces at ingest (capabilities, widget slots/kinds, action
effects, JSON-Schema subsets) and prints the bundle's sha256 digest — the
value an operator pins in a registry entry.

## Install

Settings → Plugins → import (zip upload or registry entry). The staged
review shows the surface declarations verbatim; grant the named capability
to register the entries. Entries exist only while the install is enabled
and the grant stands.
