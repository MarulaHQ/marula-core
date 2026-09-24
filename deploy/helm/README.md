# Helm chart — deploy/helm/meridian

Values-driven chart mirroring `deploy/docker-compose.yml`: Postgres
(pgvector image, prod parity; or external), Redis, migrations (pre-upgrade
hook), core-api, realtime, worker, web; Mailpit and Garage (HEADLESS stateful
service + layout-bootstrap job, replacing the MinIO sample) as opt-in
profiles; optional ingress.

Image policy (TASKLIST U10b): every image reference is digest-pinned —
`digestPinningEnforced: true` (default) refuses to render images without
`images.*.digest` set. Digests are published per release with SBOM +
provenance attestations (`.github/workflows/release.yml`).

Validate locally (helm 3.14+):

    helm lint meridian
    helm template meridian --set digestPinningEnforced=false

Non-purpose: not an operator dashboard; no cross-env secret management
beyond documented values (set `existingSecret` for production secret
plumbing).
