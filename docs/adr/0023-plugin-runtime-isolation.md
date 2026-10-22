# ADR 0023 — Plugin execution: a wasmtime sandbox, a supervised pool, and six host functions

**Status:** Accepted · 2026-10-22 (Wave 53 lane 1, `p53-runtime-adr`)
**Scope:** how a signed WASM artifact is executed, what the guest may call, what
each call is allowed to do, what a runaway costs, and where plugin state lives.
It does not decide the marketplace, the pack catalogue, or what the PDK looks like
to an author beyond the ABI the host enforces.

## Context

The contract is finished and the executor is missing, in the words of
`docs/reference/plugin-runtime.md`: "The contract is real. The executor is not
yet." Concretely, at this commit:

- `crates/plugin-host/src/lib.rs:62-74` defines `PluginRuntime` with `KIND`,
  `INTERFACE_VERSION`, `id`, `name` and `enabled`. There is no `invoke`.
- `authorize_invocation` (`:101-126`) runs five ordered gates and returns a typed
  reason for each refusal. Nothing behind it executes anything.
- `plugins/wasm/{escalation-widget,triage-note-action,asset-tag-validator}` are
  410-byte core modules exporting `marula_plugin_entry() -> i32`. Their README
  documents an Extism build that has never run.
- No crate in the workspace depends on `wasmtime`, `wasmer` or `extism`.
- The only plugin behaviour that executes is the `add_internal_comment` effect,
  and core hardcodes it: the automation runner skips every other effect name it
  finds on an action entry (`crates/core-api/src/routes/internal.rs:1083-1088`,
  enumerated in `crates/core-domain/src/automations.rs:478-494`).

So this ADR builds the execution layer under an authorization layer that already
works, and it must not become a second authorization layer. The wave it belongs
to forbids eval, plugin-defined schema, undeclared network, filesystem or secret
access, a second auth system, tier knowledge, and an HQ-only hook.

Two forces pull in opposite directions and the rest of this document is the
settlement. Untrusted code wants the strongest available boundary, which points
at a separate process. A PSA's latency budget wants no boundary at all, which
points at a function call. The measurement decides how much boundary the budget
affords, and it turns out to afford all of it.

Numbers below come from one detached benchmark crate: a 4 KiB payload, an FNV-1a
digest over every byte, a 64-byte output, both runtimes on wasmtime 43.0.2, three
interleaved runs, 20,000 samples per call path, nearest-rank percentiles, median
of the runs reported. Microseconds unless stated.

## Decision 1 — Runtime: raw wasmtime, exact-pinned, with the host's own ABI

**Core executes plugin modules directly on wasmtime. It does not adopt Extism.**
`wasmtime` is pinned to an exact patch version, and an engine bump is an
amendment to this ADR rather than a `cargo update`.

The choice was made against numbers, not preference:

| Measurement | raw wasmtime 43 | Extism 1.30 |
|---|---|---|
| call p50 | 5.29 µs | 10.00 µs |
| call p95 | 5.50 µs | 12.46 µs |
| call p99 | 5.88 µs | 17.83 µs |
| instantiate from precompiled module, p50 | 4.38 µs | 43.88 µs |
| compile a module, p50 | 410.8 µs | 947.9 µs |
| clean build, wall | 57.8 s | 76.6 s |
| linked binary | 14.3 MiB | 20.1 MiB |
| crates compiled | 168 | 300 |
| guest artifact for the same plugin | 796 B | 93.3 KiB |

Extism costs about one extra call per call, ten times the instantiation cost, 132
more crates, 5.8 MiB of binary, and 93 KiB in every plugin artifact. Latency
alone does not settle it, because 4.7 µs is 0.005% of the anchor's p95. The ABI
and the dependency surface do.

**Extism's harness is the wrong shape for this security model.** Three items in
its built-in host surface have to be switched off to satisfy the wave:

- `var` is a key-value store Extism keys by plugin instance and nothing else: no
  org, no install, no quota, no export or erase path. Tenant state behind an
  identity the tenancy model cannot address fails deletion provability before it
  fails anything else.
- `http_request` is governed by `allowed_hosts`, a list of URL patterns in the
  manifest with no capability behind it. Consent here renders a capability and a
  connector profile, so a raw pattern list is either shown unexplained or promoted
  into a second consent vocabulary.
- `config` is a manifest-declared map the guest reads directly, which is a second
  path to secrets beside the vault, with no capability and no consent step.

What remains is the input/output convention and the PDK. The convention is about
150 lines against `Store` and `Memory`, and the PDK would have to be forked to
expose typed host functions instead of `i64` handles. Paying for a harness and
then removing its surface is the worst of both.

