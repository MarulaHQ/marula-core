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

[Unreleased]: https://github.com/MarulaHQ/marula-core/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/MarulaHQ/marula-core/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/MarulaHQ/marula-core/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/MarulaHQ/marula-core/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/MarulaHQ/marula-core/releases/tag/v0.1.0
