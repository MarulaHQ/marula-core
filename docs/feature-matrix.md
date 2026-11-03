# Feature matrix

What Marula Core does today, what is built and waiting for a release, and what
is planned. Every row is checked against the code, not against a plan.

**Status**

- **Shipped** means the capability is in a tagged `0.3.x` release.
- **Built** means it is implemented and tested in this repository and is not yet
  in a tagged release.
- **Later** means it is on the roadmap and not built.

**Surfaces:** Web, Native, Desktop, Portal, API, CLI.

Last checked against `main` on 2026-11-03. The API column counts the documented
surface: 658 paths in 59 groups, pinned in `fixtures/openapi/v1/openapi.json`.

## 1. Service desk

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Ticket workbench | Notes, timer, conversation, documents and the agent panel in one view | Web | Shipped | `apps/web/app/tickets/[id]` |
| Ticket list | Filters, saved views, pins and an accurate total count | Web, API | Shipped | `apps/web/app/tickets` |
| Boards | Statuses, transitions, groups, favourites, navigation order | Web, API | Shipped | `crates/core-api/src/routes/boards.rs` |
| Board access | Per-board membership, role checks and client workflow overrides | Web, API | Shipped | `crates/core-api/src/routes/board_access.rs`, `crates/core-api/src/routes/workflow_overrides.rs` |
| Custom fields | Field definitions per board with validated values | Web, API | Shipped | `crates/core-api/src/routes/custom_fields.rs` |
| Checklists | Per-ticket checklists and board-level templates | Web, API | Shipped | `crates/core-api/src/routes/checklists.rs` |
| Taxonomy and tags | Categories, types, retire and reparent, cross-board move | Web, API | Shipped | `crates/core-api/src/routes/taxonomy.rs`, `crates/core-api/src/routes/tags.rs` |
| Ticket relationships | Links, merge and a rollup of merged work | API | Shipped | `crates/core-api/src/routes/ticket_links.rs` |
| Conversation | Internal notes, client replies and email on the same record | Web, API | Shipped | `crates/core-api/src/routes/ticket_emails.rs` |
| Notes and tasks | One notes-led model, with tasks derived from it | Web | Shipped | `docs/notes-tasks-workbar.md` |
| Presence | Who is viewing or editing, with a short lease | Web, API | Shipped | `crates/core-api/src/routes/presence.rs` |
| Undo | Server-held undo stack and revert per record and per field | Web, API | Shipped | `crates/core-api/src/routes/undo.rs` |
| Instant writes | Idempotent autosave, typed conflicts, snapshot reads, delta reads | Web, API | Shipped | `docs/instant-first.md`, `crates/core-api/src/instant.rs` |
| Attachments | Upload, download, scan gate, quarantine, transcription | Web, API | Shipped | `crates/core-api/src/routes/attachments.rs`, `crates/core-api/src/scan_gate.rs` |
| Similar tickets | Semantic neighbours on the open ticket | Web, API | Shipped | `crates/core-api/src/routes/similar_tickets.rs` |
| SLA | Policies, business-hour calendars, ladders, live verdicts | Web, API | Shipped | `crates/core-api/src/routes/sla_admin.rs` |
| Missing time | Gap detection per person and day, audited by the worker | Web, API | Shipped | `crates/core-api/src/routes/time.rs` |
| Approvals | Queues for time and record changes | Web, API | Shipped | `apps/web/app/approvals` |
| Dispatch | Queue, technician lanes, capacity, planning, scheduling and travel | Web | Shipped | `apps/web/app/dispatch` |
| Dispatch connector | Scheduling and status exchange with an outside dispatch tool | API | Shipped | `crates/core-api/src/routes/dispatch_connector.rs` |
| Today and day pack | Stops grouped into trips, a consolidated grab list, availability | Web | Shipped | `apps/web/app/today`, `crates/core-api/src/routes/day_pack.rs` |
| Alerts | Sources, signed ingest, acknowledge, resolve | Web, API | Shipped | `crates/core-api/src/routes/alerts.rs` |
| Multi-window | Dockable panes with persisted layout and size limits | Web | Shipped | `packages/layout` |
| Keyboard | One command registry, an overlay and a cheat sheet | Web | Shipped | `apps/web/app/settings/shortcuts` |
| In-product help | Contextual help from a route map, plus guided tours | Web | Shipped | `docs/site/help-manifest.json` |
| Onboarding | Org wizard, per-user orientation, each step skippable | Web, API | Shipped | `crates/core-api/src/routes/onboarding.rs` |
| Search | Full-text and vector search with pins | Web, API | Shipped | `crates/core-api/src/routes/search.rs` |
| Classic ticket | The single-column ticket view remains available | Web | Shipped | `apps/web/app/tickets/classic` |
| Accessibility | Keyboard paths, focus handling and an axe baseline in CI | Web | Shipped | `e2e/flows/axe-audit.spec.ts` |

