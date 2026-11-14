# The connector surface contract

> **Purpose:** every place a connector appears in the staff product, drawn
> once, so that a connector built in a community repo feels as native as the
> one in `marula-core/plugins/connectors`. This is the reference a connector
> author reads before writing a manifest, and the checklist a reviewer uses
> before approving one.
> **Companions:** [ADR 0032](../adr/0032-connector-connection-model.md) (the
> model this draws), `docs/theme.md` (tokens, motion),
> `docs/instant-first.md` (envelope rules).
> **Status:** proposed, 2026-11-14.

## 1. The rule

A connector contributes **data**, never layout. Six surfaces exist, all six
are owned by Core, and every connector reaches them by declaring what it
needs in its manifest. There is no per-connector component, no per-connector
menu item, and no per-connector settings form. A connector that wants
something the renderer cannot express is a request to grow the renderer,
because the alternative is a marketplace author copying Core's layout and
drifting from it within one release.

The six surfaces:

| # | Surface | Where | Answers |
|---|---|---|---|
| S1 | Connector roster | Settings → Connectors | What is available, what is installed, what is broken |
| S2 | Connection detail | Roster → a connection | Credentials, settings, destinations, health, access |
| S3 | Share | A record, a message, a selection | Where does this go, and who sent it |
| S4 | Inbound action | A vendor message | One press does one audited thing |
| S5 | Contextual actions | A configuration, a ticket's configurations, a recommendation | Open the same object in the vendor, or start a session |
| S6 | Access and audit | Settings → Connectors → Access, record activity | Who may use it, what was done with it |

## 2. S1 — the roster

**One page. `Settings → Connectors`.** Today's `/settings/integrations` is
the webhook and inbound-source manager and stays that. The pinned settings
navigation calls it "Chat and phone", which is the wrong label for a
delivery log. Connectors get their own entry beside Plugins, because a
connection is not a webhook.

Layout is a card list, not a table: every real product that does this well
(Calendly, Chili Piper, Tettra, Invision, PandaDoc) shows a logo, a name, a
one-line purpose, a status, and exactly one primary action. A table would
force a column decision per vendor and would not survive the marketplace.

The card shows, in this order:

1. **Identity** — logo, name, one line. The line is the manifest's
   `description`, truncated at clause boundaries, never re-authored in Core.
2. **Status pill** — `Connected`, `Needs attention`, `Not connected`,
   `Disabled`. One pill, derived from the connection with the worst status. A connector with three healthy client connections and one failed one
   reads `Needs attention` and says "1 of 4".
3. **Action** — `Connect` (none), `Manage` (one or more), `Fix` (any
   `auth_failed`). The action changes with state. A disabled `Manage` on a
   broken connection is the failure mode this avoids.
4. **Secondary line** — the count of connections and, when scoped, the
   clients they serve by name. Two clients, two names, never "2 items".

Search filters the roster by name and purpose. No filter chips for
categories until the catalogue has more than twenty entries. A filter bar
over six vendors is decoration.

**Empty state** names the standard set and says where the rest come from,
with one action that opens the plugin marketplace tab. It does not offer a
"create connector" button. Connectors are code.

## 3. S2 — the connection detail

A connection opens as a page, not a modal, because it holds four things a
modal cannot: a settings form that may be long, a destination list, a health
timeline, and an access panel. The modal is reserved for destructive
confirmation.

Four sections, in this order:

1. **Settings**, rendered from the manifest's `settings_schema`. Field
   types map onto the existing form primitives: string → `Input`, enum →
   `Select`, boolean → `Switch`, number → `Input type=number`,
   client-reference → a `Select` fed by the client list, `writeOnly` → a
   secret field that renders as a masked placeholder after save and can only
   be replaced, never read back. Validation happens on blur and on submit. Server errors land on the field that caused them.