**The ABI is a core module, not a component.** v1 does not use the component
model or a WIT world. The guest exports `marula_alloc(i32) -> i32`,
`marula_entry(i32, i32) -> i64` (packed pointer and length) and `marula_free(i32,
i32)`, and imports the `marula` namespace listed in Decision 3. A WIT world would
give typed records at the boundary, and it would also tie the plugin ABI to
wasmtime's WIT tooling release by release. Six functions do not justify that
yet. The revisit trigger is a host function that needs a structured, nested
argument, which is the point where pointer arithmetic in the guest stops being
defensible.

**Pinning.** The repository builds on `channel = "stable"`
(`rust-toolchain.toml`), which is rustc 1.95.0 here. wasmtime 48.0.7 declares
MSRV 1.95 and is the newest release that fits. wasmtime 49 needs 1.96, and Extism
1.41 requires wasmtime 49, which is a second, independent reason not to adopt it
today. **The measurements above ran on 43.0.2**, because the comparison needed
one engine under both harnesses. The host lane re-runs the archived benchmark
against the pinned version and records the result. That is a formality against
the decision, which turns on two orders of magnitude, and it is the honest
sequence: the version that ships is the version that is benchmarked.

## Decision 2 — Isolation: a supervised pool on both profiles, and it clears the guardrail

**Plugin invocations run in a long-lived worker process, not inside core-api.**
`MARULA_PLUGIN_ISOLATION` selects `pool` (default) or `in_process`. The worker is
the shipped image in another role, so there is no new container to deploy
(`docs/deploy-descriptor.md`: one image, `role_arg` selects the binary).

| Path, 4 KiB, one caller | p50 | p95 | p99 |
|---|---|---|---|
| in process, one instance | 5.29 | 5.50 | 5.88 |
| pool, one worker | 9.33 | 13.12 | 25.25 |
| **added by the process boundary** | **+4.04** | **+7.62** | **+19.37** |

Under 8 concurrent callers, 8,000 samples each:

| Path | p50 | p95 | p99 |
|---|---|---|---|
| in process, 4 instances | 5.83 | 7.92 | 15.58 |
| pool, 4 workers | 26.46 | 43.71 | 56.92 |
| pool, 1 worker | 10.29 | 14.42 | 29.62 |
| **added by the pool, 4 workers** | **+20.63** | **+35.79** | **+41.34** |

A cold worker start (spawn, compile, first call answered) is 6.22 ms at the
median over 30 starts. One warm worker answers 107,200 invocations per second,
against an anchor of 695 requests per second in total, so the pool is not a
throughput constraint in any configuration this ADR allows.

**The guardrail.** The recorded anchor is 695.3 requests/second, p50 25.9 ms, p95
93.2 ms, p99 294.8 ms (`docs/perf.md:369`). The wave allows the pool only if it
adds at most 5% to p95 or 10% to p99. Assuming one plugin invocation on every
request, which is the reading that charges the p95 rather than hiding behind an
invocation share:

| Budget | Absolute |
|---|---|
| 5% of p95 | 4,660 µs |
| 10% of p99 | 29,480 µs |

**The pool lands inside the guardrail.** Its worst measured increment, 35.79 µs
at p95 with 8 callers on 4 workers, is 0.77% of the p95 budget, which leaves a
factor of 130. The uncontended increment is 0.16% of the budget, a factor of 611.
Read the other way, the pool's whole p95 is 0.047% of the anchor's p95.

What the guardrail buys is fault isolation, and that is the reason to spend it:

- **Crash** — a wasmtime sandbox is memory-safe but not crash-safe. A panic in a
  host function, an allocator failure in the pool, or a JIT defect takes down the
  process it runs in. In a worker, core-api answers the request with a typed
  refusal (`plugin_worker_failed`) and restarts the member.
- **Memory** — a worker's resident size is its own. An instance that exceeds its
  cap is killed with the worker, not with the API. In process, the only defence
  is `StoreLimits` inside an address space shared with everything else.
- **CPU** — the worker is a scheduler entity. A plugin that burns its fuel cannot
  starve the request loop no matter what the host does wrong.

**The worker performs no authorization.** Every gate in `authorize_invocation`
runs in core-api before a frame is written, and the frame carries the resolved
`(org_id, install_id, entry, payload)`. The worker's job is to enforce limits and
to run only modules it has verified against a digest the parent named, so a bug
in the worker cannot grant a capability the parent refused. It can run a module it
should not have; it cannot decide that a call is allowed.

**Per profile:**

