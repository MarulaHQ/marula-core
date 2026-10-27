# Writing a plugin (PDK v1)

**Decision of record:** [ADR 0023](../adr/0023-plugin-runtime-isolation.md)
**PDK:** `packages/plugin-kit/pdk/rust` (`marula-pdk`) · **Kit:** `packages/plugin-kit`
**Reference plugins:** `plugins/wasm/{escalation-widget,triage-note-action,asset-tag-validator}`

A plugin is a core WebAssembly module with no WASI, three exports and one import namespace. The PDK
supplies the parts every guest would otherwise write by hand: the exports, a panic handler that
traps instead of looping, a per-invocation arena allocator, and typed wrappers over the nine host
functions.

This page takes you from an empty directory to a bundle an operator can install. The
[host-function reference](plugin-host-functions.md) is the companion page for what each call does.

## The module contract

| Export | Signature | What the host does with it |
| --- | --- | --- |
| `marula_alloc` | `(i32) -> i32` | calls it to reserve space for the input, then writes the payload there |
| `marula_entry` | `(i32, i32) -> i64` | calls it with the input's `(ptr, len)` and reads the packed `(ptr, len)` reply |
| `marula_free` | `(i32, i32) -> ()` | calls it after reading the reply, and a trap here cannot change the answer |
| `memory` | an exported linear memory | every buffer crosses as `(pointer, length)` into it |

The only import namespace is `marula`. A module that imports anything else, WASI included, is
refused at load before it runs. So is a module that omits an export or declares one with the wrong
type.

Zero-length output is a valid answer and means "nothing to say". A guest that returns the zero word
is treated the same as one that returns an empty range.

## A minimal plugin

```rust
#![no_std]
extern crate alloc;

use alloc::{format, vec::Vec};
use marula_pdk::host;

fn handle(input: &[u8]) -> Vec<u8> {
    host::log(host::Level::Info, "handling a ticket pane");
    format!("{{\"generated_at\":{}}}", host::now()).into_bytes()
}

marula_pdk::plugin!(handle);
```

The macro supplies the three exports, the panic handler and the allocator. `#![no_std]` is not a
style choice: the module links no libc and has no syscalls, and `alloc` is available through the
guest allocator the PDK provides.

## The handler's memory model

Allocations live for one invocation. The host allocates the input before `marula_entry` and reads
the reply after it returns, so the allocator keeps freed blocks on a free list and reuses them. It
never returns memory to the system, because WebAssembly gives it no way to. The sandbox's 64 MiB
instance cap is the ceiling that matters.

A guest that grows past that cap traps. The host sets `trap_on_grow_failure`, so a failed growth is
a trap rather than a `-1` the guest could ignore and then write through.

## Calling the host

The typed wrappers return `Result` where the ABI has somewhere to put a refusal, and nothing where
it does not.

| Wrapper | Returns | Notes |
| --- | --- | --- |
| `host::log(level, message)` | nothing | over budget, the line is dropped, and the guest cannot tell |
| `host::now()` | `i64` | milliseconds since the epoch, the host's clock |
| `host::kv_get(ns, key)` | `Result<Vec<u8>, Refusal>` | an absent key is `Ok(empty)`, not a refusal |
| `host::kv_put(ns, key, value)` | `Result<(), Refusal>` | quota and size refusals land here |
| `host::kv_delete(ns, key)` | `Result<(), Refusal>` | |
| `host::kv_list(ns, prefix, cursor)` | `Result<Vec<u8>, Refusal>` | one page of up to 500 keys, JSON |
| `host::http_request(profile, request)` | `Result<Vec<u8>, Refusal>` | the request is a JSON `HttpRequest` |
| `host::emit_event(event_type, payload)` | `Result<(), Refusal>` | refuses `plugin_not_implemented` today |
| `host::read_model_query(query_id, params)` | `Result<Vec<u8>, Refusal>` | refuses `plugin_not_implemented` today |

`Refusal` is an enum over the twenty-five codes in the ABI, with the same strings the audit row
carries. `Refusal::parse` and `Refusal::from_code` exist so a plugin can log what it caught without
guessing.

