# plugins

Every first-party extension lives here, one subfolder per kind. The word we use
in code, the API, and the plugin lifecycle is **plugin** (ADR 0009 defines a
plugin as a WASM executable or an HTTP connector — both install through the same
signed, consent-gated lifecycle). "Extension" is the plain-language word in
docs; it never appears in code.

```
plugins/
├── connectors/   # HTTP connectors: web apps over the public API (the standard set)
└── wasm/         # first-party Extism/WASM plugins, built on packages/plugin-kit
```

## Kinds

| Kind | Folder | What it is | Status |
|---|---|---|---|
| **Connector** | `connectors/` | An HTTP adapter that talks to Marula only through the typed API, signed webhooks, and the feed-upsert seam. The standard set (ADR 0013) ships Apache-2.0 here. | Active |
| **WASM plugin** | `wasm/` | A sandboxed Extism plugin built on `packages/plugin-kit`, installed through the plugin-host lifecycle. | Scaffold |
| **Agent skill** | — | Assistant data (markdown + resources), distributed as checksummed skill packs. Not executable; no folder here yet. | Future |
| **Theme pack** | — | A token/asset bundle distributed through the marketplace. Lives with the design system, not here. | Future |

## What does not live here

Marketplace connectors beyond the standard set. They are published through the
connector marketplace (ADR 0013) and developed in the commercial repos — the
first-party ones are the per-connector repositories in
`relay/connectors/` (IT Glue, Auvik, ImmyBot, Zapier). Nothing premium ever
lands in this folder, and no code here carries an entitlement check; premium
packs verify their own license inside the pack.

The shared support crates for connector authors live in `connectors/`:
`adapters-core` (transport, auth, pagination, org mapping) and the feeder/DMS
engines (`ci-feeders`, `dms-sync`) whose public mappers community connectors
build on.
