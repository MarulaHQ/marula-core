# Plugin host functions

**Decision of record:** [ADR 0023 § Decision 3](../adr/0023-plugin-runtime-isolation.md)
**Catalogue as data:** `packages/plugin-kit/abi/host-abi.v1.json`
**Source of truth:** `crates/plugin-host/src/host_fns/`

Nine entries over six host functions. Every one is declared in the registry
(`crates/plugin-host/src/host_fns/mod.rs:331`), published as data for the PDK and the TypeScript
kit, and compared against that data by a test that fails when they disagree
(`crates/plugin-host/tests/abi_catalogue.rs`).

Nothing else is importable. A module that imports `wasi_snapshot_preview1`, an unknown `marula`
entry, or an entry with the wrong signature is refused at load, before it can run.

## The entries

| Entry | Guest signature | Returns | Gate | Shipped |
| --- | --- | --- | --- | --- |
| `log` | `(level: u32, ptr: i32, len: i32)` | none | install scope | yes |
| `kv_get` | `(ns_ptr: i32, ns_len: i32, key_ptr: i32, key_len: i32)` | `i64` data frame | install scope | yes |
| `kv_put` | `(ns, key, value_ptr, value_len, expires_ptr, expires_len)` | `u32` status | install scope | yes |
| `kv_delete` | `(ns, key)` | `u32` status | install scope | yes |
| `kv_list` | `(ns, prefix, cursor_ptr, cursor_len)` | `i64` data frame | install scope | yes |
| `http_request` | `(profile_ptr, profile_len, request_ptr, request_len)` | `i64` data frame | egress | yes |
| `emit_event` | `(type_ptr, type_len, payload_ptr, payload_len)` | `u32` status | event | **refuses** |
| `read_model_query` | `(query_id: u32, params_ptr, params_len)` | `i64` data frame | query | **refuses** |
| `now` | `()` | `i64` | install scope | yes |

"Install scope" means the install's own record is the grant: the call addresses the invocation's
own `(org_id, install_id)` and can reach nothing else. The three gated entries resolve a
capability first, and the capability must be in the install's granted set.

## How a refusal reaches the guest

The ABI has four return shapes, and a refusal takes a different route through each. This is the
part that surprises a plugin author, so it is a table rather than prose.

| Return shape | On refusal | What the guest sees |
| --- | --- | --- |
| `none` (`log`) | the line is dropped | nothing. There is no in-band channel, so the invocation continues and the dropped line is not captured. |
| `u32` status (`kv_put`, `kv_delete`, `emit_event`) | the refusal code is the return value | `Err(Refusal)` in the PDK, matching on the code |
| `i64` data (`kv_get`, `kv_list`, `http_request`, `read_model_query`) | a tagged error frame is written into guest memory and its `(ptr, len)` is returned | `Err(Refusal)` in the PDK, which decodes the frame |
| `i64` value (`now`) | `now` carries no gate and never refuses | the clock, or `0` if the host clock is unreadable |

A guest that ignores the status word keeps running until a limit stops it. The host does not trap
on a refusal, because a refusal is an answer.

## The hosted-function limits

| Limit | Value | Constant |
| --- | --- | --- |
| `log` line | 4 KiB | `MAX_LOG_LINE_BYTES` |
| `log` lines per invocation | 100 | `MAX_LOG_LINES` |
| KV namespace label | `[a-z0-9_.-]{1,64}` | `KV_NAMESPACE_MAX_BYTES` |
| KV key | 512 bytes | `KV_KEY_MAX_BYTES` |
| KV value | 1 MiB | `KV_VALUE_MAX_BYTES` |
| KV page | 500 keys | `KV_LIST_PAGE` |
| KV quota per install | 100,000 keys and 256 MiB | `KV_KEYS_PER_INSTALL`, `KV_BYTES_PER_INSTALL` |
| `http_request` connect and read | 5 s | `HTTP_TIMEOUT_MS` |
| `http_request` response | 2 MiB | `HTTP_RESPONSE_MAX_BYTES` |
| `http_request` redirects | 0 | `HTTP_MAX_REDIRECTS` |
| Host calls per invocation | 64 | `MAX_HOST_CALLS_PER_INVOCATION` |

The host-call cap is the one that matters for a guest that loops on KV. Once an invocation has
spent its 64 calls, every later call answers `plugin_fuel_exhausted` with no store work behind it.
The guest's own fuel and wall clock still bound it between calls, so the cap cannot extend an
invocation.

## KV is per install, not per plugin

The namespace a guest names is a label inside the install's own slot. The store derives the tenant
from the invocation, `(org_id, install_id)`, so two installs of the same plugin cannot read each
other's keys, and a guest cannot name another org. An illegal label is
`plugin_kv_namespace_denied`; a key the store would not accept is `plugin_abi_frame_invalid`,
because the ABI has no key-specific refusal and inventing one would make the audit trail say the
wrong thing.

Quota is enforced inside the write transaction, so a refused write did not happen. Export and
erase are operator actions, documented in the data-lifecycle reference.

## `http_request` sends nothing the manifest did not declare

