---
description: 'The docs coverage ledger: which Core surfaces have a page, which are planned and where each gap is owned.'
---

# Docs coverage audit & IA plan (Wave 27 lane 1)

Status: audit-only deliverable · Audit basis: Waves 11–26 GA product as of 2026-09-22

This is the coverage matrix and information-architecture (Diátaxis) plan for
the open-source docs. Lane 2 (`p27-docs-fill`) executes the work order at the
bottom. This page itself is the audit record, wired into the site so the gap
stays visible until it is filled.

Method: every scheduled product surface was captured and reviewed in a real
running stack (76 screenshots, `internal/design/qa-p27-docs-audit-ia/captures/`,
verdicts in `internal/design/qa-p27-docs-audit-ia/REVIEW.md`), then cross-checked
against this site's existing pages and the self-host doc sources
(`deploy/README.md`, `deploy/docker-compose.yml` comments,
`deploy/runbooks/README.md` scaffold, `docs/desktop-release.md`,
`docs/reference/`, ADRs 0005–0011).

Legend: **✅ exists** · **◐ partial** (some coverage, not the task in front of
the user) · **❌ missing** · **🌀 generated** (never hand-edited, staleness is
mechanically checked, currently clean).

## 1. Coverage matrix (product domains × Diátaxis quadrants)

| Domain (surfaces) | Tutorial | How-to (admin) | How-to (user) | Explanation | Reference |
|---|---|---|---|---|---|
| Core ticket workbench — today, tickets list, ticket detail, boards, dispatch, classic ticket (9) | ◐ `first-ticket.md` is API-only, not the UI | ❌ board administration, workflow categories, SLA clocks | ❌ work a ticket: notes, timer, workbar | ◐ `instant-first.md` (interaction), ❌ notes/tasks/workbar model | ❌ status/category taxonomy |
| Clients & devices — clients, client detail, devices, device perspectives (4) | ❌ | ❌ client onboarding, assets & connectors | ❌ | ❌ provenance link-state story | ◐ read-model `v1-clients` etc. |
| Billing & finance — billing, invoices, credit notes, contracts, FX settings (5) | ❌ first-invoice lesson | ❌ invoice lifecycle, FX rate book, reconciliation additivity | ❌ how time turns billable | ❌ multi-currency/FX contract | ◐ ADR 0002 |
| Sales — pipeline, quotes, catalog (3) | ❌ | ❌ quote → invoice flow | ❌ | ❌ | ❌ |
| Approvals & calendar & PM schedules (3) | ❌ | ❌ policy setup, schedule templates | ❌ calendar use | ◐ decision/model context (ADRs) | ◐ agent-proposals reference exists |
| Automations & alerts (2) | ❌ | ❌ rule authoring, alert routing | ❌ | ◐ `customizations-are-data.md` (row claim) | ❌ trigger/action catalog |
| Search, dashboards, reports, scorecards (4) | ❌ | ❌ report builder (compile-on-demand, versioned views), schedules, widget grid | ❌ pinned dashboards | ❌ why widgets-only / compile-on-demand | ◐ read-model catalog + `v1_*` views |
| Assets & documents (4) | ❌ | ❌ import/connector link state, document versioning semantics | ❌ | ❌ per-connector link-state-as-data, immutable versioning | ◐ ADR 0004 |
| Provenance, audit, imports (4) | ❌ | ❌ dry-run-first imports, saved mappings | ❌ | ❌ provenance (field-level) vs audit (event-level), append-only | ❌ |
| Knowledge base & secret vault (2) | ❌ | ❌ KB category grants (ADR 0006), secret reveal auditing | ❌ | ❌ restricted-chains grant model | ◐ ADR 0006 |
| Agent platform, AI settings, plugins (3) | ❌ first-agent-run lesson | ❌ enroll/approve/run-once, expose-to-automations, AI master switch + per-surface grants, plugin consent lifecycle | ❌ using granted AI surfaces | ❌ deny-by-default decision harness, digest-verified distribution | ◐ ADR 0008/0010/0011, agent-proposals & agent-skills in API ref |
| Settings — workspace/people/access, security, API keys, notifications (7) | ❌ | ❌ 2FA/sensitive-changes gate, API-key lifecycle (show once → rotate → revoke), invite flow, quiet hours | ❌ preferences | ❌ scope picker model | ❌ |
| Integrations & connectors, FX/system (2) | ❌ | ❌ per-connector setup | ❌ | ❌ | ❌ |
| Auth — login, magic link, SSO, password reset, verify email, invite acceptance (7) | ❌ new-org bootstrap ("Sign in / Create org") | ❌ SSO round-trip, MFA gating | ❌ passwordless parity ("same full sessions") | ◐ `auth-threat-model.md` in `docs/reference/` | ◐ `auth-provider-contract.md` |
| Portal — portal login, root, requests/invoices/knowledge/schedule (5) | ❌ MSP provisions → client first sign-in | ❌ board exposure controls, portal users | ❌ client-side portal usage | ❌ per-client isolation | ❌ |
| Self-host & operations — compose stack, profiles (stage2/observability), desktop app, dev hygiene | ❌ first-inventory-sync style bring-up | ◐ compose comments + `deploy/README.md` (not in site), ❌ as site page | ❌ | ❌ role-split (migration owner vs RLS app role) | ◐ `docs/desktop-release.md` (site-hosted ✅ for packaging), runbooks are a scaffold |
| API & upgrade (cross-cutting) | — | ✅ `upgrade.md`, ◐ `call-the-api.md`, ★ `read-model.md` | — | ✅ `upgrade-policy.md`, ✅ `customizations-are-data.md` | 🌀 `api.md` (451 paths, pin-synced), ★ `read-model.md` (+ `docs/reference/` per-view docs), 🌀 `upgrade-matrix.md` |

