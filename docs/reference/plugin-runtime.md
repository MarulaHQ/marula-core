# Plugin runtime

The plugin contract is real. The executor is not yet.

Marula Core ships the declarative half of the plugin system: manifest v2,
capabilities, installs and per-install grants, signed distribution with digest
pinning, surface profiles, and workflow node contracts. The host in
`crates/plugin-host` authorises an invocation and derives the surfaces an install
may contribute. Three reference plugins live in `plugins/wasm/`, and the
`plugin.action` workflow path applies a small trusted effect set.

What the core does **not** yet do is execute plugin code. That runtime is scoped
as **Wave 53** in [`TASKLIST.md`](../../TASKLIST.md). It covers the sandboxed
WASM host, the host-function ABI, resource limits, connector egress and the
lifecycle around them.

Until then:

- a plugin manifest can be published, installed, granted and surfaced
- a plugin action runs only through the trusted effects the core implements
- no plugin can execute arbitrary code, open a socket, read a file or mint a
  session

The security model that the runtime must preserve is already written down in
`crates/plugin-host/src/lib.rs`: deny-by-default, admin-enabled, capabilities
mapped onto the existing permission matrix, and every lifecycle step audited. The
runtime wave adds execution around that contract, not a second contract.

For the boundary between the core plugin host and the premium pack catalogue, see
[`core-vs-premium.md`](core-vs-premium.md).
