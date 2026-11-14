# The connector specification

> **Purpose:** the bar a connector meets, and how the repository proves it
> meets it. This is the document a reviewer checks a connector against, and
> the one a marketplace author reads before writing a manifest. Where it
> disagrees with an older planning document, this wins, because this one is
> paired with the code.
> **Companions:** [connector-capabilities.md](connector-capabilities.md) (what
> a connector can reach), [connector-surface.md](connector-surface.md) (what
> the staff UI draws), [extension-points.md](extension-points.md) (what a
> connector may build on), ADR [0032](../adr/0032-connector-connection-model.md)
> and [0033](../adr/0033-record-links-and-external-references.md).
> **Status:** adopted 2026-11-14.

## 1. What a connector is

An HTTP adapter that talks to a vendor's API and to Marula's public API, and
nothing else. It is installed as an artifact with a typed manifest, consented
capability scopes, and a settings schema. It contributes data to the staff UI
through declarations, never layout.

A connector is one artifact. There is no wrapper, no companion plugin, and no
per-connector build target in `apps/web`. An HTTP connector and a WASM plugin
differ in **where their code runs**, not in what surfaces they may declare, so
the same manifest reaches the same UI.

Five behaviours exist and a connector chooses among them: `open`, `launch`,
`dial`, `share`, `sync`. A new behaviour is an RFC to Core. A new product kind
is a manifest string. The boundary is in
[connector-capabilities.md](connector-capabilities.md) §1.

## 2. The standard set, reconciled

The set ships Apache-2.0 in `plugins/connectors/`, needs no account, and is a
promise rather than a starter kit. This table is the list. It replaces the
list in `docs/planning/22-connector-marketplace.md` §3.1, which did not carry
the two category references and named a folder convention that has since
changed.

| Connector | Kind | Home | Record types it acts on |
|---|---|---|---|
| NinjaOne | `rmm` | `ninjaone/` + `ci-feeders/vendors/{ninjaone,ninjaone_alerts}.rs` | configuration |
| Hudu | `dms` | `hudu/` + `dms-sync` (export, live, render, link check) | kb_article, configuration |
| Microsoft 365 Graph | `identity` | `m365-graph/` | configuration |
| UniFi | `network` | `unifi/` + `unifi/manifest.json` | configuration |
| QuickBooks Online | `accounting` | `qbo-export-sync/` | invoice, payment, credit_note |
| Xero | `accounting` | `xero-export-sync/` | invoice, payment, credit_note |
| 3CX | `telephony` | `threecx/` | contact, ticket, configuration |
| Slack, Teams, Rewst | `notifications`, `crm` | `app-bridges/` | ticket, kb_article |
| Generic REST DMS | `dms` | `dms-sync/source/rest.rs` | kb_article, configuration |
| Signed-webhook alert ingestor | — | `crates/core-api/src/routes/alerts.rs` + `ci-feeders` | configuration |
| Cloudflare | `dns` | `cloudflare/` + `adapters-core::dns` | domain |
| Mapbox | — (routing provider, ADR 0014) | `mapbox/` | — |

`adapters-core` and `ci-feeders` are support crates, not connectors: transport,
auth, pagination, org mapping and the feeder engine. They are the surface a
community connector builds on, and they carry no vendor identity of their own.

Two corrections are recorded here rather than in a separate note. **Cloudflare**
and **Mapbox** are labelled "ADR 0013 standard set" in `Cargo.toml` and in
`plugins/connectors/README.md`, while the planning table omitted them. They are
in the set, which is why they appear above. Mapbox is a `RoutingProvider`
(ADR 0014) rather than an HTTP connector, and it is the one exception to the
"no import of a core crate" rule: it depends on `crates/core-routing` by
design, because it implements that trait.

## 3. Requirements

Each row is a requirement, the evidence that satisfies it, and who owns the
evidence. "Test" means it runs in CI. "Review" means a human checks it in the
pull request, and the reviewer is expected to say so.

