---
description: "A plugin is a signed WebAssembly module with no WASI and three exports. This walkthrough takes a handler from an empty crate to an installable bundle."
---

# How to write and build a plugin

> **Audience:** developer — you are writing the guest module and its
> manifest. An admin installs and grants it afterwards, and that half is
> [the plugin consent flow](admin-plugin-consent.md).

## Prerequisites

- Rust with the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).
- Node 20 or newer, for the build and signing scripts in `packages/plugin-kit`.
- A stack where an admin can install your bundle, if you want to see it run.
- Read one of the reference plugins in `plugins/wasm/` before writing your own. They are
  deliberately small.

## 1. Know what the host will accept

The host refuses a module before running it when the module breaks any of these:

- it imports anything but the `marula` namespace, so no WASI and no `env`;
- it imports a `marula` entry the ABI does not declare, or one with the wrong signature;
- it lacks `marula_alloc`, `marula_entry`, `marula_free` or an exported `memory`, or declares one
  with the wrong type.

There are nine host entries over six functions. Two of them, `emit_event` and
`read_model_query`, pass their gate and then refuse as not implemented in this build. The full
list, with each gate and every refusal code, is
[the host-function reference](../../reference/plugin-host-functions.md).

## 2. Create the crate

A plugin is a detached crate: its own `Cargo.toml`, its own target directory, outside the Marula
workspace.

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
marula-pdk = { path = "../../packages/plugin-kit/pdk/rust" }

[workspace]
```

The empty `[workspace]` table is what keeps the crate out of the Marula workspace, so your build
does not pull in the product's dependency graph.

## 3. Write the handler

```rust
#![no_std]
extern crate alloc;

use alloc::{format, vec::Vec};
use marula_pdk::host;

fn handle(input: &[u8]) -> Vec<u8> {
    host::log(host::Level::Info, "my-plugin: handling");
    let cached = host::kv_get("cache", "last-seen").unwrap_or_default();
    format!(
        "{{\"input_bytes\":{},\"cache_bytes\":{},\"at\":{}}}",
        input.len(),
        cached.len(),
        host::now()
    )
    .into_bytes()
}

marula_pdk::plugin!(handle);
```

`marula_pdk::plugin!` supplies the three exports, a panic handler that traps rather than loops, and
an allocator whose blocks live for one invocation. `host::*` returns `Result<_, Refusal>` wherever
the ABI has somewhere to put a refusal, and nothing where it does not: `log` is the one entry a
guest cannot observe a refusal from, because it has no return value.

`kv_get` on an absent key is `Ok` with an empty vector, not a refusal. A refusal means the store
said no, and the two are worth keeping apart in your own logging.

## 4. Declare the manifest

`manifest.json` is what the consent screen renders and what the install is granted against.

```json
{
  "name": "My plugin",
  "version": "0.1.0",
  "interface_version": 1,
  "description": "Caches the last seen ticket blurb for the pane.",
  "capabilities": ["ui.widgets"],
  "surface_profile": {
    "widgets": [
      {
        "id": "last_seen",
        "title": "Last seen",
        "slot": "ticket_pane",
        "kind": "list"
      }
    ]
  }
}
```

Two rules the validator enforces, because the host enforces them too:

- **`capabilities` is a closed list.** A grant outside it is rejected at write time, so an admin
  cannot widen your plugin past what you declared.
- **Each surface kind registers under exactly one capability.** A widget needs `ui.widgets`, an
  action needs `automation.actions`, and a validator needs `fields.validate`. The mapping is in
  [the capability catalogue](../../reference/plugin-capabilities.md#which-capability-a-surface-needs).

If your plugin calls `http_request`, declare `connector_profile.outbound_scopes` as well. That
declaration is the allowlist the gate reads, and a call to a host outside it is refused.

## 5. Build and validate

```sh
cargo build --manifest-path plugins/wasm/my-plugin/Cargo.toml \
  --target wasm32-unknown-unknown --release --target-dir plugins/wasm/target
cp plugins/wasm/target/wasm32-unknown-unknown/release/my_plugin.wasm \
  plugins/wasm/my-plugin/plugin.wasm

node packages/plugin-kit/bin/build.mjs plugins/wasm/my-plugin
```

The build step validates the manifest against the same closed vocabularies the host uses, checks
the module against ABI v1, and writes `dist/<slug>-<version>.zip`. It prints the bundle's sha256,
which is the digest an operator pins in a registry entry. A bundle the host would refuse is never
produced.

## 6. Sign it if you are publishing

```sh
node packages/plugin-kit/bin/sign.mjs catalog.json --key publisher.pem --key-id 2026-q4
```

The statement binds the artifact digest and the canonical manifest digest, and an admin's consent
screen shows **manifest signed** when a trusted key verifies it. A local zip or a GitHub pull
cannot show that, because neither carries a publisher statement. The digest still proves the bytes
have not changed since they were fetched.

## 7. Watch it run

Install the bundle through [the consent flow](admin-plugin-consent.md), grant the capability, and
enable it. Then find the surface where you declared it — a ticket-pane widget appears on a ticket.

Every invocation leaves one audit row with the outcome, the refusal reason, the capability, the
fuel spent, the duration and the worker that answered. That row is the fastest way to find out why
something did not happen, and
[the operator runbook](../../reference/plugin-operator-runbook.md#triage-by-symptom) maps each
reason to its cause.

## What to expect when it goes wrong

| What you see | What it means |
| --- | --- |
| the plugin never appears in the pane | the install is disabled, the grant is missing, or the widget id does not match the manifest |
| `plugin_fuel_exhausted` | a loop, or 64 host calls in one invocation — the cap is per invocation |
| `plugin_guest_trap` | a panic, an out-of-bounds access, or growth past 64 MiB |
| `plugin_output_too_large` | your handler claimed a reply over 1 MiB |
| `plugin_not_implemented` | you called `emit_event` or `read_model_query`, both deferred in this build |
| your log lines are missing | over 100 lines, or a line over 4 KiB — the extra lines are dropped and the guest cannot tell |
