# Changelog

All notable changes to Marula are recorded here, most recent first.
Each bullet says what changed for the person using the product, not how the
work was organized. This file follows the
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, and versions
follow [semantic versioning](https://semver.org/spec/v2.0.0.html). It is the
single source for the changelog on [getmarula.com/changelog](https://getmarula.com/changelog):
the website builds its page and feed from this file, so the two never drift.

Every change lands as a reviewed pull request. Contributions follow
[CONTRIBUTING.md](CONTRIBUTING.md).

## [Unreleased]

## [0.6.1] - 2026-11-20

### Fixed

- The release's air-gapped bundle builds again, so a machine with no network can install from it.

## [0.6.0] - 2026-11-20

### Added

- The release archive now installs itself. Unpack it, run ./install.sh, and the installer checks the archive against its own checksums before it starts.
- On a server, pass --dir to install into a service directory and read the plan before anything changes, then add --apply.

### Changed

- The split and demo install shapes now dial Postgres directly, and the connection pooler is a profile you turn on when instances multiply rather than a service every install starts.

## [0.5.0] - 2026-11-19

### Added

- A tagged release publishes the four application images to GHCR and records their digests in the manifest, so an install from a release bundle can pin the images that were published instead of refusing for a missing digest.
- Hosted and automated deployments can pull the Marula Core Helm chart from the release registry. Each release also publishes one machine-readable file naming every image it built by digest, so a deployment pins exactly what shipped.

### Fixed

- The image prefix the CLI and the installer suggest is now `ghcr.io/marulahq/marula`; the previous one named a registry namespace that does not exist, so copying it failed on the first pull.

## [0.4.0] - 2026-11-17

### Added

- One AI control with three states. **Off** runs nothing, **shadow** runs the
  models and records what they would have said on the shadow page, and **live**
  lets each surface act on its own mode.
- Tickets, sites and documents can be archived and restored. Archived records
  stay readable, leave the lists, boards and pickers, the search results and the
  portal, and link strips stop pointing at them.
- Quote lines can be marked optional: priced and shown, outside the subtotal
  and the tax, and refused as the only line on a convertible quote.
- The knowledge home has a client scope picker, and search applies that scope
  on the server before the result limit.
- A project pause can carry a reason the client reads, and resuming returns the
  project to the status it held before the pause.

### Changed

- Taxability from a proposed quote change now survives the change instead of
  returning to the catalogue default.
- Money fields on change orders, billing releases and phase burn read as
  hidden rather than zero for people without financial access.
- Template-created projects check the billing contract in the same transaction
  that creates them, so a contract cannot expire between the check and the
  insert.

### Fixed

- The invoice run opens with both ends of its window filled, so Preview and
  issuing invoices work on first open instead of refusing a missing date.

### Security

- A connector's webhook destination is screened before it is stored and again
  before it is sent, so an installed consent can never become a request to the
  host the API runs on.
- The token that carries a sign-in from the password to the second factor rides
  an httpOnly cookie instead of browser storage a script on the page can read.

## [0.3.8] - 2026-11-17

### Added

- Guided tours on each surface, with the spotlight moving by transform only and
  every exit reversing before the page changes.
- AI setup inside the operator's onboarding.

### Security

- A restricted client now also gates tickets, assets and racks, plus credentials and
  knowledge search, and a stock movement asks for the `stock:move` grant instead
  of none. A write that moves money requires financial read access.

## [0.3.7] - 2026-11-16

### Added

- A decision surface says what it decides instead of citing a reference, Today
  names the gap in yesterday's time, and Tickets offers a changed-since-you-looked
  shortcut.
- Last contact with a client is derived from email, portal replies and calls,
  with the trail it came from.

### Changed

- The command palette opens at the pack's width with grouped results.

### Fixed

- The AI shadow page says AI is off instead of showing an empty picker.

## [0.3.6] - 2026-11-15

### Added

- A project plan: phases, tasks and dates, with the schedule engine that places
  the work on working days and calendars.
- Reorder phases and tasks, and move a task into another phase, in one save.
- Describe a quote or a contract change in plain English and get the draft back.

### Changed

- Today leads the rail, sections open in place, and long-tail pages become tabs
  on one screen.

## [0.3.5] - 2026-11-13

### Fixed

- A portal address shared across tenants resolves to the tenant that asked
  instead of to nothing.
- Small controls keep a 32px pointer target at the dense setting, including
  chips and segmented controls.
- A field holding a wrong value draws the same destructive edge on the sign-in
  and portal screens as everywhere else.
- The desktop build starts on macOS 27: dependency artifacts are no longer
  stripped into a form the dynamic loader refuses.

## [0.3.4] - 2026-11-12

### Added

- Install Marula from the download page: the install guide for each platform,
  the pointer that sends a visitor to the right build, and signed installers
  produced when the signing material is present.
- Onboarding by invitation: the invitation email, a guided accept flow, the
  role chosen while accepting, and mail templates an administrator can edit.

### Changed

- The shell navigation bar is back to the composition the design pack draws, so
  the same destinations sit in the same place at every width.

## [0.3.3] - 2026-11-05

### Added

- Describe a report in plain English on the reports page or your dashboard. The
  draft comes back with a sentence saying what it will show and a list of what
  changed against your current draft. Accepting saves it. Editing opens the
  builder. Rejecting discards it.
- Every draft is checked against your own data before it is offered. A chart
  with no rows, a table without headers, a stat without a number, or a window
  wider than its refresh is caught and repaired when the model can repair it.
  A failure that survives is named plainly on the draft.
- Describe a schedule the same way: "every Monday at 09:00" becomes the fields
  the schedule form takes. A phrase the schedule shape cannot carry is refused
  with the part it could not read, instead of being guessed at.
- Dashboard cards take their width and height from the report catalogue, and a
  saved report can be placed on a dashboard with the card it was authored with.
- A saved report shows what it will run with, on the saved row and on the
  dashboard card: its window and refresh interval, beside the size it draws at.
- An MCP client is offered only the tools its own account can call, rather than
  the whole catalogue with a refusal waiting at call time.
- The desktop client ships on Linux and Windows as the connected client,
  beside the macOS app.
- Loading surfaces use the kit's skeleton composer instead of a hand-rolled
  splash.

### Changed

- A report definition stores the card it was authored with, so accepting a
  draft keeps its window, refresh and size, and editing a saved report no
  longer clears them.
- The reports page, the dashboards page and the authoring route share one
  describe-it panel.

### Fixed

- A stat card drawn from real data came back as having no value. The run wrote
  its total under one name and the card looked for another.
- The schedules form could not create a weekly or monthly schedule at all,
  because it had no day field. It also called its time UTC while sending the
  browser's own zone.
- Saving a dashboard arrangement failed, because the settings key the page
  writes was never declared.
- A dashboard card whose saved report was deleted, or moved out of your reach,
  showed a hole. It now states why and offers to remove the card.
- The authoring page showed an AI refusal without the reason for it or the
  remedy.

### Removed

- `timers:read` and `timers:write` can no longer be written onto an API key. A
  timer is the signed-in person's own work, and a key has no person behind it:
  the key authenticated and the endpoint then refused it, so the pair described
  access that could never happen. Keys minted before this keep working and lose
  only that grant. Timers stay available to a signed-in session, where they
  belong.

## [0.3.2] - 2026-11-04

### Added

- An install can accept a signed entitlement file from the provider that sells
  you a hosted plan. Marula verifies the signature, stores the document and
  reports the revision it holds, so the two sides can agree on what was bought.
  Marula does not read the file to decide what a screen may do.
- The Marula Installer, a desktop app that walks five decisions and writes the
  same `marula.env` as the command line. It measures the machine first, shows
  the configuration as it changes, and offers a recommended path of seven
  questions or the full surface for an operator who wants every parameter.
- `marula install --ui` serves that wizard over loopback for a machine where
  the app cannot be installed, and `marula install` prints the same plan as
  text. All three use one capacity check, so the recommendation cannot differ
  between them.
- An install can pull the published container images. The release manifest can
  carry their digests, the pipeline says when it does not, and a job publishes
  them for both architectures.
- A guide to checking a release before installing it: the signed checksum list,
  the archive, the extracted tree, and the image digests.
- A demo tier the installer can stand up. `marula install --profile demo` and a
  card in the wizard run the documented demo overlay (seeded, loopback-only, no
  outbound mail), and `marula demo reset` replaces the demo data between
  showings.

### Changed

- The desktop release channel has a publisher, so an installed shell can be
  offered a new build from the repository instead of a hand-copied download.
- The install guide covers both ways in, the app and the terminal, and points
  at the deployment shapes and the floors each one asks for.

### Fixed

- An install that pulls the published images now pins the migration image too.
  It was left out of the override, so the one-shot that runs the schema
  migration built itself from source on the target — which fails outright on a
  release bundle, where the sources are not shipped.
- The ticket workbench keeps its status and SLA cells readable in a narrow
  window.
- The Today day strip keeps its bound labels inside the card.
- The rail toggle responds to a click again, and the top bar spans the window
  with the rail below it.

## [0.3.1] - 2026-10-27

### Fixed

- The published API document reports the release it was built from. It used to
  carry a version string no release ever matched, which left a client pinning a
  release with two version identities to reconcile.

## [0.3.0] - 2026-10-27

The working product: a ticket workbench built for daily use, AI assistance
with consent and an audit trail, an ecosystem layer, and a plugin runtime
that runs untrusted code inside limits.

### Added

- A ticket workbench built for real work: three panes with inline notes,
  a time clock beside the work, scheduling and dispatch, editable contact
  fields, instant saves, and a layout that stacks into one column on narrow
  windows.
- AI assistance wherever admins grant it: a note inside a ticket, a drafting
  pass on a document, an email reply. The catalog behind it holds your own
  org or personal API keys in a sealed credential store, append-only usage
  records with org and personal caps, and per-surface model picks that stick.
- An AI agent that reads your tickets and knowledge base through scoped,
  consent-gated tools and proposes record changes for human approval. Every
  model call and tool call lands in the usage ledger and the audit log.
- Suggestion surfaces for triage, dispatch, alerts, contract scope, billing
  audits, and record matching. They stay suggestions: they apply only
  above per-surface thresholds you configure, and nothing acts when no decision
  model is configured.
- Theme system outside the default: a Tangerine start plus Slate and
  high-contrast presets, per-user override, and custom themes installed from
  theme packs that are validated so a pack can recolor the product without
  breaking its typography or motion rules.
- Guided merge workbench for client configurations: side-by-side source
  comparison, field-by-field decisions with one click on confident merges,
  batch actions, and contact and client dedupe across feeds like NinjaOne,
  IT Glue, Hudu, or Microsoft 365.
- New connectors and apps: a Zapier app over the webhook catalog, UniFi
  controller inventory, and 3CX call journals with click-to-call from tickets.
- An ecosystem layer: marketplace with ratings and verified publishers, a
  Rewst bridge plus Slack and Teams apps, a VoIP connector API, distributor
  pricing feeds, an advanced custom-roles editor, and a Stripe payment
  provider with typed decline reasons.
- The Tauri 2 desktop app for macOS with a signed bundle, automatic updates,
  and app icons regenerated from the brand source; a release and notarization
  runbook ships with the repo.
- This website's self-host guide, and a documentation site that covers the
  product: howtos organized by task for admins and daily staff, tutorials that
  carry a first-timer through a first ticket, a first inventory sync, a first
  invoice, and a first agent run, and short explanations for why things work
  the way they do. Every page
  is tagged by audience and link-checked.
- A public roadmap and changelog that both build from the files in the
  repository, with an RSS feed for the changelog.

- A plugin runtime: each plugin runs in an isolated worker where its spend is
  bounded, keeps its own data in a per-install key/value store, and reaches the
  network only through hosts its manifest declares and an admin consented to.
  Installs start disabled, a grant can never exceed what the manifest asks for,
  three worker failures in a row quarantine the install until an operator
  clears it, and every attempt lands one audit row whether it ran or was
  refused.

### Changed

- Every app area, from tickets to the client portal, passed a surface-by-surface
  design review in real browsing sessions: light and dark parity, WCAG 2.2 AA
  on the core flows, subtle motion that respects `prefers-reduced-motion`, and
  white-label portal branding.
- The design system is enforced by lint on every build: raw colors and
  off-contract styles fail CI before they ship.
- Documentation moved to a deterministic publishing pipeline: one renderer,
  byte-identical output, verified links, published straight from the markdown
  in this repository.
- Behind a proxy, the API now identifies the calling client from the address
  the connection actually came from, and only trusts `X-Forwarded-For` or
  `CF-Connecting-IP` from a proxy you declared with `TRUSTED_PROXIES` (default:
  loopback and private networks). A caller that reaches the API directly can no
  longer pick the address its throttling and audit entries are recorded under.
  A deploy that is reachable from the internet but still uses that default logs
  a warning at startup naming the fix; the reverse-proxy runbook carries the
  Cloudflare recipe. Rate limits and locks still hang off the account, not the
  address — this changes whose name is on the record, not what the record
  blocks.

### Security

- Password vault hardening: per-org write budgets that survive restarts,
  paced bulk ingestion, chunked streaming decryption for secret files,
  one-time share links, rotation reminders and automation hooks on passwords,
  and improved password-side link management.
- Code signing for plugins and skills with key rotation support, junior-role
  default grants on restricted chains, and a `vault:ingress` ciphertext-only
  plugin capability.
- A hardening pass to external-audit readiness: fail-closed realtime auth, and
  an envelope-encryption audit over every ciphertext surface in the product.

## [0.2.0] - 2026-09-03

The commercial engine and the platform underneath it, per the original build
plan.

### Added

- Contracts, invoicing drawn from recorded time, quotes with catalog lines,
  multi-currency with FX snapshots, the sales pipeline, approval queues, and a
  Stripe-class payment seam.
- Boards with real workflows, structured ticket notes, custom fields, email
  that opens tickets by parsing inbound messages, one server-authoritative
  timer per person, and missing-time detection.
- SLA-aware dispatch, scheduling, calendar sync, and the scoped client portal
  with invoices and knowledge base.
- The reporting engine: fixed-vertical-row specs, scorecards, and a report
  builder with keyboard-navigable tables beside every chart.
- Knowledge base with category trees, password vault with per-category RBAC,
  documents with immutable versions.
- Client asset lifecycle and configuration items with identity keys, dedupe
  candidates, and a merge queue that replays cleanly across sources.

## [0.1.0] - 2026-08-26

Stage 2, the daily driver: the open-core PSA an MSP runs on.

### Added

- Tickets, boards, structured notes, custom fields, and client records.
- Email-to-ticket with threading and per-channel notification preferences.
- Time entries, the workbar, one timer per person, and missing-time detection.
- Dispatch with SLA awareness and scheduling.
- The client portal with tickets, invoices, and the knowledge base.

[0.5.0]: https://github.com/MarulaHQ/marula-core/compare/v0.4.0...v0.5.0
[0.6.0]: https://github.com/MarulaHQ/marula-core/compare/v0.5.0...v0.6.0
[0.6.1]: https://github.com/MarulaHQ/marula-core/compare/v0.6.0...v0.6.1
[Unreleased]: https://github.com/MarulaHQ/marula-core/compare/v0.6.1...HEAD
[0.4.0]: https://github.com/MarulaHQ/marula-core/compare/v0.3.8...v0.4.0
[0.3.8]: https://github.com/MarulaHQ/marula-core/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/MarulaHQ/marula-core/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/MarulaHQ/marula-core/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/MarulaHQ/marula-core/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/MarulaHQ/marula-core/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/MarulaHQ/marula-core/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/MarulaHQ/marula-core/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/MarulaHQ/marula-core/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/MarulaHQ/marula-core/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/MarulaHQ/marula-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/MarulaHQ/marula-core/releases/tag/v0.1.0