★ = genuinely thin but technically current. The every-quadrant ❌ verdict for
product domains is the headline: the docs site today describes the *platform
mechanics* (API, upgrades, read model, customization-as-data) but none of the
*product usage* MSP staff and admins live in daily.

## 2. UI/API mismatch notes found during the audit (feed lane 2 / product lanes)

- Settings→SLA escalation rows render `[object Object]m` for P1–P4 offsets — UI bug. Don't document names from that broken render.
- Settings→Notifications showed raw event slugs (`report_delivery`, `ai_cap_warn`, `secret_rotation`) without friendly labels — **resolved** in Wave 34 lane 8: every kind now renders a friendly label (`lib/use-notifications.ts` `KIND_LABELS`).
- Settings→Mail was an intentional placeholder ("backend lands with the mail administration wave") — **lifted** in Wave 34 lane 8: `/settings/email` is the mail administration surface, and [how-to/admin-email.md](how-to/admin-email.md) documents it. `/settings/mail` redirects.
- Board-fields and security pages have plausible transient error states ("Fields could not be loaded…", "Failed to load security state") — distal UI bug indicators, not doc content.
- `docs/site/index.md` "Where the rest lives" names `docs/runbooks/` which does not exist at that path (runbooks live under `deploy/runbooks/` as a scaffold) — fix the pointer while wiring lane 2.

## 3. Diátaxis IA plan (target structure)

Three audiences, stated up front on every page: **site operator / MSP admin**
(self-hosts, owns settings), **MSP staff** (mtime-to-billing practitioners),
**client portal users** (end customers — separate trust zone: portal docs
never assume settings access, and portal auth is provisioned by the MSP).

Conventions for lane 2:

- Prerequisites block on every how-to/tutorial (audience + what must already be true).
- Product-behavior claims in how-to/explanation pages must match the verified UI strings in `REVIEW.md` captures — behavior claims are evidence-backed, not invented.
- Generated pages (`reference/api.md`, `reference/upgrade-matrix.md`) stay generated. `reference/read-model.md` stays a catalog pointing at `docs/reference/read-model/*.md`.
- No describing unbuilt surfaces. When a surface ships, its placeholder line here is lifted and a how-to lands with it (mail lifted in Wave 34 lane 8 — see [how-to/admin-email.md](how-to/admin-email.md)).

