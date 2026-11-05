# Helm chart — deploy/helm/marula

Values-driven chart that mirrors `deploy/docker-compose.yml`. It deploys
Postgres (the pgvector image for parity with production, or an external
instance), Redis, a migrations job wired as a pre-upgrade hook, core-api,
worker and web. Mailpit and Garage are opt-in profiles. Garage
runs as a headless stateful service with a layout-bootstrap job, which
replaced the MinIO sample. An ingress ships with the chart and is disabled
until you want it.

Image policy (the private build tracker U10b): every image reference is digest-pinned —
`digestPinningEnforced: true` (default) refuses to render images without
`images.*.digest` set. Digests are published per release with SBOM +
provenance attestations (`.github/workflows/release.yml`).

Validate locally (helm 3.14+):

    helm lint marula
    helm template marula --set digestPinningEnforced=false \
      --set postgresql.ownerPassword=local-owner \
      --set postgresql.appPassword=local-app

The two passwords are required whenever the chart mints its own Secret
(`existingSecret` unset and `postgresql.deploy` true). A render without them
fails on purpose: the chart used to fall back to `changeme-*` placeholders,
which is a credential a careless install would keep.

Non-purpose: not an operator dashboard; no cross-env secret management
beyond documented values (set `existingSecret` for production secret
plumbing).

## Staging profile for alpha testers and the penetration test

`values-staging-pentest.yaml` in this directory takes every credential from one
Secret that already exists, keeps `/api/dev/*` off, lowers the app's own rate
limits per principal, turns on TLS with a placeholder host, and points mail at a
sink inside the cluster so recovery links can be read without leaving it. The
operator's sequence (create the Secret, pin the digests, seed one account per
tester, rotate a password, collect evidence, tear down) stays with whoever holds
the cluster and is not published here. Testers read `SECURITY.md` and the
known-issues list the operator hands out with their credentials.

## The public front door (the private build tracker V2)

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

In-cluster probes do not go through the ingress: they read the pod directly on
`/livez` (process only), `/readyz` (dependency and drain aware) and, for the
worker, `/healthz`. Those paths are not part of the routed surface and do not
appear in the pinned OpenAPI document.

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
opens 29 client connections at the defaults (20 app + 8 reporting + 1 LISTEN,
`docs/perf.md` §5), and the LISTEN connection does not go through a pooler.
With `pooler.enabled` the database sees the pooler's server side instead, 15
per (database, user) pair, so replicas no longer multiply what Postgres pays.
Without it, six replicas would ask for 270 connections against Postgres's
default `max_connections = 100`. Either enable the pooler
(`docs/reference/connection-pooling.md`) or raise `max_connections` to match.
The worker does not share that shape: it claims work with
`FOR UPDATE SKIP LOCKED` (`core-db/src/outbox.rs:47`,
`core-db/src/queue.rs:105`), so replicas never double-process a row.

## Pooler

The chart does not deploy one. It declines to deploy production Postgres for
the same reason, and a pooler is on the same footing: the operator brings the
instance that has to survive a failover. Turn the wiring on with the two URLs:

    helm upgrade marula deploy/helm/marula \
      --set pooler.enabled=true \
      --set pooler.appUrl=postgres://marula_app:PASS@pgbouncer:6432/marula \
      --set pooler.ownerUrl=postgres://marula:PASS@pgbouncer:6432/marula \
      --set pooler.directAppUrl=postgres://marula_app:PASS@postgres:5432/marula \
      --set pooler.directOwnerUrl=postgres://marula:PASS@postgres:5432/marula

With it on, the chart sets `DATABASE_URL` to the pooled URL, sets
`MARULA_LISTEN_DATABASE_URL` to the direct one (a `LISTEN` cannot ride a pooled
connection) and sets `MARULA_DB_STATEMENT_CACHE=0` (a cached statement name
cannot survive the pooler's reset between transactions). The pooler itself must
run the config in `deploy/pooler/pgbouncer.ini`; `deploy/pooler/README.md` has
a Kubernetes manifest for it.

## Lifecycle: stopping a pod without cutting work

Both binaries install the same stop path (`crates/worker/src/shutdown.rs`):
SIGINT or SIGTERM, whichever arrives first, stops accepting work and gives
what is in flight a bounded window to finish. Before this, both processes
waited on Ctrl-C (SIGINT) only, the signal a container runtime never sends, so
every rolling deploy was a hard kill of whatever the process was holding.

What counts as in flight: a request core-api has accepted and not yet
answered. Event streams are the documented exception. A stream is answered by
closing, and a client reconnects and resyncs from its snapshot, so one that
outlives the window is cut on purpose.

The numbers, and how they relate:

| Value | Where | Default |
|---|---|---|
| drain window | `MARULA_SHUTDOWN_GRACE_SECS` (`coreApi.shutdownGraceSeconds`, `worker.shutdownGraceSeconds`) | 20s |
| pod grace period | `.spec.terminationGracePeriodSeconds` | 45s core-api, 35s worker and web |
| pre-stop delay | `.spec.lifecycle.preStop` (`preStopSleepSeconds`) | 5s |
| compose stop grace | `deploy/docker-compose.yml` `stop_grace_period` | 30s |

`terminationGracePeriodSeconds` has to exceed `preStopSleepSeconds` plus the
drain window, or the runtime kills the process mid-drain and the window is
decorative. If you raise the drain window, raise the pod grace period with it.

Each application answers three probes, and they are not interchangeable:

| Probe | core-api | worker | web |
|---|---|---|---|
| startup | `/livez` | `/livez` | `/` |
| liveness | `/livez` | `/healthz` | `/` |
| readiness | `/readyz` | `/readyz` | `/` |

- `/livez` reads nothing but the process's ability to answer, and stays 200
  inside the drain window. A drain that failed liveness would restart the pod
  it is retiring.
- `/readyz` is 503 while draining, and 503 when a dependency does not answer
  (core-api runs a `SELECT 1` with a 2s ceiling; the worker reads its own loop
  flags). Readiness used to point at `/api/v1/health`, which is a constant
  `ok`, so a replica with an unreachable database stayed Ready and kept
  taking traffic.
- The worker's liveness reads `/healthz` because a worker owns no traffic: a
  dead loop there is a restart-worthy fault, and readiness alone would leave
  it running dead. `/healthz` stays 200 while draining, and now answers 503
  (instead of 200 with a body saying `unhealthy`) when a loop has died.

`podDisruptionBudget.enabled` renders a `maxUnavailable: 1` budget for each
component. It is not `minAvailable: 1` on purpose: a one-replica install with
`minAvailable: 1` cannot be evicted at all, so every node drain hangs until
someone deletes the budget. `topologySpread` is soft (`ScheduleAnyway`) for
the same reason — a single-node cluster still has to schedule.

`networkPolicy.enabled` is off by default. Turning it on restricts ingress to
the release namespace plus the ingress controller's namespace, and everything
else must be listed in `networkPolicy.extraIngress`; egress is untouched.
Check that kubelet probes still reach the pods after enabling it — some CNIs
treat node-originated traffic differently, and the symptom is a probe failing
with nothing in the container log.
