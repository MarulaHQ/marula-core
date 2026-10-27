# Upgrade-matrix fixtures (policy §8 / the private build tracker U7)

Purpose: seeded production-like snapshot fixtures per tagged release — data
exercising every customization surface (custom fields, board statuses +
transitions, checklist template, automations, webhook filters, settings
registry keys, SLA calendars/policies + escalation ladders, report
definitions, KB, documents, contracts/projects, portal config, plus
representative operational data) so CI can restore fixture N−2 → migrate to
HEAD → run conformance suites.

## Format

Per tagged release `X.Y.Z`:

| File | What it is |
|---|---|
| `vX.Y.Z.seed.sql.gz` | `pg_dump` (schema + data + migration state) of the seeded org, gzipped. THE fixture. |
| `vX.Y.Z.manifest.json` | The U5 customization manifest emitted from the seeded DB at capture time (`marula fixture --manifest`) — the preflight-consumable summary of covered surfaces. |
| `vX.Y.Z/seed_org.sql` | The reviewed seed source that produced the dump (deterministic UUIDs/timestamps, re-runnable). |

## Capture procedure (AT TAG TIME — old snapshots cannot be reconstructed)

Run against a scratch Postgres (`pgvector/pgvector:pg18` — the image
`scripts/e2e-stack.sh` and the CI services run, so the capture and its
consumer are the same major):

1. `marula migrate` with `DATABASE_URL` pointing at the scratch DB, from a
   tree at exactly the tag being released.
2. Seed: `psql -v ON_ERROR_STOP=1 -f fixtures/upgrades/vX.Y.Z/seed_org.sql`
3. Emit the manifest:
   `DATABASE_URL=… marula fixture --manifest --org-id 00000000-0000-4000-8000-000000000001 --out fixtures/upgrades/vX.Y.Z.manifest.json`
4. Dump:
   `pg_dump --no-owner --no-privileges … | gzip > fixtures/upgrades/vX.Y.Z.seed.sql.gz`
5. Restore-verify: restore into a fresh database, re-emit the manifest from
   the restore, and compare with the captured manifest (ignore
   `generated_at`); then sanity-count the core surfaces.

The org id anchor is `00000000-0000-4000-8000-000000000001`; the seed is
idempotent (it deletes that org first).

First fixtures generated at v0.2.0 (the v0.1.0-era capture was scoped out of
its wave; the N−2 matrix anchors on v0.2.0 onward).
