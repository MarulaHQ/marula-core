# Changesets — TS package versioning (TASKLIST U10b)

Version bumps for the publishable TS packages (`packages/sdk`,
`packages/ui`, `packages/layout`, `packages/plugin-kit`) ride Changesets:

1. On a PR that changes a package's consumers-visible surface, add a
   changeset: `pnpm exec changeset` → pick packages → pick bump (policy §3
   semver: 0.x allows breaking at minor, each documented) → describe.
2. CI (`.github/workflows/changesets.yml`) turns pending changesets into a
   "version packages" PR; merging it commits the bumps + CHANGELOGs.
3. The next `vX.Y.Z` tag carries those versions; the release pipeline
   (`.github/workflows/release.yml`) publishes images/charts, not npm
   packages (`privatePackages.version` only — no registry publishing yet).

`web` and `website` are intentionally ignored (applications, not packages).
