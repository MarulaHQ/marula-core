# Plugin runtime

The executor ships. The route that calls it does not yet.

Marula Core carries both halves of the plugin system. The declarative half is
manifest v2, the capability vocabulary, installs and per-install grants, signed
distribution with digest pinning, surface profiles, and workflow node contracts.
The execution half is the sandbox in `crates/plugin-host/src/isolation.rs`, the
worker pool in `runtime.rs`, and the guest ABI in `abi.rs` that ADR 0023 fixes.
Three reference plugins live in `plugins/wasm/` and run on that ABI.

What is still missing is the request path. No HTTP route invokes the pool, so a
plugin action runs only through the trusted effects the core implements, and no
audit row is written for an invocation. That belongs to the rest of Wave 53:
`plugin.action` in core-api, the KV store, connector egress, and the admin
surface for quarantine.

What works today:

- a plugin manifest can be published, installed, granted and surfaced
- a signed module runs in a supervised worker under the limits ADR 0023 sets out,
  which `cargo test -p plugin-host` exercises against a real worker process
- `log` and `now` answer a guest; the other seven ABI entries resolve their gate
  and then refuse with `plugin_not_implemented`
- a guest that loops, floods its log, returns an oversized payload or grows past
  its memory cap is refused with a typed reason instead of taking the API down

The security model is unchanged from `crates/plugin-host/src/lib.rs`:
deny-by-default, admin-enabled, capabilities mapped onto the existing permission
matrix, and every lifecycle step audited. The runtime adds execution around that
contract rather than a second contract. The worker performs no authorization of
its own, and a capability the parent refused cannot be granted inside it.

The boundary between the core plugin host and the commercial pack catalogue is
deliberately not described here: the core ships the seam and the security model,
and what a pack builds on top of it belongs to whoever builds the pack. The
extension points a pack may rely on, point by point, are in
[extension-points.md](extension-points.md).

## Manifest authentication by source

`manifest_authenticated` records whether a publisher signature covers the
manifest an approver reads. It is not the artifact check: the digest pin is a
separate fact, and every source records one. The consent screen renders this
column as **manifest signed** or **manifest unsigned**.

| Source | `manifest_authenticated` | Why |
|---|---|---|
| Marketplace registry, v2 statement, trusted key configured | `true` | The signature names the artifact digest and the manifest digest, and the statement is verified against both before the row is staged. |
| Marketplace registry, legacy v1 statement | `false` | The signature covers the artifact digest alone. The entry is still accepted, because refusing it would strand plugins already published, and the staged row carries the weaker claim. |
| GitHub repository pull | `false` | A repository carries no publisher signature. The pinned tag and the artifact digest say which bytes were fetched and that they have not changed since; they say nothing about who published them. |
| Local zip or manual upload | `false` | There is no publisher claim to verify. |

A registry document ingested with no trusted signing key configured is
structural-only and lands `false` whatever it claims.

The GitHub row is a decision, not a gap. There is no publisher statement in a
repository for the pull path to verify, so `false` is the intended end state for
that source, and the pull path does not grow a signature it cannot check.
Verifying the bytes it fetched is what the tag and digest are for; claiming a
publisher is not a thing that path can honestly do.
