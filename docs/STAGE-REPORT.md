# Meridian Stage Reports

> One section per build stage, appended as stages close (BUILD-PROMPT §7:
> each stage ends with a report listing what shipped, criteria status, and
> known gaps).

## Stage 0 — Scaffold

**Status:** ~95% complete. Infra + backend services fully up; `apps/web`
shipped and merged. Remaining: MinIO + Mailpit are declared compose slots
(`--profile stage2`) but unexercised until their Stage-2 consumers land.

### What shipped

| Area | State |
|---|---|
| Monorepo scaffold | `crates/{core-domain,core-db,core-events,core-api,realtime,worker,meridian-cli}`, `packages/{ui,layout,sdk}`, `apps/`, `deploy/`, `design/mockups/` |
| Postgres (+pgvector pg17) & Redis | dev ports 15432 / 16379; migrations self-provision the RLS-constrained `meridian_app` role |
| core-api | Axum :8080 · argon2id + JWT/refresh rotation · org/user/role · clients hierarchy · boards/tickets/comments/checklists · timers/time entries · notifications · audit · utoipa→Scalar OpenAPI at `/openapi.json` |
| realtime | WS hub :8081 · JWT-verified handshakes · per-user/per-board routing · presence |
| worker | outbox→Redis relay host :8082 (`/healthz`); hosts `core_events::run_relay` |
| meridian-cli | `migrate`, `serve-worker`, `serve-realtime` |
| Event bus contract | transactional outbox → versioned envelope → single Redis stream (`meridian:events`) |
| packages/ui | Workbench tokens (light+dark from one set), 14 base components |
| packages/layout | multi-pane engine, persisted panel state, 66 tests |
| packages/sdk | typed client generated from committed `openapi.json`; 43 unique operationIds; auth-aware `MeridianClient` (openapi-fetch) |
| deploy | `docker-compose.yml` (pg, redis, migrate one-shot, api, realtime, worker) + parameterized Rust image |
| apps/web | Next.js 15 + Tailwind v4: `/login`, `/today`, `/tickets/[id]` three-pane workspace; token bridge to `packages/ui`; light+dark verified live |
| core-api hardening | axum 0.8 route syntax, CORS layer gated on `CORS_ORIGIN`, bound-fetch SDK fix |
| acceptance | `scripts/acceptance.ts` (`pnpm accept`): 18 live checks — register → ticket → timer → close → refresh rotation → RLS rejection → **WS push chain** → p95 latency, all green |
| core-db tests | testcontainers Postgres-17 suite (14 tests) + standalone RLS-coverage gate parsing migration SQL |
| CI gates | rust matrix · frontend · migration-safety (`--breaking` lint) · RLS coverage · brand-distinctness scan |
| web image | `deploy/Dockerfile.web` (standalone Next output) + compose `web` service; verified serving locally |

### Criteria status

- [x] Full Rust workspace green: fmt · clippy `-D warnings` · 93 tests
- [x] API round-trip verified live: register → JWT → authenticated `/api/me`
- [x] Generated SDK typechecks against the live spec
- [x] Both UI themes generated from one token set; components cover both modes
- [x] `docker compose up` starts **web** service (image build + serve verified)
- [x] MinIO + Mailpit slots declared (compose `stage2` profile; exercised in Stage 2)
- [x] CI brand-distinctness check
- [x] CI migration-diff check (`scripts/check-migrations.sh`)

### Known gaps / deferred

1. General job queue in `worker` is intentionally Stage 2 scope.
2. Webhooks/plugin consumption of the stream: Stage 2+.
3. Refresh-token reuse detection (revoked-chain tracking): Stage 2 hardening.
4. Compose images are built from source each run (no registry/cache yet).

## Stage 1 — Foundation (MER-P0)

**Status:** COMPLETE. Acceptance harness green end-to-end (18/18 live),
including the full push chain and the latency budget.

### Criteria status