## 2. Time

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Timer | One server-owned timer per person, pause, resume across tickets | Web, Native, API | Shipped | `crates/core-api/src/routes/timers.rs` |
| Time entries | Recorded from the timer or by hand, with fix and reopen | Web, Native, API | Shipped | `crates/core-api/src/routes/time.rs` |
| Categories | Time categories, capture defaults and work-type mapping | Web, API | Shipped | `crates/core-api/src/routes/time_categories.rs` |
| Approvals | Submit and decide queues for recorded time | Web, API | Shipped | `crates/core-api/src/routes/time.rs` |
| Billing suggestions | A suggested billing action per entry and per ticket | Web, API | Shipped | `crates/core-api/src/routes/time.rs` |
| Expenses and costs | Ticket cost lines, expenses, invoice candidates | Web, API | Shipped | `crates/core-api/src/routes/costs.rs` |
| Settings | Rounding and time policy per organisation | Web, API | Shipped | `crates/core-api/src/routes/time.rs` |

## 3. Billing and money

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Invoices | Draft, issue, send, render, void, chase | Web, API | Shipped | `crates/core-api/src/routes/invoices.rs` |
| Invoice runs | Batch billing from approved time | Web, API | Shipped | `crates/core-api/src/routes/billing.rs` |
| Billing preview | What a period would bill before it is issued | Web, API | Shipped | `crates/core-api/src/routes/billing.rs` |
| Verification | Review queue, per-entry action and a sign-off | Web, API | Shipped | `crates/core-api/src/routes/verification.rs` |
| Credit notes | Apply, refund and void against the original invoice | Web, API | Shipped | `crates/core-api/src/routes/credit_notes.rs` |
| Contracts | Types, billing preview, change orders and SLA credits | Web, API | Shipped | `crates/core-api/src/routes/billing.rs` |
| FX | Append-only rate book, resolution, reconciliation | Web, API | Shipped | `crates/core-api/src/routes/fx.rs` |
| Payments | A payment provider seam, a Stripe adapter and typed declines | Web, API | Shipped | `crates/core-api/src/routes/billing_provider.rs` |
| Project billing | Phase and milestone releases with cost rollup | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |

## 4. Sales

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Catalog | Items, pricing, archive | Web, API | Shipped | `crates/core-api/src/routes/catalog.rs` |
| Pricing feeds | Distributor batches applied with reconciliation | API | Shipped | `crates/core-api/src/routes/catalog.rs` |
| Pipeline | Deals, stages, stage moves | Web, API | Shipped | `crates/core-api/src/routes/pipeline.rs` |
| Quotes | Lines, send, accept or decline, convert to invoice | Web, API | Shipped | `crates/core-api/src/routes/quotes.rs` |
| Client approval | A client accepts or declines in the portal | Portal | Shipped | `apps/web/app/portal/quotes` |

