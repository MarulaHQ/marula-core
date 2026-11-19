# Install Marula on a machine

This page takes a release bundle to a running stack on a server. You download
two files, check the archive against its checksum, unpack it, and run the
installer inside. The installer either walks you through a browser wizard or
prints a plan for you to apply.

There is no hosted `curl … | sh`. The bundle is the install path, and a bundle
you can read before running is the point.

## What you need

- A Linux host, x86_64 or arm64. macOS on Apple silicon runs the stack too, and
  the release publishes a `macos-arm64` bundle for it.
- Docker with Compose 2.24 or newer. The installer's registry override uses
  Compose's `!reset` key to remove the base file's build instructions, and
  Compose older than 2.24 ignores that key and builds instead.
- A budget for the whole box, not one process: four cores, 8 GB of memory and
  40 GB of free disk for the default shape, or 30 GB of disk on `all-in-one`.
  The largest line in that budget is the shipped embedding model, which the
  `ai` compose profile runs, plus room for Postgres and the object store. Leave
  `ai` off and the install has headroom to spare.
- These ports free on the host:
  - `3000`: the workspace and the client portal.
  - `8080`: the API and its `/docs` reference.
  - `8082`: the worker's health endpoint. `split` and `demo` serve it from the
    worker container, and `all-in-one` serves it from the API process.
  - `15432`: Postgres, published for local tooling. The application does not
    use this port, so block it or leave it on loopback.
- No account, licence key or hosted service. Docker Hardened Images are free
  and pull without a login. If a pull ever asks for credentials, `docker login
  dhi.io` with a free Docker account is all it takes.

## Install

### 1. Download the archive and its checksum list