### Tutorials (learning paths — worked end to end)

1. `first-ticket.md` (exists, rewrite) — currently API-first. Keep as the developer edition and add the UI edition: run stack → seeded org → work one ticket to billable time.
2. **First inventory sync** (new) — connect one connector → device inventory → convert a device to an asset → link to client (assets/connector link-state).
3. **First invoice** (new) — billable time → invoice → credit/void behavior → FX note for multi-currency stacks.
4. **First agent run** (new) — enroll agent (Settings→Agent platform 7-step ladder) → plan & approve → run once → read the ledger, plus turning an AI surface on for one role (AI master switch → grant → use).
5. **Portal onboarding** (new) — provision a portal user, expose boards, client first sign-in, first request.

### How-to guides (task-indexed, with the index page as the task catalog)

Admin tasks: upgrade ✅ (exists) · customize ✅ (exists, extend with board/category editing) · manage users & invites · configure SLA calendars & escalation ladders · set workflow categories · author automation rules · manage API keys (show once → rotate via replacement → revoke) · connect an IdP (SSO) · set up the portal (users + exposure) · run a dry-run import with saved mappings · record FX rates & read reconciliation (per-kind, never summed) · configure the plugin consent flow · manage dashboards (widget grid) & report schedules.

Staff tasks: work a ticket (notes → approve → bill) · dispatch from the board · use provenance on a record · use the provenance/audit distinction when explaining history · use KB class grant paths · use the keyboard shortcuts (Alt registry, hold-Alt overlay, ? cheat sheet) ✅ Wave 29.

Client tasks: portal sign-in · submit & follow requests · invoices · schedule.

### Reference

- Generated API/upgrade-matrix stays as-is (both currently regenerate clean).
- `read-model.md` per-view docs exist — wire the remaining views (`v1-clients`, `v1-contacts`, `v1-first-response`, `v1-fx-reconciliation`, board-statuses already linked. Audit catalog vs `ops.v1_*` list).
- Add: semantic category/taxonomy reference, event catalog (notifications + domain events + audit), settings surface map (the audited left-rail structure), plugin capability surfaces table.

### Explanation

- Exists: `why-marula.md`, `instant-first.md`, `customizations-are-data.md`, `upgrade-policy.md`, `upgrade-compatibility.md` catalog.
- New: provenance vs audit (both append-only, different granularity) · FX additivity & the append-only rate book · deny-by-default AI & the decision harness · crawly "customizations are data" applied to boards/taxonomy (already ◐) · portal isolation model · KB-class categories & restricted chains (ADR 0006) · secret vault reveal auditing · instant-first conflicts page.

### Self-host/operations lane