## 5. Clients and field supply

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Clients and contacts | Records, detail view and portal provisioning | Web, API | Shipped | `crates/core-api/src/routes/clients.rs` |
| Config items | Devices, interfaces, field precedence and observed values | Web, API | Shipped | `crates/core-api/src/routes/config_items.rs` |
| Configuration identity | Merge candidates, batch decisions, split, a workbench | Web, API | Shipped | `crates/core-api/src/routes/config_items.rs`, `apps/web/app/inventory/workbench` |
| Entity dedupe | Client and contact matching across feeds with confidence | Web, API | Shipped | `crates/core-api/src/routes/entity_dedupe.rs` |
| Connectors | NinjaOne, Hudu, M365 Graph, UniFi, 3CX, QuickBooks and Xero exports, DMS sync, CI feeders, Mapbox, Cloudflare | API, CLI | Shipped | `plugins/connectors` |
| Connector admin | Enable, disable, test and read the sync ledger | Web, API | Shipped | `crates/core-api/src/routes/connector.rs` |
| Assets | Asset records and taxonomy with a client scope | Web, API | Shipped | `crates/core-api/src/routes/assets.rs` |
| Stock | Items, locations, movements, consumables | Web, Native, API | Shipped | `crates/core-api/src/routes/stock.rs` |
| Tools and kits | Tool catalog, kits, check-out and return | Web, Native, API | Shipped | `crates/core-api/src/routes/tools.rs` |
| Client loans | Devices on loan to a client, with return tracking | Web, API | Shipped | `crates/core-api/src/routes/loans.rs` |
| Purchasing | Vendors, purchase orders, receiving, billing, deeplinks | Web, API | Shipped | `crates/core-api/src/routes/purchasing.rs` |
| Labels and printing | Templates, preview and ZPL through the desktop shell | Web, Desktop, API | Shipped | `crates/core-api/src/routes/labels.rs`, `desktop/src/printers.rs` |
| Scanning | Keyboard-wedge and camera scans resolved to records | Web, Native, API | Shipped | `crates/core-api/src/routes/labels.rs`, `native/app/(tabs)/scan.tsx` |
| Imports | Dry run, chunked staging, saved templates and finish | Web, API | Shipped | `crates/core-api/src/routes/imports.rs` |
| Vendor migration | Pull and push sessions by data family | API | Shipped | `crates/core-api/src/routes/connector.rs` |

## 6. Projects

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Projects | Phases, tasks, dependencies, completion | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |
| Views | Gantt and phase burn | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |
| Templates | Templates instantiated with their phases and tasks | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |
| Saved views | Stored filters and shapes | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |
| Billing releases | Phase or milestone billing with cancellation | Web, API | Shipped | `crates/core-api/src/routes/projects.rs` |
| Recurring schedules | Recurring ticket templates and a worker run ledger | Web, API | Shipped | `crates/core-api/src/routes/recurring.rs` |

## 7. Knowledge and documents

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Knowledge base | Articles, category trees, rich content and search | Web, Portal, API | Shipped | `crates/core-api/src/routes/kb.rs` |
| Embeds | Provider registry, embed resolution and per-article embeds | Web, API | Shipped | `crates/core-api/src/routes/kb.rs` |
| External documents | Synced documents with link checking | API | Shipped | `crates/core-api/src/routes/kb.rs` |
| Knowledge import | Bundle ingest with external keys and visibility | CLI, API | Shipped | `crates/core-kb/src/import.rs` |
| Documents | Typed layouts, client scope, immutable versions and links | Web, API | Shipped | `crates/core-api/src/routes/documents.rs` |
| Passwords | Staff credential store with history | Web, API | Shipped | `crates/core-api/src/routes/passwords.rs` |
| Templates | Organisation or global templates, fork, restore, render | Web, API | Shipped | `crates/core-api/src/routes/templates.rs` |
| Reply snippets | Fork and render per organisation or person | Web, API | Shipped | `crates/core-api/src/routes/reply_snippets.rs` |
| Email signatures | Per-person signatures under an organisation policy | API | Shipped | `crates/core-domain/src/email_signature.rs` |
| Sign-offs | A witnessed signature with the consent wording recorded | Web, API | Shipped | `crates/core-api/src/routes/sign_offs.rs` |