Both files come from the [Releases
page](https://github.com/MarulaHQ/marula-core/releases). Pick the pair that
matches the machine:

| Platform | Archive | Checksum list |
|---|---|---|
| Linux x86_64 | `marula-<version>-linux-x64.tar.gz` | `SHA256SUMS-linux-x64` |
| Linux arm64 | `marula-<version>-linux-arm64.tar.gz` | `SHA256SUMS-linux-arm64` |
| macOS arm64 | `marula-<version>-macos-arm64.tar.gz` | `SHA256SUMS-macos-arm64` |

Check the archive before you unpack it. The command differs by platform:

```sh
# Linux
sha256sum -c SHA256SUMS-linux-x64

# macOS
shasum -a 256 -c SHA256SUMS-macos-arm64
```

An `OK` line for the archive is the pass. This check proves the download arrived
intact. It does not prove who published it: the `.sig` file published beside the
list is the release key's signature, and [`docs/release.md`](release.md) gives
the `ssh-keygen -Y verify` command, the identity to expect, and what each
failure means.

Unpack it and step inside:

```sh
tar -xzf marula-<version>-linux-x64.tar.gz
cd marula-<version>-linux-x64
```

The directory holds the `marula` binary, `install.sh`, the compose files, the
wizard front end and a `SHA256SUMS` over all of it. The binary is not installed
on `PATH`, so commands on this page that call `marula` are run as `./marula`
from this directory.

### 2. Choose an entry point

`./install.sh` checks the tree against its own `SHA256SUMS`, then runs the
bundled `./marula install` with the bundle's compose file and wizard front end.
It has two behaviours, and they are not the same command:

- **No arguments: the wizard.** `./install.sh` adds `--ui`. The wizard serves a
  page on `127.0.0.1`, plans and applies on its own, and targets `~/marula`.
  There is no separate `--apply` step. To point it elsewhere, pass
  `--ui --dir <path>`.
- **With arguments: they go to `./marula install`.** Passing `--ui` yourself is
  how you run the wizard on a chosen port. Without `--ui`, the command prints
  the plan and writes nothing; `--apply` is what writes the environment file
  and starts the stack on that path. `--ui` refuses `--apply`, `--set` and
  `--profile`, because the wizard asks for those itself.

The wizard prints a URL with a one-time token. On a headless host, pick a port,
skip the browser and forward it over SSH:

```sh
./install.sh --ui --port 8899 --no-open
# then, from your workstation:
ssh -L 8899:127.0.0.1:8899 you@server
```

On a server, skip the wizard and read the plan as text:

```sh
./install.sh --dir /srv/marula --images registry
```

The script fixes the compose file and the wizard front end to the bundle's own
copies, so `--compose-file` and `--ui-dir` are refused. Run it from the unpacked
directory.

### 3. Choose where the images come from

The installer takes `--images build|registry`. The default is `build`, which
compiles the four application images from a source checkout. A release bundle
carries no sources, so every install from a bundle uses `--images registry`. The
wizard asks the same question and offers the published images.

Registry mode pulls these four images by digest instead of building them:

- `marula-core-api`
- `marula-worker`
- `marula-migrate`
- `marula-web`

The digests come from the bundle's `release.json`, which the installer finds on
its own, so nothing is copied by hand. A digest is what the release signed, and
a tag can move.

### 4. Apply and check the result

`/srv/marula` does not need to exist. The installer creates it, and any missing
parent, when it writes the environment file. What has to be true is that the
user running it can create a directory under `/srv`. If not, run the install
with `sudo`: `marula.env` and the override files are then owned by root, and
every later `doctor`, Compose or backup command has to run as root or with
`sudo`.

```sh
./install.sh --dir /srv/marula --images registry --apply
```

`--apply` writes `marula.env`, starts the stack, and returns when the services
it started pass their health checks, which is what `docker compose up --wait`
waits for. A zero exit means every service the shape starts answered its check.
It does not mean TLS, mail, backups or alerting are configured; the plan's
"still yours to do" list stays yours.

If you used the wizard, it has already applied. Skip to the doctor.

```sh
./marula doctor --dir /srv/marula
```

The doctor reads `marula.env` and checks the host, Docker, the required values,
the ports and the embedding model. It exits non-zero when a check fails.

## What you get

| URL | What it is |
|---|---|
| http://<host>:3000 | The workspace: tickets, dispatch, projects, billing |
| http://<host>:3000/portal | The client portal |
| http://<host>:8080 | `core-api`, the only writer to the database |
| http://<host>:8080/docs | The API reference, generated from the OpenAPI document |
| http://<host>:8082/healthz | The worker's health endpoint. `all-in-one` serves it from the API process |

Open `/login` and switch to **Create org** to make your organisation and the
first owner account. [Bootstrap your first
organisation](site/how-to/admin-org-bootstrap.md) walks that screen and the four
settings worth changing before your team logs in.

Nothing here terminates TLS. Put a proxy in front before you open the port to
the internet: the `edge` compose profile ships a Caddy configuration with three
certificate modes, and
[`docs/site/reference/reverse-proxy-tls.md`](site/reference/reverse-proxy-tls.md)
covers the one you pick.

## Running commands against the install

Keep the unpacked release directory. The compose file and the commands below run
from it. The installer writes an override file beside the env file for restart
policies, `marula.restart.yml`, and a second one for images,
`marula.images.yml`, when the install used `--images registry`. A compose
command that leaves them out describes a different stack, and in registry mode
it tries to build images the bundle cannot build.

Set `INSTALL` to the directory you installed into and define this once in the
shell you use. This is the registry form, which is what a bundle install uses:

```sh
cd /path/to/marula-<version>-linux-x64
INSTALL=/srv/marula          # or $HOME/marula after a wizard install
marula_compose() {
  docker compose --env-file "$INSTALL/marula.env" \
    -f deploy/docker-compose.yml \
    -f "$INSTALL/marula.restart.yml" \
    -f "$INSTALL/marula.images.yml" "$@"
}
marula_compose ps
```

An install that built from a source checkout has no `marula.images.yml`, so
with the same `cd` and `INSTALL`, use this form instead:

```sh
marula_compose() {
  docker compose --env-file "$INSTALL/marula.env" \
    -f deploy/docker-compose.yml \
    -f "$INSTALL/marula.restart.yml" "$@"
}
```

For an `all-in-one` or `demo` install, add the shape's overlay as another `-f`
line inside the function, before the final `"$@"`:
`-f deploy/docker-compose.all-in-one.yml` or
`-f deploy/docker-compose.demo.yml`. The `-f` flags have to come before the
subcommand, which is why they live in the function rather than on each call.

The compose file sets the project name to `marula`, so the containers are
`marula-core-api-1` and so on. Run this as the user that ran the install, or
with `sudo` if the install did.

On `all-in-one`, start named services (`marula_compose up -d core-api web`)
instead of a bare `up -d`, because the base file also defines the separate
worker and the two would fight over port 8082.

## The eight secrets

The stack refuses to render without eight values. The installer generates any
that are missing when it applies, using 32 random bytes rendered as hex, and
writes them resolved into `marula.env`. The wizard does the same when you
finish it.

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

`MARULA_DB_POOL_MAX` is one instance's database pool ceiling, and 40 is an
example. Run `./marula deploy describe --json` from the release directory to
print the whole catalogue of names the installer accepts. A name outside that
catalogue is refused before anything is written, so a typo is an argument error
rather than a setting that does nothing. `--set` also accepts the four
infrastructure values the installer generates itself: both database passwords,
the object store secret and the error tracker key.

## Shapes

A shape is a promise about processes. The installer scores the host against
each shape's floors and takes one that fits, preferring `split`. The floors are
a budget for the whole box, not one process's requirement: the shipped
embedding model (which only the `ai` profile runs) is the largest line, plus
room for Postgres and the object store. An install with `ai` off has headroom
to spare.

The three shapes differ in how many processes share the box: `split` and `demo`
run the API and the worker separately, while `all-in-one` folds the worker loops
into the API process and asks for 10 GB less disk.

| Shape | What runs | Cores | Memory | Free disk |
|---|---|---|---|---|
| `split` | core-api and the worker as separate containers | 4 | 8192 MB | 40 GB |
| `all-in-one` | one core-api process, with the worker loops inside | 4 | 8192 MB | 30 GB |
| `demo` | `split` plus a seeded organisation, loopback only | 4 | 8192 MB | 30 GB |

`split` is the shipped topology: a stalled worker restarts without interrupting
the API, and the two processes scale apart. `all-in-one` trades that for one
container on a small box. `demo` is never chosen for you, because it carries a
published password and an unauthenticated reset surface, and the installer pins
its API to loopback.

Name a shape when you want it:

```sh
./install.sh --dir /srv/marula --images registry --apply --profile all-in-one
```

That `--profile` flag belongs to the installer and selects a shape. Compose has
its own `--profile` flag, which selects optional services, and the next section
is about that one. Where this page says "shape", it means the installer's flag.

A host the shape cannot carry is refused with the floor it missed. On a
platform that publishes no memory reading, declare it with `--memory-mb 16384`
or `MARULA_HOST_MEMORY_MB`, and the same host is scored again. `--allow-unfit`
plans a shape the host misses anyway. That decides whether the plan runs on a
smaller box; it does not change the flooring rules and does not promise that a
stack below them will boot.

## Compose profiles

Compose profiles add moving parts an install does not need on day one. They
stay off until you name them with `--profile <name>` on a compose command. This
is the second `--profile`, and it is unrelated to the installer's shape flag.

| Compose profile | What it adds | When to turn it on |
|---|---|---|
| `pooler` | PgBouncer in transaction mode, plus its auth renderer | Several API instances, or many clients against a Postgres that caps connections |
| `stage2` | Garage object storage and a Mailpit mail sink | Attachments on the host, or mail captured locally instead of sent |
| `observability` | OTel collector, Tempo, Loki, Prometheus with the alert pack, Grafana, GlitchTip, Uptime Kuma | You want traces, logs, metrics and a status page on the box |
| `ai` | A local Ollama runtime, and a one-shot that pulls the embedding model | You want embeddings and transcription on your own hardware |
| `backup` | WAL-G point-in-time recovery and restic snapshots | You want scheduled backups instead of a nightly dump |
| `edge` | Caddy terminating TLS | The host faces the internet |

Start one with the shell function from [Running commands against the
install](#running-commands-against-the-install):

```sh
marula_compose --profile stage2 up -d
```

The `edge` profile is the exception: it also needs its overlay and the domain
settings, so use the command in [`deploy/edge/README.md`](../deploy/edge/README.md)
rather than the one above. Every other profile works with the function as it
stands.

The pooler is opt-in because a single-node install does not need it. core-api
and the worker dial Postgres directly, and one instance's connections fit
inside the 100 a stock Postgres allows. Turn the profile on when you add a
second API instance, and set `MARULA_APP_DATABASE_URL` and
`MARULA_WORKER_APP_DATABASE_URL` to `pgbouncer:6432` with
`MARULA_DB_STATEMENT_CACHE=0` in the env file at the same time.
[`deploy/README.md`](../deploy/README.md) has the connection arithmetic and
[`docs/reference/connection-pooling.md`](reference/connection-pooling.md) has
the measurements.

### The embedding model is optional

The default embedding provider is a local Ollama model,
`embeddinggemma-2:270m-q8_0-text`, and it runs only when the `ai` compose
profile is on. A default install does not start it.

The API still boots without it. Until a model answers, the vector leg stays
dark and search falls back to full text, which the AI reference page describes
in [Local AI](site/reference/self-host-local-ai.md). Two ways to turn it on:

```sh
# Local model, no account, runs on the host.
marula_compose --profile ai up -d
```

```sh
# Cloud provider instead.
./install.sh --dir /srv/marula --images registry --apply \
  --set MARULA_EMBEDDING_PROVIDER=openai --set MARULA_EMBEDDING_API_KEY=<key>
```

The API refuses to start only when the local model you configured needs more
cores or memory than the host has. The shipped default fits inside every
shape's budget, and an install that leaves `ai` off runs no model at all, so a
default install does not meet that refusal. If you raise
`MARULA_EMBEDDING_TIER` or pin a larger model on a small box, the error names
the model, the tier, both numbers and the two levers that fix it.

## Upgrade

Upgrades move one release at a time, and Marula supports jumps from either of
the two previous tagged releases. Read the release notes before you start: they
carry a **Breaking & deprecated** section even when it is empty.

1. Back up first, even if you have no backup habit yet. The [backup
   section](#backup-and-restore) below covers the database and `marula.env` in
   two commands, and says what to do about the object store. The drill in [the
   backup runbook](site/reference/backup-restore.md) is how you find out the
   backup restores, and it is worth doing on a spare database, but it is not a
   precondition for this upgrade.
2. Download the new archive and its checksum list, and check the archive the
   same way you did the first one.
3. Unpack the new release and run its installer against the directory you
   already use:

   ```sh
   ./install.sh --dir /srv/marula --images registry --apply
   ```

   The plan reads the existing `marula.env` first, so the eight values stay put
   and the new images replace the old ones. Migrations run before the API
   starts, and the command returns when the health checks pass.

4. Run `./marula doctor --dir /srv/marula` and open the workspace.

**Upgrading from a release that defaulted to the pooler.** Earlier releases
started a connection pooler and pointed both database URLs at `pgbouncer:6432`.
If your `marula.env` still names that host, it outranks the new direct default,
and a stack started without the pooler has nothing to answer on that port. Pick
one: keep the pooled URLs and start the stack with `marula_compose --profile
pooler up -d`, or change `MARULA_APP_DATABASE_URL` and
`MARULA_WORKER_APP_DATABASE_URL` to `postgres:5432` in `marula.env` before you
run `marula_compose up -d`. One core-api instance fits inside a stock Postgres
connection ceiling, so the direct URLs are the default for a single host.

A release that publishes a signed update manifest can be applied without the
download step. `./marula update --check --manifest <file>` says whether a newer
release exists, and `./marula update --apply --manifest <file>` verifies the
signature, backs up the files the install owns, applies through the same target
and rolls the files back when the doctor disagrees. Rollback restores
configuration, not data, so the backup in step 1 is still yours to take.
[`docs/update.md`](update.md) has the signature and key flags.

## Backup and restore

Everything lives in Postgres, and once object storage is on, the attachments
live in the bucket as well. Back up both or neither: a database-only restore
leaves attachments pointing at objects that are not there.

Write the backup outside the unpacked release directory, which the next upgrade
replaces. `/var/backups/marula` is a good place on a server; use any directory
you own if you did not install with `sudo`. `INSTALL` and `marula_compose` are
the ones you defined in [Running commands against the
install](#running-commands-against-the-install).

```sh
BACKUP_DIR=/var/backups/marula
mkdir -p "$BACKUP_DIR"
cp "$INSTALL/marula.env" "$BACKUP_DIR/marula.env-$(date +%F)"
marula_compose exec -T postgres pg_dump -U marula -d marula -Fc \
  > "$BACKUP_DIR/marula-$(date +%F).dump"
```

The copy of `marula.env` is part of the backup. The eight values are in it and
nothing else reproduces them: a stack rebuilt with new signing keys signs
everyone out.

The dump command runs `pg_dump` inside the Postgres container and connects over
its local socket, which the image's `initdb` leaves on trust, so it needs no
password. A host that changed the container's local auth would need
`PGPASSWORD`.

Dump as `marula`, not `marula_app`. The app role is constrained by row-level
security and cannot produce a complete dump. Use a `pg_dump` that matches the
server's major version; the postgres image carries the right one.

To restore, stop the application containers, restore the dump into a database
of the same major version, then start them again:

```sh
marula_compose stop core-api worker
# restore the dump into a fresh database here
marula_compose up -d core-api worker
```

Restore the object store in the same window. If rows and objects come back at
different times, leave `MARULA_OBJECT_GC` off until the restore finishes, or the
integrity pass marks live attachments as missing. On `all-in-one`, stop and
start `core-api` alone, because the worker loops live inside it. [The backup and
restore runbook](site/reference/backup-restore.md) has the restore order, the
restic and WAL-G paths, and the drill that proves a restore works.

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

Hardened pulls need no account. The images are free, and a fresh host with no
credential store pulls them like any other public image. That is Docker's
policy rather than a promise from this repository, so if a pull ever asks for
credentials, `docker login dhi.io` with a free Docker account is the fallback.

To run from a mirror, edit the image references in the bundle's
`deploy/docker-compose.yml`. The installer reads that file, so your edit reaches
the next install run, whether that is the wizard or `--apply`. Keep the digests,
so a mirror cannot substitute a different image behind your back. The four
Marula images are the exception: `--images registry` writes a generated
override that re-pins them to the release's digests, so pass
`--image-base ghcr.io/your-org/marula` instead of editing those lines. The
release's own prefix comes from `release.json` when you leave the flag out, and
each service is `<base>-<service>`.

## Troubleshooting

| Symptom | What it means |
|---|---|
| `sha256sum: command not found` on macOS | Use `shasum -a 256 -c SHA256SUMS-macos-arm64`. |
| `unauthorized` while pulling `dhi.io/...` | Docker asked for credentials, which the hardened images do not normally require. `docker login dhi.io` with a free Docker account is the fix. |
| `no shipped profile fits this host` | The host misses a floor in the shape's budget and the message names it. Free disk and cores are the usual ones. On a platform with no memory reading, pass `--memory-mb`. |
| `port is already allocated` | Something holds 3000, 8080, 8082 or 15432. Stop it, or move the service. |
| The API refuses to start after an install | Only when the local model you configured needs more cores or memory than the host has. With the `ai` profile off, the shipped default fits every accepted shape. Lower `MARULA_EMBEDDING_TIER` or set `MARULA_EMBEDDING_PROVIDER=openai`. |
| `--apply` stopped partway | Read the step it named, then run `marula_compose logs --tail 100 <service>` for the service that did not come up, and `./marula doctor --dir /srv/marula`. The env file is already written, so a second `--apply` continues from the values on disk. |
| Compose rejects `!reset` | Your Compose is older than 2.24. Upgrade Docker Compose and apply again. |
| `marula.env` is lost | Restore the copy from your backup, then run `marula_compose up -d`. Without that file the eight values are gone, and a stack rebuilt with new signing keys signs everyone out. |
| `permission denied` reading `marula.env` | The install ran with `sudo` and you did not. Run the command with `sudo`, or change the file's owner once. |

`marula_compose` is the shell function from [Running commands against the
install](#running-commands-against-the-install). A completed install means every
service the shape starts answered its health check, and `marula doctor` names
the check that failed.

For a demo install, `./marula demo reset --dir "$INSTALL"` replaces the seeded
data between showings.