The gate has three questions, asked in this order:

1. **The capability.** A `GET`, `HEAD` or `OPTIONS` needs `automation.reads`; anything else needs
   `automation.actions`. Absent grant, `plugin_capability_not_granted` before the profile is read.
2. **The profile.** The name must appear in the install's consented egress scopes. Absent,
   `plugin_egress_profile_missing`.
3. **The URL.** Scheme must be `https` and the host must match the profile's allowlist, matched by
   the parent again before the request goes out. Wrong scheme is `plugin_egress_scheme_denied`;
   a host outside the list is `plugin_egress_host_denied`; a response over 2 MiB is
   `plugin_egress_response_too_large`; a redirect is never followed.

The worker holds no HTTP client. The request crosses a pipe to the parent, which performs it
(`crates/core-api/src/plugin_http.rs`), so `nm` on the worker binary finds no `reqwest`, `rustls`
or `hyper`. A credential a profile needs is resolved parent-side and never reaches the guest.

## The two deferred entries refuse today

Both entries pass their full gate and then answer `plugin_not_implemented` (code 13). The reasons
differ and both are recorded in the registry:

- `emit_event` needs the event-type registry and the core-events producer, which are request-path
  resources.
- `read_model_query` needs core-db and the caller's transaction.

The gate still runs first: an ungranted `emit_event` is `plugin_capability_not_granted` (1), not
the stub's answer, and an undeclared event type is `plugin_event_type_not_declared` (10). That
order is deliberate, so a stub cannot hide a missing grant. A plugin that calls either function
today gets a typed refusal and should treat it as "not available in this build".

The four query ids the gate recognises, and the capability each needs, are in
[plugin-capabilities.md](plugin-capabilities.md#the-four-model-queries).

## The refusal vocabulary

Twenty-five values, stable strings and append-only numeric codes. The string is what the audit row
and the API envelope carry; the code is what a `u32`-returning entry hands the guest.

| Code | String | Raised by |
| --- | --- | --- |
| 1 | `plugin_capability_not_granted` | a gate whose capability is not in the install's grants |
| 2 | `plugin_log_budget_exceeded` | more than 100 lines, or a line over 4 KiB |
| 3 | `plugin_kv_namespace_denied` | a namespace label outside `[a-z0-9_.-]{1,64}` |
| 4 | `plugin_kv_quota_exceeded` | the write would cross the install's byte or key quota |
| 5 | `plugin_kv_value_too_large` | a value over 1 MiB |
| 6 | `plugin_egress_profile_missing` | the profile is not in the consented scopes |
| 7 | `plugin_egress_host_denied` | the host, or the address it resolved to, is outside the allowlist |
| 8 | `plugin_egress_scheme_denied` | not `https` |
| 9 | `plugin_egress_response_too_large` | over 2 MiB |
| 10 | `plugin_event_type_not_declared` | the manifest does not declare the type |
| 11 | `plugin_query_unknown` | an id outside the closed query table |
| 12 | `plugin_worker_failed` | the worker died or answered a frame the parent could not trust |
| 13 | `plugin_not_implemented` | a deferred entry, after its gate passed |
| 14 | `plugin_unknown_host_fn` | an entry not in the `marula` namespace |
| 15 | `plugin_abi_frame_invalid` | a frame, an argument word or a memory range did not parse |
| 16 | `plugin_input_too_large` | the invocation payload is over 1 MiB |
| 17 | `plugin_output_too_large` | the guest claimed a reply over 1 MiB |
| 18 | `plugin_fuel_exhausted` | the fuel budget, or the host-call allowance, ran out |
| 19 | `plugin_wall_clock_exceeded` | the 250 ms deadline fired before fuel ran out |
| 20 | `plugin_guest_trap` | a panic, `unreachable`, an out-of-bounds access, a stack overflow, or growth past the memory cap |
| 21 | `plugin_install_quarantined` | three consecutive worker failures and no operator clear |
| 22 | `plugin_rate_limited` | over 300 invocations a minute for one install |
| 23 | `plugin_concurrency_limited` | no org, install or worker slot inside the wait budget |
| 24 | `plugin_runtime_disabled` | plugin execution is admin-disabled on the deployment |
| 25 | `plugin_credential_refused` | a profile needs a credential the parent could not present |

Codes 18 to 25 are decided by the sandbox, the pool or the executor around the guest, so a guest
sees them only through the audit row — except 18, which also arrives in band when the host-call
allowance is spent.

`plugin_credential_refused` deliberately merges three causes into one code: no binding, a withdrawn
category grant, or a record with no material field. One code means a guest cannot probe the shape
of an org's vault. The cause is in the log line, not the frame.

## What a guest may not call

| Attempt | Result |
| --- | --- |
| `wasi_snapshot_preview1::*` | module refused at load |
| `env::*` or any namespace but `marula` | module refused at load |
| `marula::secret_get` or any entry the ABI does not declare | module refused at load |
| a declared entry with the wrong signature | module refused at load |
| a declared entry whose gate the install does not satisfy | typed refusal, invocation continues |