| Profile | Worker count | Where the worker runs | Why |
|---|---|---|---|
| `single_box` (all-in-one) | 1 | child of core-api, same container | one box, one container, no second moving part to operate |
| `durable` (split) | 2 | own process, own role, restart supervised | the profile already runs separate processes and expects to scale them |

Two workers is a deliberate small number. On this box, 4 workers under 8 callers
were slower than 1 worker under 8 callers (26.46 µs against 10.29 µs at p50),
because the extra processes cost more in scheduling than the queue costs in
waiting. Sizing the pool by core count is the reflex this measurement rejects.
The pool is there on the fast path only if you never oversubscribe it, and one
worker already covers the anchor's entire request rate as plugin calls.

**`in_process` remains available and is not a tier.** It exists for one purpose:
a box that cannot spare the 13.5 MiB resident cost of a worker, and the guardrail
fallback this wave asked about. It is one knob, it is documented, it does not
change what a plugin may do, and it selects the same authorization path, the same
host functions and the same limits.

## Decision 3 — Host functions v1: six functions in nine entries, each with a gate and a typed refusal

The guest sees one import namespace, `marula`. Nothing else is importable. There
is no WASI: a plugin that needs a clock calls `now()`, and a plugin that needs a
socket calls `http_request`, because a WASI instance would hand out `fd_*`,
`path_*`, `sock_*` and `environ_*` for free and the wave forbids exactly those.

| Host function | Guest signature | Gate: capability → permission | Typed refusal |
|---|---|---|---|
| `log` | `(level: u32, ptr: i32, len: i32)` | none beyond an enabled install, bounded (4 KiB per line, 100 lines per invocation) | `plugin_log_budget_exceeded` |
| `kv_get` | `(ns_ptr, ns_len, key_ptr, key_len) -> i64` | none: the namespace is derived from the invocation's `(org_id, install_id)`, never supplied by the guest | `plugin_kv_namespace_denied` |
| `kv_put` | `(ns, key, value_ptr, value_len) -> u32` | same | `plugin_kv_namespace_denied`, `plugin_kv_quota_exceeded`, `plugin_kv_value_too_large` |
| `kv_delete` | `(ns, key) -> u32` | same | `plugin_kv_namespace_denied` |
| `kv_list` | `(ns, prefix, cursor) -> i64` | same, capped at 500 keys per page | `plugin_kv_namespace_denied` |
| `http_request` | `(profile_ptr, profile_len, req_ptr, req_len) -> i64` | `automation.reads` for a read, `automation.actions` for a write, plus the install's consented egress profile | `plugin_egress_profile_missing`, `plugin_egress_host_denied`, `plugin_egress_scheme_denied`, `plugin_egress_response_too_large` |
| `emit_event` | `(type_ptr, type_len, payload_ptr, payload_len) -> u32` | `events.emit`, and the type must be in the manifest's declared `emits` and the `event_types` registry | `plugin_event_type_not_declared` |
| `read_model_query` | `(query_id: u32, params_ptr, params_len) -> i64` | per query id: `ticket.by_id` and `board.list` need `tickets.read`, `client.by_id` needs `clients.read`, `contact.by_client` needs `clients.read` | `plugin_query_unknown`, `plugin_capability_not_granted` |
| `now` | `() -> i64` | none | — |

Nine ABI entries, six functions once the four `kv_*` verbs are read as one store,
and five capability gates across them: `automation.reads`, `automation.actions`,
`events.emit`, `tickets.read` and `clients.read`. `log`, the KV store and `now`
carry no capability because the install's own scope is the grant. Every refusal
above is a static string, in the manner of
`crates/plugin-host/src/lib.rs:50-56`, so an audit row and an API envelope carry a
reason code and never prose to parse.

Each gate is a second use of the authority that
`authorize_invocation` already resolved. No host function invents authority: a
capability resolves onto a `Permission` in the existing matrix, and the host call
runs the same `policy::check` a human actor's action would run. `log`, `kv_*` and
`now` carry no permission because the namespace and the clock are the install's
own scope, and a store scoped to `(org_id, install_id)` cannot reach another
tenant by construction.

**What is deliberately absent, and the route back:**

- **`secret_get` is not in v1.** ADR 0006 fixed the plugin vault boundary at
  ciphertext ingress: the core never takes a plaintext secret from a plugin
  surface. Handing plaintext *out* to plugin code is a different decision with a
  worse composition, because an install holding both `secret_get` and
  `http_request` can read an org's vault and post it anywhere in one call, and its
  consent screen would show two ordinary-looking grants. Adding it wants its own
  ADR, a per-folder scope, and a consent screen that names the composition and
  refuses the pair.
