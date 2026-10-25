# The pooler

PgBouncer in transaction mode, in front of Postgres, for every deployment that
runs more than one application instance. `docs/reference/connection-pooling.md`
is the reasoning and the measurements; this file is how to run it.

## Files

| File | What |
|---|---|
| `pgbouncer.ini` | The shipped config. Transaction pooling, `DISCARD ALL` after every transaction, no prepared-statement caching. Every line is load-bearing and carries its own note. |
| `userlist.txt` | The auth file: one `"user" "password"` pair per role. Dev values only. PgBouncer refuses the whole file over a comment line, so this one carries no notes: they live here. |

**`userlist.txt` is not a secret store.** It cannot read the environment, so
it does not follow `POSTGRES_PASSWORD` / `MARULA_APP_PASSWORD`. Every other
component in the stack takes its Postgres password from those variables; this
file is the one copy an operator edits by hand. Change a role password with
`ALTER ROLE`, then rewrite the matching line here (`"role" "password"`) and
restart the pooler. The alternative is to let PgBouncer authenticate against
an `auth_query`, which needs a `SECURITY DEFINER` function on the database.
This lane did not add one, and the compose header says so where the required
variables are listed.

## Compose

The default stack starts it. Postgres keeps the published port (`15432`), the
pooler takes `6432`, and the application dials `pgbouncer:6432`:

```sh
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml exec postgres psql -U marula -d marula \
  -c "select count(*) from pg_stat_activity where datname = 'marula'"
```

The count is the server side, and it stays near `default_pool_size` whatever
the instance count. For the client side and the per-pool view:

```sh
docker compose -f deploy/docker-compose.yml exec postgres psql -U marula -d marula \
  -c "show max_connections"
# and, on a machine with psql, the pooler's own console:
psql "postgres://marula@127.0.0.1:6432/pgbouncer" -c "SHOW POOLS" -c "SHOW STATS"
```

## Kubernetes

The chart does not deploy a pooler: it declines to deploy production Postgres
for the same reason (`deploy/helm/README.md`). Point `pooler.enabled` at yours
and the chart rewires the environment — `DATABASE_URL` to the pooler,
`MARULA_LISTEN_DATABASE_URL` straight at Postgres, and the statement cache off.
A pooler that runs exactly this config:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: pgbouncer
data:
  pgbouncer.ini: |
    [databases]
    marula = host=postgres port=5432 dbname=marula
    [pgbouncer]
    listen_addr = 0.0.0.0
    listen_port = 6432
    auth_type = scram-sha-256
    auth_file = /etc/pgbouncer/userlist.txt
    pool_mode = transaction
    server_reset_query = DISCARD ALL
    server_reset_query_always = 1
    max_prepared_statements = 0
    default_pool_size = 15
    max_db_connections = 40
    max_client_conn = 400
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: pgbouncer
spec:
  replicas: 2            # two, so a rollout is not an outage
  selector:
    matchLabels: { app: pgbouncer }
  template:
    metadata:
      labels: { app: pgbouncer }
    spec:
      containers:
        - name: pgbouncer
          image: dhi.io/pgbouncer:1.26@sha256:17157120154cd111541281b39e9ad846e567e7fc1ea2e964abdc8c4bf4547fff
          ports: [{ containerPort: 6432 }]
          securityContext:
            runAsNonRoot: true
            runAsUser: 65532
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities: { drop: ["ALL"] }
          readinessProbe:
            tcpSocket: { port: 6432 }
            periodSeconds: 5
          volumeMounts:
            - { name: config, mountPath: /etc/pgbouncer, readOnly: true }
      volumes:
        - name: config
          configMap: { name: pgbouncer }
---
apiVersion: v1
kind: Service
metadata:
  name: pgbouncer
spec:
  selector: { app: pgbouncer }
  ports: [{ port: 6432, targetPort: 6432 }]
```

The readiness probe is a TCP connect rather than `pg_isready`: the hardened
image carries the pooler and nothing else, so there is no client binary inside
it to ask.

## Sizing

`default_pool_size` is per (database, user) pair. The worker connects as the
owner role and core-api as `marula_app`, so two pairs, 15 each, 30 server
connections at rest — the number that has to stay under Postgres's
`max_connections`, not the 54 connections per instance. Raise
`default_pool_size` only against a measured queue, and keep
`max_db_connections` as the hard stop.

## Rollback

Point `DATABASE_URL` back at Postgres (`MARULA_APP_DATABASE_URL` overrides it
in compose), clear `MARULA_LISTEN_DATABASE_URL`, and either set
`MARULA_DB_STATEMENT_CACHE` to a capacity you have made room for or leave it at
0. The connection ceiling returns with it: one instance is 54, two are 108
against a default Postgres of 100.
