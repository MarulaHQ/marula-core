# Marula

<p>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/MarulaHQ/marula-core/main/.github/assets/logo/dark.png">
    <img src="https://raw.githubusercontent.com/MarulaHQ/marula-core/main/.github/assets/logo/light.png" height="34" alt="Marula logo lockup" title="Marula logo lockup">
  </picture>
</p>

<p>
  <a href="LICENSE"><img src="https://img.shields.io/badge/core-AGPL--3.0-5E7F3A" alt="Marula Core is licensed under the AGPL-3.0" height="20"></a>
  <a href="packages/sdk/LICENSE"><img src="https://img.shields.io/badge/sdk%20%2F%20plugins-Apache--2.0-E8B83A" alt="The SDK, plugins and connectors are licensed under the Apache-2.0" height="20"></a>
  <a href="docs/site/LICENSE"><img src="https://img.shields.io/badge/docs-CC--BY--4.0-7C746A" alt="The documentation in docs/site is licensed under CC BY 4.0" height="20"></a>
</p>

**The open-source PSA that MSPs own.** Tickets, time, clients, dispatch, projects, invoicing, knowledge base: everything an IT service team runs on, self-hosted, in a database you control.

> **Status:** pre-1.0 (0.3.x), built in the open. [`ROADMAP.md`](ROADMAP.md) says what ships today and what is next; [`CHANGELOG.md`](CHANGELOG.md) records what changed in each release.

<!-- TODO(ci): CI badges once the pipelines are public. Rendered now, a
     workflow badge on a repository with no runs reads as "no status", which
     is worse than no badge.
