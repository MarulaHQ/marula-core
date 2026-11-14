# ADR 0032: The connector connection model

Status: proposed (2026-11-14)

## Context

Two connector models exist side by side, and they disagree about almost
everything.

The **plugin model** (ADR 0009) is the mature one. An install carries a
typed manifest with a closed capability vocabulary, a `settings_schema` that
passes a structural gate (`crates/core-domain/src/entities/plugins.rs`), a
signed artifact, staged approval, and a re-consent diff on update. It has a
UI: `apps/web/app/settings/plugins`.

The **connector model** is a table from migration 0020. It carries four
kinds, hardcoded in `crates/core-api/src/routes/integrations.rs:parse_kind`
(`slack`, `teams`, `rewst`, `voip`), a fixed scope list per kind in
`default_scopes`, one opaque encrypted config blob, and one row per org and
type (`connector_installations_type_unique`). It has no UI at all: no route
in `apps/web` calls `/api/v1/connectors`, and
`apps/web/app/settings/integrations/page.tsx` says in its own header that
connector installation consent "is not this page".

That second model cannot carry the product we have promised:

- **Per-client vendors.** A UniFi connector must point at two controllers
  for two clients, one Hostifi-managed and one on-prem, with a credential
  failure scoped to one client rather than the whole run. One row per org
  cannot express that.
- **Share destinations.** Sharing a ticket into a channel needs a named
  channel the org has chosen, not a single webhook URL sealed in a blob.
- **Settings.** Every connector has different settings. A hand-written form
  per connector is the "afterthought" surface this ADR exists to prevent,
  and the manifest already knows how to describe them.
- **Access.** "Who may install", "who may share through it" and "who may
  see it exists" are three different questions. The current row answers
  none of them.
- **Marketplace parity.** A marketplace connector must reach the same
  surfaces as a core one. Any capability a core connector gets by being a
  Rust enum arm is unavailable to everyone else, which is the opposite of
  an ecosystem.

## Decision

**Separate the definition from the instance, and scope the instance.**

### Installation (the definition)

An installation is what the org agreed to: a connector identity, a version,
a signed artifact digest, the granted capability scopes, and the manifest
that declares its `settings_schema` and its `surfaces`. Installation is the
act of consent. It is the existing plugin install record, extended with a
`connector` artifact kind — there is one lifecycle, not two.

### Connection (the instance)

A connection is one configured use of an installation:

| Field | Meaning |
|---|---|
| `installation_id` | Which definition this connection was made from |
| `scope` | `org`, or `client:<client_id>` |
| `settings` | Values that satisfy the manifest's `settings_schema` (non-secret, queryable) |
| `secrets` | Sealed credentials, keyed by the schema's `writeOnly` fields |
| `status` | `connected`, `needs_attachment`, `auth_failed`, `degraded`, `disabled` |
| `status_detail` | Typed reason with the vendor's message kept verbatim, never parsed into prose |
| `last_success_at` | The last proven call. A status without a timestamp is a claim |

A connection at `scope = client` is the answer to per-client vendors. A run
that touches three clients holds three connections. One `auth_failed` marks
that connection degraded and leaves the other two alone. The acceptance test
for G11 falls directly out of this shape.

### Binding (the destination)

A binding is a named destination a connection can post to: a Slack channel,
a Teams conversation, a Rewst workflow, a VoIP route. Bindings belong to a
connection, carry a display label the operator chose, and are what the share
dialog lists. A connection with no binding can still pass its canary test,
but it cannot be a share target, and the UI says so rather than offering a
button that fails.

Bindings are also the audit join: a message id, a channel, a connection and
a ticket id.

### Kinds come from the manifest

`parse_kind` and `default_scopes` are deleted. A connection's settings are
validated against its installation's `settings_schema`. Its capability
scopes are the ones consented at install, intersected with the closed
vocabulary the core already gates. An unknown connector kind is no longer a
validation error with a four-item allowlist — it is "no installation by that
name", which is the honest answer and works for the marketplace.

### Access is three questions, answered separately

| Question | Answer |
|---|---|
| Who may install or change a connection? | The admin surface's existing `special:manage_integrations` |
| Who may use a connection (share, act)? | The connection's audience: `all_staff` (default) or an explicit role/team list |
| Who may see a client-scoped connection? | Whoever the client-access rules already let see that client. A connection never widens client visibility |

