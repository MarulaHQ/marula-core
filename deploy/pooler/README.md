# The pooler

PgBouncer in transaction mode, in front of Postgres, for every deployment that
runs more than one application instance. `docs/reference/connection-pooling.md`
is the reasoning and the measurements; this file is how to run it.

## Files

| File | What |
|---|---|
| `pgbouncer.ini` | The shipped config. Transaction pooling, `DISCARD ALL` after every transaction, no prepared-statement caching. Every line is load-bearing and carries its own note. |
| `render-userlist.sh` | Writes PgBouncer's auth file (`"role" "password"`, one pair per line) from `POSTGRES_PASSWORD` and `MARULA_APP_PASSWORD`. The compose stack runs it in the `pooler-auth` one-shot and mounts the result at `/auth/userlist.txt`. |

**No credential is checked in.** The auth file is generated, in a volume, by
`render-userlist.sh` on every `up`, so the two database passwords have exactly
one source each: the environment. The file that used to live here held the
published dev values, which are also what migration 0114 mints for the app
role — a working login for anyone who had read the repository. It is gone,
along with the hand edit that used to be the only way to follow a rotation.

The alternative a deployment can use instead is `auth_query`: PgBouncer asks
the database to check a password rather than reading one, which needs a
`SECURITY DEFINER` function on the server. That is a schema change (a
migration), not a deployment one, and it is not in the chain today. Until it
is, the rendered file is the mechanism.

## Running the renderer yourself

A pooler that compose does not manage still needs the file. Run the same
script where the two variables are already exported — it takes the output path
as its only argument:

```sh
POSTGRES_PASSWORD=... MARULA_APP_PASSWORD=... \
  sh deploy/pooler/render-userlist.sh /auth/userlist.txt
```

It writes mode 644 — readable by the pooler's own uid, which is what a file
written by one container and read by another has to be once `CAP_CHOWN` is
dropped — and refuses to run with either variable empty, so a missing
password fails the container instead of producing a pooler that authenticates
nobody. On Kubernetes that is an `initContainer` beside the pooler (the
manifest below does it), or a step in the deploy job that writes a Secret the
pod mounts.

Rotating either password means running it again and restarting the pooler:
the file is read at startup, so a running instance keeps the old one.

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
    auth_file = /auth/userlist.txt
    pool_mode = transaction
    server_reset_query = DISCARD ALL
    server_reset_query_always = 1
    max_prepared_statements = 0
    default_pool_size = 15
    max_db_connections = 40
    max_client_conn = 400
---
# The renderer, shipped once: kubectl create configmap pgbouncer-scripts \
#   --from-file=render-userlist.sh=deploy/pooler/render-userlist.sh
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
      # The auth file lives in an emptyDir this group owns, so the renderer and
      # the pooler — different containers, same uid — can both use it.
      securityContext:
        fsGroup: 65532
      initContainers:
        # Runs on every pod start, so a rotated Secret reaches a new pooler pod
        # without anyone hand-editing a file. Restart a pooler to pick it up.
        - name: render-userlist
          image: dhi.io/pgvector:0.8-pg18@sha256:069119d1ad25e1155e165786230ef3b4a570f1857b315166924b895c4d2c6946
          command: ["sh", "/pooler/render-userlist.sh", "/auth/userlist.txt"]
          env:
            - name: POSTGRES_PASSWORD
              valueFrom:
                secretKeyRef: { name: marula-secrets, key: POSTGRES_OWNER_PASSWORD }
            - name: MARULA_APP_PASSWORD
              valueFrom:
                secretKeyRef: { name: marula-secrets, key: POSTGRES_APP_PASSWORD }
          securityContext:
            runAsNonRoot: true
            runAsUser: 65532
            runAsGroup: 65532
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities: { drop: ["ALL"] }
          volumeMounts:
            - { name: pooler-scripts, mountPath: /pooler, readOnly: true }
            - { name: auth, mountPath: /auth }
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
            - { name: auth, mountPath: /auth, readOnly: true }
      volumes:
        - name: config
          configMap: { name: pgbouncer }
        - name: pooler-scripts
          configMap: { name: pgbouncer-scripts }
        - name: auth
          emptyDir: {}
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