## 8. Secrets

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Folder trees | Nested folders, fork and per-tree grants | Web, API | Shipped | `crates/core-api/src/routes/secrets.rs` |
| Records | Records, bulk create, file payloads and links | Web, API | Shipped | `crates/core-api/src/routes/secrets.rs` |
| Access control | Category RBAC, restricted chains and junior defaults | Web, API | Shipped | `crates/core-domain/src/policy.rs` |
| Reveal auditing | Every reveal is an audited event | API | Shipped | `crates/core-api/src/routes/secrets.rs` |
| Sharing | One-time share links with claim and revoke | Web, API | Shipped | `crates/core-api/src/routes/secret_share.rs` |
| Rotation | Hooks, reminders, an automation trigger | API | Shipped | `docs/reference/plugin-capabilities.md` |
| Scale-out | Write budgets, paced ingestion and streaming decrypt | API | Shipped | `crates/core-api/src/vault_scale.rs` |
| Plugin ingress | A ciphertext-only `vault:ingress` capability | API | Shipped | `docs/reference/plugin-capabilities.md` |

## 9. Automation and workflows

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Automation rules | Typed triggers, conditions and actions, with a test run | Web, API | Shipped | `crates/core-api/src/routes/automation_authoring.rs` |
| Workflow documents | Versioned and validated documents behind the canvas | Web, API | Shipped | `crates/core-api/src/routes/workflow_documents.rs` |
| Node registry | Built-in nodes and plugin-contributed nodes | Web, API | Shipped | `crates/core-domain/src/node_registry.rs` |
| Runs | Run ledger, steps, retries and per-client overrides | Web, API | Shipped | `crates/core-api/src/routes/workflow_runs.rs` |
| Canvas | The visual authoring surface | Web | Shipped | `apps/web/app/automations/canvas` |
| Domain events | A typed event catalogue with a deprecation clock | API | Shipped | `crates/core-events` |
| Outbound webhooks | Subscriptions, signed deliveries, a ledger and a test | Web, API | Shipped | `crates/core-api/src/routes/webhooks.rs` |
| Inbound webhooks | A generic source with HMAC verification and alert ingest | API | Shipped | `crates/core-api/src/routes/webhook_receiver.rs` |
| Outbox | Transactional outbox, consumer deliveries and redrive | API, CLI | Shipped | `crates/worker/src/consumer.rs` |
| Notifications | Kinds, preferences, org defaults, digests, email | Web, API | Shipped | `crates/core-api/src/routes/notifications.rs` |

## 10. AI and agents

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Master switch | AI is off by default and the switch is a kill switch | Web, API | Shipped | `apps/web/app/settings/ai` |
| Availability matrix | Per-surface grants per role or person | Web, API | Shipped | `crates/core-api/src/routes/ai_policy.rs` |
| Surfaces | Note assist, document drafting, email drafting, chat, plus plugin surfaces for automation writing, dashboards, analysis, transcription | Web, API | Shipped | `docs/site/how-to/admin-ai-access.md` |
| Chat runner | Sessions and turns on granted surfaces | API | Shipped | `crates/core-api/src/routes/chat_runner.rs` |
| Agent proposals | Proposed record changes that a person approves or declines | Web, API | Shipped | `crates/core-api/src/routes/agent_platform.rs` |
| Agent skills | Installed skill artefacts that can be enabled or disabled | Web, API | Shipped | `crates/core-api/src/routes/agent_platform.rs` |
| MCP | A stdio and HTTP MCP server with a scoped tool catalogue | API, CLI | Shipped | `crates/mcp-server` |
| Tool routing | Confidence-gated routing and loop termination | API | Shipped | `crates/core-api/src/routes/decision_edge.rs` |
| Decision harness | Nine decision surfaces, shadow then live, thresholds, a run log | Web, API | Shipped | `crates/core-api/src/routes/decision_config.rs` |
| Credentials | Organisation and personal keys in a sealed store | Web, API | Shipped | `crates/core-api/src/routes/byok.rs` |
| Model catalogue | Catalog entries and a model choice per surface | Web, API | Shipped | `crates/core-api/src/routes/ai_policy.rs` |
| Usage and caps | An append-only ledger, caps, a graceful pause | Web, API | Shipped | `crates/core-api/src/routes/ai_policy.rs` |
| Retrieval | Chunks, query, per-source delete and deletion receipts | API, CLI | Shipped | `crates/core-api/src/routes/rag.rs` |
| Retrieval modes | Embedding spaces, retrieval mode and reindex on change | API, CLI | Shipped | `docs/site/how-to/operator-embedding-model.md` |
| Suggesters | Triage, dispatch, alerts, contract scope, billing audit and record match | Web, API | Shipped | `crates/core-api/src/routes/decision_suggestions.rs` |
| Transcription | Audio attachments turned into text under a surface grant | API | Shipped | `crates/core-api/src/routes/transcription.rs` |