| # | Requirement | Evidence |
|---|---|---|
| R1 | **One artifact.** The connector installs from a single signed manifest with `artifact_kind` declared. No companion plugin, no per-connector UI code. | Review: the manifest; `PluginManifestV2::validate` |
| R2 | **Public API only.** No import of a `crates/` module. | Test: `scripts/check-boundary.sh`. Review for the Mapbox exception |
| R3 | **None mode is real and documented.** The adapter runs with no vendor credentials and reports what it would have done, with zero Marula writes. | Test: each crate's conformance suite (`NoneExportSink`, `NoneFeederSink`, `NoneCallLogSink`, `NoneDnsRecordProvider`). Review: the README |
| R4 | **Deterministic identity.** Records converge onto one configuration through source GUID, serial then hostname, and a second sync updates its own rows rather than creating twins. | Test: `ci-feeders` identity suite and `dms-sync` sync suite. The unique identity indexes in migration 0185 |
| R5 | **Typed errors, vendor text preserved.** No string-matched vendor prose, and the vendor's own message is kept verbatim beside the typed reason. | Test: conformance suites assert the error enum per status. Schema: `status_detail_vendor` |
| R6 | **Per-client where the vendor is per-client.** A controller, PBX or tenant that belongs to one client is a `connection` scoped to that client, and its failure degrades that connection only. | Test: the resolver suite. Schema: `connector_connections.scope_client_id` |
| R7 | **Surfaces are declared, not coded.** A connector reaches a chip through `connector_surfaces`, and the product kind decides grouping. | Test: manifest validation refuses a malformed block. Review: the block is present for a UI-bearing connector |
| R8 | **A template is evidenced.** `open_url` and `launch.url_template` are declared only where the vendor documents the URL, and the review records the source and date. | Review: evidence table in [connector-capabilities.md](connector-capabilities.md) §3.4. Test: scheme and placeholder validation |
| R9 | **A launch is audited, and a secret never leaves its connection.** A press writes one audit row naming actor, connection, record and outcome. Credentials are unsealed only inside the adapter. | Review: the launch path. Test: the action audit |
| R10 | **Conformance tests run against stubs.** Contract-shaped stub servers reproduce the vendor's own auth flow and paging rules, plus the error surfaces a caller really meets. No live credential, no live tenant. | Test: every connector crate's suite. Review: no secret in a fixture |
| R11 | **No entitlement check, Apache-2.0 for the set.** The core never gates a connector, and a paid pack verifies its own licence offline. | Test: `scripts/check-boundary.sh`. Review |
| R12 | **A canary and a status timestamp.** A connection's status is paired with the time it was last proven, and the roster shows both. | Test: `record_connection_outcome`. Review: the canary calls a cheap typed endpoint |
| R13 | **A README that states purpose, non-purpose and none mode.** | Review: the file exists and is current |
| R14 | **Access is declared, not assumed.** A connection with an audience is invisible to a caller outside those roles on every surface that reads it, and a client-scoped connection is still refused to a caller who cannot see that client. Empty audience means every caller the module permission allows. | Test: the audience cases in `core-db/tests/connector_actions_resolution.rs` and `core-api/tests/http_flows/connector_write_flow.rs`. Schema: `connector_connections.audience_roles` |

### 3.1 The settings subset

A connector's `settings_schema` is not JSON Schema at large. It is one subset,
and both halves of the contract honour exactly that subset: the manifest gate
refuses a declaration outside it, and the value validator refuses a value the
declaration cannot judge. A keyword the gate accepted while the validator
ignored it would let a manifest publish settings that can never be saved.

