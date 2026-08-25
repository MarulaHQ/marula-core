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
