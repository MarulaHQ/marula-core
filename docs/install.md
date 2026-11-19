# Install Marula on a machine

This page installs Marula on a server that serves a team. You download the
release bundle for your platform, unpack it, and run the installer that ships
inside it. The installer measures the machine, prints the plan, and applies it
when you say so.

There is no hosted `curl … | sh` yet. The bundle is the install path, and a
bundle you can read before running is the point.

## What you need

- A Linux host, x86_64 or arm64. macOS runs the stack too, for testing.
- Docker with Compose 2.24 or newer. Older Compose ignores the `!reset` key
  that pins a registry install to the release's images.
- Four cores, 8 GB of memory and 40 GB of free disk for the default shape.
  The smaller shapes have lower disk floors; the installer refuses a host that
  misses one and names the number it missed.
- Ports 3000, 8080 and 15432 free on the host. A profile can add more. 15432
  is Postgres, published for local tooling, and the `edge` profile takes it off
  the network.
- No account, licence key or hosted service. Docker Hardened Images are free
  and pull without a login. If a pull ever asks for credentials, `docker login
  dhi.io` with a free Docker account is all it takes.

Nothing else has to be installed. The bundle carries the binary, the wizard
front end, the compose files and the files they mount.

## Install

Download the archive for your platform from the
[Releases page](https://github.com/MarulaHQ/marula-core/releases), then check it
against the signed checksum list before you unpack it:

```sh
sha256sum -c SHA256SUMS-linux-x64       # shasum -a 256 -c on a Mac without coreutils
tar -xzf marula-<version>-linux-x64.tar.gz
cd marula-<version>-linux-x64
```

[`docs/release.md`](release.md) has the signature check, why it comes first,
and what each failure means.

Read the plan. It writes nothing:

```sh
./install.sh --dir /srv/marula --images registry
```

With no arguments at all, the script opens the wizard instead and installs into
`~/marula`. Run it from the unpacked archive, because it fixes the compose file
and the wizard front end to the bundle's own copies and refuses `--compose-file`
or `--ui-dir`.

The script verifies the bundle's `SHA256SUMS`, then hands every other flag to
the bundled `marula install`. `--images registry` stops Compose from building
and pins the four application images to the digests in the bundle's
`release.json`, which the installer finds on its own, so no digest is copied by
hand. The plan names the host facts it measured, every step with the reason it
exists, and the work that stays yours: the hostname and TLS certificate, the
backup destination, an alert receiver.

Apply it when the plan looks right:

```sh
./install.sh --dir /srv/marula --images registry --apply
```

`--apply` writes one environment file, `marula.env`, in the directory you
named, starts the stack, and returns when the profile's health checks pass. A
successful exit means the API answers, not that containers started.

If the directory is not writable by your user, run the same commands with
`sudo`, as the README block does.

```sh
./marula doctor --dir /srv/marula
```

The doctor reads `marula.env` and checks the host, Docker, the required values,
the ports and the embedding model. It exits non-zero when a check fails, so it
is safe to put in a runbook or a cron job.

## What you get

| URL | What it is |
|---|---|
| http://<host>:3000 | The workspace: tickets, dispatch, projects, billing |
| http://<host>:3000/portal | The client portal |
| http://<host>:8080 | `core-api`, the only writer to the database |
| http://<host>:8080/docs | The API reference, generated from the OpenAPI document |
| http://<host>:8082/healthz | The worker's health endpoint |

Open `/login` and switch to **Create org** to make your organisation and the
first owner account. [Bootstrap your first
organisation](site/how-to/admin-org-bootstrap.md) walks that screen and the four
settings worth changing before your team logs in.

Nothing here terminates TLS. Put a proxy in front before you open the port to
the internet: the `edge` profile ships a Caddy configuration with three
certificate modes, and
[`docs/site/reference/reverse-proxy-tls.md`](site/reference/reverse-proxy-tls.md)
covers the one you pick.

## The eight secrets

The stack refuses to render without eight values. `--apply` generates any that
are missing, using `openssl rand -hex 32` equivalents, and writes them resolved
into `marula.env`.

| Value | What it protects |
|---|---|
| `JWT_SECRET` | Signs access and refresh tokens. Changing it signs everyone out. |
| `MARULA_SECRET_KEY` | Encrypts sealed integration secrets at rest. core-api and the worker must hold the same value, or sealed secrets become unreadable. |
| `MARULA_INTERNAL_TOKEN` | The bearer half of the worker-to-API seam. |
| `EMAIL_INTERNAL_TOKEN` | The email half of the same seam. |
| `POSTGRES_PASSWORD` | The `marula` owner role's password. Postgres applies it when it first creates the data volume, so a later change to the file does not change the role. |
| `MARULA_APP_PASSWORD` | The `marula_app` role's password, which the `app-role-password` one-shot applies on every start. Safe to rotate. |
| `GARAGE_RPC_SECRET` | Garage's inter-node RPC secret, used by the `stage2` object store. |
| `GLITCHTIP_SECRET_KEY` | GlitchTip's Django signing key, used by the `observability` profile. |

Two rules keep the file useful. A re-run reads what `marula.env` already holds
before it generates anything, so applying twice does not rotate a signing key or
a database password. And the file holds resolved values rather than templates,
because Compose does not expand variables inside an env file.

`deploy/dev.env` in the repository is not an install: its values are published,
so anyone who has read the repository can mint an owner token against a stack
running on them. The stack warns at boot while they are in use.

To set a value yourself, pass it once:

```sh
./install.sh --dir /srv/marula --images registry --apply --set MARULA_DB_POOL_MAX=40
```

A name the descriptor does not publish is refused before anything is written,
so a typo is an argument error rather than a setting that does nothing.
`--set` also accepts the four infrastructure values the installer generates
itself: both database passwords, the object store secret and the error tracker
key.

## Shapes

A shape is a promise about processes. The installer scores the host against
each one and takes the highest that fits, preferring `split`.

| Shape | What runs | Cores | Memory | Free disk |
|---|---|---|---|---|
| `split` | core-api and the worker as separate containers | 4 | 8192 MB | 40 GB |
| `all-in-one` | one core-api process, with the worker loops inside | 4 | 8192 MB | 30 GB |
| `demo` | `split` plus a seeded organisation, loopback only | 4 | 8192 MB | 30 GB |

`split` is the shipped topology: a stalled worker restarts without
interrupting the API, and the two processes scale apart. `all-in-one` trades
that for one container on a small box. `demo` is never chosen for you, because
it carries a published password and an unauthenticated reset surface, and the
installer pins its API to loopback.

Name a shape when you want it:

```sh
./install.sh --dir /srv/marula --apply --profile all-in-one
```

A host the shape cannot carry is refused with the floor it missed. On a
platform that publishes no memory reading, declare it with `--memory-mb 16384`
or `MARULA_HOST_MEMORY_MB`, and the same host is scored again. `--allow-unfit`
plans a shape the host misses anyway, which is a decision to proceed with less
than the shape asks for.

## Profiles

Compose profiles add moving parts an install does not need on day one. They
stay off until you name them.

| Profile | What it adds | When to turn it on |
|---|---|---|
| `pooler` | PgBouncer in transaction mode, plus its auth renderer | Several API instances, or many clients against a Postgres that caps connections |
| `stage2` | Garage object storage and a Mailpit mail sink | Attachments on the host, or mail captured locally instead of sent |
| `observability` | OTel collector, Tempo, Loki, Prometheus with the alert pack, Grafana, GlitchTip, Uptime Kuma | You want traces, logs, metrics and a status page on the box |
| `ai` | A local Ollama runtime | The org runs a local embedding or chat model |
| `backup` | WAL-G point-in-time recovery and restic snapshots | You want scheduled backups instead of a nightly dump |
| `edge` | Caddy terminating TLS | The host faces the internet |

Start one from the unpacked release, with the install's env file:

```sh
docker compose -f deploy/docker-compose.yml --env-file /srv/marula/marula.env \
  --profile stage2 up -d
```

The pooler is opt-in because a single-node install does not need it. core-api
and the worker dial Postgres directly, and one instance's connections fit
inside the 100 a stock Postgres allows. Turn the profile on when you add a
second API instance, and set the two database URLs to `pgbouncer:6432` and
`MARULA_DB_STATEMENT_CACHE=0` in the env file at the same time.
[`deploy/README.md`](../deploy/README.md) has the connection arithmetic and
[`docs/reference/connection-pooling.md`](reference/connection-pooling.md) has
the measurements.

## Upgrade

Upgrades move one release at a time, and Marula supports jumps from either of
the two previous tagged releases. Read the release notes before you start: they
carry a **Breaking & deprecated** section even when it is empty.

1. Take a backup that has restored once. The next section is the short version,
   and [the backup runbook](site/reference/backup-restore.md) is the long one.
2. Download the new bundle and check its checksum the same way you did the
   first one.
3. Run the new installer against the directory you already use:

   ```sh
   ./install.sh --dir /srv/marula --images registry --apply
   ```

   The plan reads the existing `marula.env` first, so the eight values stay put
   and the new images replace the old ones. Migrations run before the API
   starts, and the command returns when the health checks pass.

4. Run `./marula doctor --dir /srv/marula` and open the workspace.

**Upgrading from a release that defaulted to the pooler.** Earlier releases
started a connection pooler and pointed both database URLs at `pgbouncer:6432`.
If your `marula.env` still names that host, it keeps winning over the new
default, and a stack started without the pooler has nothing to answer on that
port. Pick one: start the stack with the `pooler` profile
(`docker compose --profile pooler …`), or change `MARULA_APP_DATABASE_URL` and
`MARULA_WORKER_APP_DATABASE_URL` to `postgres:5432` before you recreate the
stack. One core-api instance fits inside a stock Postgres connection ceiling, so
the direct URLs are the supported default for a single host.

A release that publishes a signed update manifest can be applied without the
download step. `marula update --check` says whether a newer release exists, and
`marula update --apply` verifies the signature, backs up the files the install
owns, applies through the same target and rolls the files back when the doctor
disagrees. Rollback restores configuration, not data, so the backup in step 1
is still yours to take. [`docs/update.md`](update.md) has the flags.

## Backup and restore

Everything lives in Postgres, and once object storage is on, the attachments
live in the bucket as well. Back up both or neither: a database-only restore
leaves attachments pointing at objects that are not there.

Take a dump as the `marula` owner, from the unpacked release directory:

```sh
docker compose -f deploy/docker-compose.yml --env-file /srv/marula/marula.env \
  exec -T postgres pg_dump -U marula -d marula -Fc > marula-$(date +%F).dump
```

Dump as `marula`, not `marula_app`. The app role is constrained by row-level
security and cannot produce a complete dump. Use a `pg_dump` that matches the
server's major version; the postgres image carries the right one.

To restore, stop core-api and the worker, restore the dump into a database of
the same major version, start them again, and restore the object store in the
same window. If rows and objects come back at different times, leave
`MARULA_OBJECT_GC` off until the restore finishes, or the integrity pass marks
live attachments as missing. [The backup and restore
runbook](site/reference/backup-restore.md) has the restore order, the restic and
WAL-G paths, and the drill that proves a restore works.

## Hardening

The stack is built to run without root and without a writable filesystem.

- Every third-party slot that has a hardened equivalent uses a Docker Hardened
  Image: Postgres with pgvector, Valkey, PgBouncer, Grafana, Tempo, Loki and
  ClamAV. Garage, Mailpit, GlitchTip, Uptime Kuma and the OpenTelemetry
  collector run upstream images under the same pinning rules.
- Every image reference carries an exact tag and a registry digest. The digest
  is what runs, and the release's manifest names the digests it published.
- `cap_drop: [ALL]` and `no-new-privileges` apply to every service. Most mount
  a read-only root filesystem and write only to their own volume or a tmpfs.
  Postgres, Grafana and ClamAV keep a writable root because their own boot
  logic needs one, and each says so in the compose file.
- The application images drop to a fixed numeric user at build time. Nothing
  runs as root except Postgres' one-time first-boot init, which re-execs as
  `postgres` before it accepts a connection.

Hardened registry pulls need no account. The images are free, and a fresh host
with no credential store pulls them like any other public image. That is
Docker's policy rather than a promise from this repository, so if a pull ever
asks for credentials, `docker login dhi.io` with a free Docker account is the
fallback. The login is cached on the host, so an install and its later pulls
share it. If your fleet cannot reach `dhi.io`, mirror the hardened images into a
registry you control and change the image references in the compose file to your
mirror. Keep the digests, so a mirror cannot substitute a different image behind
your back.

## Troubleshooting

| Symptom | What it means |
|---|---|
| `unauthorized` while pulling `dhi.io/...` | Docker asked for credentials, which the hardened images do not normally require. `docker login dhi.io` with a free Docker account is the fix. |
| `no shipped profile fits this host` | The host misses a floor and the message names it. Free disk and cores are the usual ones. On a platform with no memory reading, pass `--memory-mb`. |
| `port is already allocated` | Something holds 3000, 8080 or 15432. Stop it, or move the service. |
| The plan refuses the embedding model | The profile's local model needs more memory than the host reports. Set `MARULA_EMBEDDING_PROVIDER=openai` and `MARULA_EMBEDDING_API_KEY`, or give the host more memory. |
| `--apply` stopped partway | Read the step it named, then run `./marula doctor --dir /srv/marula` and `docker compose logs` for the service that did not come up. The env file is already written, so a second `--apply` continues from the values on disk. |
| Compose rejects `!reset` | Your Compose is older than 2.24. Upgrade Docker Compose and apply again. |
| `marula.env` is lost | The eight values are gone with it. Restore the file from your backup; a stack rebuilt with new signing keys signs everyone out. |

The install's logs are the compose services' logs, and `marula doctor` names
the check that failed. A stack that answers is the same event as a command
that exits zero, which is why the installer waits for the health checks.

For a demo box, `marula demo reset --dir /srv/demo` replaces the seeded data
between showings.