| Keyword | Means |
|---|---|
| `type` | `string`, `number`, `integer`, `boolean`, `object`, `array` |
| `properties` + `required` | An object's fields, and the ones that must be present. A required name no property declares is refused at publish time, because nothing could ever satisfy it |
| `items` + `minItems` / `maxItems` | An array's element schema and its length bounds |
| `enum` | A closed set of scalars |
| `minimum` / `maximum` | Numeric bounds |
| `minLength` / `maxLength` | String length in characters |
| `pattern` | An unanchored Rust `regex`; say `^…$` yourself to anchor it |
| `writeOnly: true` | The field is a credential: sealed, never returned, never stored in settings |

`oneOf`, `anyOf`, `allOf`, `$ref`, `additionalProperties`, `multipleOf`,
`exclusiveMinimum`/`exclusiveMaximum` and the rest are **refused where they are
declared**. An annotation the subset does not interpret (`title`, `description`,
`default`, `format`) is accepted and ignored.

Two rules make the subset safe to store against:

- **Unknown keys are refused at every object level**, nested ones included. A
  settings object that names a field the connector never declared is a typo,
  and accepting it means the vendor silently receives nothing.
- **A credential never travels in `settings`.** `writeOnly` marks it, the write
  route refuses it in the settings document, and it is stored sealed. Every
  response names the credential fields that hold a value (`secrets_set`) and
  returns no value for any of them.


### The record type an asset hangs off

G10 asked for an `external_refs` block on the "asset" record, and offered to add
`asset` to `RecordType` if it was missing. It is missing, and it should stay
that way: **the configuration item is the asset in this product.** `assets` is
the table behind `RecordType::Configuration`, the resolver already scopes a
configuration to its client, and the chip row already draws it. A second
`asset` value would give one row two names and force every connector to pick
between them. UniFi therefore declares its block on `configuration`, and
`plugins/connectors/unifi/manifest.json` is the single place that says so.

The vocabulary is unchanged by this wave. Adding a record type still costs the
list page, detail page, permissions, export mapping and search placement that
§5 of [connector-capabilities.md](connector-capabilities.md) describes.

### Document sync: link checking and article visibility

Two DMS behaviours are contract, not polish, and both are tested in `dms-sync`:

- **A document's link targets are checked on sync.** A target that answers
  `404`/`410` lands as a broken link with the target retained, and the run
  report counts it. The check is an SSRF surface because the target comes from
  customer content: only `http`/`https`, loopback, link-local, private, CGNAT,
  unique-local and RFC 5737 documentation ranges refused, DNS resolved once and
  every answer judged, the connection pinned to the judged addresses, no
  redirect followed, a bounded timeout, and `HEAD` only so no response body is
  read. A refused target is `unverified`, never `dead`, because "we would not
  fetch this" and "the document is broken" are different facts.
- **A KB article's visibility crosses with it.** Hudu's `public` and Marula's
  `client_visible` are one flag with two names: the pull maps it onto the
  article, the push renders it back, and an article with no flag is never made
  public by default.

The guard's ranges and ordering are stated in
`plugins/connectors/dms-sync/src/links.rs`, beside the code that applies them.
They mirror `core-domain::egress::address_is_public`; the duplication is the R2
boundary (a connector may not import a `crates/` module), and the two lists are
kept in step by review.

### Per-client connections in one run

R6 is a schema rule, but a connector has to honour it at runtime. `unifi` reads
a client's own connection first and the org-scoped connection only as a
fallback (`connection_for_client`). A refused credential is recorded against
that client, in its `ClientRun` and in one report entry, and the next client's
leg still runs. It never aborts the run, and one client's base URL is never used for
another client.

## 4. The enforcement matrix, and what it does not cover

Mechanical where it can be, honest where it cannot. This table exists so that
"enforced by CI" is never claimed for a requirement no gate checks.

