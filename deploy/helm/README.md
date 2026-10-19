# Helm chart — deploy/helm/marula

Values-driven chart that mirrors `deploy/docker-compose.yml`. It deploys
Postgres (the pgvector image for parity with production, or an external
instance), Redis, a migrations job wired as a pre-upgrade hook, core-api,
worker and web. Mailpit and Garage are opt-in profiles. Garage
runs as a headless stateful service with a layout-bootstrap job, which
replaced the MinIO sample. An ingress ships with the chart and is disabled
until you want it.

Image policy (TASKLIST U10b): every image reference is digest-pinned —
`digestPinningEnforced: true` (default) refuses to render images without
`images.*.digest` set. Digests are published per release with SBOM +
provenance attestations (`.github/workflows/release.yml`).

Validate locally (helm 3.14+):

    helm lint marula
    helm template marula --set digestPinningEnforced=false

Non-purpose: not an operator dashboard; no cross-env secret management
beyond documented values (set `existingSecret` for production secret
plumbing).

## The public front door (TASKLIST V2)

The chart's `ingress` block is the public surface. It routes the four seams
and applies the proxy tuning the app depends on, so a bare `ingress.enabled:
true` gives a working front door rather than a starting point.

```yaml
ingress:
  enabled: true
  className: nginx
  host: marula.example.com
  tlsMode: direct      # or "edge"
  tlsSecretName: marula-tls
  proxy:
    bodySizeMb: 25     # rendered as 25Mi (binary), matching core-api's
                       # mb * 1024 * 1024 — not 25M, which is decimal
    readTimeoutSeconds: 3600
    sendTimeoutSeconds: 3600
    buffering: "off"
coreApi:
  trustedProxies: "173.245.48.0/20,104.16.0.0/13"   # edge + controller
web:
  nextPublicApiUrl: https://marula.example.com      # baked at build time
```

| Path | Backend | Why |
|---|---|---|
| `/`, everything else | web | console, `/_next/*`, 404s |
| `/api`, `/api/*` | core-api | REST, canonical `/api/v1` and legacy `/api` |
| `/openapi.json` | core-api | SDK contract |
| `/api/health` | core-api | liveness for probes outside the cluster (`/api/v1/health` on the canonical mount; a bare `/healthz` is a core-api 404) |

`/internal/*` is deliberately not routed: that is the worker-to-core effect
seam, guarded by its own token. An unmatched path falls through to web.

### The two TLS modes

| `ingress.tlsMode` | Who holds the certificate | Use when |
|---|---|---|
| `direct` | The controller, or cert-manager in front of it (add the issuer through `ingress.annotations`) | DNS points at the cluster |
| `edge` | You do: a Cloudflare Origin CA pair, or a cert-manager DNS-01 certificate | DNS sits behind Cloudflare with the orange cloud on |

`edge` mode means TLS is terminated further out and the controller passes
the handshake through, so the secret you upload is whatever that outer edge
wants to see. With Cloudflare, pair an Origin CA certificate with SSL/TLS
mode **Full (strict)** — Full accepts any certificate, and Flexible has the
edge fetch your origin over plaintext http.

The Cloudflare dashboard steps behind that, the Origin CA versus DNS-01
choice, and the origin IP-range firewall guidance are in
[`docs/site/reference/reverse-proxy-tls.md`](../../docs/site/reference/reverse-proxy-tls.md).
The same two modes exist for the compose stack, in `deploy/edge/` — the
decision does not change with the orchestrator.

### Two settings that are easy to miss

- **`coreApi.trustedProxies`.** Behind an edge or CDN, list BOTH the edge's
  published ranges AND the ingress controller's. The real-client walk skips
  every trusted hop, so a controller that is not in the list makes the
  resolver fall back to its own address and every request collapses onto
  one IP — per-actor throttle keys stay authoritative, but the per-IP signal
  and the audit records go quiet.
- **`coreApi.publicBaseUrl`.** Core-api derives the session cookies' `Secure`
  flag from it. Set it to the internal service name and every cookie on a
  public deployment drops the flag.

## Scaling

Every app component is stateless and scales horizontally. The chart gives
each of the three a real per-service resource envelope, replacing the one
chart-wide default, and each block carries an `autoscaling` stanza that is
off by default.

| Component | requests (cpu / memory) | limits (cpu / memory) | HPA max |
|---|---|---|---|
| core-api | 500m / 256Mi | 2 / 512Mi | 6 |
| worker | 250m / 256Mi | 1 / 512Mi | 6 |
| web | 100m / 128Mi | 1 / 512Mi | 4 |

The limits mirror `deploy/docker-compose.yml`'s per-service `cpus` and
`mem_limit`, the one measured per-service ceiling the repo carries, and the
requests sit at a sensible fraction of those. core-api takes the largest CPU
limit because it holds the hot read path: 259 rps at concurrency 8, p95
49.2 ms, roughly 3x headroom (`docs/perf.md` §5). Lane R5, the
multi-endpoint load harness, will replace these with measured numbers.

Turn an HPA on with, for example:

    helm upgrade marula deploy/helm/marula \
      --set coreApi.autoscaling.enabled=true \
      --set coreApi.autoscaling.maxReplicas=4

While `autoscaling.enabled` is true the Deployment omits `.spec.replicas`, so
the HPA owns the count. While it is false, set `replicaCount` as before. CPU
targets need a metrics server in the cluster.

**Read this before raising `coreApi.maxReplicas`.** Each core-api replica
opens its own Postgres connections: 45 at the R1 defaults (20 app + 25
reporting), per `docs/perf.md` §5. Six replicas would ask for 270 connections
against Postgres's default `max_connections = 100`. Scale core-api only
behind a session pooler (PgBouncer or pgcat), or with `max_connections`
raised to match. The worker does not share that shape: it claims work with
`FOR UPDATE SKIP LOCKED` (`core-db/src/outbox.rs:47`,
`core-db/src/queue.rs:105`), so replicas never double-process a row.
