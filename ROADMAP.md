# Roadmap

Marula is developed in the open: every change lands as a reviewed pull
request. This file says what works today, what is being built now, and where
the next work goes.

Three columns, no dates we do not own. When something ships, it moves down and
the [changelog](CHANGELOG.md) records what changed. The capability breakdown
behind this page is in the [feature matrix](docs/feature-matrix.md). The live
board is the [public roadmap](https://github.com/orgs/MarulaHQ/projects/3).

Last updated 2026-11-20.

## Shipped

- **The daily driver.** Tickets, boards with real workflows, inline notes,
  checklists, custom fields, email that opens tickets, one
  server-authoritative timer per person, missing-time detection, SLA-aware
  dispatch, scheduling, the calendar, and archiving that keeps a record
  readable after it leaves the lists and boards.
- **The field side.** Stock with locations and a movement ledger, tools and
  kits with check-out, devices on loan to clients, purchase orders with
  receiving and vendor bills, label templates printed from the desktop app,
  keyboard and camera scanning, and a Today day pack that groups stops into
  trips and names what to grab.
- **The money side.** Contracts, invoicing drawn from recorded time, credit
  notes, billing verification, quotes with a catalog, multi-currency with FX
  snapshots, the sales pipeline, approval queues, and a Stripe payment provider
  with typed decline reasons. Describe a quote or a contract in typed notes,
  speech or an uploaded document and the agent drafts it from your catalogue,
  past quotes, budget and terms, shows what it understood, and hands the draft
  to the ordinary builder. Nothing is saved until a person saves it.
- **AI assistance, gated.** AI help switched off, to shadow, or live for each
  surface and role (ticket notes, document drafts, email replies), with your
  own keys in a sealed credential store, append-only usage accounting, and an
  agent that reads your tickets through consented tools and proposes changes
  for a human to approve. A shadow run records what the model would have said
  without acting on it. The command palette carries an Ask mode that starts
  from the screen in front of you, names its sources, and files any proposal
  for approval.
- **Secrets.** The password vault with category RBAC, one-time share links,
  rotation hooks, and streaming decryption for secret files.
- **Delivery work.** Projects with a plan and a timeline, drag ordering that
  previews the dates before it writes, baselines you can measure change
  against, a quarter portfolio grouped by client, templates with a gallery and
  lessons, a task workbench with pods for time, materials and blockers, My work
  across projects, and change orders whose approval, apply and billing are
  separate acts with the client's acknowledgement in the portal.
- **Knowledge that belongs to a client.** A client hub that gathers articles,
  documents, devices, credentials, racks, networks, domains and procedures,
  one search across every store, an expiry rollup with an owner for every date,
  and a portal that shows published help and shared documentation without
  leaking a draft.
- **Integrations.** NinjaOne, Hudu, Microsoft 365, UniFi, 3CX, Slack and
  Teams, Rewst, and the QuickBooks and Xero exports ship in the core: each
  source feeds one identity-keyed record with a merge queue, so the same device
  or contact never forks into duplicates. IT Glue, Auvik, ImmyBot and Zapier are
  the first marketplace connectors, built in their own repos and published
  through the marketplace. A connector roster configures each installed
  connector as one or more scoped connections, with destinations, a health
  canary and an audience that cannot widen what a person can see.
- **The connector marketplace.** A public catalog for connectors beyond the
  standard set, with first-party and community listings installed through the
  same signed, consent-gated lifecycle as every plugin.
- **Getting started.** A first-run wizard for the org, a three-step invite
  that recommends a role and previews what it grants, a guided accept page, a
  short orientation for each new user, help on every surface that opens the
  matching documentation, guided tours, and one keyboard registry with an
  overlay and cheat sheet. The invitation email is a template an admin can
  edit without touching the flow.
- **A desktop app.** The Tauri 2 macOS shell with signed bundles and automatic
  updates, plus themes (Tangerine, Slate, a high-contrast preset) that ship
  validated from the same tokens as the web app.
- **An install you can watch.** The Marula Installer measures the machine,
  offers a recommended path or the full surface, shows the configuration as it
  changes, and writes the same `marula.env` as `marula install`. The release
  archive installs itself with `./install.sh`, and a server install can read
  its plan with `--dir` before applying it with `--apply`. The terminal and
  loopback paths stay, so a headless machine gets the same plan.
- **Docs that replace a support channel.** A documentation site organized by
  task, with guides for installing on your own machine, upgrading, backing
  up, and restoring; every page tags its audience and gets its links checked
  in CI. The same site covers the knowledge base, projects, the connector
  roster, email templates, the assistant, and the two operator recovery
  readers (dead work and object integrity).
- **The client's vote.** Quote approval from the client portal (same domain
  guard as the staff legs), satisfaction surveys on resolution, and canned
  replies with a settings page for the desk.
- **A guided merge workbench** for client configurations and a Zapier app over
  the webhook catalog.

## In flight

- **Inventory that works end to end.** A product placed on a ticket or a
  project stays workable through purchase, custody and invoicing. Buying for
  stock, the internal fleet, the rental pool, or a client is one recorded act,
  with a recharge rule of cost or markup and drop-ship where the vendor ships
  direct. Landed costs are a per-organisation choice, and each layer keeps its
  own FX date. Rentals have a pool of their own. Client e-waste has a custody
  trail and a certificate on the way out. The work is tracked in
  [#133](https://github.com/MarulaHQ/marula-core/issues/133) and on the
  [public roadmap board](https://github.com/orgs/MarulaHQ/projects/3).
- **The phone.** A React Native app on the same typed API as the web: field
  ops, camera scanning, live sync, timers, and a proper tablet layout
  ([#127](https://github.com/MarulaHQ/marula-core/issues/127)).

## Later

- **Reports and dashboards, described.** Say what you want to see, typed or
  spoken, and the agent drafts it from the views, measures, filters, charts and
  cards the product already serves: the right lens, the right chart, the cards
  sized and placed where they read best, and the delivery schedule if you named
  one. Before it hands the draft over it runs the thing against your own data
  and checks what it built: a chart with nothing to plot, a card that resolves
  to no rows, a filter nobody thought to widen. It fixes what it can, and says
  plainly what it could not. Every draft lands in the builders you already use
  as a proposal you edit and approve, under the same per-surface gate as every
  other AI help
  ([#128](https://github.com/MarulaHQ/marula-core/issues/128)).
- **Your own language.** The interface, the emails, the help text and the client
  portal read in the language the person uses. British and American English are
  two locales, not one with a spelling switch: dates, money, numbers and the
  start of the week follow the reader, so a Monday meeting is Monday in both.
  French and Spanish come next, then Mandarin and Arabic, and Arabic lays the
  interface out right to left rather than only translating it. A language ships
  once someone who speaks it has read the strings
  ([#129](https://github.com/MarulaHQ/marula-core/issues/129)).
- **A final audit.** Before the milestones above are called done, a full walk
  of every surface by someone who has not touched it before
  ([#130](https://github.com/MarulaHQ/marula-core/issues/130)).

Maintainers: this page and the [changelog](CHANGELOG.md) are maintained at
merge time per [CONTRIBUTING.md](CONTRIBUTING.md). The website builds both
pages from these files, so edits here are public edits.