- [x] E2E acceptance: create ticket → timer → close → time entry appears — API→DB→events via `pnpm accept`; web-UI leg verified manually (Playwright: register → status change → comment → checklist → timer)
- [x] p95 UI interaction <100 ms against localhost — harness check [11]: GET list 5.7 ms, GET ticket 9.6 ms, POST comment 14.4 ms
- [x] RLS-coverage CI test green (standalone gate in `.github/workflows/ci.yml`)
- [x] WS push chain verified live: `ticket.created` delivered api → outbox → relay → Redis → hub → subscribed socket (harness check [10]; surfaced + fixed worker defaulting to the RLS-constrained DB role)

### Known gaps / deferred

1. SLA clock evaluation loop (engine math shipped + tested in domain; scheduler lands with job queue).
2. Custom-field UI surfaces (schema + queries exist).

## Stage 2 — wave 3 (first four daily-driver items)

**Status:** COMPLETE and merged to main (`06d5f53`). Built by four parallel
Agent Manager lanes with disjoint migration windows / test-seed ranges /
private dev ports; each lane shipped its tests with the implementation.

### What shipped

| Item | Lane | Highlights |
|---|---|---|
| 2.1 Workflow statuses/transitions | `stage2-statuses` | `0011_board_statuses.sql` — per-board status adjacency, composite FKs make cross-board edges impossible, defaults-first seeding of 4 category statuses, defensive non-destructive backfill; pure transition policy in `core-domain/workflow.rs` + proptest; workflow-enforced status changes; web status control grouped by semantic category |
| 2.2 Notifications + @mentions | `stage2-notifications` | `0012_notification_prefs.sql` — per-channel prefs, tz-aware quiet hours (paired-bound constraint), org defaults rows, partial unread index; mention parser + member resolution in comments; REST inbox/unread/mark-read; worker email fan-out honoring prefs behind a MailTransport seam (noop until transports merged); notification dock + prefs screen |
| 2.3 Auth hardening | `stage2-auth-mfa` | `0013_auth_hardening.sql` — TOTP (totp-rs) + WebAuthn (webauthn-rs) MFA w/ hashed single-use recovery codes, mfa_pending challenge step, generic OIDC SSO (signed state binding nonce+PKCE, conservative link/JIT), refresh-token reuse detection revoking chains, AES-GCM at-rest secret encryption, audit coverage + filtered admin query API, security/MFA/SSO UI |
| 2.4 Email-in/out v1 | `stage2-email` | new `core-mail` crate: `MailTransport` (Postmark/SMTP-mailpit/noop) + `ScanProvider` (clamd/noop) traits, none-mode documented; HMAC-authenticated inbound webhook → scan → thread (References/In-Reply-To → subject token → routed new ticket), message-id idempotency; outbound send loop in worker claiming via core-api (single-writer preserved); deterministic two-simulated-weeks zero-loss CI harness |

### Criteria status

- [x] Per-board custom statuses bound to semantic categories; behavior binds to category only
- [x] @mentions + notification center with per-channel prefs + quiet hours
- [x] Audit log coverage for auth events + query surface
- [x] TOTP **and** WebAuthn MFA; OIDC SSO; refresh reuse detection
- [x] Email-in/out end-to-end; zero-loss mail harness green in CI-style run

### Integration notes (coordinator)

- Shared-file merges resolved additively (`routes/mod.rs`, `openapi.rs`, worker loop wiring); SDK snapshot regenerated from the live merged API (**58 paths**, up from 43)
- One real integration bug found & fixed: two lanes drew the same db_suite seed slot (21) → org-PK collision under parallel tests; slot ranges are now pre-assigned per lane (`ec8a84c`)
- Testcontainer name made env-overridable so concurrent lanes never collide (`855f886`, `MERIDIAN_TEST_PG_NAME`)

### Known gaps / deferred

1. WebAuthn state serialization uses `danger-allow-state-serialisation`; verify server-side-only storage during security pass.
2. OIDC discovery/JWKS plumbing is hand-rolled on reqwest+jsonwebtoken; consider `openidconnect` crate during hardening.
3. Notification email fan-out runs on noop transport until settings-hub wires real transport config.
4. Inbound email malware scanning is clamd-optional (noop default) — document operator runbook when attachments land.

