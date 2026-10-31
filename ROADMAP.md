# Roadmap

Marula is developed in the open: every change lands as a reviewed pull
request. This file says what works today, what is being built now, and where
the next work goes.

Three columns, no dates we do not own. When something ships, it moves down and
the [changelog](CHANGELOG.md) records what changed.

## Shipped

- **The daily driver.** Tickets, boards with real workflows, inline notes and
  custom fields, email that opens tickets, one server-authoritative timer per
  person, missing-time detection, SLA-aware dispatch, scheduling, and the
  calendar.
- **The money side.** Contracts, invoicing drawn from recorded time, quotes
  with a catalog, multi-currency with FX snapshots, the sales pipeline,
  approval queues, and a Stripe payment provider with typed decline reasons.
- **AI assistance, gated.** AI help, turned on where admins allow it (ticket
  notes, document drafts, email replies), per surface and per role, with your
  own keys in a sealed
  credential store, append-only usage accounting, and an agent that reads your
  tickets through consented tools and proposes changes for a human to approve.
- **Secrets.** The password vault with category RBAC, one-time share links,
  rotation hooks, and streaming decryption for secret files.
- **Integrations.** NinjaOne, Hudu, Microsoft 365, UniFi, 3CX, Slack and
  Teams, Rewst, and the QuickBooks and Xero exports ship in the core: each
  source feeds one identity-keyed record with a merge queue, so the same device
  or contact never forks into duplicates. IT Glue, Auvik, ImmyBot and Zapier are
  the first marketplace connectors, built in their own repos and published
  through the marketplace.
- **A desktop app.** The Tauri 2 macOS shell with signed bundles and automatic
  updates, plus themes (Tangerine, Slate, a high-contrast preset) that ship
  validated from the same tokens as the web app.
- **Docs that replace a support channel.** A documentation site organized by
  task, with guides for installing on your own machine, upgrading, backing
  up, and restoring; every page tags its audience and gets its links checked
  in CI.
- **The client's vote.** Quote approval from the client portal (same domain
  guard as the staff legs), satisfaction surveys on resolution, and canned
  replies with a settings page for the desk.
- **A guided merge workbench** for client configurations and a Zapier app over
  the webhook catalog.

## In flight

- **Onboarding.** A first-run wizard for the org and a short orientation for
  each new user, skippable at every step and switchable off org-wide.
- **Help, in the product.** A help affordance on every surface, showing the
  documentation that matches, plus short guided tours on first visit.
- **A keyboard standard.** One command registry so the same verb carries the
  same chord on every surface, an overlay and cheat sheet that read it, and a
  deeper accessibility pass across every shipped surface.
- **Purchasing and stock.** Purchase orders with reseller deeplinks, stock
  with locations and a movement ledger, tools with check-out, devices on loan
  to clients, label printing, keyboard-wedge scanning, and a Today grab list
  that understands a van run.
- **The connector marketplace.** A public catalog for connectors beyond the
  standard set: first-party listings and community ones, installed through
  the same signed, consent-gated lifecycle as every plugin.
- **The phone.** A React Native app on the same typed API as the web: field
  ops, camera scanning, live sync, timers, and a proper tablet layout.

## Later

- **Commercial intelligence.** Describe a new quote or contract in plain
  language, or drop in the meeting notes from the call, and the agent drafts
  the document from the catalog, pricing and terms you already keep, under the
  same per-surface gate as every other AI help. Drafts are proposals: edited
  and approved by a person before anything reaches a client.
- **Reports and dashboards, described.** Say what you want to see, typed or
  spoken, and the agent drafts it from the views, measures, filters, charts and
  cards the product already serves: the right lens, the right chart, the cards
  sized and placed where they read best, and the delivery schedule if you named
  one. Before it hands the draft over it runs the thing against your own data
  and checks what it built: a chart with nothing to plot, a card that resolves
  to no rows, a filter nobody thought to widen. It fixes what it can, and says
  plainly what it could not. Every draft lands in the builders you already use
  as a proposal you edit and approve, under the same per-surface gate as every
  other AI help.
- **More of the plugin surface.** UI widgets, automation steps, and report
  renderers for plugins, with first-party reference plugins in the repo.
- **Email identities.** Per-board inbound and outbound addresses, a
  settings surface for the whole mail system, and a branded notification
  wrapper.
- **A second wave of product pages on this website.**
- **A final audit.** Before the milestones above are called done, a
  full walk of every surface by someone who has not touched it before.

Maintainers: this page and the [changelog](CHANGELOG.md) are maintained at
merge time per [CONTRIBUTING.md](CONTRIBUTING.md). The website builds both
pages from these files, so edits here are public edits.
