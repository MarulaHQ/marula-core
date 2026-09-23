# Roadmap

Meridian is developed in the open: the work lives in this repository's tracked
task list and lands as reviewed pull requests. This file answers the question
behind it: what works today, what is being built now, and where the next work
goes.

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
- **Integrations.** NinjaOne, Hudu, IT Glue, ImmyBot, Microsoft
  365, Auvik, UniFi, 3CX, QuickBooks and Xero export: each source feeds one
  identity-keyed record with a merge queue, so the same device or
  contact never forks into duplicates.
- **A desktop app.** The Tauri 2 macOS shell with signed bundles and automatic
  updates, plus themes (Tangerine, Slate, a high-contrast preset) that ship
  validated from the same tokens as the web app.
- **Docs that replace a support channel.** A documentation site organized by
  task, with guides for installing on your own machine, upgrading, backing
  up, and restoring; every page tags its audience and gets its links checked
  in CI.
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
- **The phone.** A React Native app on the same typed API as the web: field
  ops, camera scanning, live sync, timers, and a proper tablet layout.

## Later

- **Client-facing finishing holes.** Quote approval from the client portal,
  satisfaction surveys on resolution, canned replies for ticket work.
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