## Stage 2 — waves 4–6 (daily-driver bulk)

**Status:** merged through `d376d25`. Six more parallel lanes landed on top
of wave 3; every lane shipped tests with implementation. Live-stack
acceptance re-run on merged main: **18/18** incl. WS push chain, p95 ≤ 38 ms.

### What shipped

| Item | Lane | Highlights |
|---|---|---|
| 2.7 SLA engine + automations v1 | `stage2-sla` | `0015_sla_v1.sql` — org business-hours calendars (DST/holiday aware), policy/ladder attachment, fire-once escalation ledger; pure ladder evaluation (proptests); general job queue in worker (SKIP LOCKED, retries, dead-letter) driven by DB clocks — clock-skew tolerant by construction; automations consumer turns matched rules into effect jobs applied via core-api's token-gated internal surface (§4.1 single-writer preserved) |
| 2.5 Settings hub + registry | `stage2-settings` | `0018_settings_registry.sql` — typed settings registry (key/kind/default/scope + validation) rendered FROM / read THROUGH; settings shell with General/Preferences/Security pages consolidating the notifications-prefs and MFA/OIDC surfaces; skippable Tier-1 org wizard (defaults-first: skipping changes nothing) |
| 2.9/2.10 Timer discipline + missing time | `stage2-timer` | `0017_timer_discipline--breaking.sql` — single-active invariant via generated `is_running` column + partial unique index (paused timers dock, not block); switch = atomic auto-pause-with-reason + start; gap-audit ledger with once-per-day nudge claim; EOD nudge scheduler loop feeding the notification center (`missing_time` kind honors prefs/quiet hours); docked timer bar + missing-time strip + quick-log dialog per Today mockup |
| 2.11 Board organize + RBAC + my-work | `stage2-mywork` | `0019_board_organize_rbac.sql` — board groups/favorites, board_members roles (viewer/member/admin) with deny-wins matrix over ops, cross-board my-work endpoint (priority→SLA-due ordering); left-nav board tree + Today my-work queue per mockup |
| 2.13 Client portal v1 | `stage2-portal` | `0021_portal_v1.sql` — portal_users (contact/approver/it_manager), separate JWT audience, per-client RLS scoping ON TOP of org RLS; ticket view/reply restricted to client-visible notes (it_manager additionally reads in-house updates, never internal); staff admin CRUD for portal users + board exposure flags; distinct /portal shell |
| 2.15 Webhooks out + API keys + connectors | `stage2-webhooks` | `0020_webhooks_connectors.sql` — HMAC-signed deliveries w/ timestamp anti-replay, exponential backoff, per-attempt delivery logs visible via API; scoped rotatable API keys (hash-at-rest, show-once, last-used tracked, x-api-key machine auth); NinjaOne + Hudu reference connectors under `connectors/` written against ONLY @meridian/sdk |

### Criteria status

- [x] SLA provably correct under clock-skew tests (DB-clock sweeps, host clock paces cadence only)
- [x] Escalation ladders → notification center kinds (prefs/quiet hours apply automatically)
- [x] Automations v1 event-triggered with audit
- [x] Single-active-timer workflow server-authoritative + WS events
- [x] Missing-time EOD nudges + manager/self gap reports
- [x] Board groups/favorites + my-work + board-level RBAC enforcement
- [x] Portal v1 with IT-manager tier and client-visible-only content
- [x] Signed webhooks with retry/backoff and in-app-visible delivery logs; third-party-pattern connectors on public API only

### Known gaps / deferred

1. Admin config screens (2.6): workflow editor, SLA/calendar editors, webhook delivery-log UI — backends all exist; UI consolidation is the remaining settings-hub slice.
2. Webhook subscription secrets dead-letter fail-closed when `MERIDIAN_SECRET_KEY` unset on worker (documented none mode).
3. Portal MFA deferred to portal v2.
4. Scheduling reminders/notification hooks deferred (data model dispatch-ready).

