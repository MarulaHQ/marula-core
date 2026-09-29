# How to extend Meridian with plugin surfaces

> **Audience:** admin and plugin developers — registering widgets,
> automation actions, report render kinds, and custom-field validators
> is a consented, capability-scoped decision.

## Prerequisites

- Admin access to **Settings→Plugins**.
- The consent posture from [plugin consent](admin-plugin-consent.md) —
  this page builds directly on it.

## One sentence of doctrine

> "A plugin ships declarations, never code your browser or your rule
> engine runs — Meridian renders and executes every surface itself."

## 1. What a plugin may contribute

A plugin's manifest can declare four kinds of surface registration:

| Surface | What it adds | Capability gate |
| --- | --- | --- |
| Widgets | Cards on dashboards and the ticket pane | `ui.widgets` |
| Automation actions | New action kinds in the automations editor | `automation.actions` |
| Report render kinds | Entries in the report builder's chart list | `reports.render` |
| Field validators | JSON-Schema checks on board custom fields | `fields.validate` |

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
- **Report builder:** registered plugin render kinds are listed under
  the chart picker. The selectable chart vocabulary stays Meridian's own
  trusted set.
- **Board fields:** field editors offer registered validators; once a
  field carries one, every value write passes the schema check, and a
  write fails closed if the validator becomes unavailable.

## 4. Build your own: the reference plugins and the PDK

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

- Plugins contribute **declarations**; Meridian renders and executes
  them through its own trusted components.
- Every surface section is gated by its own capability, shown on
  consent, and deregisters the moment the grant goes away.
- The reference plugins plus `@marula-core/plugin-kit` are the template for
  building your own.
