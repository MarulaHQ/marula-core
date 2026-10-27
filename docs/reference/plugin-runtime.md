# Plugin runtime

**Decision of record:** [ADR 0023](../adr/0023-plugin-runtime-isolation.md)
**Code:** `crates/plugin-host/` (sandbox, pool, ABI, executor), `packages/plugin-kit/` (PDK),
`plugins/wasm/` (three reference plugins)

A plugin is a signed WebAssembly module with no WASI, three exports, and one import namespace.
It runs in a supervised worker process, under limits fixed in the ABI, and every invocation
leaves one audit row. This page is the map: what runs today, what refuses, and which page
answers the next question.

## Read it in this order

| If you want to | Read |
| --- | --- |
| build a plugin | [plugin-pdk.md](plugin-pdk.md) |
| know exactly what a guest may call, and what each refusal means | [plugin-host-functions.md](plugin-host-functions.md) |
| know which grant an install needs for a surface | [plugin-capabilities.md](plugin-capabilities.md) |
| understand the threat model and the limits | [plugin-security-model.md](plugin-security-model.md) |
| run plugins in a deployment | [plugin-operator-runbook.md](plugin-operator-runbook.md) |
| understand what an agent may do with a plugin's surfaces | [plugin-agent-guide.md](plugin-agent-guide.md) |

## What a plugin may be

Six surface kinds, each registering under exactly one capability (`crates/plugin-host/src/surface.rs:269`).

| Surface | Registers as | Capability |
| --- | --- | --- |
| Ticket-pane and dashboard widget | `widget` | `ui.widgets` |
| Automation action kind | `automation_action` | `automation.actions` |
| Report render kind | `report_render` | `reports.render` |
| Custom-field validator | `field_validator` | `fields.validate` |
| Automation trigger source | `automation_trigger` | `automation.triggers` |
| Read node returning external data | `read` | `automation.reads` |

Only `automation_action` applies a domain effect, and the effect is `add_internal_comment`. The
guest decides whether the call is accepted. It does not choose the effect
(`crates/plugin-host/src/exec.rs:1090`).

## What runs today

The execution path is live. Two call sites reach it, both in the request path:

- the `plugin.action` workflow node (`crates/core-api/src/routes/internal.rs:3169`);
- the automation runner's plugin action arm (`crates/core-api/src/routes/internal.rs:1259`).

Both call `exec::invoke_surface_with`, which resolves the registry row, the install, the pinned
manifest and the pinned module bytes, authorises the capability, runs the guest, applies the
consented effect in the caller's transaction, and writes one audit row whatever happened.

| Piece | State |
| --- | --- |
| manifest v2, capability vocabulary, installs and grants, consent diffing | shipped before the runtime |
| signed distribution, digest pinning, staging and approval | shipped (W53-4) |
| sandbox: fuel, wall clock, memory, output, log budgets | shipped (W53-3) |
| supervised worker pool, crash containment, quarantine | shipped (W53-3) |
| six surface kinds registered from a manifest | shipped (W53-5) |
| KV namespace per org and install, with quota, export and erase | shipped (W53-6) |
| `http_request` through a consented egress profile, credentials bound per install | shipped (W53-7) |
| `log`, `now`, `kv_get`, `kv_put`, `kv_delete`, `kv_list`, `http_request` | shipped |
| `emit_event` | refuses `plugin_not_implemented` after its gate passes |
| `read_model_query` | refuses `plugin_not_implemented` after its gate passes |
| an admin route to clear a quarantine | not built, see the operator runbook |
| conformance and security battery | `crates/plugin-host/tests/` (W53-8) |

The two deferred entries are honest stubs, not gaps discovered later: each carries
`ImplStatus::Deferred { owner: "W53-5" }` in the host-functions registry, refuses with a typed
code, and is pinned by the conformance corpus so a page cannot claim it works.

## The one-paragraph contract

The parent authorises; the worker executes. `authorize_invocation` resolves the capability
against the install's own grant set and the existing permission matrix, and the frame the worker
receives carries the grants already resolved. The worker performs no authorization of its own: it
re-validates the frame, refuses one that does not validate whole, and can run only the module whose
sha256 the parent named. A bug in the worker can run a module it should not have. It cannot decide
that a host call is allowed. Each host function's gate is documented in
[plugin-host-functions.md](plugin-host-functions.md), and the limits that bound a hostile guest are
in [plugin-security-model.md](plugin-security-model.md).

## Manifest authentication by source

`manifest_authenticated` records whether a publisher signature covers the manifest an approver
reads. It is not the artifact check: the digest pin is a separate fact, and every source records
one. The consent screen renders this column as **manifest signed** or **manifest unsigned**.

| Source | `manifest_authenticated` | Why |
| --- | --- | --- |
| Marketplace registry, v2 statement, trusted key configured | `true` | The signature names the artifact digest and the manifest digest, and the statement is verified against both before the row is staged. |
| Marketplace registry, legacy v1 statement | `false` | The signature covers the artifact digest alone. The entry is still accepted, because refusing it would strand plugins already published, and the staged row carries the weaker claim. |
| GitHub repository pull | `false` | A repository carries no publisher signature. The pinned tag and the artifact digest say which bytes were fetched and that they have not changed since; they say nothing about who published them. |
| Local zip or manual upload | `false` | There is no publisher claim to verify. |

A registry document ingested with no trusted signing key configured is structural-only and lands
`false` whatever it claims.

The GitHub row is a decision, not a gap. There is no publisher statement in a repository for the
pull path to verify, so `false` is the intended end state for that source, and the pull path does
not grow a signature it cannot check. Verifying the bytes it fetched is what the tag and digest are
for. Claiming a publisher is not a thing that path can honestly do.

## Where a pack stops and the core starts

The core ships the seam and the security model. What a pack builds on top of it belongs to whoever
builds the pack, and no entitlement check exists in this repository. The extension points a pack may
rely on, point by point, are in [extension-points.md](extension-points.md).