## Stage 2 — close-out (2.18–2.6 remainder)

**Status: STAGE 2 COMPLETE.** Final three lanes merged and verified on main:

| Item | Lane | Highlights |
|---|---|---|
| 2.18 OTel baseline | `stage2-otel` | `core-otel` crate (opt-in OTLP, disabled-by-default none mode), HTTP/WS/worker spans + `meridian.http.*` metrics, redaction+cardinality policy tested; observability compose profile with collector→Tempo/Loki + Grafana provisioning |
| 2.19 Dispatch board v1 | `stage2-dispatch` | capacity math + day-view assembly + conflict-checked bulk-move under EXCLUDE constraints; org-channel WS subscriptions (own-org authz) for presence/appointment events; drag/drop grid per mockup with ghost preview, conflict badges, optimistic reconcile; headless drag-state reducer w/ vitest |
| 2.6 Admin config screens | `stage2-admin-ui` | workflow editor (category-chipped statuses, transition matrix, checklist template), SLA calendar+ladder editors with referenced-by counts, webhook manager (event multiselect, rotate, paged delivery log, test-fire), API-keys page (scopes, show-once secret, revoke, last-used); SDK-typed mock-fetch vitest suites |

**Final verification on merged main:** fmt · clippy `-D warnings` · **46 Rust
suites / 74 web tests green** · spec snapshot at **122 paths** · live-stack
acceptance **18/18** incl. WS push chain and p95 ≤ 15.2 ms.

### Stage-2 exit criteria (BUILD-PROMPT §7 line vs. evidence)

- Email-in/out two-simulated-weeks zero-loss harness — CI-style in-repo ✓ (2.4)
- SLA provably correct under clock skew — DB-clock sweeps + property tests ✓ (2.7)
- Workflow customization bound to semantic categories only ✓ (2.1/2.11)
- @mentions + notification center w/ prefs/quiet hours ✓ (2.2)
- Notes taxonomy + enforced pre-close checklists ✓ (2.8)
- Single-active timer + missing-time EOD nudges ✓ (2.9/2.10)
- Client portal v1 incl. IT-manager tier ✓ (2.13)
- OIDC SSO + TOTP/WebAuthn MFA + audit coverage ✓ (2.3)
- Plugin runtime v1 (signed webhooks, scoped API keys) + NinjaOne/Hudu refs ✓ (2.15)
- Scheduling/calendars/availability + dispatch board v1 ✓ (2.12/2.19)
- OTel instrumentation ✓ (2.18)
- Settings hub + admin configuration screens ✓ (2.5/2.6)

### Known gaps carried into Phase 3

1. WebAuthn state serialization review (`danger-allow-state-serialisation`) during security pass.
2. OIDC discovery/JWKS on reqwest+jsonwebtoken — consider `openidconnect` crate.
3. Portal MFA; scheduling reminders; report definitions UI (R3/R6 tracks added).
4. Upgrade-policy retrofit backlog U1–U10 tracked in TASKLIST.
5. Per-ticket SLA badge shipped for detail view; list-column integration pending views work.

## Release v0.1.0

Tagged at Stage-2 close per upgrade-policy §3/U10: strict semver anchor for
the N−2 chain (U7), release notes carry an empty "Breaking & deprecated"
section (§7) — no released API existed before this tag to break.

## Wave 7 — Phase-3 opener (U1 · R0/R1 · D2 core · website)

**Status: merged and validated on main (`74a6eb4`).** Four parallel lanes off
`225b81c`, three of four completed first-run; import-core required one revival
(docker outage recovery) then shipped clean.

