# Contributing to Marula

Thanks for helping build the open-core PSA. The short version: **PRs from everyone (including maintainers and AI agents), trunk-based, squash-merged, DCO-signed.**

## Quick start

```bash
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
pnpm install
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env up -d --build   # postgres+pgvector, Valkey, core-api, worker, web
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env stop web
pnpm dev                                              # Next.js on :3000 against the API on :8080, with hot reload
```

`deploy/dev.env` carries the published development secrets, so the stack warns at boot and belongs on a machine you own. Without the `stop web` step the container serves the UI on `:3000` and `pnpm dev` has nowhere to bind.

## macOS 27: `E0432: unresolved imports` is a build artifact, not your code

On macOS 27 a proc-macro dylib can fail to load, and rustc reports it as
missing imports inside the crate that uses the macro — usually `sqlx`:

```
error: .../libsqlx_macros-<hash>.dylib: dlopen(...): mis-aligned LINKEDIT string pool
error[E0432]: unresolved imports `self::encode::Encode`, ...
```

dyld 27 checks that a dylib's `LC_SYMTAB` string table starts on an 8-byte
boundary. Stripping a linked dylib moves that table to wherever the shortened
symbol list ends, so the alignment depends on how many symbols the crate
happens to have. Dependencies are therefore never stripped here: the profiles
in `Cargo.toml` set `strip = "none"` on `[profile.*.package."*"]`, and only our
own artifacts follow the profile's strip setting. A fresh clone needs nothing.

A build cache created before that setting still holds the bad files, and they
stay bad until relinked. Name them, then delete and rebuild:

```bash
scripts/p13-build-env.sh probe                  # exits 1 and names each dylib
rm .kilo/target-shared/debug/deps/libsqlx_macros-*.dylib   # or your target dir
cargo build -p core-api                          # relinks; probe is clean after
```

`python3 scripts/check-dylib-alignment.py --selftest` proves the parser on
synthetic Mach-O images. Swapping in `ld64.lld` does not help: it lands on the
same offsets, so the fix belongs in the profile, not the linker.

## Ground rules

1. **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:` …). Release notes are generated from them.
2. **DCO sign-off** on every commit (`git commit -s`). The trailer must read
   `Signed-off-by: Your Name <your-email>` and that identity must **match the
   commit's author** — write it in a name and address you are willing to publish.
   GitHub noreply addresses are welcome and recommended. The sign-off certifies
   you have the right to submit the code under AGPL-3.0 (or Apache-2.0 for the
   SDK, plugin kit, plugins and connectors); see
   [developercertificate.org](https://developercertificate.org). No CLA is required.
   Documentation contributions under `docs/site/` are submitted under
   [CC BY 4.0](docs/site/LICENSE), and the same sign-off covers both.
3. **No `unwrap()` outside tests.** `cargo clippy -D warnings` and `tsc --noEmit` must pass.
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

`CHANGELOG.md` and `ROADMAP.md` are the only sources for the public changelog
and roadmap. Any page that mirrors them is generated at build time and never
edited by hand.

## Review policy

- All changes go through PRs. Branch protection applies to maintainers too.
- **Two approvals required:** `crates/core-domain`, and anything touching authz/RLS/payments paths.
- One approval elsewhere, with green CI (fmt, clippy, eslint/tsc, tests, migration-diff check, preview environment).

## Bigger changes

Significant architecture or behavior changes go through an RFC first: open `docs/rfc/NNN-title.md` for discussion before implementation. Changes to licensing, the open/closed boundary, or the semantic-category model require an ADR.

## Extension developers

You probably don't need this repo! See `packages/plugin-kit` and `connectors/` — HTTP connectors are just web apps that talk to our API and receive signed webhooks. The NinjaOne and Hudu connectors are the reference implementations.