## 11. Reporting

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Report catalogue | Kinds, measures, filters, views | Web, API | Shipped | `crates/core-reports/src/catalog.rs` |
| Definitions | Saved specs that run on demand | Web, API | Shipped | `crates/core-api/src/routes/report_defs.rs` |
| Ad-hoc run | A spec run server-side | API | Shipped | `crates/core-api/src/routes/reports.rs` |
| Schedules | Delivery on a schedule with history | Web, API | Shipped | `crates/core-api/src/routes/report_schedules.rs` |
| Dashboards | A widget grid of pinned views | Web | Shipped | `apps/web/app/dashboards` |
| Scorecards | Organisation, client, personal scorecards | Web, API | Shipped | `crates/core-api/src/routes/scorecards.rs` |
| Read models | Documented `ops.v1_*` views behind a read API | API, CLI | Shipped | `docs/reference/read-model.md` |
| Export | CSV from tables and organisation exports with signed links | Web, API | Shipped | `crates/core-api/src/routes/export.rs` |

## 12. Client portal

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Portal auth | Password and magic-link sign-in on an isolated session | Portal, API | Shipped | `crates/core-api/src/routes/portal.rs` |
| Requests | Submit and follow tickets on exposed boards | Portal | Shipped | `apps/web/app/portal/tickets` |
| Invoices | List, open, download and pay | Portal | Shipped | `apps/web/app/portal/invoices` |
| Quotes | Approve or decline | Portal | Shipped | `apps/web/app/portal/quotes` |
| Knowledge | Browse and search client-visible articles | Portal | Shipped | `apps/web/app/portal/kb` |
| Scheduling | Available slots, appointment requests and cancellation | Portal | Shipped | `apps/web/app/portal/schedule` |
| CSAT | Survey tokens and a resolution survey page | Portal, API | Shipped | `crates/core-api/src/routes/portal.rs` |
| Branding | Logo, colour and a custom domain with DNS verification | Web, API | Shipped | `crates/core-api/src/routes/org_custom_domain.rs` |

## 13. Email

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Inbound email | Routing to boards, per-board identities and collision checks | Web, API | Shipped | `crates/core-api/src/routes/email_admin.rs` |
| Outbound email | SMTP and SES transports with sealed credentials | Web, API | Shipped | `crates/core-mail` |
| Delivery ledger | Per-message state and permanent versus transient failures | Web, API | Shipped | `crates/core-api/src/routes/email_admin.rs` |
| Holds | An inbound hold queue with a human decision | Web, API | Shipped | `crates/core-api/src/routes/email_admin.rs` |
| Test send | A probe against the configured transport | API | Shipped | `crates/core-api/src/routes/email_admin.rs` |
| Sender binding | The inbound sender must match the bound identity | API | Shipped | `crates/core-db/migrations` |
| Notification email | Branded notifications and digest policy | API | Shipped | `crates/core-mail/src/brand.rs` |