| Item | Lane | Highlights |
|---|---|---|
| U1 API path versioning | `p3-api-versioning` | `/api/v1` canonical mount beside one-generation legacy aliases via shared path-builder; spec documents v1 ONLY (128 paths); parity tests (payloads, 401 shapes, mounts); acceptance harness proves aliases keep CI green unchanged; shipped `dump-openapi` bin + served-spec drift guard test |
| R0+R1 Reporting | `p3-reports` | `0026_reports_read_model.sql`: security-invoker `ops.v1_*` views (tickets/time/SLA outcomes/gap days/first-response) with per-column docs; `core-reports` field catalog + typed report kinds + parameterized compiler (no raw identifiers); CSV streaming; self/manager RBAC; read-only BI role recipe documented (ADR 0001 realized) |
| D2 core Import | `p3-import-core` | `0027_import_framework.sql` mappings/batches w/ RLS; pure engine — CSV/JSON parsing matrix, conflict policies (skip/update/fail), proptests; idempotency via external-ids; `Actor::Import` audit provenance; dry-run then chunked batch lifecycle endpoints (org-admin gated) for clients/sites/contacts |
| Website phase 1 | `p3-website` | Standalone waitlist site (own pnpm package): double opt-in with hashed tokens + SMTP none-mode fallback, public-safe roadmap copy from repo docs only, theme-token styling without product-code imports, 32 vitest incl. live-PG happy path |

### Validation on merged main

fmt · clippy `-D warnings` clean · **63 Rust suites / 0 failures** · web vitest
green (74 app + 34 website) · TS 0 errors · spec snapshot **128 paths
(v1-only)** regenerated authoritatively · acceptance **18/18** incl. WS push.

### Integration fixes (coordinator)

1. `default-run = core-api` — new `dump_openapi` bin broke plain `cargo run -p core-api`.
2. Reporting-pool field added to two e2e AppState initializers missed by lane rebases.
3. Route-literal → `rp()` idiom conversions during conflict resolution.
4. Stale-fingerprint quirk after migrations landed: forced `core-db` rebuild re-embedded migrator.

### Open ops item

U7 fixture capture for v0.1.0: worktree checked out at
`../meridian-fixture-v0.1.0`; build→seed→dump into `fixtures/upgrades/`
remains the next coordinator task before Wave 8.

## Phase 3 progress — Waves 8–9 (interim, as of 2026-09-03)

> Interim append: Phase 3 (commercial engine) runs as parallel lane waves;
> per-wave acceptance detail lives in TASKLIST.md's roadmap + checkbox notes.
> This section records what shipped and the standing UI gaps between waves.

### Wave 8a (2026-08-30) — envelope-first
IF1–IF3 instant-first write path (migration 0028; idempotent writes,
read-your-write, typed error envelope centrally) · U5 customization-manifest
export (`GET /org/manifest`, golden-stability + secret-leak suites) · R2
dashboards UI (My day / Team load / SLA health). Merge order doctrine: IF →
U5 → R2, zero conflicts.

### Wave 8b (2026-08-31 → 09-02) — commercial substrate on the envelope
All lanes merged: contracts + recurring billing rules (0029) · projects line
(0030, 23 endpoints, milestone billing seam) · native KB v1 (0031, --breaking)
+ D4 DocumentVersion (0032) · D1 org-data export + D2 remainder (0033) · R3
report definitions as data (0034) · D5 KB import (0035) · IF4–IF6 web client
read models (seq-keyed cache, offline replay, hydrate-once) · U2 event
conformance + U4 deprecation clock (contract registry, goldens, CI gate).

### Wave 9 (2026-09-02/03) — monetization + platform surface, 11 lanes
Lanes 1–9 backend, then the UI close-out (doctrine introduced this wave:
every wave ENDS with its web close-out on merged APIs only, whole features
end-to-end or visibly absent, typed SDK only, dark parity + motion law):

