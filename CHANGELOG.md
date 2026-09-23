# Changelog

All notable changes to Meridian are recorded here, most recent first.
Each bullet says what changed for the person using the product, not how the
work was organized. This file follows the
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, and versions
follow [semantic versioning](https://semver.org/spec/v2.0.0.html). It is the
single source for the changelog on [meridian.dev/changelog](https://meridian.dev/changelog):
the website builds its page and feed from this file, so what you read there is
exactly what the repository commits.

Every change lands as a reviewed pull request. Contributions follow
[CONTRIBUTING.md](CONTRIBUTING.md).

## [Unreleased]

### Added

- A ticket workbench you can actually work in: three panes with inline notes,
  a time clock beside the work, scheduling and dispatch, editable contact
  fields, instant saves, and a layout that stacks into one column on narrow
  windows.
- AI assistance wherever admins grant it: a note inside a ticket, a drafting
  pass on a document, an email reply. Underneath sits a model catalog, your
  own org or personal API keys in a sealed credential store, append-only usage
  records with org and personal caps, and per-surface model picks that stick.
- An AI agent that reads your tickets and knowledge base through scoped,
  consent-gated tools and proposes record changes for human approval. Every
  model call and tool call lands in the usage ledger and the audit log.
- Suggestion surfaces for triage, dispatch, alerts, contract scope, billing
  audits, and record matching. Suggestions stay suggestions: they apply only
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
- This website's self-host guide, and a documentation site that finally has
  depth: howtos organized by task for admins and daily staff, tutorials that
  carry a first-timer through a first ticket, a first inventory sync, a first
  invoice, and a first agent run, and short explanations for why things work
  the way they do. Every page
  is tagged by audience and link-checked.
- A public roadmap and changelog that both build from the files in the
  repository, with an RSS feed for the changelog.

### Changed

- Every app area, from tickets to the client portal, passed a surface-by-surface
  design review in real browsing sessions: light and dark parity, WCAG 2.2 AA
  on the core flows, subtle motion that respects `prefers-reduced-motion`, and
  white-label portal branding.
- The design system is enforced by lint on every build: raw colors and
  off-contract styles fail CI, so the interface you see is the interface the
  design system describes.
- Documentation moved to a deterministic publishing pipeline: one renderer,
  byte-identical output, verified links, published straight from the markdown
  in this repository.

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

Stage 2, the daily driver: the open-core PSA core an MSP runs on.

### Added

- Tickets, boards, structured notes, custom fields, and client records.
- Email-to-ticket with threading and per-channel notification preferences.
- Time entries, the workbar, one timer per person, and missing-time detection.
- Dispatch with SLA awareness and scheduling.
- The client portal with tickets, invoices, and the knowledge base.

[Unreleased]: https://github.com/Marlin-Hofmeyer/meridian/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Marlin-Hofmeyer/meridian/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Marlin-Hofmeyer/meridian/releases/tag/v0.1.0
