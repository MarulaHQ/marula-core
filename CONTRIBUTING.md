# Contributing to Marula

Thanks for helping build the open-core PSA. The short version: **PRs from everyone (including maintainers and AI agents), trunk-based, squash-merged, DCO-signed.**

## Quick start

```bash
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
pnpm install
docker compose -f deploy/docker-compose.yml up -d   # postgres+pgvector, redis, minio, mailpit
cargo run -p marula-core-cli -- dev                    # core-api + worker + web with hot reload
```

## Ground rules

1. **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:` …). Release notes are generated from them.
2. **DCO sign-off** on every commit (`git commit -s`). The trailer must read
   `Signed-off-by: Your Name <your-email>` and that identity must **match the
   commit's author** — write it in a name and address you are willing to publish.
   GitHub noreply addresses are welcome and recommended. The sign-off certifies
   you have the right to submit the code under AGPL-3.0; see
   [developercertificate.org](https://developercertificate.org). No CLA is required.
3. **No `unwrap()` outside tests**; `cargo clippy -D warnings` and `tsc --noEmit` must pass.
4. **Migrations:** additive-first. A destructive migration requires the `--breaking` flag plus upgrade docs, and fails CI otherwise. A migration that adds or renames a unique constraint must add a matching entry to `map_unique_constraint` (`crates/core-api/src/error.rs`) — otherwise clients get the sanitized generic `conflict` (with a server-side log) instead of a typed, user-friendly error. Customization-aware caveat: if the migration modifies a construct referenced by customization rows, an in-transaction transform is required (upgrade-compatibility policy §4.5).
5. **RLS:** every new user-data table needs an `org_id` column and an RLS policy — the CI coverage test fails otherwise.
6. **Design tokens only:** no raw hex values outside `packages/ui` token files.

## Public copy you may be asked to touch

Three files carry prose aimed at users, and the website builds pages straight
from two of them, so an edit here is a public edit:

- **`CHANGELOG.md`** (Keep a Changelog format). Add or touch bullets under
  `[Unreleased]` **at merge time**, for any PR that changes something a user
  can see or do. A bullet answers "what changed for me?", never "what commits
  merged": no lane or wave numbers, no internal planning references.
- **`ROADMAP.md`**. Move items between the Shipped / In flight / Later columns
  **at merge time**, when status actually changes. Never put a date on an item
  nobody owns.
- **Releases** happen **at tag time**: the maintainer moves the
  `[Unreleased]` bullets into a dated heading, adds the tag's link reference,
  and pushes the tag. Nothing else rewrites history in the file.

Mechanical check: `node website/scripts/sync-public.mjs --check` regenerates
the website's changelog page data, roadmap data, and RSS feed from these
files, and fails if the generated output is stale or if `CHANGELOG.md` does
not parse as Keep a Changelog. Run it locally after editing either file; the
website build runs the generator and the check on every build.

## Review policy

- All changes go through PRs; branch protection applies to maintainers too.
- **Two approvals required:** `crates/core-domain`, and anything touching authz/RLS/payments paths.
- One approval elsewhere, with green CI (fmt, clippy, eslint/tsc, tests, migration-diff check, preview environment).

## Bigger changes

Significant architecture or behavior changes go through an RFC first: open `docs/rfc/NNN-title.md` using the template, and let it be discussed before implementation. Changes to licensing, the open/closed boundary, or the semantic-category model require an ADR.

## Extension developers

You probably don't need this repo! See `packages/plugin-kit` and `connectors/` — HTTP connectors are just web apps that talk to our API and receive signed webhooks. The NinjaOne and Hudu connectors are the reference implementations.