## 14. Identity and access

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Login | Password sign-in, sessions, refresh, logout, rate limits and locks | Web, API | Shipped | `crates/core-api/src/auth` |
| Verification and reset | Email verification and password reset | Web, API | Shipped | `apps/web/app/verify-email` |
| Magic link | Passwordless staff sign-in on the same session machinery | Web, API | Shipped | `apps/web/app/magic-link` |
| MFA | TOTP, WebAuthn, recovery codes and step-up | Web, API | Shipped | `crates/core-api/src/auth` |
| SSO | OIDC, a federation bundle seam and conservative linking | Web, API | Shipped | `crates/core-api/src/routes/oidc.rs` |
| Edge auth | Signed proxy assertions exchanged for a session | API | Shipped | `crates/core-api/src/routes/edge.rs` |
| Directory groups | Groups, membership, role mapping | Web, API | Shipped | `crates/core-api/src/routes/directory_groups.rs` |
| Roles | Preset roles and a custom role editor | Web, API | Shipped | `crates/core-api/src/routes/roles.rs` |
| Tenancy | Organisation scope on every query, with Postgres RLS | API | Shipped | `docs/adr/0019-tenancy-boundary.md` |
| API keys | Scoped keys with rotation and revoke | Web, API | Shipped | `crates/core-api/src/routes/api_keys.rs` |
| Service accounts | Machine principals with scopes | API | Shipped | `crates/core-api/src/routes/service_accounts.rs` |
| Audit | An append-only event trail with an export cursor | Web, API | Shipped | `crates/core-api/src/routes/audit.rs` |
| Provenance | Field-level history beside the event trail | Web, API | Shipped | `crates/core-api/src/routes/provenance.rs` |
| Data lifecycle | Retention, purge, export before suspension | Web, API, CLI | Shipped | `crates/marula-core-cli/src/data.rs` |
| Quarantine | A scan-only quarantine store with operator release | Web, API | Shipped | `crates/core-api/src/routes/quarantine_admin.rs` |
| Trusted proxies | Client addresses trusted only from declared proxies | API | Shipped | `crates/core-api/src/client_ip.rs` |
| Organisation governance | Suspension notice, usage reporting and verified entitlements | Web, API | Shipped | `crates/core-api/src/routes/org_suspension.rs` |

## 15. Platform and extensibility

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Plugin runtime | WASM isolation, limits, per-install storage, quarantine, audit | API, CLI | Shipped | `crates/plugin-host` |
| Capabilities | A closed vocabulary, with grants that cannot exceed the manifest | Web, API | Shipped | `docs/reference/plugin-capabilities.md` |
| PDK and SDK | An Apache-2.0 plugin kit, the ABI and signed builds | CLI | Shipped | `packages/plugin-kit`, `packages/sdk` |
| Distribution | Zip, GitHub, registry, skills and theme packs with staged consent | Web, API | Shipped | `crates/core-api/src/routes/plugin_distribution.rs` |
| Signing | Artifact signing with key rotation and a trust store | CLI, API | Shipped | `packages/plugin-kit/bin/sign.mjs` |
| Marketplace | Catalogue, publishers, ratings, verification, revoke | Web, API | Shipped | `crates/core-api/src/routes/plugins.rs` |
| Plugin surfaces | Widgets, automation actions, report kinds, validators, triggers, read nodes | Web, API | Shipped | `crates/plugin-host/src/surface.rs` |
| Plugin credentials | Secrets handed to a plugin without exposing them | API | Shipped | `crates/core-api/src/routes/plugin_credentials.rs` |
| Install health | Pins, lifecycle state, last-good digest and quarantine | Web, API | Shipped | `crates/plugin-host/src/lifecycle.rs` |
| Themes | Three presets, per-person override and theme packs | Web, API | Shipped | `packages/ui/src/theme.ts` |
| API contract | 658 documented paths, a canonical v1 mount and golden pins | API, CLI | Shipped | `fixtures/openapi/v1/openapi.json` |
| SDK | A typed client generated from the OpenAPI pin | CLI | Shipped | `packages/sdk` |
| API reference | An interactive reference served by the API | API | Shipped | `crates/core-api/src/routes/openapi.rs` |
| Entitlement artifact | A signed file is verified and its revision reported | API | Built | `crates/core-api/src/routes/entitlements.rs` |