A share action is permitted only when the user can read the record *and*
belongs to the connection's audience. A press on a Slack action button
authenticates as the mapping between the vendor identity and a Marula user. Where that mapping does not exist, the action answers with a typed refusal
that names the missing link and offers the link step — never a silent
no-op.

### Surfaces are declared, not coded

The manifest gains one optional block, `surfaces`, and the UI becomes a
renderer for it (see `docs/reference/connector-surface.md`):

```json
{
  "product_kinds": ["rmm", "remote_access"],
  "surfaces": {
    "settings": { "schema_ref": "#/settings_schema" },
    "share": { "records": ["ticket", "kb_article"], "label": "Share to Slack" },
    "actions": [
      { "id": "take_ticket", "label": "Take it", "record": "ticket",
        "required_scope": "tickets:write", "confirm": false }
    ],
    "external_ref": {
      "record": "configuration",
      "open_url": "{base_url}/device/{external_id}",
      "launch": { "method": "remote_session", "label": "Start session" }
    },
    "inbound": { "events": ["ticket.created", "ticket.updated"] }
  }
}
```

`product_kinds` is what makes a connector contextual: it tells the UI which
kind of vendor this is (RMM, remote access, DMS, DNS, network, identity,
accounting, notifications) without the UI ever naming a brand. A connector
that declares none still gets settings and a canary. It simply contributes
no contextual actions.

**UI capability belongs to the manifest, never to the runtime.** An HTTP
connector and a WASM plugin differ in where their code runs and in nothing
else: both carry this same document, so neither needs a wrapper of the
other to be present in the staff UI.

### The external identity is the join

`config_source_links` (migration 0061) already records that a
configuration exists in an external system: `(config_item_id,
source_system, external_id)`. It gains a nullable `connection_id`, because
`source_system = 'ninjaone'` cannot say *which* NinjaOne, and the deep link
or session launch needs the base URL and the credentials of exactly one
connection. A link without a connection resolves to no action rather than
to a guess, and the configuration surface reports it as an unresolved link.

**Open and launch are different acts.** `open` is a URL built from the
connection's base URL and the external id. It needs no secret and can be
rendered as a plain link. `launch` starts something and is always audited,
but it has two forms: a deterministic `url_template` that mints nothing
(ScreenConnect's `/Join` suffix, `tel:` for a handset dial), and a `method`
that spends the connection's sealed credential and returns a one-time URL
(3CX `MakeCall`, a Teams call). Neither is ever a bare `window.open` of an
unverified string.

**A template is a claim about someone else's URL.** It is allowed only
where the vendor documents it. Verified 2026-11-14: Slack and Teams
document theirs. So does Xero. QuickBooks Online explicitly does not support
transaction links, because `txnId` is not the API `Id` and a constructed link
can open another company's invoice. A connector that cannot document its link declares
none, and the reference carries an optional `external_url` for a URL the
connector *learned* from the vendor rather than composed.

A core connector and a marketplace connector that declare the same block
get the same depth, because the renderer reads data. A connector that
declares nothing gets the one universal behaviour: it is a connection with
settings and a canary test.

## Consequences

**Easier.** G11 (per-client controllers) is a scope on a row, not a feature.
G8 (share from the ticket) is a dialog over bindings. G7 (interactive
callbacks) joins actions to the audit row that already exists. A
marketplace connector reaches the same surfaces with no core change.

**Harder.** Two migrations: create the new tables, backfill from
`connector_installations`, then remove the old ones in a later release
because a rollback must still find its data. The four typed kinds need
manifest-shaped equivalents before the allowlist can go. `settings_schema`
must grow two things it lacks today: `writeOnly` marking for secrets, and a
`select` source that can name a client list.

**Refused.** No per-connector table. No per-connector form component. No
connector-specific menu item anywhere in the staff UI. If a connector needs
a surface the renderer does not have, the renderer grows a generic one and
every connector gets it.

## Alternatives rejected

- **Keep `connector_installations` and add a `client_id` nullable column.**
  Rejected: it makes every read that means "the org's connection" also
  mean "or a client's", and the sealed blob cannot hold per-client
  credentials without a second layer of indirection anyway.
- **Put connectors in `plugins` and leave the consent table alone.** This
  is what created the two models. The install lifecycle and the connection
  lifecycle are genuinely different objects. Merging them fully would make
  an install carry runtime state.
- **One connection per org, with vendor-side multi-tenancy.** Rejected:
  UniFi controllers and 3CX PBXs are physically separate per client, and
  Hostifi versus on-prem is exactly the case that breaks it.