2. **Destinations** (bindings). A list of named channels or targets with a
   label the operator chose. `Add destination` opens a picker that queries
   the connection's own vendor surface (Slack channel list, Teams
   conversation list) where the credential allows it, and falls back to a
   paste field where it does not. A destination can be disabled without
   being deleted. A share to a disabled destination is refused with the
   reason, not silently dropped.
3. **Health.** Status, last success, last failure with the vendor's message
   verbatim, and a `Test connection` button that runs the connector's canary
   and writes the result. The canary result is not a toast. It updates the
   section, because a toast is gone before the operator has read it.
4. **Access.** The audience for using this connection: `All staff` (default)
   or named roles and teams. Below it, one line naming what install and
   removal require, which is not editable here.

Per-client connections appear under the connector as a secondary list
(`Client` → connection), with the same detail page. This is the G11 surface:
two controllers, two credentials, one roster entry, one status per client.

## 4. S3 — share

This is the surface most likely to feel bolted on, so it is specified
tightly.

**One entry point per record, and it is not a new toolbar button.** A record
header already ends with one primary action (`SurfaceHeader`'s action slot).
`Share` becomes a secondary action beside it — a single button, labelled
"Share", with a menu only when more than one destination family exists. It
is **not** "Share to Slack" plus "Share to Teams" plus a third button per
future connector: that is the design that stops scaling at two connectors
and is why this contract exists.

**The dialog is a picker, not a deep link.** The user opens `Share`, sees
the destinations they are allowed to use, in the order the connection's
audience list defines, grouped by connector:

```
Share this ticket

  Slack — Acme workspace
    #helpdesk-alerts
    #tier-2
  Microsoft Teams — Contoso
    Service Desk / Tier 1

  [ Copy link ]                      [ Cancel ]  [ Share ]
```

Rules:

- One destination selected at a time on first release. Multi-select is a
  later addition and must not change the layout when it arrives: the list
  becomes checkable, the button count stays at three.
- The dialog lists **only binds the user can use**. A connection the user is
  outside the audience for is absent, not disabled-with-a-tooltip. The
  absence is what "no access" looks like.
- Posting returns a **permalink and a confirmation in place**, with
  `View in Slack` as the one follow-up action. It does not close and toast. The user may want to undo, and the message id is the proof it landed.
- The card posted is the same card in both products and is scoped to what
  the sharer could see. It carries the ticket number, title, client, status,
  assignee, and a link back. It never carries internal-only notes, and the
  scoping decision is made by Core, not by the connector.
- The deep link (`slack://channel?team=…`) is the **fallback**, used only
  when no destination exists, and then the dialog explains why the direct
  picker is unavailable and offers the one-time setup. A product that
  silently hands the user to another application's UI has not integrated
  anything.

**Message-level share.** A ticket comment has a per-message overflow. That
menu needs a **new `Menu` primitive**: `@marula-core/ui` has `Dialog`,
`Select` and `Tooltip` today and no menu. It therefore ships as a shared
component rather than being improvised inside the ticket page. Its first
item is `Share to channel`. It opens the same dialog with the message's
permalink and a quoted excerpt as the card body. Sharable message types are
comments and notes marked visible to staff. A private draft never shares.

**Bulk share.** The list surfaces already carry `bulk-action-bar`. Selecting
several records adds `Share` there, which opens the same dialog once and
posts one card per record — never one card with a list, which is unreadable
in the channel and impossible to act on.

**Inbound from the channel.** A channel message can carry a ticket card. The card's own buttons belong to S4.

## 5. S4 — inbound actions

A vendor message carries buttons. Each button is an action the manifest
declared, with the scope it requires:

```
{ "id": "take_ticket", "label": "Take it", "record": "ticket",
  "required_scope": "tickets:write" }
```

The callback is verified against the vendor's own scheme (Slack signing
secret with a five-minute anti-replay window, Teams Bot Framework JWT
issuer and audience), resolved to a connection, then to the Marula user
bound to the vendor identity. Then, in one transaction: the permission
check, the action, the audit row.

- A press by an unlinked identity answers with a link step in the channel
  ("Connect your Marula account to take tickets"), and the button stays live
  after linking.