| Lane | Shipped |
|---|---|
| 1 invoicing (0036) | invoice runs → lines → `PaymentProvider` (none-mode) → dunning; "billed once, ever" partial-unique guards; print-ready HTML doc; PDF gap recorded |
| 2 R4 delivery (0038) | `report_schedules` + `report_deliveries`, DST-safe cadence, worker delivery loop, two-weeks harness |
| 3 portal v2 (0039) | invoice view+pay, KB self-service, appointment requests (intent table + staff approval booking) |
| 4 MCP v1 + IF8 (0040) | mcp-server read tools from OpenAPI, capability registry deny-by-default, IF8 proposals (CAS-guarded decisions) |
| 5 IF7 undo (0041) | `ops.v1_*_undo_stack` views + revert-as-IF1-write; open vs reverts_only modes |
| 6 D3 connector (0042) | bulk push/pull, checkpoint cursors, per-key rate limits, conformance suite |
| 7 IF9 provenance (0043) | per-field `provenance_events` projection + `GET /api/provenance`; `ProvenanceContext` on the envelope |
| 8 adapters (0044) | QBO/Xero export-sync crates, `CompletionProvider` none-mode, BYOK sealed keys |
| 9 dispatch v2 (0045) | resource planning, connector API, travel/mileage drafts |
| 10 web close-out (74e362c3) | billing pages, dispatch planning/travel, agent-platform admin, AI/BYOK settings, provenance browser, undo binding |

Spec at **180+ v1 paths**, 45 migrations, event contract registry gates every
lane merge. Known integration lessons: lanes branched before each other's
merges type-break on widened structs (`RecordSpec.provenance`, tri-state
`TicketPatch.body`) — resolved at merge with the canonical patterns.

### Known gaps carried forward (see TASKLIST "UI buildout backlog")

1. Staff UI landed for projects, clients CRM, contracts, board admin (Wave 10);
   documents, import wizard, KB authoring, alerts, R3 definitions, R4 schedules
   ride the Wave 11 money-docs/ops close-out lanes (in flight).
2. Invoice documents print-to-PDF via browser only (no server PDF binary).
3. Portal quote approval deferred until the quotes lane exists.
4. Org-users management UI lands with the Wave 11 close-out (backend live:
   directory, invite flow with one-time setup tokens, activate/deactivate,
   role assignment).

### DB access scoping (doctrine, 2026-09-09)

After the RLS-scope incident (70+ handlers 500ing/returning empty under RLS),
all route-level DB access is org-bound: handlers acquire a transaction,
`core_db::set_org_and_user(&mut tx, org, user)` sets `app.org_id`/`app.user_id`
(is_local), queries ride `&mut *tx`, then commit. Raw-pool queries against
RLS-protected tables are FORBIDDEN — fresh connections hide rows (NULL scope)
and previously-scoped connections fail casting the `''` placeholder. Session
creation and a few auth resolvers are the only exceptions (SECURITY DEFINER
bridges: `app_resolve_login`, `app_create_session`, `app_create_portal_session`,
`app_resolve_session`). Pattern reference: `crates/core-api/src/routes/notifications.rs`
CI grep gate for new pool-reads: tracked in TASKLIST (RLS-scope audit).

### Waves 10–11 (2026-09-04 → 09-09, interim)
Wave 10 landed: nav pass (grouped nav + active states, global header) + workspace-core UIs (clients/projects/contracts/board admin). QA findings fixed: systemic RLS-scope 500s (~70 handlers moved to org-bound txs), search index seeding, SLA none-mode, register slug UX tracked. Wave 11 fired (8 lanes): vision-driven UI quality pass, org-users backend, ticket taxonomy, scheduling links, assets UX, money+docs UI, ops UI, workflow constructs (tags/merge/costs/PM tickets). Typography changed by owner decision: JetBrains Mono headings via shadcn preset b3J8o4J600 (theme.md + BUILD-PROMPT updated).

## Wave 12 — alpha tranche 3 remainder (2026-09-11 → 09-12, LANDED)

Eleven+ lanes closed Phase 3 (the alpha gate). All merged to main; landing
records in `TASKLIST_COMPLETED.md`.

**Backends (tranches A+B):**
- `p12-main-green` b8b944c — the four gate-blockers cleared: contract-registry
  goldens for document_layout.*/password.*/link., worker search_indexer
  compile fix, BYOK delete-500 hardening (0057), vitest DOM isolation.
- `p12-quote-catalog` 32ba768 (0060) — quote builder, product/service catalog,
  sales pipeline, full-stack backend.
