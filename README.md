# Marula

<p>
  <img src=".github/assets/logo-lockup-ink@2x.png" height="34" alt="Marula logo lockup" title="Marula logo lockup">
</p>

<p>
  <a href="LICENSE"><img src="https://img.shields.io/badge/core-AGPL--3.0-5E7F3A" alt="Marula Core is licensed under the AGPL-3.0" height="20"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/sdk%20%2F%20plugins-Apache--2.0-E8B83A" alt="The SDK, plugins and connectors are licensed under the Apache-2.0" height="20"></a>
</p>

**The open-source PSA that MSPs own.** Tickets, time, clients, dispatch, projects, invoicing, knowledge base: everything an IT service team runs on, self-hosted, in a database you control.

> **Status:** Phase 3 (commercial engine) — alpha gate closed 2026-09-12; Wave 12 (quotes, catalog, pipeline, inventory + configuration identity, vector search, FX, per-client overrides, live collaboration) landed. See [Roadmap](#roadmap).

<!-- TODO(ci): CI badges once the pipelines are public. Rendered now, a
     workflow badge on a repository with no runs reads as "no status", which
     is worse than no badge.
![CI](https://github.com/MarulaHQ/marula-core/actions/workflows/ci.yml/badge.svg) -->

---

## Who Marula is for

Whether you run the service desk, coordinate the dispatch board, work tickets in the field, or send out monthly invoices, Marula gives your team one place to work:

- **Technicians** keep a single active timer running, log customer-facing and internal notes, scan assets and parts from their phone, and close out day-end time gaps in a couple of minutes.
- **Dispatchers and service leads** see real technician capacity on the calendar, spot SLA risk before a ticket breaches, and schedule visits with live drive-time estimates.
- **Office managers and owners** review approved hours against client retainers and project milestones, issue invoices and credit notes directly from the work already logged, and give clients a clean self-service portal.
- **Systems administrators** host the entire stack on standard PostgreSQL and Valkey, control AI and plugin permissions down to individual roles, and inspect a tamper-evident audit trail for every change.

## Why we're building this

Every commercial PSA taxes tickets, locks the data, and bends your workflow to its roadmap. We got tired of it.

Marula is complete on its own. Native invoicing, quotes, knowledge base, CSAT surveys, and dispatch scheduling all come built in, with no third-party subscriptions required to run a real MSP.

Some things we care about enough to enforce in CI:

**Workflows that don't break.** Customize boards however your shop works. Every status maps to a core stage underneath (`open`, `in_progress`, `waiting`, or `closed`). SLA timers and billing rules, along with reports and automations, bind to that underlying stage rather than your custom label. Rename any column on your board, and your reports and SLA clocks keep working without a hitch.

**Time that actually gets billed.** One active timer per tech. End-of-day missing-time checks. Approvals before invoices go out. The gap between hours worked and hours billed stops being a mystery.

**Assistants that ask first and leave a paper trail.** Skills are versioned manifests, not prompt spaghetti. Guardrail rules run before every tool call. Assistant-suggested changes to a ticket or record wait as *proposed, awaiting approval*: nothing silently rewrites your team's work, and client-visible or external actions always require a human click. The audit log records who did what, person or assistant, and stays append-only. Answering "who changed this" takes one search. There is an MCP server too, so Claude, DeepSeek, or your own scripts can work inside the PSA under the same rules as everyone else. See [`docs/site/`](docs/site/index.md), [`docs/instant-first.md`](docs/instant-first.md), and [`docs/10-agent-platform.md`](docs/10-agent-platform.md).

**Fast by default, with no Save button to chase.** Ticket list p95 stays under `150 ms` at `50k` tickets, and live-stream delivery stays under `100 ms`. Those are CI gates, not aspirations. Edits save automatically with instant feedback and full undo, so you can press Ctrl/Cmd+Z to step back through changes the server already recorded.

## Quick start

```bash
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
docker compose -f deploy/docker-compose.yml up -d     # Postgres+pgvector, Redis, MinIO, services, web
open http://localhost:3000                            # workspace UI
```

Developer harness with hot reload:

```bash
cargo install --path crates/marula-core-cli
marula dev
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
        │                │   outbox event trail ──► SSE live stream on core-api
        │                ├── worker (jobs: email · SLA · sync · AI hooks)
        │                └── plugin-host (Extism WASM + HTTP connectors)
        └── @marula-core/sdk clients · MCP server · signed webhooks
```

One writer. Every mutation emits a versioned domain event. REST, the SSE live stream, webhooks, plugins, and MCP are surfaces over one contract.

## Documentation

| | |
|---|---|
| Install & self-host | [`docs/`](docs/) — install guide, config reference, upgrade runbooks |
| Upgrades & compatibility | [`docs/upgrade-compatibility-policy.md`](docs/upgrade-compatibility-policy.md) — support windows, breaking-change rules, what's guaranteed |
| Interaction contract | [`docs/instant-first.md`](docs/instant-first.md) — autosave/undo, agent approval-by-default, the core's read models |
| API reference | Generated from OpenAPI (`utoipa` → Scalar UI on `core-api`) |
| Build plugins & connectors | [`packages/plugin-kit`](packages/plugin-kit/) · [`plugins/connectors/`](plugins/connectors/) reference implementations |
| Architecture decisions | [`docs/adr/`](docs/adr/) |
| Workflows | [`docs/reference/workflow-node-catalogue.md`](docs/reference/workflow-node-catalogue.md) — every node type a workflow can use, by tier · foundation decided in [ADR 0016](docs/adr/0016-workflow-builder-foundation.md) |
| Tech stack | [`docs/tech-stack.md`](docs/tech-stack.md) — every layer, with the versions the manifests pin |
| Visual identity | [`docs/theme.md`](docs/theme.md) |

## Roadmap

Development runs in stages: foundation → daily driver → commercial engine → GA hardening → ecosystem. Current stage and near-term items are tracked in repository milestones. The full staged plan with acceptance criteria lives in [`docs/prompts/BUILD-PROMPT.md`](docs/prompts/BUILD-PROMPT.md).

## Contributing

PRs welcome — trunk-based, DCO-signed, conventional commits. Two approvals are required for `crates/core-domain` and anything touching authz/RLS. Start with [`CONTRIBUTING.md`](CONTRIBUTING.md); significant changes begin as RFCs in `docs/rfc/`.

Extension authors usually don't need this repo: connectors are ordinary web apps using [`@marula-core/sdk`](packages/sdk/) and receiving signed webhooks — the NinjaOne and Hudu connectors are the reference implementations. Beyond the standard set, connectors publish through the connector marketplace, first-party and community alike (ADR 0013).

## Security

Report vulnerabilities privately — see [`SECURITY.md`](SECURITY.md). Please do not open public issues for security reports.

## License

- Core (`crates/`, `apps/`): [AGPL-3.0](LICENSE)
- SDK, plugin kit, plugins & connectors in this repo: [Apache-2.0](packages/sdk/LICENSE)
- Third-party: [`ACKNOWLEDGMENTS.md`](ACKNOWLEDGMENTS.md) (fonts OFL, embedded PostgreSQL/pgvector, full npm inventory, vendored tooling)

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
├── worker/         # Postgres-backed job queue + job definitions
├── plugin-host/    # Extism WASM runtime + connector proxy
├── mcp-server/     # MCP protocol adapter — derived from OpenAPI + capability registry
└── marula-core-cli/   # `marula dev`, setup, admin tasks
apps/web/           # Next.js app (workspace UI + client portal)
packages/
├── sdk/            # @marula-core/sdk — typed client generated from OpenAPI (Apache-2.0)
├── ui/             # design system components — Workbench theme tokens (Apache-2.0)
├── layout/         # multi-pane layout engine
└── plugin-kit/     # TS plugin PDK + dev harness (Apache-2.0)
plugins/            # first-party extensions (Apache-2.0): connectors/ (HTTP) + wasm/
  connectors/       # the standard connector set + the shared support crates
deploy/             # docker-compose.yml, Helm chart, upgrade runbooks
docs/               # Diátaxis docs, ADRs, RFCs, theme spec, build brief
.github/            # CI, templates, actions
```

Every module's README states its purpose *and its non-purpose* — read them before touching code.
</details>

## Acknowledgments

Marula is built on Rust, Axum, SQLx, PostgreSQL, Next.js, Tailwind CSS, shadcn/ui, Extism, and faster-whisper. Thanks to everyone behind them.