| Requirement | Mechanised today | Gap |
|---|---|---|
| R1 | No | No gate reads a manifest and asserts the artifact is single. A review step. |
| R2 | Partially | `check-boundary.sh` refuses a dependency outside the repository. It does **not** refuse a `plugins/` crate depending on `crates/`; `mapbox` does so legitimately and would pass either way. |
| R3 | No | Each suite exercises its none sink, but nothing asserts a README documents it. |
| R4 | Yes | The unique indexes refuse a duplicate. The suites assert convergence. |
| R5 | Partially | Suites assert the typed error per status. Nothing enforces that vendor text is stored rather than paraphrased. |
| R6 | Yes | The schema and the resolver suite. |
| R7 | Yes | `PluginManifestV2::validate` refuses a malformed or dead block. |
| R8 | Partially | Validation refuses a bad scheme, a stray placeholder, `{base_url}` off the prefix, and an undeclared `{settings.*}`. Whether the URL is *real* is a review step, which is why the evidence table records the source. |
| R9 | Partially | The schema stores the outcome. The audit row is a review step until the route lands. |
| R10 | Partially | Suites exist per crate. Nothing asserts a fixture carries no credential. |
| R11 | Yes | `check-boundary.sh` scans source for entitlement checks. |
| R12 | Yes | Column pair plus the outcome writer. |
| R13 | No | A review step. |
| R14 | Yes | The audience predicate is one function applied by the resolver and the roster; both surfaces are asserted in tests. The UI rendering of it is a review step until the Access panel ships. |

## 5. Deliberately not required

- **A hosted OAuth app.** The operator supplies their own credentials through
  the sealed store. Hosting and operating connections is commercial-layer work.
- **Write-back to the vendor as a record action.** Writes belong to `sync` or to
  a vendor-declared workflow action with its own scope.
- **A per-connector settings page, menu item or component.** All three exist only
  as renderers of declarations.
- **Live-credential smoke tests.** A connector that needs a real tenant to
  demonstrate its contract has not described the contract.

## 6. Open conformance debt

Recorded so it is not discovered twice. Each item is a row, not a wish.

| Item | Why it fails a requirement | Owner |
|---|---|---|
| Microsoft 365 Graph pulls devices, not seats | It was included because "seat counts drive billing correctness" (planning doc 22 §3.1) and it cannot serve that | `m365-graph` |
| NinjaOne and Hudu connectors are outside the workspace | R10, R13. They now have 85 tests between them, but `pnpm-workspace.yaml` still does not include `plugins/**`, so CI never typechecks them and they are the two packages the README tells newcomers to copy | `plugins/connectors/{ninjaone,hudu}` |
| The NinjaOne receiver refuses every ticket event | It posts an activity onto a device, and Core's ticket event carries no device reference, so the honest answer is a typed refusal. Fixing it needs `record_links` surfaced as `linked_devices` on the ticket event | `plugins/connectors/ninjaone` |
| QuickBooks and Xero do not yet send a void or a refund | Invoices and payments are sent, and so are credit notes. R7 stays unmet for the two record types they still do not claim. A free-standing credit note is refused; only one that adjusts an exported invoice is sent | `qbo-export-sync`, `xero-export-sync` |
| The accounting orchestration has no end-to-end route test | Adapter wire shapes, the store and the held/failed routing are tested; the route itself compiles and its pure pieces are covered. A sandbox run would still have to confirm token refresh. It would also have to answer whether a paged read behaves the same against a real tenant, and how a rotated credential reads | `crates/core-api` |
| Rewst triggers workflows and has no results path | The promise is "workflow triggers and results over the public API". The trigger is signed and returns; nothing accepts a result | `app-bridges` |
| `document_links` and `record_links` are two edge tables | Deliberate, documented in ADR 0033, and the shapes match so a merge stays mechanical | — |

## 7. Answering the two questions a reviewer asks

**"Does this connector need a plugin?"** No. One artifact, one manifest, one
consent. The surfaces a connector may reach are declared in that manifest and
rendered by Core.

**"Why is this connector's vendor not in the capabilities table?"** Because the
product kind is open. Declare one, map its records onto the nearest existing
record type, and the UI draws it. A record type Core cannot show is the only
thing that needs an RFC, and the API says so in the refusal.
