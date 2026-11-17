# Release notes

One file per pull request, in `unreleased/`. The file declares the version bump
that change needs and carries the changelog bullet in the words a user would
use. `scripts/release-note.mjs plan` adds the fragments up; `apply` writes the
release. A pull request that changes product code without a fragment fails the
`Release note` check.

## The three answers

| `bump`  | When | Version |
| ------- | ---- | ------- |
| `minor` | A capability someone can use: a new module, a new surface, a new connection, a new field on a form that saves it. | 0.3.7 → 0.4.0 |
| `patch` | A fix to something that already works, or a change too small to be a capability. | 0.3.7 → 0.3.8 |
| `none`  | Product code with no user-visible change: a refactor, a dependency bump that changes nothing a person sees, groundwork nothing reads yet. | no release |

Tests, docs, `e2e/`, `scripts/` and `.github/` need no fragment at all. The
check decides that from the paths in the diff, not from an opinion.

## Writing one

```sh
node scripts/release-note.mjs add --bump patch --slug portal-address-fix \
  --section Fixed --note "A portal address shared across tenants resolves to the tenant that asked."
```

That writes `unreleased/2026-11-17-portal-address-fix.md`:

```markdown
---
bump: patch
---

### Fixed

- A portal address shared across tenants resolves to the tenant that asked.
```

Rules the parser enforces:

- **One heading**, from the changelog's own vocabulary: `### Added`,
  `### Changed`, `### Deprecated`, `### Removed`, `### Fixed`, `### Security`.
  A `bump: none` fragment uses `### Internal` instead and never reaches the
  changelog.
- **At least one bullet**, in the user's voice: what changed for them, not how
  the work was organized. No wave, lane, or PR numbers.
- **No file numbers, no internal planning.** The changelog is published.

## The release

```sh
node scripts/release-note.mjs plan     # the next version, and why
node scripts/release-note.mjs apply --date 2026-11-17
```

`apply` folds the fragments into `CHANGELOG.md` under one new heading, deletes
them, and bumps the version in `Cargo.toml` and the eight `package.json` files
that carry it. It then prints what remains, because the API version is the
crate version and the pinned spec has to be rebuilt:

1. `scripts/regen-openapi-pins.sh`
2. `node docs/site/scripts/generate-api-reference.mjs`
3. `git tag -s v<version> -m v<version>`

## Asking for a suggestion

```sh
node scripts/release-note.mjs advise --base origin/main
```

A path-based guess: a migration, a route module, a page, a shared component or
a connector capability reads as `minor`; any other product change reads as
`patch`; nothing under a product path reads as `none`. It prints the file that
made it think so. The suggestion is a starting point — the fragment is the
decision, and a reviewer can see it.