- `p12-inventory-ci` 0027320 (0061) — inventory/asset lifecycle (rental,
  depreciation) + configuration identity core CI1–CI6 on the lane-3 seam
  (ADR 0003).
- `p12-vector-search` 15e1fb9 (0062) — pgvector embeddings + RRF hybrid
  fusion, exactly-once side table, tenant-scoped exact KNN (ADR-recorded
  non-feature: no global ANN in v1).
- `p12-workflow-overrides` 67466d4 (0063) — per-client workflow overrides:
  typed closed-key registry, resolution precedence (client set > client deny
  > org default), undo-trailed.
- `p12-ticket-email-gaps` d62702a (0064) — per-ticket email read (reply
  chains + delivery projection), checklist N-A column, email
  greeting/sign-off settings, pod-visibility store, ticket read-view
  classification, scheduling auto-email through the outbound ledger + 31-day
  horizon lift.
- `p12-ci-feeders` 422db4e — CI7 feeder conformance: NinjaOne/IT Glue/Auvik
  through `ConfigSourceLink` (deterministic identity keys, never
  blind-insert; second sync updates its own link), machine-key scoped access
  on /api/config-items; also restored lane-6-merge-lost event registrations.
- `p12-adapters2` 91af407 — live REST transports: adapters-core shared
  primitives (VendorAuth incl. OAuth2 CC, PageFlow, OrgMapping), IT Glue
  live, M365 Graph, ImmyBot, DMS live sources; none/dry modes everywhere.
- `p12-fx` b47a5bf (0065–0067) — FX1–FX7 (ADR 0002): org base currency,
  Money generalization, append-only fx_rates, five snapshot lock moments,
  half-even conversion, manual-entry provider, ops.v1_fx_reconciliation.

**Review remediation be53a13 (coordinator pass, 2026-09-12):** 13 findings
fixed — SSRF continuation-URL host pinning (adapters-core), negative-rate
ledger corruption (`-0.5` parsed positive), FX lock path riding the org-bound
tx (pool-starvation hazard), integer-space half-even fact amounts (no floats
near money; view + domain share one rule), typed 409 for amount edits on
snapshot-bearing facts, base-currency guard counting base-denominated money,
header base totals = Σ line snapshots, 0065 constraints NOT VALID +
VALIDATE (upgrade-safe), reconciliation partial indexes + additivity
contract.

**Web close-out `p12-t3-closeout` 8a75306:** all Wave 12 APIs wired into
staff UI per the close-out doctrine — quotes/catalog/pipeline, inventory +
CI merge queue, workflow-overrides, FX surfaces (base currency, rate book,
reconciliation), vector search in unified search, DMS connector admin,
ticket email timeline rows, N-A tri-state, greeting/sign-off editors,
pod-visibility matrix; screenshot evidence under `design/qa-t3-closeout/`;
SDK at 320 paths (DX12 pin gate active).

**In flight:** `p12-realtime` (lane 12, user-added) — SSE fan-out off the
event trail via LISTEN/NOTIFY + ticket presence (big-team live collaboration).

**Verification posture:** every lane suite-green at close-out on its own
lane DB; coordinator re-verified merges on main (fresh-DB migration chain
0001–0067, db_suite 76, core-domain 255, connectors 60, core-events 22,
conformance 12, web typecheck + vitest 177). Alpha gate = end of Wave 12 —
CLOSED 2026-09-12.

## Wave 13 — hardening + platform tranche (2026-09-13, LANDED)

All 10 lanes merged (envelope-first ordering held: lane 1 alone, then tranches B/C):

- **`p13-error-envelope` 916e54b** — typed IF3 error envelope: reason-code registry
  (`core-api/src/error.rs`, 25 codes, conformance-tested), constraint-name registry
  (`map_unique_constraint`) closing raw `orgs_slug_key`-class leaks, `typed_rejections`
  middleware sanitizing all framework/serde rejections, portal audit swallow fix.
