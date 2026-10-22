# plugin-host

> **Purpose:** the plugin-runtime host: the v1 guest ABI (`abi`, `host_fns` — the `marula` namespace and its nine entries), deny-by-default capability authorization, the manifest/consent surface, and the install lifecycle. The wasmtime executor that drives the ABI is W53-3.
> **Non-purpose:** No plugin business logic of its own; first-party plugins live in /plugins, connectors in /connectors.