![CI](https://github.com/MarulaHQ/marula-core/actions/workflows/ci.yml/badge.svg) -->

---

## Who Marula is for

Whether you run the service desk, coordinate the dispatch board, work tickets in the field, or send out monthly invoices, Marula gives your team one place to work:

- **Technicians** keep a single active timer running, log customer-facing and internal notes, and close out day-end time gaps in a couple of minutes.
- **Dispatchers and service leads** see real technician capacity on the calendar, spot SLA risk before a ticket breaches, and schedule visits with travel-time estimates.
- **Office managers and owners** review approved hours against client retainers and project milestones, issue invoices and credit notes from the work already logged, and give clients a self-service portal.
- **Systems administrators** host the entire stack on standard PostgreSQL and Valkey, control AI and plugin permissions down to individual roles, and inspect an append-only audit trail for every change.

## Why we're building this

Every commercial PSA taxes tickets, locks the data, and bends your workflow to its roadmap. We got tired of it.

Marula is complete on its own. Native invoicing, quotes, knowledge base, CSAT surveys, and dispatch scheduling all come built in, with no third-party subscriptions required to run a real MSP.

Some things we care about enough to enforce in CI:

**Workflows that don't break.** Customize boards however your shop works. Every status maps to a core stage underneath (`open`, `in_progress`, `waiting`, or `closed`). SLA timers and billing rules, along with reports and automations, bind to that underlying stage rather than your custom label. Rename any column on your board and your reports and SLA clocks keep working.

**Time that actually gets billed.** One active timer per tech. End-of-day missing-time checks. Approvals before invoices go out. The gap between hours worked and hours billed stops being a mystery.

**Assistants that ask first and leave a paper trail.** Skills are versioned manifests, not prompt spaghetti. Guardrail rules run before every tool call. Assistant-suggested changes to a ticket or record wait as *proposed, awaiting approval*: nothing silently rewrites your team's work, and client-visible or external actions always require a human click. The audit log records who did what, person or assistant, and stays append-only. Answering "who changed this" takes one search. There is an MCP server too, so Claude, DeepSeek, or your own scripts can work inside the PSA under the same rules as everyone else. See [`docs/site/`](docs/site/index.md), [`docs/instant-first.md`](docs/instant-first.md), and [`docs/10-agent-platform.md`](docs/10-agent-platform.md).

**Fast by default, with no Save button to chase.** Ticket list p95 stays under `150 ms` at `50k` tickets, and CI fails the build if it slips past. A committed write reaches other viewers over the live stream in under `100 ms`. Edits save as you type, with instant feedback and full undo, so Ctrl/Cmd+Z steps back through changes the server already recorded.

## Quick start

Run the whole stack from a checkout:

```bash
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env up -d --build
# workspace UI on http://localhost:3000 · API on :8080 · Scalar docs at :8080/docs
```

`deploy/dev.env` carries the published development secrets, so the stack warns at every boot and belongs on a machine you own. An install that serves anyone else generates its own eight secrets: [`deploy/README.md`](deploy/README.md) lists them, and [`docs/install.md`](docs/install.md) walks the installer.

Working on the web app instead? Keep the stack up, free port 3000, and run the UI on the host with hot reload:

```bash
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env stop web
pnpm install && pnpm dev                             # Next.js on :3000 against the API on :8080
```

> Full installation guide, configuration reference, and upgrade runbooks live in [`docs/`](docs/); the generated API reference (Scalar UI) is served by `core-api` at `/docs`.

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
| Install & self-host | [`docs/install.md`](docs/install.md): the installer, the plan, what stays manual. [`deploy/README.md`](deploy/README.md): the compose stacks, secrets and runbooks |
| Documentation site | [`docs/site/index.md`](docs/site/index.md): task guides for owners, admins, technicians and operators |
| Upgrades & compatibility | [`docs/upgrade-compatibility-policy.md`](docs/upgrade-compatibility-policy.md): support windows, breaking-change rules, what's guaranteed |
| Interaction contract | [`docs/instant-first.md`](docs/instant-first.md): autosave and undo, agent approval by default, the core's read models |
| API reference | Generated from OpenAPI (`utoipa` → Scalar UI on `core-api` at `/docs`) |
| SDK & plugins | [`packages/sdk/`](packages/sdk/) for the typed client, [`packages/plugin-kit/`](packages/plugin-kit/) for the PDK. [`plugins/connectors/`](plugins/connectors/) holds the reference implementations |
| Architecture decisions | [`docs/adr/`](docs/adr/) |
| Workflows | [`docs/reference/workflow-node-catalogue.md`](docs/reference/workflow-node-catalogue.md): every node type a workflow can use, by tier. The foundation is [ADR 0016](docs/adr/0016-workflow-builder-foundation.md) |
| Tech stack | [`docs/tech-stack.md`](docs/tech-stack.md): every layer, with the versions the manifests pin |
| Visual identity | [`docs/theme.md`](docs/theme.md): the token set and motion rules this UI is built from |

## Roadmap

Development runs in stages: foundation → daily driver → commercial engine → GA hardening → ecosystem. [`ROADMAP.md`](ROADMAP.md) keeps three columns (shipped, in flight, later) and never dates work nobody owns. [`CHANGELOG.md`](CHANGELOG.md) is the record of what each release changed.

## Contributing

PRs welcome. Changes land trunk-based and squash-merged, with conventional commit subjects and a DCO sign-off. Two approvals are required for `crates/core-domain` and anything touching authz/RLS/payments. Start with [`CONTRIBUTING.md`](CONTRIBUTING.md). Significant changes begin as RFCs in [`docs/rfc/`](docs/rfc/).

Extension authors usually don't need this repo: connectors are ordinary web apps using [`@marula-core/sdk`](packages/sdk/) and receiving signed webhooks. The NinjaOne and Hudu connectors are the reference implementations. Beyond the standard set, connectors publish through the connector marketplace, first-party and community alike.

## Security

Report vulnerabilities privately through [`SECURITY.md`](SECURITY.md). Please do not open a public issue for a security report.

## License

- Everything not listed below is [AGPL-3.0](LICENSE): the Rust workspace (`crates/`), both apps (`apps/`), the desktop and mobile shells (`desktop/`, `native/`), the installer (`installer/`), and the UI packages (`packages/ui/`, `packages/ui-native/`, `packages/layout/`)
- SDK, plugin kit, plugins & connectors: [Apache-2.0](packages/sdk/LICENSE)
- Documentation (`docs/site/`): [CC BY 4.0](docs/site/LICENSE), with attribution. The terms are in [`docs/site/NOTICE`](docs/site/NOTICE)
- Third-party: [`ACKNOWLEDGMENTS.md`](ACKNOWLEDGMENTS.md) (fonts OFL, embedded PostgreSQL/pgvector, full npm inventory, vendored tooling)

The Marula name, wordmark and fruit mark are trademarks: [`TRADEMARK.md`](TRADEMARK.md). None of these licences grants trademark rights.

---

## Repository layout

<details>
<summary>Module map</summary>

```
crates/
├── core-api/         # Axum HTTP API: the only database writer
├── core-domain/      # entities, policies, domain services (pure, heavily tested)
├── core-db/          # SQLx queries + migrations
├── core-events/      # outbox, versioned event envelope, live-stream publisher
├── core-billing/     # contracts, recurring billing rules, invoice runs
├── core-projects/    # phases, tasks, dependencies, templates, milestone billing
├── core-kb/          # knowledge base domain rules
├── core-documents/   # document versions and their permissions
├── core-mail/        # inbound and outbound email
├── core-reports/     # reporting field catalog and query compiler
├── core-routing/     # distance and duration provider seam (maps)
├── core-decision/    # the provider-general decision edge
├── core-store/       # object-store edge
├── core-otel/        # the shared OpenTelemetry baseline
├── worker/           # Postgres-backed job queue + job definitions
├── plugin-host/      # Extism WASM runtime + connector proxy
├── mcp-server/       # MCP protocol adapter over the public API
├── perf-harness/     # the measurement harness behind docs/perf.md
└── marula-core-cli/  # `marula`: install, migrate, upgrade, seed, operator tasks
apps/
├── web/              # Next.js app (workspace UI + client portal)
└── installer/        # the installer wizard UI
packages/
├── sdk/              # @marula-core/sdk: typed client generated from OpenAPI (Apache-2.0)
├── ui/               # Marula design system: tokens + components (AGPL-3.0)
├── ui-native/        # the same tokens for the mobile surface (AGPL-3.0)
├── layout/           # multi-pane layout engine (AGPL-3.0)
└── plugin-kit/       # plugin PDK: ABI bindings + build and sign tooling (Apache-2.0)
plugins/              # first-party extensions (Apache-2.0): connectors/ (HTTP) + wasm/
deploy/               # compose stacks, Helm chart, TLS edge, backup, runbooks
desktop/              # Tauri 2 shell for macOS
native/               # the mobile app (React Native / Expo)
installer/            # the `marula install` crate behind the wizard
e2e/                  # Playwright flows and the stack harness they run against
docs/                 # published docs site, engineering notes, ADRs, RFCs
fixtures/             # upgrade fixtures, event fixtures and the pinned OpenAPI documents
release-keys/         # the release signing keys a download is verified against
scripts/              # dev, CI and verification tooling
.github/              # CI, templates, actions
```

Most modules carry a README stating the module's purpose *and its
non-purpose*. [`crates/core-api/README.md`](crates/core-api/README.md) and
[`packages/ui/README.md`](packages/ui/README.md) are the two to read first.
</details>

## Acknowledgments

Marula is built on Rust, Axum, SQLx, PostgreSQL, Next.js, Tailwind CSS, shadcn/ui, Extism, and faster-whisper. Thanks to everyone behind them.