- **`p13-web-graceful` 4a2f346** — web/console leg: reason→copy map
  (`apps/web/lib/error-copy.ts`), ConnectionBanner offline/reconnecting states,
  register one-click slug suggestion accept, dev-page `already_seeded` force-reseed,
  correlation-id middleware (`otel_http::correlate`), mfa webauthn fixed copy.
- **`p13-rls-audit` 8bf681d** — pool-read sweep found ZERO unscoped authed-user
  stragglers (15 raw-pool reads all sanctioned pre-auth SECURITY DEFINER bridges,
  allowlisted with rationale); CI grep gate `scripts/check-rls-pool-reads.sh` +
  `rls-pool-reads` job (negative-tested).
- **`p13-kb-rich` ec15c52** — KB1 oEmbed allowlist registry + KB2 linked external
  documents (migration 0069), server-side link-check as data, published-only search
  mirroring (draft/archived articles invisible to staff search AND portal).
- **`p13-template-registry` 4a30cde** — KB3/KB4 `Template` entity (migration 0070):
  closed-variable render engine, 7 shipped immutable defaults, fork/restore semantics.
- **`p13-preset-roles` 1b1c3b9** — RB1 preset roles as data (migration 0071,
  8-preset roster, forked_from + restore) + RB2 `financial:read` gating.
- **`p13-identity-federation` dff2164** — ID1–ID4 (migration 0072): OIDC presets
  (Google/Entra/Okta), Cf-Access edge-assertion auth, deny-by-default versioned
  bundle interface, federated MFA gate.
- **`p13-perf-baseline` 3358e87** — ticket-list p95 29.9ms @50k (budget 150ms,
  ~5× headroom); CI `perf-smoke` job; `crates/perf-harness` seed/measure/load;
  baselines in `docs/perf.md`.
- **`p13-realtime-resume` fb2b266** — SSE resume (`Last-Event-ID` bounded outbox
  replay, documented past-buffer cutoff) + presence multi-replica fan-out
  (`meridian_presence` pg NOTIFY relay).
- **`p13-realtime-ui` 0f870a1** — realtime web wiring: EventSource →
  `/api/events/stream` feeding the instant reconcile path, presence rosters,
  `version_conflict` as reconcile copy.

Migration chain 0068→0072 complete. Wave 12's realtime lane follow-ups fully closed.

## Wave 14 — Phase 4 opening tranche (2026-09-13 → 09-14, tranches A+B LANDED; C+D remain)

- **Tranche A (all merged):** `p14-time-approvals` d995296 (migration 0073 —
  approval state machine, decision trail, `ops.v1_time_entries` extended),
  `p14-calendar-sync` 48076e5 (migration 0074 — `CalendarSyncProvider` seam +
  first-party iCal, bidirectional), `p14-report-builder-v1` 327e516 (R6 builder UI,
  field catalog, injection-proof `$N` run surfaces), `p14-docs-site` f5c20c2
  (Diátaxis docs site, 19 pages, generated API reference + upgrade matrix with a
  byte-identity check gate).
- **Tranche B (all merged):** `p14-coaching-scorecards` a39a86e (migration 0075 —
  R5 aggregation over `ops.v1_*` + staff-facing scorecard pages) and
  `p14-plugin-v2` 98edd74 (migration 0076 — plugin runtime v2 + marketplace
  registry, declarative capability grants on the existing permission matrix,
  deny-by-default sandbox seam).
- **Remaining:** lane 7 `p14-web-closeout` (tranche C — wires approvals, calendar
  sync, scorecards into staff UI per the close-out doctrine) then lane 8
  `p14-e2e-ci` (tranche D — Playwright E2E for the 10 core flows in both themes).

**Verification posture:** every lane suite-green at close-out; coordinator
re-verified merges on main (typecheck + pins byte-identical + migration lint).
Offline-compile gap (missing `.sqlx` test-target entries) found and healed.
DX6 drift hardening: setup script strips session `CARGO_TARGET_DIR` overrides,
dev-clean reclaims drifted private caches.
