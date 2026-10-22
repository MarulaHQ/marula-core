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