The two deferred entries are worth designing for now: a plugin that wants to publish an event or
read a model query should treat the refusal as "not in this build" and carry on, rather than
treating it as a fault. The [host-function reference](plugin-host-functions.md#the-two-deferred-entries-refuse-today)
records why each is deferred.

## The manifest

`manifest.json` is the v2 shape. The fields that decide what an install can do:

| Field | Why it matters |
| --- | --- |
| `interface_version` | must equal the runtime's, or the entry is refused |
| `capabilities` | the closed list the plugin may be granted, and a grant outside it is rejected at write time |
| `artifact_kind` | `wasm` or a connector kind, `wasm` when absent |
| `surface_profile` | the entries this plugin contributes, per surface kind |
| `connector_profile.outbound_scopes` | the egress allowlist `http_request` reads, legal on a wasm manifest since ADR 0023 |
| `settings_schema` | optional JSON Schema the Plugins surface renders as a settings form |
| `description`, `name`, `version` | shown on the consent screen |

A surface's `params_schema` is checked before the guest runs: the caller's params must validate, or
the attempt is refused with `plugin_params_refused` and nothing executes. An `automation_action`
must also name its `effect`, and `add_internal_comment` is the only effect the core applies.

## Build

```sh
# 1. Build the module. A plugin is a detached crate: its own Cargo.toml, its own
#    target dir, outside the Marula workspace.
cargo build --manifest-path plugins/wasm/escalation-widget/Cargo.toml \
  --target wasm32-unknown-unknown --release --target-dir plugins/wasm/target
cp plugins/wasm/target/wasm32-unknown-unknown/release/escalation_widget.wasm \
  plugins/wasm/escalation-widget/plugin.wasm

# 2. Validate and bundle. The kit checks the manifest against the same closed
#    vocabularies the host enforces, checks the module against ABI v1, and
#    prints the bundle's sha256 — the value an operator pins.
node packages/plugin-kit/bin/build.mjs plugins/wasm/escalation-widget
```

The build refuses before producing a bundle when the manifest names a capability the core does not
define, when a surface id or label breaks its shape, when a schema leaves the supported subset, or
when an egress scope is not an `https` exact host. A bundle the host would refuse is never produced.

The ABI surface check runs the same code you can run yourself: `pdk/marula-abi.mjs` exports
`inspectModule` and `validateModuleAbi`, so a plugin authored in another language can be checked
from its own tooling.

## Sign, publish, install

```sh
# Sign a catalog entry with the publisher key. The statement binds the artifact
# digest and the canonical manifest digest. See bin/sign.mjs for the exact bytes.
node packages/plugin-kit/bin/sign.mjs catalog.json --key publisher.pem --key-id 2026-q4
```

Publishing a registry entry, staging it, and approving it are admin actions. The
[operator runbook](plugin-operator-runbook.md) walks the install path. A local zip or a GitHub pull
also works, and both land `manifest_authenticated = false` because neither carries a publisher
statement (see [plugin-runtime.md](plugin-runtime.md#manifest-authentication-by-source)).

## Test before you install

Two layers, and the second is the one that catches a signature mismatch rather than a bug.

| Layer | How |
| --- | --- |
| the module's own logic | `cargo test` on the plugin crate, with the host functions stubbed |
| the ABI surface | `node packages/plugin-kit/bin/build.mjs <dir>` — refuses at the first violation |
| the real host | install on a stack, invoke the surface, read the audit row |

The three reference plugins stay deliberately small and on shipped host calls: a widget using
`log` and `now`, an action using `log`, and a validator using `log`. They are the corpus a new
plugin should copy rather than a template to outgrow.

## What not to do

- **Do not ship browser-executed code.** A widget is a declarative spec (a closed render kind and
  JSON-Schema props) rendered by core components. The plugin never runs in the browser.
- **Do not parse the input as trusted JSON without a falling-over story.** The payload is whatever
  the caller sent, and the schema check happens before the guest only for a declared `params_schema`.
- **Do not loop on KV waiting for data.** Sixty-four host calls per invocation is the whole
  allowance, and a long poll is what the wall clock is for.
- **Do not log a secret.** Logs are captured, forwarded and readable by an operator. A credential
  that reaches `log` has left the vault.
- **Do not rely on `emit_event` or `read_model_query` yet.** Both refuse today. Treat the refusal
  as the contract and revisit when the entry ships.
