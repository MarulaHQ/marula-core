# plugin-host

> **Purpose:** the plugin-runtime host: the v1 guest ABI (`abi`, `host_fns` — the `marula` namespace and its nine entries), deny-by-default capability authorization, the manifest/consent surface, the install lifecycle, and the executor that runs a module (`isolation` for the wasmtime sandbox and its limits, `wire` for the parent-to-worker frames, `runtime` for the supervised worker pool, quarantine and the concurrency gates). The worker binary is `src/bin/marula-plugin-worker.rs`.
> **Non-purpose:** No plugin business logic of its own; first-party plugins live in /plugins, connectors in /connectors. No authorization in the worker: every gate runs in the parent before a frame is written, and `authorize_invocation` is the only thing that decides what a plugin may do.