- A press by a linked identity without the scope answers a typed refusal
  naming the missing permission. It does not silently do nothing, and it
  does not leak whether the ticket exists.
- Every press writes an audit row: connection, binding, message id, actor,
  action, outcome.

The action itself is Core's action, not the connector's. "Take it" is the
same domain call the ticket header's `Take it` makes. The connector supplies
the trigger and the identity, never the effect.

## 6. S5 — contextual actions

This is the surface that decides whether a connector feels native or
bolted on, because it is the one the technician touches all day. It is
organised by **what the technician wants to do**, never by which vendor
they bought.

### 6.1 The anchor is the configuration, not the ticket

A configuration item is one real thing. It may exist in several external
systems at once: the same laptop is a NinjaOne device, a ScreenConnect
machine, a Hudu asset and an M365 device. The UI's unit of display is the
**configuration**, and its external presences are a property of it.

`config_source_links` already stores that. Two things must be added:

- `connection_id` on the link, so two NinjaOne orgs or two ScreenConnect
  instances resolve to the right base URL and credential.
- a display kind derived from the connection's connector `product_kinds`,
  so the UI can group an RMM presence and a remote-access presence without
  hardcoding a brand list.

An existing link with no resolvable connection renders as a muted,
non-clickable mark with the reason in its title, and the configuration
surface lists it as unresolved. It never renders as a button that fails on
press.

### 6.2 One chip per capability, not per brand

A configuration detail shows a row of capability chips: `RMM`,
`Remote access`, `Documentation`, `Network`, `Identity`. Each chip carries
the **primary** provider's mark and name, and a second provider is never a
second chip — it is inside the first chip's menu.

```
NinjaOne device · Locked down?      [ RMM: NinjaOne ▾ ]  [ Remote: ScreenConnect ▾ ]
```

- Pressing the chip body runs the primary provider's action: open for an
  RMM, launch for remote access.
- The caret lists every provider for that capability on this
  configuration, each with its own status.
- With exactly one provider the chip has no caret and no menu, so the
  common case is one click and no decision.
- A capability with no provider renders nothing at all. An empty row is
  what an unlinked tool looks like. A greyed chip with a caret leading to
  nothing is what a demo looks like.

### 6.3 Multiple tools for the same capability

An MSP with two remote-access tools and two RMMs is the normal case, not
the edge. The rules:

1. **A default per capability, per org, overridable per client.** Settings
   → Connectors holds "Remote access: ScreenConnect (primary), AnyDesk". The client record may override it when the client mandates one. The
   default decides which provider the chip body runs. It never hides the
   others.
2. **Never a silent choice.** If a configuration has two RMM presences and
   no default resolves, the chip body opens the menu instead of guessing,
   and the header says "Choose a system".
3. **Group by (capability, external identity).** Two feeds reporting the
   same RMM device are one presence, deduplicated by external id. Two
   different systems reporting the same device are two presences, because
   they are two different places the technician may need to go.
4. **A degraded provider stays visible and marked.** Its action explains
   the failure and offers `Fix`, which opens that connection's detail. It
   is never removed from the menu, because a missing tool reads as "we
   don't have that" and sends the technician looking elsewhere.
5. **No access means no presence.** A user outside a client's access sees
   neither the configuration nor any of its chips. A chip is not a hint
   that the client exists.

### 6.4 Open versus launch

| | `open` | `launch` |
|---|---|---|
| What it does | Builds a URL from the connection base and external id | Asks the adapter to start a session and return a one-time URL |
| Spends a secret | No | Yes |
| Audited | The interaction is logged as a view | Yes, as an action with actor, connection, configuration |
| Scope required | none beyond read | the connection's launch scope |
| Fallback when refused | — | the `open` action, if the connector declares one, labelled honestly |

Starting a remote session is a security act, so it is never a static href
and never a bare `window.open`. The button is `Start session`. Between the
press and the session there is one adapter call, one permission check and
one audit row. If the scope is missing, the chip offers `Open in
ScreenConnect` instead and says why, rather than a disabled button.

