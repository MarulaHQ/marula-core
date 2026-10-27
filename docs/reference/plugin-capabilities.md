# Plugin capability catalogue

**Source:** `crates/core-domain/src/entities/plugins.rs:184` (the closed vocabulary) and
`:275` (the permission mapping)
**Related:** [plugin-host-functions.md](plugin-host-functions.md), [plugin-pdk.md](plugin-pdk.md)

A plugin declares the capabilities it may be granted, and an admin grants a subset at install time. The
vocabulary is closed: a manifest naming a key the core does not define cannot exist, because the
validator rejects it at ingest and the grant table rejects it at write.

Every key resolves onto a permission the core already understands. There is no second
authorization system, and no key invents authority beyond the gate it lands on. Authorization for a
plugin acting under a grant is the same `policy::check` a human passes, with these resolved
permissions in the granted set.

## The three groups

The keys come in three shapes, and the shape predicts what a grant can do.

| Shape | What a grant licenses | Example |
| --- | --- | --- |
| **Entity read or write** | the named module's view or edit gate | `tickets.read` → `tickets:view` |
| **Surface participation** | registering a surface the core renders or executes, behind the gate that surface already sits behind | `ui.widgets` → `reporting:view` |
| **Declaration only** | naming a kind, source or scope the core then governs itself | `automation.triggers` → `plugins:view` |

A declaration-only key never confers an action. `automation.reads` lets a manifest declare a read
node; the node's own execution carries its own permission checks. `service_accounts.manage` lets a
runtime declare a service-account surface, while minting the key stays a human admin action.

## The catalogue

Thirty-five keys, in declaration order.

| Key | Resolves to | What it licenses |
| --- | --- | --- |
| `tickets.read` | matrix `tickets:view` | read tickets and board rows |
| `tickets.write` | matrix `tickets:edit` | edit tickets, including automation actions |
| `clients.read` | matrix `clients:view` | read clients |
| `clients.write` | matrix `clients:edit` | edit clients |
| `time.read` | matrix `time:view` | read timers and time entries |
| `time.write` | matrix `time:edit` | edit time entries |
| `projects.read` | matrix `projects:view` | read projects |
| `projects.write` | matrix `projects:edit` | edit projects |
| `kb.read` | matrix `kb:view` | read knowledge base articles |
| `kb.write` | matrix `kb:edit` | write knowledge base articles |
| `documents.read` | matrix `documents:view` | read documents |
| `documents.write` | matrix `documents:edit` | write documents |
| `reporting.read` | matrix `reporting:view` | read reporting data |
| `financial.read` | `ViewFinancials` | the financial-visibility gate (cost and contract rates, margin) |
| `vault:ingress` | matrix `secrets:create` | submit ciphertext-only payloads into a scoped vault folder |
| `routing.matrix` | matrix `scheduling:view` | routing matrix lookups |
| `routing.route` | matrix `scheduling:view` | route lookups |
| `routing.geocode` | matrix `scheduling:view` | geocoding |
| `routing.isochrone` | matrix `scheduling:view` | isochrone lookups |
| `ui.widgets` | matrix `reporting:view` | register a ticket-pane or dashboard widget |
| `automation.actions` | matrix `tickets:edit` | register an automation action kind |
| `reports.render` | matrix `reporting:view` | register a report render kind |
| `fields.validate` | matrix `tickets:view` | register a custom-field validator |
| `automation.triggers` | matrix `plugins:view` | declare an automation trigger source |
| `automation.reads` | matrix `plugins:view` | declare a read node, and call `http_request` for a read |
| `events.emit` | matrix `plugins:view` | declare event types the plugin may publish |
| `service_accounts.manage` | matrix `plugins:view` | declare a service-account surface |
| `org_usage.read` | `ViewOrgUsage` | read usage counts and quota settings |
| `rag.read` | matrix `kb:view` | read retrieval-augmented context |
| `rag.write` | matrix `kb:edit` | write retrieval-augmented context |
| `org_custom_domain.manage` | `ManageOrgCustomDomain` | declare a custom-domain surface |
| `org_suspension.read` | `ViewOrgSuspension` | read suspension state |
| `observability.read` | `ViewObservability` | read the audit trail and observability data, and nothing else |
| `directory_groups.read` | `ViewDirectoryGroups` | read groups and their members |
| `directory_groups.write` | `ManageDirectoryGroups` | write groups and their members |

The `vault:ingress` key keeps the ADR's literal spelling with a colon rather than a dot, because
ADR 0006 names the scope that way and the vocabulary is a wire contract.

## Which capability a surface needs

A surface registers under exactly one capability, and the mapping is closed in code
(`crates/plugin-host/src/surface.rs:313`). A caller cannot ask for a different one, and the
invocation path checks the install holds that exact key.

| Surface (`plugin_surface_entries.surface`) | Capability |
| --- | --- |
| `widget` | `ui.widgets` |
| `automation_action` | `automation.actions` |
| `report_render` | `reports.render` |
| `field_validator` | `fields.validate` |
| `automation_trigger` | `automation.triggers` |
| `read` | `automation.reads` |

A plugin whose manifest declares `ui.widgets` but whose install was granted nothing registers no
surface: entries exist only while the install is enabled and the grant stands.

## Which capability a host call needs

| Entry | Capability |
| --- | --- |
| `log`, `now`, `kv_get`, `kv_put`, `kv_delete`, `kv_list` | none; the install's own scope is the grant |
| `http_request` with `GET`, `HEAD` or `OPTIONS` | `automation.reads` |
| `http_request` with any other method | `automation.actions` |
| `emit_event` | `events.emit`, plus the type must be declared in the manifest |
| `read_model_query` | the capability the query id names, below |

The `http_request` method decides the capability, read from the parsed request rather than from a
claim the guest makes. A write is anything that is not `GET`, `HEAD` or `OPTIONS`.

## The four model queries

`read_model_query` takes an id, never SQL. Two properties follow from that: a guest cannot invent a
query, and an id outside the table is refused before any capability question is asked, so a guest
cannot map an install's grants by varying the id.

| Id | Query | Capability |
| --- | --- | --- |
| 1 | `ticket.by_id` | `tickets.read` |
| 2 | `board.list` | `tickets.read` |
| 3 | `client.by_id` | `clients.read` |
| 4 | `contact.by_client` | `clients.read` |

The table is append-only: an id is never reused. The fetch behind each id is not implemented in
this build, so a fully granted call still answers `plugin_not_implemented` (see
[plugin-host-functions.md](plugin-host-functions.md#the-two-deferred-entries-refuse-today)).

## What a grant cannot do

- **It cannot exceed the manifest.** A grant outside the manifest's declared capability list is
  rejected at write time, so an admin cannot widen a plugin past what its author declared and
  signed.
- **It cannot widen at runtime.** The frame the worker receives carries the install's granted set.
  A payload that names capabilities is bytes. The grant set comes from the install row and nowhere
  else, and the conformance battery asserts it
  (`crates/plugin-host/tests/escalation.rs`).
- **It cannot outlive the install.** A revoked or disabled install registers no surface and runs
  nothing, and a revoke is terminal for every other lifecycle verb.
- **It cannot add a permission.** Every key maps onto a permission that already exists for a human.
  Adding one is a vocabulary decision, not a shortcut onto an admin special.