Do not fork `deploy/README.md` or `docs/desktop-release.md` into the site. Instead add **linked** operator pages in the site that embed the canonical commands: local stack (`deploy/docker-compose.yml` — ports/profiles/role-split), observability profile (`OTEL_EXPORTER_OTLP_ENDPOINT` gate, Grafana datasources), backup/restore runbook (the `deploy/runbooks/` scaffold's first real page), desktop packaging pointer. Author `deploy/runbooks/` content under `docs/site/reference/` per its own README.

## 4. Route → doc mapping (for p26 contextual help)

Every in-app surface carries an in-app help entry point (`p26-onboarding-flows`). Mapping rule: each route's help entry links to the canonical page for its task, target structure:

| Route family | Canonical doc |
|---|---|
| Tickets workbench, boards, dispatch | how-to: work a ticket / dispatch from the board |
| Invoice/Billing surfaces | how-to: invoice lifecycle (+ tutorial: first invoice) |
| Approvals & automations | how-to: author automation rules |
| Reports/dashboards | how-to: dashboards & reports |
| Assets/documents | how-to: import & connectors, document versioning |
| Provenance/audit | explanation: provenance vs audit |
| KB/vault | explanation: KB-class categories |
| Settings→General … Security | how-to subset per quadrant (see §3 how-to catalog) |
| Agent platform/AI | tutorial: first agent run |
| Integrations | how-to: connectors |
| Portal routes | tutorial: portal onboarding + client how-tos |
| Email (Settings→Email) | how-to: configure email (routing, SPF/DKIM/DMARC, transport, runbook) |
| Unmapped (transients) | UI-bug lane notes |

## 5. Lane 2 (`p27-docs-fill`) work order

1. Fix `docs/site/index.md` runbooks pointer (`deploy/runbooks/`).
2. Rewrite `tutorials/first-ticket.md` as the UI edition, keep the API edition cross-linked, and add the N−1 prerequisite mention of the seeded org (`owner@demo.local / demo-password-123`, 16 open tickets).
3. Tutorials: inventory sync, first invoice, first agent run, portal onboarding.
4. How-to task catalog per §3 (splitting by audience), starting with the P0 set: upgrade ✅ keep, SLA, users/invites, API keys, automation rules, board workflows.
5. Explanation set per §3 (start: provenance vs audit, FX additivity, deny-by-default AI).
6. Self-host pages per §3 operations lane + first runbook.
7. Reference catalog expansion (taxonomy, events, settings map) — hand-written parts only. Generated pages remain generated.
8. Every behavior claim cites the capturing verdict (screens supply the copy). Keep the "do not document" list from §2.

Exit criteria for lane 2: `node docs/site/scripts/check-docs-site.mjs` green,
this page's ❌/◐ rows re-scored, coverage re-audited against the same
captures before landing.

## 6. Lane 2 delivery note (`p27-docs-fill`, 2026-09-22)

Lane 2 shipped the P0 work order. The lesson on the first ticket is now
the UI edition, with the API edition cross-linked and the seeded org named
as a prerequisite. It added five tutorials (inventory sync, invoice, agent
run, portal onboarding), the audience-split how-to catalog (12 admin
guides plus staff, client, and SSO), five explanation pages, and the
self-host runbook spine under `reference/` (local stack, observability,
backup/restore, reverse proxy/TLS). The
docs-site checker now fails on any how-to/tutorial page missing its
**Audience:** tag.

## 7. Wave 34 lane 8 lift note (`p34-email-config-closeout`, 2026-09-26)

The mail do-not-document placeholder is lifted. `/settings/email` ships the
mail administration surface (inbound routing with collision validation,
per-board outbound identities, transport selection with sealed credentials,
the delivery ledger, the test-send probe, notification-email brand and
defaults, digest policy), and [how-to/admin-email.md](how-to/admin-email.md)
documents the operator path end to end: the inbound webhook contract, SPF
/ DKIM / DMARC, the transport choices (including the no-TLS SMTP
limitation), the retry and delivery-state semantics, notification/digest
behaviour, reply threading, the clamd-optional scan runbook, and the
self-host environment variables. The Stage-2 report's two mail honest gaps
(no transport configuration UI, mail undocumented) are resolved here. The
one remaining operator dependency is a real provider credential, which the
test-send probe and delivery log surface rather than hide.

## 8. Information-architecture pass (2026-10-20)

The matrix above is an audit record from 2026-09-22 and is left as written.
Since then the site was re-organised around the reader rather than the product
menu:

- [`start/`](start/index.md) holds one entry page per reader (owner, admin,
  technician, operator, agent author). Each lists tasks in the words people use
  and links the page that answers them. Tasks with no page are listed as gaps
  rather than filled with invented behaviour.
- [`troubleshooting.md`](troubleshooting.md) is entered by symptom, and sits
  outside the four quadrants for that reason.
- [`reference/glossary.md`](reference/glossary.md) defines the recurring PSA and
  Marula vocabulary.
- `node docs/site/scripts/check-docs-ia.mjs` fails on an orphan page, a page
  missing from its quadrant index, a dead `#anchor`, or an unknown audience tag.
  It runs inside `check-docs-site.mjs`, so the existing gate covers it.

The measured audit, the gaps it found and what was left open are recorded in
`internal/design/qa-p52-docs-ia/REVIEW.md` in the repository.