### 6.5 On a ticket

A ticket shows the configurations it involves, because that is how the
technician thinks ("the printer at their second site"). This relation does
not exist in Core today: tickets carry client, site and contact, and no
configuration link. It is added as a many-to-many `ticket_configurations`,
with the ticket detail rendering a **linked-records strip** above the
activity timeline:

```
Configurations  (2)                      [ + Link configuration ]
  Dell Latitude 5540 · NIN-4412         [ RMM ▾ ] [ Remote ▾ ]
  HP LaserJet M404 · NIN-7781           [ RMM ▾ ]
```

- Each row is a configuration, not an aggregate. Icons from three
  configurations are never merged into one row. A merged row hides which
  device the technician is about to act on.
- More than four linked configurations collapse to four rows plus
  `Show all (7)`. The strip is a summary, not a table.
- A ticket with none shows the strip's empty state once, with the link
  action — not on every ticket, only where a configuration could plausibly
  attach (a client with configurations).
- Linking happens from the ticket (search by name, serial or external id)
  and from the configuration ("linked tickets").

### 6.6 On a recommendation

An agent proposal already carries `entity`, `entity_id`, `tool` and
`capability`. When the recommendation involves working in a vendor system,
the card gains **one** action resolved the same way a chip is:

```
Recommended next step
  Restart the print spooler on NIN-7781. The driver update did not clear it.
  [ Open in NinjaOne ]  [ Dismiss ]
```

Rules: the action is resolved from the proposal's target configuration and
the capability its `tool` names. It never appears when the resolver returns
nothing (and the rationale says the tool is not linked). It never appears
for a user outside the client's access. And a `launch` action on a
recommendation still requires the launch scope. The recommendation supplies
*intent*; Core supplies the *capability resolution* — an agent never invents
a URL, a device id or a session token.

### 6.7 The contract a connector implements

One read endpoint serves every surface:

```
GET /api/v1/config-items/{id}/external-actions
→ {
    capabilities: [
      { kind: "rmm", providers: [
          { connection_id, connector, label: "NinjaOne", status: "connected",
            actions: [{ kind: "open", label: "Open in NinjaOne", url }] } ] },
      { kind: "remote_access", providers: [
          { connection_id, connector, label: "ScreenConnect", status: "connected",
            actions: [{ kind: "launch", label: "Start session", method: "remote_session" }] } ] }
    ]
  }
```

A connector author implements the manifest declarations plus, for a launch,
one adapter method. The chips on a configuration, the strip on a ticket and
the action on a recommendation all read this one payload, so a third-party
connector reaches all three on the day it is installed, with no core change
and no UI code.

### 6.8 Beyond configurations

The resolver is not configuration-specific. Its object is *any record with
an external reference*, and the chip row is the same wherever it appears:

| Record | Chip | Body action |
|---|---|---|
| Configuration | `RMM: NinjaOne`, `Remote: ScreenConnect` | open / start a session |
| Client or contact phone number | `Call: 3CX` | dial from the caller's own extension |
| Invoice, payment, credit note | `Accounting: QuickBooks` | open the document in the vendor |
| KB article | `Documentation: Hudu` | open the article in the vendor |
| Domain | `DNS: Cloudflare` | open the zone |

Two rules keep this from sprawling:

1. **A new record type needs no new UI.** The chip row, the resolver and the
   states are shared. Only the record's own surface changes, and it already
   exists.
