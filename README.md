# Meridian

**The open-source PSA that MSPs own.** Tickets, time, clients, dispatch, projects, invoicing, knowledge base — everything an IT service team runs on, self-hosted, in a database you control.

> **Status:** Phase 3 (commercial engine) — alpha gate closed 2026-09-12; Wave 12 (quotes, catalog, pipeline, inventory + configuration identity, vector search, FX, per-client overrides, live collaboration) landed. See [Roadmap](#roadmap).
> **License:** AGPL-3.0 · SDK, plugins & connectors: Apache-2.0

<!-- TODO(ci): badge row once pipelines exist
![CI](https://github.com/Marlin-Hofmeyer/meridian/actions/workflows/ci.yml/badge.svg)
![License](https://img.shields.io/badge/license-AGPL--3.0-blue) -->

---

## Why we're building this

Every commercial PSA taxes tickets, locks the data, and bends your workflow to its roadmap. We got tired of it.

Meridian is complete on its own. Invoicing, knowledge base, CSAT, scheduling — all native, no third-party subscriptions required to run a real MSP.

Some things we care about enough to enforce in CI:

**Workflows that don't break.** Customize boards however you like. Every status maps to a semantic category underneath (open / in progress / waiting / closed), and SLA timers, billing, automations, and reports bind to those categories — never your labels. Rename anything; nothing silently changes meaning.

**Time that actually gets billed.** One active timer per tech. End-of-day missing-time audits. Approvals before invoices go out. The gap between hours worked and hours logged stops being a mystery.

**Agents you can audit, and that default to asking.** Skills are versioned manifests, not prompt spaghetti. Guardrail rules evaluate in layers before every tool call. Agent-originated changes to a record default to *proposed, awaiting approval* — nothing silently rewrites your work, and client-visible or external actions never bypass a human. The audit log records who did what — human or agent — and it's append-only. "Who changed this" is a query, not a sift. There's an MCP server too, so Claude, DeepSeek, or your own scripts can work inside the PSA under the same rules as everyone else. See [`docs/instant-first.md`](docs/instant-first.md) and [`docs/10-agent-platform.md`](docs/10-agent-platform.md).

**Fast, measurably — and there's no Save button to slow you down.** Ticket list p95 under 150 ms at 50k tickets. WebSocket delivery under 100 ms. These are CI gates, not aspirations. The dominant path is autosave with optimistic render and rich undo: you never chase a Save button or a page reload to feel committed, and Ctrl/Cmd+Z walks back through edits the server already accepted.

## Quick start

```bash
git clone https://github.com/Marlin-Hofmeyer/meridian && cd meridian
docker compose -f deploy/docker-compose.yml up -d     # Postgres+pgvector, Redis, MinIO, services, web
open http://localhost:3000                            # workspace UI
```

Developer harness with hot reload:

```bash
cargo install --path crates/meridian-cli
meridian dev
```

> Full installation guide, configuration reference, and upgrade runbooks live in [`docs/`](docs/); generated API reference (Scalar UI) is served by `core-api` at `/docs`.

---

## What's inside

| Module | |
|---|---|
| **Ticketing** | Boards, custom workflows bound to semantic categories, SLA timers with calendar awareness, automations, structured notes, email-in/out threading |
| **Time** | Single-active-timer workflow, segment tracking, missing-time audits, approvals, cost/bill rates |
| **Dispatch** | Capacity board, unassigned queue, conflict detection, travel estimates, resource planning across service + project work |
| **Projects** | Phases, tasks, dependencies, templates, milestone billing |
| **Commercial** | Contracts, quotes, catalog, inventory, pipeline → **native invoicing** with payments and dunning |
| **Knowledge base** | Articles, categories, full-text search, client-facing visibility |
| **Client portal** | Tickets, invoices + payment, quote approval, KB self-service |
| **Agent harness** | Skills (versioned playbooks), layered rules, MCP server over the domain API, provenance-complete audit |

## Architecture

```
web (Next.js) ──► core-api (Rust/Axum) ──► PostgreSQL (RLS · FTS · pgvector)
        │                │   outbox ──► Redis streams ──► realtime (WebSocket hub)
        │                ├── worker (jobs: email · SLA · sync · AI hooks)
        │                └── plugin-host (Extism WASM + HTTP connectors)
        └── @meridian/sdk clients · MCP server · signed webhooks
```

One writer. Every mutation emits a versioned domain event. REST, WebSocket, webhooks, plugins, and MCP are four surfaces over one contract.

## Documentation

| | |
|---|---|
| Install & self-host | [`docs/`](docs/) — install guide, config reference, upgrade runbooks |
| Upgrades & compatibility | [`docs/upgrade-compatibility-policy.md`](docs/upgrade-compatibility-policy.md) — support windows, breaking-change rules, what's guaranteed |
| Interaction contract | [`docs/instant-first.md`](docs/instant-first.md) — autosave/undo, agent approval-by-default, the core's read models |
| API reference | Generated from OpenAPI (`utoipa` → Scalar UI on `core-api`) |
| Build plugins & connectors | [`packages/plugin-kit`](packages/plugin-kit/) · [`connectors/`](connectors/) reference implementations |
| Architecture decisions | [`docs/adr/`](docs/adr/) |
| Visual identity | [`docs/theme.md`](docs/theme.md) |

## Roadmap

Development runs in stages: foundation → daily driver → commercial engine → GA hardening → ecosystem. Current stage and near-term items are tracked in repository milestones; the full staged plan with acceptance criteria lives in [`docs/prompts/BUILD-PROMPT.md`](docs/prompts/BUILD-PROMPT.md).

## Contributing

PRs welcome — trunk-based, DCO-signed, conventional commits. Two approvals are required for `crates/core-domain` and anything touching authz/RLS. Start with [`CONTRIBUTING.md`](CONTRIBUTING.md); significant changes begin as RFCs in `docs/rfc/`.

Extension authors usually don't need this repo: connectors are ordinary web apps using [`@meridian/sdk`](packages/sdk/) and receiving signed webhooks — the NinjaOne and Hudu connectors are the reference implementations.

## Security

Report vulnerabilities privately — see [`SECURITY.md`](SECURITY.md). Please do not open public issues for security reports.

## License

- Core (`crates/`, `apps/`): [AGPL-3.0](LICENSE)
- SDK, plugin kit, first-party plugins & connectors: [Apache-2.0](packages/sdk/LICENSE)

---

## Repository layout

<details>
<summary>Module map</summary>

```
crates/
├── core-api/       # Axum HTTP API — the ONLY database writer
├── core-domain/    # entities, policies, domain services (pure, heavily tested)
├── core-db/        # SQLx queries + migrations
├── core-events/    # outbox, versioned event envelope, Redis stream publisher
├── realtime/       # WebSocket hub service
├── worker/         # Postgres-backed job queue + job definitions
├── plugin-host/    # Extism WASM runtime + connector proxy
├── mcp-server/     # MCP protocol adapter — derived from OpenAPI + capability registry
└── meridian-cli/   # `meridian dev`, setup, admin tasks
apps/web/           # Next.js app (workspace UI + client portal)
packages/
├── sdk/            # @meridian/sdk — typed client generated from OpenAPI (Apache-2.0)
├── ui/             # design system components — Workbench theme tokens (Apache-2.0)
├── layout/         # multi-pane layout engine
└── plugin-kit/     # TS plugin PDK + dev harness (Apache-2.0)
plugins/            # first-party WASM plugins (Apache-2.0)
connectors/         # first-party HTTP connectors (Apache-2.0)
design/mockups/     # approved UI mockups (light + dark per screen) — implementation targets
deploy/             # docker-compose.yml, Helm chart, upgrade runbooks
website/            # project marketing site — waitlist first, then live (mockup-first)
docs/               # Diátaxis docs, ADRs, RFCs, theme spec, build brief
.github/            # CI, templates, actions
```

Every module's README states its purpose *and its non-purpose* — read them before touching code.
</details>

## Acknowledgments

Meridian is built on Rust, Axum, SQLx, PostgreSQL, Next.js, Tailwind CSS, shadcn/ui, Extism, and faster-whisper. Thanks to everyone behind them.


