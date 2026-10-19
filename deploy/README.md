# deploy

> **Purpose:** docker-compose.yml (Postgres+pgvector, Redis/Valkey, MinIO (helm; Garage in compose), Mailpit, services, web), Helm chart, upgrade runbooks, self-host backup/DR guide, and the TLS edge in `edge/`.
> **Non-purpose:** No SaaS/Kubernetes control-plane infrastructure — managed multi-tenant hosting is a commercial concern outside this repo.

## What is here

| Directory | What it is |
|---|---|
| `docker-compose.yml` | The local stack: Postgres, Valkey, migrations, core-api, worker, web, plus the `stage2`, `observability`, `ai` and `backup` profiles. No TLS, no published-proxy boundary — it targets development. |
| `Dockerfile.rust` / `Dockerfile.web` | The workspace images. `NEXT_PUBLIC_API_URL` is baked into the web bundle at build time. |
| `edge/` | **The public front door.** A Caddy proxy that terminates TLS, the three TLS modes (Direct ACME, Cloudflare Origin CA, Cloudflare DNS-01), and the origin firewall helper. |
| `preview/` | The per-PR preview environment: sizing overlay, publishing options, and the CI definition for it. |
| `helm/marula/` | The Helm chart, with the same routing and both TLS modes expressed through `ingress`. |
| `backup/` | WAL-G point-in-time recovery and restic snapshots, plus the restore rehearsal. |
| `observability/` | OTel collector, Tempo, Loki and the Grafana provisioning for the `observability` profile. |
| `runbooks/` | Operator runbook scaffolds. |
| `garage.toml` | The Garage config the `stage2` profile binds read-only. |

## Starting a public deployment

The local stack publishes raw ports and speaks plaintext http, which is
the right posture for a laptop and the wrong one for a host on the
internet. Layer the edge overlay instead of editing the base file:

```sh
export MARULA_DOMAIN=marula.example.com
export ACME_EMAIL=ops@example.com          # direct and cf-dns01 modes

docker compose -f docker-compose.yml \
               -f edge/compose.edge.yml \
               --profile edge up --build -d
```

The overlay is last on purpose: it pulls the raw `8080`, `3000`,
`8082`, `15432` and `16379` publishes back off the network, and compose
only applies that replacement when the file carrying it comes last.

`edge/README.md` has the whole decision — the three TLS modes, the
Cloudflare dashboard steps behind the edge one, and the origin lockdown.
The operator runbook is
[`docs/site/reference/reverse-proxy-tls.md`](../docs/site/reference/reverse-proxy-tls.md).

## Profiles

`stage2` (object storage + mail sink), `observability` (OTLP, Tempo, Loki,
Grafana, status page), `ai` (local model runtime), `backup` (WAL-G +
restic), and `edge` (the TLS proxy) are all opt-in. `docker compose up`
without a `--profile` flag boots the same stack it always did.