- **`ai_complete` is not in v1.** No `ai` capability exists, and the AI surface
  has its own deny-by-default grant machinery (`ai_surface_grants`,
  `authorize_surface_call`). A second path into the model would be the "no second
  auth system" non-goal, so the function waits for an `ai.complete` capability
  that routes through that machinery and the consent copy that presents it.
- **`random_bytes` is absent and should not stay absent.** A guest has no entropy
  source, so a plugin cannot mint an id or a nonce today. It is named here so the
  gap is recorded rather than discovered by the first plugin that needs it. It is
  a two-line host function with no capability, and it belongs in the first
  amendment.
- **No filesystem, no socket, no environment, no threads, no WASI.**
- **No `eval`.** The runtime loads bytes from an artifact whose signature the
  distribution layer already verified. It never accepts a module body over an API.

**One manifest change this decision forces.** Egress has to be consented per
install, and the current manifest cannot express it for a wasm artifact:
`packages/plugin-kit/bin/build.mjs:102` refuses a `connector_profile` on a surface
plugin, and `PluginManifestV2::validate` refuses a `surface_profile` on an
`http_connector`. The rule exists to stop a connector from faking a UI surface,
and it caught the egress declaration with it. The narrow fix: keep
`webhook_endpoints` connector-only and allow `outbound_scopes` on any artifact
kind, in both the validator and the kit, which is where the egress allowlist
`http_request` reads comes from.

## Decision 4 — Limits, as numbers

| Limit | Value | Basis |
|---|---|---|
| Fuel per invocation | 200,000,000 | the reference plugin's 4 KiB digest costs 38,909 fuel, deterministically across 5,000 runs, so the cap is 5,139 times a real workload and about 200 ms at a conservative 1 G fuel/s |
| Wall clock per invocation | 250 ms, enforced by an epoch tick at 50 ms | the fuel cap's own ceiling, with a hard stop that does not depend on the guest reaching it |
| Worker restart threshold | 500 ms for one invocation | a guest that ignores the epoch deadline twice loses its worker |
| Linear memory per instance | 64 MiB | `StoreLimits`, below the worker's own budget so the store fails first |
| Worker resident ceiling | 256 MiB | a worker above it is restarted, not allowed to grow into the box |
| Input per invocation | 1 MiB | invocation params today are a JSON object measured in kilobytes, so the cap is headroom rather than a target |
| Output per invocation | 1 MiB | symmetric with input, and the pool frame carries it in one write |
| HTTP connect and read timeout | 5,000 ms total | below every consumer retry cap in `docs/reference/reliability-levers.md` (10 s, or 30 s on `durable`), so the plugin fails before the queue does |
| HTTP response cap | 2 MiB | the connector path's existing output ceiling shape |
| HTTP redirects | 0 | a redirect is an unconsented host |
| Concurrent invocations per worker | 1 | a worker is single-threaded, so pool capacity equals worker count, which is the number Decision 2 sized |
| Concurrent invocations per org | 1 | one org holds one slot, so a second org always has a worker |
| Concurrent invocations per install | 1 | one invocation per install keeps its own read-modify-write on KV from interleaving |
| Invocations per install | 300 per minute, burst 60 | 5 per second, 0.7% of the anchor's whole request rate, which keeps one runaway loop from filling the queue |
| KV value size | 1 MiB | matches the invocation output cap |
| KV keys per install | 100,000 | at the 1 MiB value cap that is 100 GiB, so the byte quota is the binding one |
| KV bytes per install | 256 MiB | four times the plugin's own 64 MiB memory cap, and small enough that an export is a stream rather than a backup job |

Fuel is the interesting one, because it is the only limit that is exact. The same
input costs the same fuel every time, so the cap is a quota rather than a proxy,
and metering costs nothing measurable: 5.29 µs at the median with metering and
5.29 µs without.

## Decision 5 — State: one namespaced KV table, quota'd, exportable, erasable

Plugin state lands in the tenant database, in one table with fixed columns. No
plugin defines a table, a column or an index.

```
plugin_kv_entries
  org_id      uuid  NOT NULL          -- RLS, the existing policy shape
  install_id  uuid  NOT NULL          -- ON DELETE CASCADE from plugin_installs
  namespace   text  NOT NULL          -- guest-chosen label, [a-z0-9_.-]{1,64}
  key         text  NOT NULL          -- <= 512 bytes
  value       bytea NOT NULL          -- <= 1 MiB
  expires_at  timestamptz NULL        -- TTL, swept by the existing retention job
  created_at  timestamptz NOT NULL
  updated_at  timestamptz NOT NULL
  PRIMARY KEY (install_id, namespace, key)
```