2. **The action label belongs to the connector, the position belongs to
   Core.** A connector names its action ("Start session", "Open in
   QuickBooks"). It never decides where on the record the chip sits.

Sharing works in the other direction and uses the same seam: a surface that
can show a chip can also be a share source, if its manifest declares
`share.records` for that type. That is what lets a KB article be posted to a
channel the same week a ticket can.

### 6.9 States

| State | Rendering |
| Link with no connection | Muted mark, reason on hover, listed as unresolved on the configuration |
| Provider degraded | Chip visible, marked, action explains and offers `Fix` |
| Launch refused (scope) | `Open in …` fallback with the reason, never a disabled button |
| No default with several providers | Chip body opens the menu, header says "Choose a system" |
| User outside client access | Nothing renders |
| Connector declares no `external_ref` | No chip. Settings and canary still work |


## 7. S6 — access and audit

Access has three answers and the UI shows all three plainly:

- **Install and change** — admins, gated by the existing
  `special:manage_integrations`.
- **Use** — the connection's audience (`All staff` by default, or named
  roles and teams), intersected with the user's client access for
  client-scoped connections.
- **See** — a connection never widens what a user can see. A shrinkwrap
  test in the acceptance suite asserts that a user outside a client's access
  cannot discover its connection through any surface: roster counts, search,
  or a share dialog.

Audit lives in two places and both are real: the record's activity timeline
for a share or an inbound action, and the connector's own boundary log
(delivery attempts, failures, canaries) on the connection detail.

## 8. States, and the rule against dead controls

Every surface has six states and each one is drawn: `loading` (skeleton with
the final geometry), `empty` (what to do next), `error` (the vendor's
message plus a retry), `needs_attachment` (the exact missing step, with the
step), `disabled` (why, and who can undo it), `ok`.

A control that cannot act is absent, not disabled. The one exception is a
control that the user can enable by a step this surface offers, which is
disabled *with the step in its hint*.

## 9. What has to be built in `@marula-core/ui`

- `Menu` — the record and message overflow. Anchored, keyboard-navigable,
  closes on escape, traps nothing.
- `SchemaForm` — renders `settings_schema` onto the existing field
  primitives, with `writeOnly` secrets and a client-reference source.
- `StatusPill` — the four-state connector pill, driven by the same status
  vocabulary everywhere.
- `CapabilityChip` — one capability's provider set, with the primary action
  on the body and the alternatives in the menu. Its degraded and refused
  states are part of the component, not the caller's problem.
- `LinkedRecordsStrip` — the ticket's configurations with their chips and
  the link action.
- `ConfirmDialog` — the destructive confirm (remove connection, disable
  destination), already close to `Dialog`.

Four components, each used by every connector. That is the whole UI cost of
the ecosystem.

## 10. Motion and density

Shares and inbound actions are moments, not animations. A posted card
confirms in place with a 120 ms opacity step. A status change cross-fades the
pill. Nothing bounces, nothing slides from off-screen. `prefers-reduced-
motion` removes all three. Density follows `docs/theme.md`: this is staff
work inside an existing surface, so it inherits the surface's density rather
than setting its own.

## 11. Acceptance

| Surface | Test |
|---|---|
| S1 | Roster renders one card per installed connector. A connector with one failed client connection reads `Needs attention` and its action is `Fix`, not `Manage` |
| S2 | A `settings_schema` with a client reference and a `writeOnly` secret renders without a per-connector component. Saving re-reads the secret as a placeholder and never returns it |
| S3 | Share from a ticket and from a message land the same card shape. The dialog lists only the caller's permitted destinations. The permalink is returned and a repeated submit does not post twice |
| S4 | A press by a linked user with the scope takes the ticket and writes one audit row. An unlinked press answers the link step. An unpermitted press answers a typed refusal that does not disclose the ticket |
| S5 | A configuration with a NinjaOne presence and a ScreenConnect presence shows an RMM chip and a Remote chip. Opening the RMM chip resolves through the right connection for that client, and two clients on two controllers each link to their own device page |
| S5 | Two remote-access providers on one configuration: the chip body runs the org default, the menu lists both, and with no default the body opens the menu instead of choosing |
| S5 | A launch with the scope starts a session and writes one audit row. Without the scope the chip offers `Open in …` with the reason. A link with no connection renders unresolved and never a failing button |
| S5 | A ticket with two configurations shows two rows, each with its own chips, and a user outside that client's access sees the whole strip absent |
| S6 | A user without client access discovers no trace of that client's connection on any surface |
