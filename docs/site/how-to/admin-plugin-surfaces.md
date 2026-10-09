# How to extend Marula with plugin surfaces

> **Audience:** admin and plugin developers — registering widgets,
> automation actions, automation triggers, read nodes, report render kinds,
> and custom-field validators is a consented, capability-scoped decision.

## Prerequisites

- Admin access to **Settings→Plugins**.
- The consent posture from [plugin consent](admin-plugin-consent.md) —
  this page builds directly on it.

## One sentence of doctrine

> "A plugin ships declarations, never code your browser or your rule
> engine runs — Marula renders and executes every surface itself."

## 1. What a plugin may contribute

A plugin's manifest can declare six kinds of surface registration:

| Surface | What it adds | Capability gate | Connector? |
| --- | --- | --- | --- |
| Widgets | Cards on dashboards and the ticket pane | `ui.widgets` | No — UI coherence |
| Automation actions | New action kinds in the automations editor | `automation.actions` | Yes |
| Automation triggers | Event types a rule can **fire on** | `automation.triggers` | Yes |
| Read nodes | A **fetch** from an external system | `automation.reads` | Yes |
| Report render kinds | Entries in the report builder's chart list | `reports.render` | No — UI coherence |
| Field validators | JSON-Schema checks on board custom fields | `fields.validate` | No — UI coherence |

The "Connector?" column is [ADR 0017](../../adr/0017-plugin-node-surface.md)
decision 3, and the split is by KIND, not by who wrote the plugin. A
connector is refused the three UI kinds because it has no in-product UI
surface to govern, and allowed the three executor kinds because it already
holds credentials and network egress — so an action or a read grants it
nothing it did not have. A WASM sandbox is the reverse: capability-restricted,
so granting it a network action *is* an escalation, which is why the rule
started there.

First-party and marketplace connectors hold the **same** capability. Trust
comes from consent, digest pinning, and the per-install grant — none of
which depends on authorship. There is no authorship class to be in.

Everything in that table is **declarative**: a widget is a closed render
kind (stat, metric, or list) with JSON-Schema-validated props, an
automation action maps onto a consented built-in effect (today: add an
internal comment) with typed parameters, and a validator is a schema the
core runs — the plugin never sees field values and never renders raw
content.

## 2. The consent screen shows exactly what you grant

When you review a staged import, the consent panel lists the plugin's
surface declarations next to its capability requests — widget titles,
action labels and their effect, render kinds, validator names. Nothing
registers unless you grant the matching capability, and un-granting (or
disabling, or revoking) the install removes the registrations.

## 3. Where the surfaces show up

- **Dashboards and the ticket pane:** plugin widget cards render under
  the "Plugin widgets" section with the plugin's registry key printed on
  the card. A widget whose saved configuration fails its declared schema
  renders nothing rather than rendering unvalidated.
- **Automations:** plugin action kinds appear in the "Add action" picker
  after the built-ins. The rule engine re-checks the registration at
  execution time — a rule whose plugin action lost its registration
  skips that action instead of guessing.
- **Automations — triggers:** a plugin's declared trigger source
  appears as an event type a rule can fire on. The declaration names the
  event type; it dispatches nothing itself.
- **Automations — reads:** a read node appears wherever a workflow needs
  to FETCH external context. It carries its declared guardrails, and
  those caps are enforced and reported rather than assumed.
- **Report builder:** registered plugin render kinds are listed under
  the chart picker. The selectable chart vocabulary stays Marula's own
  trusted set.
- **Board fields:** field editors offer registered validators; once a
  field carries one, every value write passes the schema check, and a
  write fails closed if the validator becomes unavailable.

## 4. The webhook surface: inbound is the default, not a surface

**Read this before you reach for anything else.** Most of a connector's
workflow value is INBOUND (a device going offline, a ticket changing, a
payment clearing), and inbound needs no surface at all.

A connector HMAC-signs a `POST` to Marula's generic signed ingest,
declares the event type it emits in the payload, and the receiver turns
that into a real envelope on the stream. The event-type registry makes the
type subscribable. No surface declaration, no capability, no relaxation:
the envelope is the integration point, and a rule fires on it exactly as it
fires on `ticket.created`.

