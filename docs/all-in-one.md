# All-in-one mode

One process, three services. `core-api` can host the background worker loops
and the realtime WebSocket hub inside itself, so a demo or a small one-box
install runs a single application container instead of three.

This is off by default. The split topology (separate `core-api`, `worker` and
`realtime` containers) is what `deploy/docker-compose.yml` ships and what
production uses. All-in-one is for the small end: a laptop demo, an evaluation
box, an appliance one operator runs.

## Turn it on

| Variable | Required | Meaning |
|---|---|---|
| `MARULA_ALL_IN_ONE` | yes (`1`) | Host the worker loops and the realtime hub in this process. |
| `MARULA_WORKER_DATABASE_URL` | yes | OWNER-role Postgres URL for the worker loops. |
| `MARULA_REALTIME_BIND_ADDR` | no (default `0.0.0.0:8081`) | Hub listen address. |
| `MARULA_WORKER_BIND_ADDR` | no (default `0.0.0.0:8082`) | Worker health-endpoint address. |

Boot refuses, with copy that names the variable, in two cases:

- `MARULA_ALL_IN_ONE=1` with `MARULA_WORKER_DATABASE_URL` unset or blank,
- two of the three listen addresses (`BIND_ADDR`, `MARULA_REALTIME_BIND_ADDR`,
  `MARULA_WORKER_BIND_ADDR`) end in the same port.

`MARULA_WORKER_DATABASE_URL` has to be the OWNER role, not the app role.
`core-api` serves requests on the RLS-scoped app role, and the worker loops
read outbox rows and SLA state across every org. A constrained role returns
zero rows with no error at all, so the outbox relay would publish nothing and
the logs would stay quiet about it.

## Compose

`deploy/docker-compose.all-in-one.yml` overrides the base file:

```
docker compose \
  -f deploy/docker-compose.yml \
  -f deploy/docker-compose.all-in-one.yml \
  up -d postgres redis migrate core-api web
```

Naming the services matters. Do not start `worker` or `realtime`, because in
this topology `core-api` itself binds ports 8081 and 8082.

## What stays the same

The writer boundary holds. The worker writes no domain rows of its own, and
its effects call `/api/internal/*` on the API. In all-in-one that base URL
points back at the same process over loopback
(`http://127.0.0.1:<port>`), derived from `BIND_ADDR` unless `INTERNAL_API_URL`
is set. Webhook delivery keeps working even when `MARULA_SECRET_KEY` is unset
and the API derived a dev key from `JWT_SECRET`, because the API hands its
effective key to the embedded worker.

The realtime hub verifies the tokens this API signs: the same `JWT_SECRET`,
plus the API's EdDSA public key when the asymmetric posture is on.

## What changes

One process holds both database roles. The API keeps its app-role pool and
its reporting pool. The embedded worker opens its own owner-role pool and the
delivery pool. All four read the env names they always read, so
`MARULA_DB_POOL_MAX` now sizes both the app pool and the worker pool inside
this process, and the connection total is the sum of the four ceilings.
`docs/perf.md` §5 has the per-pool arithmetic. Add the pools up before raising
any ceiling.

One copy of the background work. The `worker` and `realtime` containers carry
their own resource limits and, with the chart's autoscaling on, their own
replica counts. All-in-one has neither. The background work scales with the
API process or not at all, and a loop that dies is logged while the API keeps
serving, rather than restarted by an orchestrator.

Use all-in-one for a demo, an evaluation box, or a one-box appliance. Use the
split stack when the background work needs to scale on its own, or when a dead
loop has to be restarted independently.
