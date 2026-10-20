## What this changes

<!-- One or two sentences. Link the issue if there is one. -->

## Checklist

- [ ] Every commit is signed off per the [DCO](https://developercertificate.org) (`git commit -s`), and the `Signed-off-by` trailer matches the commit's author.
- [ ] Conventional commit subjects (`feat:`, `fix:`, `docs:` …) — release notes are generated from them.
- [ ] `CHANGELOG.md` updated under `[Unreleased]` if a user can see or do something different.
- [ ] No raw hex outside the `packages/ui` token files; `cargo clippy -D warnings` and `tsc --noEmit` pass.
- [ ] New user-data tables carry `org_id` and an RLS policy.
