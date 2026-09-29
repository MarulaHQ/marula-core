# BUILD-PROMPT verdicts — spec item by spec item (Wave 34 lane 5)

Every Stage 0–5 acceptance bullet, every §1 module bullet, and every §2
stack row of `docs/prompts/BUILD-PROMPT.md`, walked against the SHIPPED
repository (code, OpenAPI pin, landing records under `design/qa-*/`,
TASKLIST records). Verdict vocabulary:

- **shipped** — the spec item exists end to end with evidence.
- **shipped-with-honest-gap** — shipped and usable, with the gap named
  and owned (an owner or a follow-up row in TASKLIST.md).
- **missing-with-owner** — absent from the shipped app, claimed as an
  UNCHECKED follow-up row in TASKLIST.md so it cannot vanish.

Nothing here fails silently. This file is wired into
`docs/site/scripts/check-docs-site.mjs`: removing a verdict row or a
claimed follow-up pointer fails the docs gate.

Basis: main `3629977` with Waves 14–38 landed. Audit dates 2026-09-28.
Where the evidence is a landing record, the `design/qa-*/` path is the
citation. Where it is code, the citation names the module or endpoint.

## Stage 0 — Scaffold

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Monorepo scaffold per §3 (crates/*, packages/*, apps/, deploy/, design/mockups/) | shipped | `crates/` 23 crates (all §3 components incl. mcp-server, plugin-host, perf-harness), `packages/{sdk,ui,layout,plugin-kit}`, `apps/web`, `deploy/`, `plugins/wasm` + `plugins/connectors` |
| docker-compose up: Postgres+pgvector, Redis→Valkey, Garage, Mailpit, core-api, realtime, worker, web | shipped | `deploy/docker-compose.yml` (pgvector:pg18, valkey:8, garage, mailpit, api, realtime, worker, web, observability profile). The PG18/Valkey/Garage transition was verified by Wave 36 lanes 1–2 (`design/qa-p36-backup-wal-g-restic`, `design/qa-p36-infra-transition`) |
| OpenAPI utoipa → Scalar → generated TS SDK | shipped | Scalar at `/openapi.json`, `packages/sdk` generated client, 578 pinned paths (`packages/sdk/openapi.json`) |
| meridian-theme tokens, light+dark from one set, no hardcoded hex | shipped | `packages/ui` Workbench tokens, CI brand-distinctness scan, per-theme gate `check-theme-packs.mjs` (Wave 24 theming) |
| Mockups in `design/mockups/` light+dark | shipped | app mockups (workbench v2 is the canonical aesthetic reference) + website mockups `website-*.html` light+dark |
| CI brand-distinctness check | shipped | `ci.yml.disabled` "Brand distinctness — Workbench tokens only" job (definitions-only, triggers user-gated, see `ga-gates.yml.disabled`) |
| hello-world ticket CRUD: API → DB → event → WS → web | shipped | Stage-0/1 acceptance harness (`scripts/acceptance.ts`, 18 live checks incl. the WS push chain) — `docs/STAGE-REPORT.md` Stage 0/1 sections |
| Generated SDK compiles in a scratch TS project | shipped | SDK typechecks against the live spec on every gate run (openapi pin + regen ritual) |
| Licensing files Stage 0 | shipped | LICENSE (AGPL-3.0), SECURITY.md, CONTRIBUTING.md, CODE_OF_CONDUCT.md; Apache-2.0 on `packages/sdk`, `packages/plugin-kit`, `plugins/`, `connectors/` |
| **CI preview environment (every PR)** | shipped-with-honest-gap | The CI pipeline deploys no per-PR preview environment. All CI workflow triggers are deliberately disabled and re-enable is USER-GATED (`f00cdb3`, re-affirmed in the Phase-4 checklist and Wave 34 lane 6 record), so this leg cannot be turned on by a lane. Follow-up claimed below. |

## Stage 1 — Foundation (MER-P0)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Core entity migrations + RLS from day one | shipped | migrations to head 0109 and the `assert_rls_coverage` gate fails CI on a policy-less table (enforced by the RLS-coverage job in `ci.yml.disabled` definitions) |
| argon2id, JWT+refresh rotation, policy module, org/user/role | shipped | `core-api` auth (`crates/core-api/src/routes/auth.rs`, `core-domain/src/policy.rs`) with hardening in Wave 19 lanes 1–4 |
| Multi-timer + time entries | shipped | `/api/v1/timers/*` (start/pause/resume/stop/active/working-set), `time-entries` CRUD |
| WS events for tickets/timers | shipped | realtime hub :8081, per-user/per-board routing, presence |
| Multi-pane layout shell (Today + Workspace) | shipped | `packages/layout` panel engine + `/today`, `/tickets/[id]` three-pane workspace |
| Accept: create→work→close→time entry, p95 <100ms UI | shipped | acceptance harness green incl. latency check, ticket list p95 29.9 ms @50k (Wave 19 GA-readiness) |
| Zero known data-loss bugs, RLS-coverage CI green | shipped | RLS gate + continuous suite, no open data-loss finding on the ledgers |

## Stage 2 — Daily driver (MER-P1)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Email-in/out (inbound webhook → ClamAV scan → threading → comment, outbound `MailTransport`) | shipped | `core-mail` (Postmark + SMTP adapters, clamd scan, threader, claim/report loop), `/settings/email` + delivery ledger + test-send probe (Wave 34 lane 8), operator runbook `docs/site/how-to/admin-email.md` |
| SLA engine + automations v1, calendar awareness, escalation ladders into notification center | shipped | `/api/v1/sla/{policies,calendars}`, `/api/v1/internal/sla/verdict`, `worker/sla_sweep.rs`. Automations `/api/v1/automation-rules` |
| Scheduling/calendars/availability + dispatch board v1 | shipped | `/api/v1/{appointments,availability,calendar-exclusions,on-call-blocks,calendar/*}`, `/dispatch` board (drag/drop, conflicts, presence, unassigned queue) |
| Workflow customization v1 (custom statuses/transitions bound to semantic categories, board groups/favorites, cross-board my-work, board-level RBAC) | shipped | `/api/v1/boards/{id}/statuses|transitions|group` bound to closed semantic categories, `/api/v1/tickets/my-work` |
| @mentions + notification center (per-channel prefs, quiet hours) | shipped | `/api/v1/notifications*`, `notification-preferences` (per-kind email/in-app + quiet window), email fan-out honored by the worker notifier |
| Structured notes + taxonomy + pre-close checklists | shipped | notes taxonomy internal / client-visible / in-house update, enforced checklists on ticket close |
| Single-active-timer workflow | shipped | `single_active_timer_switch_preserves_elapsed` (core-domain), partial unique index on running timers, docked working set `/api/v1/timers/working-set` |
| Missing-time EOD nudges | shipped | `worker` missing-time nudge scheduler + gap-audit ledger (Stage 2 close-out, `docs/STAGE-REPORT.md` 2.9/2.10) |
| Client portal v1 (scoped portal roles, ticket view/reply, per-board exposure) | shipped | `/api/v1/portal/*` (login, tickets, reply, attachments, KB, invoices, quotes, appointment requests, CSAT) with portal-user tiers incl. `it_manager` |
| OIDC SSO + TOTP/WebAuthn MFA | shipped | `/api/v1/auth/oidc/*`, `/api/v1/auth/mfa/*` (TOTP + WebAuthn) and the Keycloak preset (Wave 34 lane 6) |
| Plugin runtime v1 (HTTP connectors + signed webhooks) | shipped | `worker/webhook_out.rs` (HMAC-SHA256 `t=…,v1=…`, retry/backoff, dead-letter), `/api/v1/webhooks` with delivery logs |
| Audit log | shipped | `/api/v1/audit-events`, append-only `audit_events` with `actor_kind` provenance |
| OTel instrumentation | shipped | `crates/core-otel` (OTLP exporter, redaction, opt-in), compose collector + Grafana/Tempo/Loki |
| NinjaOne + Hudu reference connectors | shipped | `plugins/connectors/{ninjaone,hudu}`, none-mode documented |
| Accept: two simulated weeks, zero lost messages | shipped | CI mail harness (two-week zero-loss claim in the Stage-2 close-out record) |
| Accept: third-party connector on public API + SDK only | shipped | connectors doctrine (`plugins/connectors/README.md`) — public API + SDK, none-mode, verified by Wave 22 lane 3 (`design/qa-p22-ecosystem-connectors-gap`) |
| Accept: SLA timers correct under clock skew | shipped | Stage-2 clock-skew tests on the SLA engine (Stage-2 close-out record) |

## Stage 3 — Commercial engine (MER-P2)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Contracts + recurring billing rules | shipped | `/api/v1/contracts*` (recurring / T&M / retainer / fixed-fee), `core-billing/rules.rs` per-period evaluation |
| Quote builder + catalog | shipped | `/api/v1/quotes*` (send/accept/decline/expire/convert), `/api/v1/catalog-items*` |
| Inventory & asset lifecycle (rental tracking, depreciation fields) | shipped | `DeviceAsset` rental fields (rental_vendor/start/end/cost) + depreciation method/rate (migration 0061), MSP-internal fleet scope (ADR 0012), Wave 32 stock/PO/tools/loans (`/api/v1/stock-items`, `purchase-orders`, `tool-checkouts`, `client-loans`) |
| Sales pipeline | shipped | `/api/v1/pipeline/*` + `/sales` surface |
| Projects (phases/tasks/views/dependencies/templates) | shipped | `/api/v1/projects*` incl. phases, tasks, dependencies, templates, `/projects/[id]` work surface. **Views**: List, phase/task groups, and a gantt-lite rail. Dated Gantt and a burn/budget view are typed NOT built (phases carry no dates, `apps/web/app/projects/[id]/page.tsx` non-purpose header). Claimed below |
| Milestone billing feeding invoicing | shipped-with-honest-gap | `project.milestone_completed` event (0062 seam), `is_milestone` phases carry amount/currency, invoice-candidate feed `/api/v1/invoice-candidates` picks up billable lines. The dedicated milestone→invoice consumer with per-phase budget read models was not landed (Wave 39 lane 4 claims exactly this: project↔contract link, phase budgets + burn, fixed-fee phase emission) |
| Unified calendar | shipped | `/api/v1/calendar/{day,week,team,deadlines}` + `/calendar` surface |
| **Dispatch v2 `project` resource planning (tech allocation across project phases; capacity forecasting / utilization spanning service + project)** | shipped-with-honest-gap | `/dispatch/planning` ships a 7-day capacity horizon per technician (available/booked/free/utilization over availability windows + exclusions + appointments) with demand depth + skill catalog (`/api/v1/dispatch/capacity`), plus dispatch-connector schedule. The gap is real and named: allocation is against dispatch appointments only. No read model allocates technicians to project phases or forecasts capacity spanning service + project work (no `forecast`-shaped code anywhere, no phase-allocation entity). Claimed below as a follow-up lane |
| Dispatch connector API v1 (appointment/resource events, availability write API) | shipped | `crates/core-api/src/routes/dispatch_connector.rs`: `/api/v1/dispatch-connector/{technicians,schedule,appointments/{id}/status}` |
| Recurring/scheduled tickets (PM schedules) | shipped | `/api/v1/pm-schedules*` + `worker/pm_sweep.rs` |
| Per-client workflow overrides | shipped | `/api/v1/clients/{id}/workflow-overrides*` (migration 0063, Wave 12) |
| Ticket costs & expenses + merge/parent-child/major-incident linking | shipped | `/api/v1/tickets/{id}/{cost-lines,expenses,links,merge}`, link kinds cover parent/child/dup/major-incident, guided merge workbench for entities (Wave 22 lane 2) |
| Missing-time audits + travel/mileage entry drafts | shipped | gap-audit + `/api/v1/dispatch/travel-drafts*` (submit → expense) |
| RMM alert dedup + generic webhook alert ingestor | shipped | `/api/v1/alerts/ingest/{source_ref}` (HMAC, raw-body verify), dedupe to one incident on alert storms (Wave 9 record) |
| Portal v2 (invoice view + pay, quote approval, KB self-service, appointment requests) | shipped | `/api/v1/portal/invoices/{id}/pay` (PaymentProvider, Stripe adapter), quote approval `/portal/quotes/{id}/(approve\|decline)` (Wave 34 lane 1), `/portal/kb/*`, `/portal/appointment-requests*`, CSAT response route (Wave 34 lane 2) |
| Native invoicing v1 (contract → run → PDF → payments → dunning) | shipped | `/api/v1/billing/invoice-runs`, `/api/v1/invoices/{id}/{document,payments,dunning}`, PDF documents, credit notes |
| Native knowledge base v1 (FTS + client-facing flags) | shipped | `/api/v1/kb/*`, portal KB with `published`-only visibility and FTS in the unified search index |
| Adapters QBO, Xero, M365 Graph, IT Glue, ImmyBot | shipped | `plugins/connectors/{qbo-export-sync,xero-export-sync,m365-graph,adapters-core}` (export/integration posture — never the system of record) |
| Import framework (CSV/JSON) + connector API for migration tools | shipped | `/api/v1/imports/{templates,batches,dry-run}` + push/pull migration sessions |
| Tags | shipped | `/api/v1/tags`, `/api/v1/tickets/{id}/tags*` |
| Static dashboards | shipped | `/dashboards` configurable surface (configurable widgets over report kinds) |
| Accept: seeded MSP billing sync to QBO + second native | shipped-with-honest-gap | QBO/Xero export connectors shipped and the native path is the system of record, but the acceptance-form two-org sandbox reconciliation run was never recorded as a gate artifact. The components exist, and the combined demo run needs a follow-up verification note (claimed below) |
| Accept: alert storm collapses to one incident | shipped | alert ingestor dedupe verified in the Wave 9 Stage record |

## Stage 4 — GA hardening (MER-P3)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Performance program (query budgets, load tests at 100k) | shipped-with-honest-gap | ticket-list p95 29.9 ms @50k (met, `design/qa-p19-ga-readiness`). `perf-harness` seed/load + the nightly perf job were REPAIRED by Wave 34 lane 6, but the actual 100k-ticket HTTP load measurement has not run (nightly triggers user-gated) |
| Security hardening to external-audit readiness | shipped | Wave 19 hardening + audits clean, secrets/leak audit zero open findings |
| Plugin runtime v2 (WASM/Extism + capability-consent UI) | shipped | `crates/plugin-host` (Extism), consent surface, deny-by-default grants |
| Marketplace registry v1 (static manifest + git tag, code-signed WASM) | shipped-with-honest-gap | `/api/v1/plugins/catalog` + staged distribution (zip/github/registry) + signing (`plugin_signing_keys` trust store). The static-registry-as-git-tag publishing flow and code-signed-artifact enforcement remain release-pipeline work claimed by Wave 38 |
| Report builder v1 | shipped | `/api/v1/report-definitions*` (`run`, schedules, deliveries) + builder UI |
| **Two-way calendar sync (Google Workspace + M365)** | shipped-with-honest-gap | iCal two-way sync shipped (`CalendarSyncProvider` seam, outbound token-scoped feed + inbound busy polling, migration 0074). Google/M365 adapter KINDs are not built: the provider seam admits only `ical` today, and the module header states the growth path. Claimed below |
| Time-entry approval workflows (manager queues, billing-period locking) | shipped | `/api/v1/time-entries/approvals/{queue,decide}`, submit/reopen, approve→locked-on-invoice |
| **Optional client approvals on time entries** | shipped-with-honest-gap | **Not built as a product leg — documented decision.** The Stage-4 "optional" leg: portal-user approver tier exists and is behavior-ready (`PortalRole.Approver`, migration 0058-era modeling), but no portal approval flow over time entries ships and the TASKLIST ownership cleanup (2026-09-23) resolved to keep the shipped decision: manager-side approval only, with the gate decision documented here. The approver tier is the seam if this is ever built. No follow-up lane is claimed unless the user asks for it |
| Tech coaching scorecards (private-by-default, opt-in gamification) | shipped | `/api/v1/scorecards*` (`me` private view + `me/settings` opt-in), CSAT rollups added by Wave 34 lane 2 |
| WCAG 2.2 AA accessibility | shipped | Wave 17 lane 10 pass + Wave 29 lane 2 deep re-audit (axe gate, zero NEW criticals, AA bar, `design/qa-p29-accessibility-deep`) |
| iCal feed | shipped | token-scoped outbound iCal feed `/api/v1/calendar-sync/feed/{token}` |
| White-label-lite (portal logo/colors) | shipped | `apps/web/lib/portal-branding.ts` (Wave 17 lane 10): org name + accent substitution, honest env-config defaults |
| Dispatch v3 (`RoutingProvider` route optimization + dispatch widget registry) | shipped | `crates/core-routing` provider seam (heuristic/OSRM/ORS/Mapbox + TravelTime/Geoapify relays), `journey` start points, dispatcher warnings + suggestions. Widget registry = the §4.6 plugin widget registry (Wave 34 lane 4) |
| CSAT surveys + canned responses/snippets + ticket templates | shipped | Wave 34 lane 2 (CSAT on resolution, none-mode honest) + lane 3 (reply-snippets) + `/api/v1/templates` |
| Tauri 2 desktop shell GA | shipped | Wave 20 lane 1 GA packaging + updater + multi-window (Wave 38 lane 1 web popouts, desktop lane 2 in roadmap) |
| Accept: perf budgets met | shipped | ticket-list budget met and WS delivery p95 within budget (Stage 1 harness + perf docs baselines) |
| Accept: self-host upgrade path tested in CI (v0.9→v1.0) | shipped-with-honest-gap | U9 cross-check landed (record in TASKLIST_COMPLETED). The N−2 upgrade matrix job is defined in `ci.yml.disabled`/`nightly.yml.disabled`, but CI triggers are user-gated, so the in-CI run has not happened continuously |
| Accept: accessibility audit clean at AA on 10 core flows | shipped | Wave 29 lane 2 re-verified ALL shipped (grown) surfaces at AA |

## Stage 5 — Ecosystem (MER-P4, stretch)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Marketplace v2 (ratings manifest fields, verified-publisher flow) | shipped | migration 0099 wave22_marketplace_v2: `/api/v1/plugins/distribution/{ratings,publishers}` |
| Rewst bridge; Slack/Teams apps | shipped | `plugins/connectors/app-bridges` (rewst/slack/teams) riding the connector-installations consent seam |
| VoIP connector API | shipped | `/api/v1/connectors/voip/call-logs` + threecx adapter |
| Distributor pricing feed adapters | shipped | `/api/v1/catalog/pricing-feeds*` immutable batches |
| Advanced RBAC custom-roles UI | shipped | `/api/v1/roles*` (fork/restore) + Wave 22 custom-roles editor in `/settings/team` |
| React Native groundwork | shipped-superseded | Wave 22 seam was superseded by the full Wave 33 mobile app (foundation + fieldops + tech-day) |
| Accept: 20+ third-party connectors/plugins in the registry, one-click marketplace install | shipped-with-honest-gap | intake seams + consent-driven one-click install flow shipped. Catalog depth (20+ listed entries) is marketplace-population work at publishing, not a product gap |

## Consumer website (§7 per docs/11-web-presence.md)

| Spec item | Verdict | Evidence / gap |
| --- | --- | --- |
| Waitlist site Phase 1 + live marketing Phase 2 | shipped | Wave 20 lane 1 → Wave 28 rebuild → Wave 34 lane 7 phase-2 (14-route sitemap complete, `/features` folded into `/compare`) |
| Mockup-first, public-safe, no app-code imports | shipped | `design/mockups/website-*` + `website/` package boundary (packages/ui primitives only) |
| **Lighthouse ≥95** | shipped-with-honest-gap | Mobile perf 95–96, a11y/BP/SEO 100 and desktop perf 100 on all six phase-2 routes (`design/qa-p34-website-phase-2/REVIEW.md` Lighthouse tables + JSON evidence). Gap: evidence predates ~35 later commits (mostly non-website: 92524cd ADR docs touched the docs bundle only) and CI does not re-run Lighthouse — re-run at publishing time. No net-new page content has shipped since |
| WCAG AA on the site | shipped | Lighthouse a11y 100 on all six routes, brief binding gates carried to every website lane |
| **PostHog aggregate analytics only** | missing-with-owner | No analytics on the site today. `website/README.md` states phase-1 ships no analytics and bars third-party scripts, and no PostHog integration exists in `website/` (verified by search): no tracking, no aggregate. The §7 bar says "if analytics ever ship, PostHog aggregate-only". The site ships none, so the bar holds vacuously while the PostHog leg itself is unshipped. Claimed as a follow-up: a bounded lane to add PostHog aggregate-only (no cookies, no per-user identifiers) behind a config flag with an explicit off state, satisfying §7 literally |
| The site never imports product app code | shipped | `website/` is a separate Next app with imports restricted to packages/ui primitives |

## §1 product-module bullets — the short list (each walked individually)

| Module bullet | Verdict | Note |
| --- | --- | --- |
| Ticketing: `boards/groups/favorites`, `my-work`, lifecycle, custom statuses, custom fields/JSONB, SLA `calendar-aware`, automations, structured notes+checklists, `email-in/out` threading, `merge/parent/major-incident`, tags | shipped | as per Stage 2 table above |
| Time: `multi-timer` `server-authoritative`, `per-segment` pause reasons, `single-active`, `missing-time` detection (activity gap + EOD nudges + manager gap reports), bulk review, manager approval queues + `billing-period` locking, cost vs bill rates, work roles/types as rate modifiers | shipped-with-honest-gap | everything shipped except: manager "gap reports" surface is the EOD strip + queue rather than a standalone report, and **work_type is free text** with no taxonomy until Wave 39 lane 1 (time categories, 0110) lands. Claimed in Wave 39 |
| Clients: company→site→contact→asset hierarchy, per-client visibility, contextual external deep links | shipped | `/api/v1/clients/{id}/{sites,contacts,assets}`, `pod-visibility`, `assets/{id}/integration-links`, OAuth-style per-tenant integration links |
| Scheduling & dispatch: personal/team/dispatch calendars, on-call, iCal feed, two-way sync, dispatch board (capacity, queue, DnD, conflicts, travel estimates, skill matching, **ghost placements**, **live presence**, `curated` per-tech `work-next` queues), **resource planning**, dispatch connector API | shipped-with-honest-gap | board DnD ships a real dashed **ghost placement** preview (drag state reducer + keyboard ghost seeding, `apps/web/app/dispatch/page.tsx`) and real-time presence (realtime hub presence events). Both shipped. The `curated per-tech work-next queues` bullet: NOT shipped. The board has the P1–P4-filterable unassigned QUEUE, skill-match (`/api/v1/dispatch/match`), and routing-grade suggestions (swap/reassign, requiring a measured provider), but no persisted per-technician ordering (no entity, no endpoint). Claimed below. Resource planning gap as per the Stage 3 table |
| Projects module | shipped-with-honest-gap | as per Stage 3 table (dated Gantt/budget view + milestone consumer open) |
| Commercial module | shipped | Stage 3 table |
| Invoicing module | shipped | Stage 3 table, QBO/Xero export posture correct |
| KB module | shipped | FT search + visibility flags |
| Service desk extras | shipped | CSAT, snippets, templates |
| Reporting: operational reports, builder v1, report API, static dashboards, tech coaching scorecards | shipped | scorecards private-by-default with opt-in |
| Platform: RBAC matrix+board roles+custom-roles UI, client visibility, OIDC SSO, TOTP/WebAuthn, audit, REST+WS, signed webhooks, plugin runtime, static marketplace, portal, PWA, Tauri 2, `CompletionProvider` BYOK, Whisper transcription + bge-class embeddings in-container, notification center with SLA ladders + channel fan-out, agent harness (typed settings, 3-tier onboarding, skills & rules, provenance audit, document versions, MCP server) | shipped-with-honest-gap | all shipped except: bundled faster-whisper/bge-models are NOT bundled in the container. Transcription ships as the ADR 0008 plugin surface edge (OpenAI-compatible `audio/transcriptions`, ollama(whisper) provider resolve documented, `crates/core-api/src/routes/transcription.rs`) and embeddings ride the provider seam with the fixed 768-dim contract. The BYOK posture deliberately keeps model binaries out of core: documented as a deployment-layer choice, not a missing leg |
| Agent harness: MCP server | shipped | `crates/mcp-server` stdio + HTTP, catalog derived from OpenAPI, reads-only with IF8 proposal pipeline for writes |

## §2 stack rows (remaining)

| Row | Verdict | Note |
| --- | --- | --- |
| Rust core services + SQLx + utoipa | shipped | 23 crates, compile-checked queries, OpenAPI from handlers |
| PostgreSQL 16+ maximalist (RLS, FTS, pgvector, LISTEN/NOTIFY, queue tables) | shipped | PG18 now, escape hatches documented behind internal traits |
| Queue: PG job queue + outbox → Redis streams, EventBus/NATS hatch | shipped | `core-events` relay with durable consumer groups on webhook delivery |
| Redis/Valkey cache | shipped | Valkey 8 in compose/helm |
| Next.js App Router + TS + Tailwind + shadcn + Radix + Zustand over WS, multi-pane layout engine, Workbench tokens, Inter/JetBrains Mono, light+dark, hex-scan | shipped | (all stack decisions honored, CI hex-scan defined in the disabled workflow set) |
| Tauri 2 shell (tray, multi-window, notifications, auto-update) | shipped-with-honest-gap | shell GA + updater + web multi-window shipped. Native multi-window + OS-integration polish is Wave 38 lane 2 (claimed in roadmap) |
| AuthN/Z stack | shipped | Stage 1/2 rows, SAML/SCIM out of scope per §10 |
| Plugins: Extism + TS SDK + capability consent + audit-logged invocations | shipped | Wave 14 lane 5 + Wave 34 lane 4 |
| AI hooks: thin `CompletionProvider` + BYOK, bundled faster-whisper + bge-class in container, Ollama documented | shipped-with-honest-gap | provider seam + transcription edge shipped (network provider model, BYOK). The container bundling leg is a deployment-layer choice: self-host docs document Ollama/whisper serving, no model binaries in the build. Claimed below as a packaging follow-up for the bundled-local path |
| Mail: Postmark/SES/self-host, inbound → ClamAV → threading | shipped-with-honest-gap | Postmark + SMTP shipped, clamd scan wired. SES adapter not built (documented third choice behind the trait, none-mode posture) |
| Storage: S3-compatible trait, Garage self-host, R2/S3 adapters | shipped | `core-store::ObjectStore`, Garage compose slot |
| Routing/geocoding: RoutingProvider trait, OSRM/ORS default, Mapbox adapter | shipped | + TravelTime/Geoapify as relays (ADR 0014) |
| Observability: OTel, Grafana/Loki/Tempo, GlitchTip, Uptime Kuma | shipped-with-honest-gap | OTel + Grafana/Loki/Tempo + GlitchTip all in compose and visible to the operator (Wave 36 lane 3). Uptime Kuma status page is not in the compose stack, claimed below |
| Docs/API: Starlight in-repo, generated API docs → Scalar | shipped | Diátaxis `docs/site/` (68+ pages, check-docs gate), generated `reference/api.md`, Scalar UI |
| Tooling: cargo workspace + pnpm + Turborepo, Changesets + conventional commits, GHCR digest-pinned + SBOM + provenance | shipped-with-honest-gap | cargo+pnpm+turbo live. Changesets is configured but unused (`changesets.yml.disabled`). GHCR+SBOM+provenance are `release.yml.disabled` definitions awaiting the user-gated trigger + Wave 38 hardening |

## §7/§8 gate rows (records the phase checklist already tracks)

| Gate | Verdict |
| --- | --- |
| Playwright E2E 10 core flows × light+dark | shipped (25 flow files incl. plugin surfaces, multi-window; both themes) |
| RLS coverage test | shipped (`assert_rls_coverage` in the migration+flow tests) |
| Domain coverage ≥80% | definition exists in `ga-gates.yml.disabled`. Measured coverage is not recorded on main and rides the same user-gated re-enable |
| Perf smoke on every merge | definition exists (`perf-smoke.sh` + job), trigger user-gated |
| Migration-diff check (`--breaking` lint) | shipped (`scripts/check-migrations.sh` in the gate set) |
| Preview environment per PR | **missing-with-owner** — see Stage 0 row |

## Claimed follow-ups (UNCHECKED rows added to TASKLIST.md "Wave 34 verdict-sweep claims")

1. **Dispatch resource-planning read models** (per-phase tech allocation
   across project phases + capacity forecasting spanning service+project).
2. **`Curated` per-tech work-next queues** on the dispatch board (the §1
   bullet: a persisted per-technician ordering, not today's filterable
   unassigned queue).
3. **PostHog aggregate analytics on the website** (aggregate-only, cookie-free,
   config-gated) behind the §7 quality bar.
4. **Preview-environment CI leg** — cannot be enabled from a lane (triggers
   are user-gated); the scaffold/compose work needed to make a preview env
   possible is claimed, activation stays with the user.
5. **Calendar-sync Google Workspace + M365 provider kinds** on the shipped
   `CalendarSyncProvider` seam (ical is the only implemented kind today).
6. **Projects dated-Gantt + phase-budget surfaces** — Wave 39 lane 4
   (`p39-project-billing`) already claims the milestone→invoice
   consumer; a UI lane owes the dated Gantt and phase budget/burn
   view beyond it.
7. **Uptime Kuma status page** in the self-host compose/observability
   profile (the one §2 observability row_item not shipped).
8. **Reconciliation-report verification run** (QBO sandbox sync vs native
   side-by-side evidence artifact, the Stage-3 acceptance form).
9. **Bundled local whisper/embeddings container leg** — package the
   documented bundled-models path (or record a final operator-side-only
   decision) for the §2 AI-hooks row.
10. **SES mail adapter** — the third `MailTransport` choice, documented
    but not built (bounded one-adapter job).

Deliberate documented decisions (NOT follow-ups): optional client
approvals on time entries stay manager-only (owner decision recorded
above). SES stays the documented third choice until item 10 is picked up
or dropped by the user. GA-gates re-enable stays USER-GATED (below).

## `ga-gates.yml.disabled` re-enable list — USER-GATED

The disabled workflow set (`ci.yml.disabled`, `ga-gates.yml.disabled`,
`nightly.yml.disabled`, `release.yml.disabled`, `native.yml.disabled`,
`changesets.yml.disabled`) carries complete gate DEFINITIONS. Re-enabling
any trigger is a USER decision — never lane-owned (committed posture
`f00cdb3`, re-affirmed by Wave 34 lane 6 and the Phase-4 checklist).
What each file would enable:

| Workflow | Gates inside |
| --- | --- |
| `ci.yml.disabled` | fmt/clippy/test matrix, web lint+types+tests, migration safety, OpenAPI pin format, RLS pool-read gate, RLS coverage, brand distinctness, deprecation clock, golden OpenAPI, E2E 10 flows light+dark, perf smoke, N−2 upgrade matrix |
| `ga-gates.yml.disabled` | API contract tests from the pin, domain-layer coverage ≥80%, U9 customization cross-check, perf smoke |
| `nightly.yml.disabled` | 100k tickets/org seed+load, deep-page tripwires |
| `release.yml.disabled` | digest-pinned GHCR images, SBOM (syft), provenance attestation, changeset notes |
| `native.yml.disabled` | native typecheck/lint/tests, macOS runner Maestro matrix |
| `changesets.yml.disabled` | changeset version PR automation |

Re-enabling is a checkbox on the user's side. The definitions are kept
byte-current by the gates that run them in CI-parity locally.

## Lane record

Landing record: `design/qa-p34-spec-verdict-sweep/` (LANDING.md +
REVIEW.md with walk evidence). Wave 34 closes with this lane; **Wave 35
(`p35-ui-improvement`, the truly-final UI audit) becomes fireable
afterward.**