- **The namespace is the install's.** `install_id` is unique per org, so the
  primary key already isolates tenants, and `org_id` is carried for the RLS policy
  and for the quota walk. The guest never names an org or an install: the host
  fills both from the invocation context. A plugin cannot address another
  install's rows even if its own authorization is broken.
- **Values are bytes.** The core stores and returns them and never parses them.
  Serialization is the plugin's business, and an unparseable value comes back as
  an unparseable value.
- **The quota is a counter row, not a `SUM`.** `plugin_kv_usage (org_id,
  install_id, bytes, keys, updated_at)` is updated in the same transaction as the
  write, because summing `octet_length(value)` across 100,000 rows on every put is
  a scan on the hot path. The counter is reconcilable: a periodic check recomputes
  it and logs a mismatch, so a lost update is a repair rather than a silent
  overrun.
- **Export** is a streamed read of `(namespace, key, value)` for one install,
  encoded outside the database, with a receipt. **Erase** is `DELETE` by
  `install_id`, a counter reset, and a receipt in the same shape the RAG namespace
  already produces. Uninstall cascades both.
- **No cross-install namespace and no org-level bucket in v1.** If two plugins
  need to share state, that is a mediated call between them, not a shared table,
  because a shared bucket has no owner and so no erasure story.

## Decision 6 — Entitlement: core enforces the manifest, HQ enforces the pack

- **Core enforces** what an artifact may do: signature and digest, interface
  version, manifest validity, the closed capability vocabulary, the grant as a
  subset of the declared set, the admin-enabled state, every host-function gate,
  every limit, and every quota. None of that is negotiable by the caller.
- **HQ enforces** what a customer may have: which packs their contract includes,
  the marketplace listing, the price, and the licence term. HQ does so by
  declining to create an install, or by declining to publish an entry into a
  catalogue the org can see.
- **The core carries no tier knowledge.** There is no plan column, no `if
  premium`, and no entitlement check in any crate
  (`docs/reference/core-and-packs.md:13-14`). The runtime branches on a
  deployment profile, which is a shape, not a tier.
- **A direct ZIP import has no entitlement beyond the org's own grants.** An
  operator who installs a signed artifact from their own registry gets exactly the
  capabilities they consented to. The absence of a pack check is not a hole: the
  manifest grants are the whole authority, and HQ was never the thing that stopped
  a plugin from reading a ticket.
- **Packs cannot add a host function.** The list in Decision 3 is core. A pack
  that needs another function needs an amendment here, which is the point of
  writing the list down.

## Consequences

- `crates/plugin-host` grows the executor and keeps its authorization. The
  `PluginRuntime` trait gains `invoke`, and `enabled()` stays the outermost gate.
- The shipped image gains a `plugin-worker` role. `all-in-one` runs it as a child
  of core-api, `split` runs it as its own process.
- Two migrations follow: `plugin_kv_entries` plus `plugin_kv_usage`, and the TTL
  sweep registration. Both are additive and both carry the existing RLS shape.
- The manifest validator and `packages/plugin-kit` relax one rule together
  (`outbound_scopes` on any artifact kind), in the same change, because they are
  mirrors of one vocabulary and a split lands an accept that the other refuses.
- `plugins/wasm/README.md` becomes true, and its build story moves from Extism to
  the core-module ABI in Decision 1.
- `docs/reference/plugin-runtime.md` stops describing a gap.
- A plugin author writes against a nine-entry import namespace and no WASI.
  That is a smaller surface than Extism's and a larger one than a no-op.

## What this does not decide, and what would reopen it

- **The engine version's own numbers.** The measurements ran on wasmtime 43.0.2
  and the pin lands on 48.0.7. Re-run the archived benchmark after the bump. If
  p95 per call triples, the guardrail still passes, so this reopens the pin
  choice only if a version stops building or a CVE forces a move.
- **Whether the pool should carry more than one plugin per worker.** v1 loads
  artifacts by digest into a worker on demand and keeps them, so a busy worker
  holds several compiled modules. Memory for that is not measured.
- **The PDK's developer experience.** This ADR fixes the ABI. Whether the kit
  generates bindings, and in which language, is open.
- **`random_bytes`, `secret_get`, `ai_complete`.** Each is named above with the
  route back.
- **A second isolation tier for high-risk plugins.** If a future pack needs to
  run beside data it should not see, the answer is a per-install worker, not a new
  profile. Nothing measures that cost yet.