## 16. Deployment and operations

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Compose stack | Development, demo, all-in-one and preview shapes | CLI | Shipped | `deploy/docker-compose.yml` |
| Kubernetes | A Helm chart | CLI | Shipped | `deploy/helm` |
| Capacity floors | Measured host requirements enforced at boot | CLI | Shipped | `deploy/profiles` |
| Installer app | Five decisions, a live config preview and a developer path | Desktop, CLI | Built | `apps/installer` |
| Install wizard | The same flow over loopback, plus a text plan | CLI | Built | `crates/marula-core-cli/src/install` |
| CLI | Install, update, upgrade scan, deploy checks, migrate and seed | CLI | Shipped | `crates/marula-core-cli` |
| Desktop app | A signed macOS shell with updates, deep links and printing | Desktop | Shipped | `desktop/` |
| Native app | Auth, offline cache, replay-safe writes, live sync, scan, time and stock | Native | Built | `native/` |
| Observability | Traces, metrics and logs with redaction and a low-cardinality contract | CLI | Shipped | `crates/core-otel` |
| Self-host view | Stack health, scanner state, quarantine counts and links | Web, API | Shipped | `crates/core-api/src/routes/self_host.rs` |
| Backup and restore | Runbooks and published targets | CLI | Shipped | `deploy/backup` |
| Upgrades | Compatibility policy, upgrade matrix and a Postgres 18 path | Web, CLI | Shipped | `docs/upgrade-compatibility-policy.md` |
| Release pipeline | Images, a manifest with digests and signing keys | CLI | Shipped | `release-keys/` |
| Local stack | A deterministic seed, isolated runs and a reaper | CLI | Shipped | `scripts/e2e-stack.sh` |
| Docs publishing | A deterministic renderer with link checks | CLI | Shipped | `docs/site` |

## 17. Settings

| Capability | What it does | Surface | Status | Where it lives |
|---|---|---|---|---|
| Organisation settings | A registry of settings with defaults | Web, API | Shipped | `crates/core-domain/src/settings.rs` |
| Appearance | Theme preset, density, appearance per person | Web, API | Shipped | `apps/web/app/settings/appearance` |
| Taxonomy | Categories, tags, labels, toolboxes | Web, API | Shipped | `apps/web/app/settings` |
| Workflow | Board statuses, transitions, templates, fields | Web, API | Shipped | `apps/web/app/settings/boards` |
| Desk configuration | SLA, time categories, snippets, surveys, recurring schedules | Web, API | Shipped | `apps/web/app/settings` |
| Team and security | People, invites, roles, MFA and sessions | Web, API | Shipped | `apps/web/app/settings/team` |
| AI and agents | Master switch, matrix, decision harness, keys, caps | Web, API | Shipped | `apps/web/app/settings/ai` |
| Integrations | Connectors, email, portal, FX, plugins and self-host | Web, API | Shipped | `apps/web/app/settings` |
| Shortcuts and onboarding | Command registry, tours, onboarding resets | Web, API | Shipped | `apps/web/app/settings/shortcuts` |
| Export and quarantine | Data export and quarantine review | Web, API | Shipped | `apps/web/app/settings/export` |

## Not built yet

| Item | What it would do | Status |
|---|---|---|
| AI-drafted commercial documents | Draft a quote or contract from the catalogue, pricing and terms, as a proposal a person edits and approves | Later |
| Reports and dashboards from a description | Build a report or dashboard from existing views and charts, run it against real data, then hand it over as an editable proposal | Later |

## Keeping this current

The list follows the code. When a capability ships, its status changes and the
release notes say so. When one is removed, its row goes with it. The counts at
the top come from the OpenAPI pin and should be regenerated with it.
