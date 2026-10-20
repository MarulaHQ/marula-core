# Glossary

The words this documentation assumes. Terms are grouped by where you meet them,
not alphabetically, because "what is a board" and "what is an optimistic write"
are asked by different people.

Each entry ends with the page that explains it properly. If a term appears in the
product and is missing here, that is a gap worth reporting through
[Get help inside Marula](../how-to/get-help.md).

## The trade

| Term | What it means |
|---|---|
| **PSA** | Professional services automation: the system an IT service business runs its tickets and its time, its contracts and its billing. Marula is one. |
| **MSP** | Managed service provider: the kind of business Marula is built for. It sells ongoing IT support to other companies. |
| **RMM** | Remote monitoring and management: the tool that watches client machines. It is a different product, and Marula can take inventory from one. See [Your first inventory sync](../tutorials/first-inventory-sync.md). |
| **Client** | A company you support. Its people, devices, contracts and portal accounts belong to it. |
| **Contact** | A person at a client. A contact can be given a portal account; staff are not contacts. |
| **Ticket** | One piece of work. It carries a client and a board, notes and time, and usually a status. |
| **Board** | The set of columns a ticket moves through. Columns are yours to rename; the **stage** underneath is what SLA, billing, automation and reporting actually bind to. See [Customizations are data](../explanation/customizations-are-data.md). |
| **Dispatch** | Assigning unscheduled work to a technician's day, usually by dragging on a calendar. See [Dispatch from the board](../how-to/staff-dispatch.md). |
| **Day pack** | The list of parts and tools a technician needs for today's trips, and where each item comes from. See [Work a day pack](../how-to/staff-day-pack.md). |
| **Workbar** | The panel on a ticket where notes and tasks are entered, along with time. |
| **CSAT** | The satisfaction survey a client answers after a ticket closes. See [Set up CSAT surveys](../how-to/admin-csat-surveys.md). |

## Contracts, time and money

| Term | What it means |
|---|---|
| **Contract** | The agreement that decides what a client is charged. Marula models six kinds: recurring, retainer, time and materials, fixed fee, per unit, per incident. See [Contract relationship types](../explanation/contract-relationship-types.md). |
| **Retainer / prepaid block** | A retainer is a per-period base that includes minutes; a prepaid block is a balance sold up front and drawn down by approved work, oldest block first. Blocks ride time-and-materials and retainer contracts only. |
| **Cap / minimum** | The ceiling or floor on what a period bills, before escalation or credits apply. |
| **Change order** | An agreed change to a contract's scope or price, recorded rather than edited into the old terms. |
| **Service credit / SLA credit** | Money given back when the service missed its promise. It is a credit, not an edit to an issued invoice. |
| **Work type category** | The label on a time entry that decides whether it bills and at what rate. See [Log billable time with categories](../how-to/staff-time-categories.md). |
| **Billing verification** | The sign-off step that walks every hour in a window before invoices are issued. See [Verify a billing period](../how-to/admin-billing-verification.md). |
| **Invoice** | The issued bill. Once issued it locks; it is not edited afterwards. |
| **Credit note** | The document that corrects an issued invoice. See [Billing corrections](../explanation/billing-corrections.md). |
| **FX rate book** | The append-only record of exchange rates. Reconciliation totals are grouped by document type, never summed across them. See [FX additivity](../explanation/fx-additivity.md). |
| **Project / phase / milestone** | Project work is tracked in phases and released for invoicing by milestone, so it does not need a dummy ticket. See [Bill project work](../how-to/project-billing.md). |

## Access, records and history

| Term | What it means |
|---|---|
| **Org** | Your workspace. Everything is scoped to it: people, clients, tickets, settings. One instance can hold one org. |
| **Instance** | The running deployment: one core API process, a worker, a database, and a cache. |
| **Provenance** | The field-level record of who changed what value, from what to what. Actor kinds are `human`, `agent` and `import`. See [Provenance vs audit](../explanation/provenance-vs-audit.md). |
| **Audit** | The event-level record of security and administrative actions: sign-ins, vault reveals, consent grants, billing sign-offs. |
| **Actor kind** | Whether an edit came from a person, an AI agent run, or an import. Every provenance row carries one. |
| **Portal account** | A login for a client's contact. It is fenced to that client and never carries staff access. See [The portal isolation model](../explanation/portal-isolation.md). |
| **Board exposure** | The admin setting that decides which boards a portal account can see at all. |
| **Restricted category chain** | The permission shape shared by knowledge-base folders and vault categories: access is granted on a folder chain, and an item you were not granted is absent from the list and from search rather than shown locked. See [Knowledge-base categories and the secret vault](../explanation/kb-class-categories.md). |
| **Grant** | Permission given to something optional, such as an AI helper or a plugin capability. Nothing is granted by default. |
| **Deny-by-default** | The rule that an ungranted capability is invisible rather than refused after the attempt. See [AI: deny-by-default](../explanation/ai-deny-by-default.md). |

## The product's own vocabulary

| Term | What it means |
|---|---|
| **Instant-first** | The interaction rule: writes apply immediately and can be undone, with no Save button to chase. See [Instant-first interaction](../explanation/instant-first.md). |
| **Optimistic write** | A write the client applies before the server confirms it. It carries a version so a conflict is detected rather than overwritten. |
| **Surface** | One screen of the product, such as the ticket list or the FX rate book. The in-app help panel is organized by surface. |
| **Read model** | The stable reporting views (`ops.v1_*`) that reports and BI tools read instead of the write tables. See [The read model view catalogue](read-model.md). |
| **Customization manifest** | The versioned export of everything an org has customized. An upgrade preflight reads it. See [Customize without code](../how-to/customize.md). |
| **Agent** | An AI helper that proposes work and runs only after a person approves the plan. See [Your first agent run](../tutorials/first-agent-run.md). |
| **Agent run** | One execution of an agent, with its plan, approvals and ledger entries kept together. |
| **BYOK** | Bring your own key: the org supplies model provider credentials instead of using a hosted default. See [Bring up AI with your own API keys](../how-to/admin-ai-bringup.md). |
| **Plugin** | An add-on distributed as a signed artifact, with a declared capability list and a consent screen. See [Configure the plugin consent flow](../how-to/admin-plugin-consent.md). |
| **Capability** | One thing a plugin may do, declared up front. The declared list is what the person installing it consents to. |
| **Digest** | The hash that pins an artifact's contents. A digest mismatch stops an install. |
| **Connector** | A first-party link to an outside system (an RMM, DNS, accounting). A plugin is a wider thing than a connector. |

## Running it

| Term | What it means |
|---|---|
| **Self-host** | You run Marula on infrastructure you control, on a database you own. |
| **Migration owner vs app role** | The database uses two roles: one that may change the schema during a migration, and one that the application runs as, under row-level security. See [The local stack](self-host-local-stack.md). |
| **N−2 window** | You can upgrade from either of the two previous tagged releases, not from any older one. See [The upgrade compatibility policy](../explanation/upgrade-policy.md). |
| **Expand-then-contract** | A migration adds the new shape, both shapes work, then the old one is removed in a later release. It is why an upgrade can be rolled back. |
| **Digest-pinned image** | A container reference that names both a tag and a digest. The digest is what runs. |
| **Live leg** | The streamed connection that carries live updates to an open page. It has to survive the proxy without buffering. See [Reverse proxy and TLS](reverse-proxy-tls.md). |
| **Quarantine** | Where the optional malware scanner puts a message it will not deliver. See [Watch your self-hosted services](../how-to/admin-self-host-services.md). |
