# Contributing to Meridian

Thanks for helping build the open-core PSA. The short version: **PRs from everyone — including maintainers and AI agents — trunk-based, squash-merged, DCO-signed.**

## Quick start

```bash
git clone https://github.com/Marlin-Hofmeyer/meridian && cd meridian
pnpm install
docker compose -f deploy/docker-compose.yml up -d   # postgres+pgvector, redis, minio, mailpit
cargo run -p meridian-cli -- dev                    # core-api + realtime + worker + web with hot reload
```

## Ground rules

1. **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:` …). Release notes are generated from them.
2. **DCO sign-off** on every commit (`git commit -s`) — you certify the Developer Certificate of Origin.
3. **No `unwrap()` outside tests**; `cargo clippy -D warnings` and `tsc --noEmit` must pass.
4. **Migrations:** additive-first. A destructive migration requires the `--breaking` flag plus upgrade docs, and fails CI otherwise.
5. **RLS:** every new user-data table needs an `org_id` column and an RLS policy — the CI coverage test fails otherwise.
6. **Design tokens only:** no raw hex values outside `packages/ui` token files.

## Review policy

- All changes go through PRs; branch protection applies to maintainers too.
- **Two approvals required:** `crates/core-domain`, and anything touching authz/RLS/payments paths.
- One approval elsewhere, with green CI (fmt, clippy, eslint/tsc, tests, migration-diff check, preview environment).

## Bigger changes

Significant architecture or behavior changes go through an RFC first: open `docs/rfc/NNN-title.md` using the template, and let it be discussed before implementation. Changes to licensing, the open/closed boundary, or the semantic-category model require an ADR.

## Extension developers

You probably don't need this repo! See `packages/plugin-kit` and `connectors/` — HTTP connectors are just web apps that talk to our API and receive signed webhooks. The NinjaOne and Hudu connectors are the reference implementations.