So consider them in this order:

1. **Inbound first.** Sign a POST to the generic ingest. This covers
   "something happened in my SaaS, wake up Marula".
2. **Outbound seeding.** A connector's `connector_profile.webhook_endpoints`
   are not decoration: when the org consents, those URLs become REAL
   outbound subscriptions attributed to the install. That is what the
   consent screen promises when it says the endpoints "seed the
   connector's outbound posture".
3. **Emit, only if inbound is genuinely awkward.** The `events.emit`
   capability lets an authorized plugin runtime publish an envelope of a
   type it declared — checked against the event-type registry, so it
   cannot emit a type nobody admitted. Prefer inbound; emit exists for a
   runtime that does not want the round trip.

### Why a read node is guarded harder than a write

`automation_action` is write-shaped, so before the `read` kind existed a
workflow could not FETCH from an external system at all. That is the gap
that stopped a workflow assembling Decision Engine context from an RMM.

Reading is where the volume and the blast radius live, so a read declares
its own guardrails and there is no default that weakens them:

| Guardrail | What it stops |
| --- | --- |
| `max_calls_per_run` | A loop over a device list hammering the customer's own RMM. |
| `min_interval_secs` | A schedule becoming a stream, across the whole org. |
| `max_output_bytes` | A ten-thousand-device list growing the run ledger by megabytes. |
| `may_return_secrets` | A vault secret arriving in a place operators read. This one may only ever be declared `false`, and the manifest refuses anything else. |

A read that declares no guardrails is refused outright, and a declaration
cannot write itself a weaker cap than the platform floor. When you consent,
you are consenting to those caps as much as to the read itself.

### What consent shows, and how it comes back

Every declared endpoint renders verbatim on the consent screen beside the
capability requests and the surface declarations. Nothing registers unless
you grant the matching capability.

The seeded subscriptions are attributed to the **install**, so the
lifecycle carries them:

| Event | What happens to the seeded rows |
| --- | --- |
| Install created | Rows are created, inert. A fresh install is created **disabled**, and a disabled install's rows do not deliver. |
| Install enabled | Rows become live automatically. Consent and delivery line up with no extra switch. |
| Install disabled | Rows go inert again. |
| One endpoint disabled by you | Only that row stops. Your choice is kept on every re-seed. |
| Install uninstalled | Rows are deleted with the install. |
| Catalog entry revoked | Rows are deleted beside the revocation, and any straggler is independently inert. |

Human-created subscriptions are untouched by all of the above: they carry
no owner, and they deliver exactly as they always did.

## 5. Build your own: the reference plugins and the PDK

Three first-party plugins ship in `plugins/wasm/` and walk the whole
story. One contributes an escalation-policy widget, one a triage-note
automation action, and one an asset-tag validator. The Plugin Development Kit
(`packages/plugin-kit`) validates a manifest against the same closed
vocabularies the host enforces and bundles `manifest.json` plus the
built WASM artifact into a zip you can import or publish:

```sh
cargo build --manifest-path plugins/wasm/escalation-widget/Cargo.toml \
  --target wasm32-unknown-unknown --release --target-dir plugins/wasm/target
cp plugins/wasm/target/wasm32-unknown-unknown/release/escalation_widget.wasm \
  plugins/wasm/escalation-widget/plugin.wasm
node packages/plugin-kit/bin/build.mjs plugins/wasm/escalation-widget
```

The printed sha256 is the digest you pin in a registry entry. Full
details live in `plugins/wasm/README.md`.

## What you learned

- Plugins contribute **declarations**; Marula renders and executes
  them through its own trusted components.
- Every surface section is gated by its own capability, shown on
  consent, and deregisters the moment the grant goes away.
- **Inbound is the default.** Sign a POST to the generic ingest before
  you declare any surface at all.
- The reference plugins plus `@marula-core/plugin-kit` are the template for
  building your own.
