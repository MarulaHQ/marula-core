# ADR 0005: Local desktop test app — embedded Postgres in a Tauri shell

Date: 2026-09-13 · Status: Accepted · Lane: desktop-local-test-app

## Context

Meridian needs a double-clickable macOS `.app` for local testing — no Docker,
no compose, no terminal. Today the only way to run the stack is
`docker compose -f deploy/docker-compose.yml up` (Postgres + Redis + core-api
+ realtime + worker + web).

Two decisions were forced:

**1. Embedded Postgres vs SQLite.** SQLite is the usual desktop-app answer and
would have made the bundle trivially self-contained. It is not viable here:

- **RLS is load-bearing.** Every org-scoped table ships a row-level-security
  policy keyed on `current_setting('app.org_id')` and the API runs as the
  RLS-constrained `meridian_app` role (migration 0001 provisions it; compose
  parity requires the `meridian` owner role for migrations/worker). SQLite has
  no RLS; every query path would need a parallel, non-RLS enforcement layer —
  a second security model to maintain and drift against.
- **SECURITY DEFINER seams.** Cross-org email routing and the alert ingestor
  deliberately use narrow `SECURITY DEFINER` functions instead of role grants
  (a role-membership grant would exempt `meridian_app` from RLS on every
  owned table). Those seams are Postgres-specific and would need rethinking
  from scratch on SQLite.
- **Email idempotency gates.** The inbound replay gate is a unique
  `(org_id, rfc_message_id)` on `email_messages`; outbound exactly-once rides
  a ledger table with lease recovery. These are transactional Postgres
  patterns the codebase depends on.
- **The core's data stays queryable** (doctrine #9, docs/STRATEGY.md): the
  same SQL, migrations, and `psql` access must work against the desktop data
  dir as against the compose stack.

So: embed real Postgres via the `postgresql_embedded` crate, data dir at
`~/Library/Application Support/Meridian` (persists across rebuilds), migrations
run incrementally on every startup by the `meridian` owner role — exactly the
compose split (owner migrates, `meridian_app` serves).

**2. core-api as a library, not a sidecar.** The Tauri Rust process depends on
`crates/core-api` and `crates/core-db` as path deps and mounts the existing
axum router on an ephemeral loopback port (`127.0.0.1:15433`, falling forward
if taken). No sidecar binary, no second copy of the API to build or version.

## Architecture

```
Meridian.app
└─ meridian-desktop (Tauri Rust process)
   ├─ postgresql_embedded → Postgres 17 on 127.0.0.1:15434
   │    data: ~/Library/Application Support/Meridian/pgdata
   │    install: …/pginstall/<version>/  (persists; binaries cached)
   ├─ core-api axum router on 127.0.0.1:15433 (library call, in-process)
   ├─ Next standalone server (bundled at Resources/resources/web)
   │    on 127.0.0.1:15435 — the webview points here
   └─ graceful shutdown: pg_ctl stop on exit; SIGTERM handler does the same
```

- **Role parity with compose:** the bootstrap superuser creates `meridian`
  (LOGIN SUPERUSER — compose's `POSTGRES_USER` is a bootstrap superuser too);
  migration 0001 provisions `meridian_app`. Migrations + demo seed run as the
  owner; the API connects as `meridian_app`.
- **Local mode:** `core_api::config::Config::from_local()` is a clean
  constructor (no env hacks) that turns every external integration off:
  email transport `none`, no inbound webhook HMAC, no internal token, object
  store on the local filesystem, dev tools on, loopback bind only.
- **pgvector:** migration 0062 requires the `vector` extension, which stock
  Postgres binaries don't ship. On first setup the app copies
  `vector.dylib` + the extension scripts from a Homebrew pgvector install
  (`brew install pgvector`) into the embedded installation. Prerequisite:
  **brew install pgvector**.
- **Demo seed:** the deterministic demo org (same path as
  `marula-core-cli seed-demo`) is seeded idempotently on startup with the dev
  JWT secret, so logins `owner@demo.local` / `dispatcher@demo.local` /
  `tech@demo.local` / `bookkeeper@demo.local` (password `demo-password-123`)
  work out of the box.
- **Release profile:** the `desktop` cargo profile uses `opt-level=2` +
  thin LTO + symbol stripping instead of default release. Tradeoff: a few
  percent of server throughput for much faster link times and incremental
  desktop rebuilds — the right trade for a test app you rebuild constantly.
  The .app lands at ~59 MB (well under the 100–150 MB target).

## Rebuild procedure

`scripts/build-desktop.sh` — one command = update the app:

1. `pnpm --filter @marula-core/web build` (Next standalone bundle)
2. Stage the standalone server into `desktop/resources/web` (symlinks
   rewritten to absolute repo paths — see Deviations)
3. `cargo tauri build --no-bundle -- --profile desktop` on the **shared**
   target dir (`.kilo/target-shared` via `.cargo/config.toml`, DX6 — never a
   worktree-local `CARGO_TARGET_DIR`), then assemble + ad-hoc sign
   `Meridian.app` (the Tauri npm CLI has no `--profile` flag and its bundler
   globs skip hidden dirs, so the script owns the bundle layout)

Measured on M-series (2026-09-13): first build ≈ 2.5–3 min (one-time binary
download + full compile); incremental web-only ≈ 2.2 min (dominated by the
Next build); Rust-touch incremental ≈ 2.3 min (cargo ~20 s of that). The data
dir is never touched by rebuilds; migrations apply incrementally on startup
(a second launch skips applied migrations and the seed logs
`demo org already seeded`).

## Deviations from the original plan (and why)

1. **Next standalone server instead of static export as Tauri assets.** The
   web app has 11 dynamic `[id]` routes without `generateStaticParams`;
   `output: "export"` fails the build, and even a prerendered export would
   404 client-side navigations to non-prerendered params. The Next standalone
   server is bundled into the .app and run as a managed child process on a
   loopback port.
2. **The bundled server uses the machine's `node`** (probing
   `/opt/homebrew/bin/node` first for GUI-launched apps) and its pnpm symlinks
   resolve into the repo checkout — this is a **local test app**, not a
   shippable product bundle. Making it fully self-contained would mean
   materializing the pnpm store (fighting self-referential links, +150 MB)
   or shipping a Node runtime. Documented here as a deliberate scope line.
3. **Bundle assembled by the build script**, not Tauri's bundler: the npm
   Tauri CLI cannot pass a custom cargo profile (`--profile` is not exposed)
   and its resource globs skip dot-directories (`.next`). The script owns the
   `.app` layout and ad-hoc codesigns it.
4. **`redis_url` is a placeholder** in local mode — core-api never connects
   to Redis in-process (realtime rides Postgres LISTEN); the field exists for
   compose wiring.

## Consequences

- First run downloads Postgres binaries (~30 MB) and applies all migrations —
  slower start; every later start reuses the cached install and data dir.
- `brew install pgvector` is a one-time prerequisite.
- Wiping `~/Library/Application Support/Meridian` resets the app to first-run
  state (troubleshooting path; see RUNBOOK-DEV.local.md).
- The desktop crate is workspace member `desktop` (profile `desktop`); it
  does not affect server builds or CI.
