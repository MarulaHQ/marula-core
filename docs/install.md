# Install Marula on a machine

This page takes a release bundle to a running stack on a server. The short way
is one line; the rest of the page is the same install by hand, then the eight
secrets, the shapes, upgrades and backups.

## One line

On Linux or Apple silicon:

```sh
curl -fsSL https://github.com/MarulaHQ/marula-core/releases/latest/download/install.sh \
  -o /tmp/marula-install.sh && sh /tmp/marula-install.sh
```

That is `scripts/release/bootstrap-install.sh`, attached to every release. It
works out your platform, downloads that archive, verifies the signature over its
checksum list, verifies the archive, unpacks it, and runs the bundle's
`./install.sh` with whatever you passed after `--`. With no arguments that means
the wizard, into `~/marula`.

The script is written to a file first so that a failed download cannot look like
a successful no-op: `curl … | sh` would hand an empty script to `sh` on a 404 and
exit zero.

```sh
# a server: print the plan, then apply
sh /tmp/marula-install.sh --dir /srv/marula
sh /tmp/marula-install.sh --dir /srv/marula --apply
# a specific version, or the air-gapped bundle
sh /tmp/marula-install.sh --version 0.6.1
sh /tmp/marula-install.sh --offline
```

On a shared machine, download into `$(mktemp)` rather than a fixed name:

```sh
T=$(mktemp)
curl -fsSL https://github.com/MarulaHQ/marula-core/releases/latest/download/install.sh -o "$T" \
  && sh "$T" --dir /srv/marula --apply
```

The script in `releases/download/vX.Y.Z/` is byte-identical to the one under
`releases/latest/`. It still installs the newest release unless you pass
`--version X.Y.Z`, because the asset name carries no version for it to read.

**Where trust comes from.** The script has no signature of its own: it arrives
over HTTPS from GitHub, the same channel as every other asset on the release
page. What it then does is different — it checks the *archive* against a
signature from the release key, and that key is pinned inside the script. So
replacing the script would also replace the key, which is why the honest
statement is "trusted as a download from this repository, and it verifies what it
fetches". Read it before you run it; it is about 8 KB at
[`scripts/release/bootstrap-install.sh`](../scripts/release/bootstrap-install.sh).

It needs `curl`, `tar` and `ssh-keygen`, plus `zstd` for an air-gapped bundle.
It refuses to install when it cannot check the signature, because a missing tool
is not a reason to trust a download. The bundle it unpacks stays on the machine at
`~/.local/share/marula/marula-<version>-<os>-<arch>`, which is the directory the
rest of this page means by "the unpacked release directory". Override it with
`MARULA_BUNDLE_DIR`.

### By hand

Prefer the archive? Each platform has one, with its own signed checksum list.

| Platform | Archive | Install |
|---|---|---|
| Linux x86_64 | `marula-<version>-linux-x64.tar.gz` | `./install.sh` |
| Linux arm64 | `marula-<version>-linux-arm64.tar.gz` | `./install.sh` |
| macOS, Apple silicon | `marula-<version>-macos-arm64.tar.gz` | `./install.sh` |
| Linux x86_64, no network | `marula-<version>-offline-linux-x64.tar.zst` | `./install-offline.sh` |
| Linux arm64, no network | `marula-<version>-offline-linux-arm64.tar.zst` | `./install-offline.sh` |

Unpack with `tar -xzf` (`tar --zstd -xf` for an air-gapped bundle), enter the
directory it makes, and run the command. On macOS check the download with
`shasum -a 256 -c SHA256SUMS-macos-arm64`; on Linux, `sha256sum -c`.

Windows is not an install target yet. The release carries a `windows-x64`
archive with `marula.exe` and the desktop setup inside it, but the bundle's
`install.sh` looks for an executable named `marula` and needs a POSIX shell, so
neither WSL nor Git Bash runs it as it stands.

## What you need

- A Linux host, x86_64 or arm64. macOS on Apple silicon runs the stack too, and
  the release publishes a `macos-arm64` bundle for it.
- Docker with Compose 2.24 or newer. `docker compose version` prints what you
  have. The installer checks that the Compose plugin answers, not which version
  it is. The registry override uses Compose's `!reset` key to remove the base
  file's build instructions, and Compose older than 2.24 refuses the file with
  an unknown-tag error rather than building. The troubleshooting table at the
  end names it.
- A budget for the whole box, not one process: four cores, 8 GB of memory and
  40 GB of free disk for the default shape, or 30 GB of disk on `all-in-one`.
  The largest line in that budget is the shipped embedding model, which the
  `ai` compose profile runs, plus room for Postgres and the object store. Leave
  `ai` off and the install has headroom to spare.
- These ports free on the host. The base compose file publishes every one of
  them, and the interface depends on the port: 3000, 8080, 8082 and 15432 on
  every interface, 16379 on loopback only.
  - `3000`: the workspace and the client portal.
  - `8080`: the API and its `/docs` reference.
  - `8082`: the worker's health endpoint. `split` and `demo` serve it from the
    worker container, and `all-in-one` serves it from the API process.
  - `15432`: Postgres, published for local tooling. The application reaches
    Postgres by service name on the internal network and does not need this
    port. Because the base file maps it on every interface, it is a database
    reachable from anywhere that can reach the host. The `edge` profile is the
    supported way to remove that mapping.
  - `16379`: Valkey, mapped to `127.0.0.1`, so only the host can reach it. It is
    there to inspect the cache, not to serve anything.
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

The CLI takes `--images build|registry`, and its default is `build`, which
compiles the four application images from a source checkout. A release bundle
carries no sources, so for a bundle the answer is always `registry`.
`deploy/install.sh` adds that flag itself when its `release.json` sits beside
it, so a script install needs nothing.

That matters only if you call `./marula install` directly: omit the flag there
and the plan still prints, because nothing looks for sources at plan time, and
`--apply` fails when Compose tries to build from the workspace root the bundle
does not carry. The wizard asks the same question and offers the published
images.

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

`--apply` writes `marula.env`, starts the stack, and returns when `docker compose
up --wait` sees it started. Only Postgres, Valkey and Garage define a
healthcheck: the application images carry no shell for one to run in, so core-api,
the worker and the web app have none on purpose. A zero exit therefore means the
containers started, not that the API is serving. `marula doctor` is the check for
that, and it names the line that failed. `--apply` also does not mean TLS, mail,
backups or alerting are configured; the plan's "still yours to do" list stays
yours.

If you used the wizard, it has already applied and the install is in
`~/marula`. The doctor takes the directory, so give it the one you have:

```sh
./marula doctor --dir "$HOME/marula"     # wizard install
./marula doctor --dir /srv/marula        # plan-path install
```

The doctor reads `marula.env` and checks the host, Docker, the required values,
the ports and the embedding model. It exits non-zero when a check fails. On a
box you know is undersized, `--allow-unfit` waives the host-floors check and
prints `waived by --allow-unfit` with the floors it missed, and the summary
counts it as waived rather than passed. Every other check still fails on its
own.

## What you get

| URL | What it is |
|---|---|
| http://<host>:3000 | The workspace: tickets, dispatch, projects, billing |
| http://<host>:3000/portal | The client portal |
| http://<host>:8080 | `core-api`, the only writer to the database |
| http://<host>:8080/docs | The API reference, generated from the OpenAPI document |
| http://<host>:8082/healthz | The worker's health endpoint. `all-in-one` serves it from the API process |

Open `/login`, choose **Setting up the first organization?** at the bottom of the
sign-in card, and submit **Create organization** to make your organisation and
the first owner account. [Bootstrap your first
organisation](site/how-to/admin-org-bootstrap.md) walks that screen and the
settings worth changing before your team logs in.

Nothing here terminates TLS, and the base compose file publishes 3000, 8080,
8082 and 15432 on every interface. The installer does not change that, so after
`--apply` anyone who can reach the host can reach the workspace and the API. Put
a proxy in front before you open the host to the internet: the `edge` compose
profile removes those four mappings and serves 80 and 443 through Caddy with one
of three certificate modes, and
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
`-f deploy/docker-compose.demo.yml`. A `demo` install also needs
`--profile demo` there, because the demo seeder and its two setup one-shots are
behind that profile and Compose refuses the project without it. The `-f` and
`--profile` flags have to come before the subcommand, which is why they live in
the function rather than on each call.

The compose file sets the project name to `marula`, so the containers are
`marula-core-api-1` and so on. Run this as the user that ran the install. If the
install used `sudo`, put `sudo` before `docker` inside the function
(`sudo docker compose …`); `sudo marula_compose` cannot work, because sudo runs
a command and a shell function is not one.

On `all-in-one`, always name the services. A bare `marula_compose up -d` starts
the base file's separate worker too, and it fights core-api for port 8082.
`marula_compose up -d core-api web` starts the stack itself, and a profile
command names the services that profile adds, as the examples below do.

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
print the catalogue of application levers. `--set` also accepts four
infrastructure values the installer generates itself, which do not appear there:
both database passwords, the object store secret and the error tracker key.
Anything outside both sets is refused before anything is written, so a typo is
an argument error rather than a setting that does nothing.

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

`demo` also builds its seeder from a source checkout: `demo-seed` has a `build:`
and no published image, so a release bundle cannot run this shape. On a bundle,
use `split` or `all-in-one` until a seed image is published.

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
| `stage2` | Garage object storage, a Mailpit mail sink and the ClamAV scanner | Attachments on the host, mail captured locally instead of sent, or uploaded files scanned |
| `observability` | OTel collector, Tempo, Loki, Prometheus with the alert pack, Grafana, GlitchTip, Uptime Kuma | You want traces, logs, metrics and a status page on the box |
| `ai` | A local Ollama runtime, and a one-shot that pulls the embedding model | You want embeddings and transcription on your own hardware |
| `backup` | WAL-G point-in-time recovery and restic snapshots | You want scheduled backups instead of a nightly dump |
| `edge` | Caddy terminating TLS | The host faces the internet |

Start one by naming the services it adds, with the shell function from
[Running commands against the
install](#running-commands-against-the-install):

```sh
marula_compose --profile stage2 up -d garage mailpit
```

Naming them is what keeps the command safe on `all-in-one`, where starting the
whole file would bring up the separate worker as well.

The `edge` profile is the exception: it also needs its overlay and the domain
settings, so use the command in [`deploy/edge/README.md`](../deploy/edge/README.md)
rather than the one above. Every other profile works with the function as it
stands.

The pooler is opt-in because a single-node install does not need it. core-api
and the worker dial Postgres directly, and one instance's connections fit
inside the 100 a stock Postgres allows. Turn the profile on when you add a
second API instance, and in the env file point the host inside
`MARULA_APP_DATABASE_URL` and `MARULA_WORKER_APP_DATABASE_URL` at
`pgbouncer:6432`, with `MARULA_DB_STATEMENT_CACHE=0`, at the same time. This is a
`split` setting. Both overlays set the database URLs themselves and point them
at Postgres, and a service-level value wins over the env file: on those shapes
the profile starts a pooler that nothing dials until those values are overridden
too. The `all-in-one` overlay pins `DATABASE_URL` and
`MARULA_WORKER_DATABASE_URL`; the `demo` overlay pins `DATABASE_URL` and
`MARULA_LISTEN_DATABASE_URL` on `core-api` and on the separate `worker`.
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
marula_compose --profile ai up -d ollama ollama-model
```

```sh
# Cloud provider instead.
./install.sh --dir /srv/marula --images registry --apply \
  --set MARULA_EMBEDDING_PROVIDER=openai --set MARULA_EMBEDDING_API_KEY=<key>
```

The API refuses to start if the local model you configured needs more cores or
memory than the host has. The shipped default fits inside every
shape's budget, and an install that leaves `ai` off runs no model at all, so a
default install does not meet that refusal. If you raise
`MARULA_EMBEDDING_TIER` or pin a larger model on a small box, the error names
the model, the tier, both numbers and the two levers that fix it.

## Upgrade

Upgrades move one release at a time, and Marula supports jumps from either of
the two previous tagged releases. Read the release notes before you start: they
carry a **Breaking & deprecated** section even when it is empty.

Two things about the install you already run are settled before step 3.

**Name the shape.** Without `--profile`, the installer recommends again from
this host, and it does not read the shape back from `marula.env`: an
`all-in-one` install can come back as `split`. Pass the shape you have:
`--profile split`, `--profile all-in-one`, or `--profile demo`.

**Settle a pooled install.** If `MARULA_APP_DATABASE_URL` or
`MARULA_WORKER_APP_DATABASE_URL` in `marula.env` points at `pgbouncer:6432`,
decide now. Either start the pooler on the running stack so it is up when the
new services are created (`marula_compose --profile pooler up -d pgbouncer`),
or change the host in both URLs to `postgres:5432`
(removing the two keys has the same effect) to take the direct default. Pooled
URLs without the profile are the one combination that starts a stack the
application cannot use: core-api exits instead of serving. A pooled install is a
`split` install, because the `all-in-one` and `demo` overlays set the database
URLs themselves.

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:e435277eb399ef50d98f2c0d7a2aabd380393acd9c79a5bbdd99eb804586f97b -->

1. Back up first, even if you have no backup habit yet. The [backup
   section](#backup-and-restore) below covers the database, `marula.env` and,
   through the runbook, the object store. When object storage is enabled, back
   up attachments in the same window as the database dump, following the
   [object-store backup procedure](site/reference/backup-restore.md#backup).
   The drill in [the backup
   runbook](site/reference/backup-restore.md) is how you find out the backup
   restores, and it is worth doing on a spare database, but it is not a
   precondition for this upgrade.

<!-- /coderabbit-ai-generated:v1 -->
2. Download the new archive and its checksum list, and check the archive the
   same way you did the first one.
3. Unpack the new release and run its installer against the directory you
   already use:

   ```sh
   ./install.sh --dir /srv/marula --images registry --profile split --apply
   ```

   `split` is the default shape; pass the profile you named above when it is
   `all-in-one` or `demo`. The plan reads the existing `marula.env` first, so
   the eight values stay put and the new images replace the old ones.
   Migrations run before the API starts, and the command returns when the
   containers are up.

4. Run `./marula doctor --dir /srv/marula` and open the workspace.

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
leaves attachments pointing at objects that are not there. The commands below
take the database and `marula.env`. The object store needs its own copy. The
`backup` profile's restic leg covers the local object store (`objectdata`) and
the nightly dump, and it mounts neither `garagedata` nor `garagemeta`, so a
deploy running Garage copies those two volumes per
[the runbook](site/reference/backup-restore.md#backup).

Write the backup outside the unpacked release directory, which the next upgrade
replaces. `/var/backups/marula` is a good place on a server. `INSTALL` and
`marula_compose` are the ones you defined in [Running commands against the
install](#running-commands-against-the-install).

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:35b49fc579fe3176b15ae8339513bdce5817c0c5db5edc94a5da6c24594fada2 -->

```sh
(
  BACKUP_DIR=/var/backups/marula
  BACKUP_DATE=$(date +%F)
  umask 077
  mkdir -p "$BACKUP_DIR" \
    && chmod 700 "$BACKUP_DIR" \
    && BACKUP_TMP=$(mktemp -d "$BACKUP_DIR/.partial.XXXXXX") \
    || exit 1
  trap 'rm -rf "$BACKUP_TMP"' 0
  trap 'exit 1' 1 2 15
  cp "$INSTALL/marula.env" "$BACKUP_TMP/marula.env" \
    && marula_compose exec -T postgres pg_dump -U marula -d marula -Fc \
    > "$BACKUP_TMP/marula.dump" \
    && mv "$BACKUP_TMP/marula.dump" "$BACKUP_DIR/marula-$BACKUP_DATE.dump" \
    && mv "$BACKUP_TMP/marula.env" "$BACKUP_DIR/marula.env-$BACKUP_DATE"
)
```

`umask 077` restricts newly created files, and `chmod 700` restricts the backup
directory even if it already exists. Both files are staged in a unique temporary
directory created by `mktemp -d`. The subshell removes that directory on exit,
including when a step fails. Neither file replaces a complete copy until
`pg_dump` exits zero: a failed dump leaves the previous dump and its env copy
in place, and a rerun on the same day replaces both.


<!-- /coderabbit-ai-generated:v1 -->
The copy of `marula.env` is part of the backup. The eight values are in it and
nothing else reproduces them: a stack rebuilt with new signing keys signs
everyone out.

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:93a4593e44b87dae29d2a06c7abe0b7929268604c7d6ab072bf7d71cbfb4e203 -->

If the install ran with `sudo`, run the whole block in a root shell, with
`INSTALL` and `marula_compose` defined there (see above). A non-root shell cannot
write `/var/backups/marula` or read a root-owned `marula.env`.

<!-- /coderabbit-ai-generated:v1 -->

The dump command runs `pg_dump` inside the Postgres container and connects over
its local socket, which the image's `initdb` leaves on trust, so it needs no
password. A host that changed the container's local auth would need
`PGPASSWORD`.

Dump as `marula`, not `marula_app`. The app role is constrained by row-level
security and cannot produce a complete dump. Use a `pg_dump` that matches the
server's major version; the postgres image carries the right one.

To restore, stop the application containers and start them again afterwards.
The restore in between is not a one-liner and this page does not cover it:
[the backup and restore runbook](site/reference/backup-restore.md) is required
reading at this step, because it gives the order (database, then object store,
then services) and the rehearsal that restores the artifact into a scratch
database before you touch live data.

```sh
marula_compose stop core-api worker        # all-in-one: core-api only
# restore here: runbook, Restore section, steps 2 to 4
marula_compose up -d core-api worker       # all-in-one: core-api only
```

Restore the object store in the same window. If rows and objects come back at
different times, leave `MARULA_OBJECT_GC` off until the restore finishes, or the
integrity pass marks live attachments as missing.

## Hardening

The stack is built to run without root and without a writable filesystem.

- Every third-party slot that has a hardened equivalent uses a Docker Hardened
  Image: Postgres with pgvector, Valkey, PgBouncer, Grafana, Tempo, Loki and
  ClamAV. Garage, Mailpit, GlitchTip, Uptime Kuma, Prometheus, Ollama and the
  OpenTelemetry collector run upstream images under the same pinning rules.
- Every image reference carries a registry digest, and every one that also
  carries a tag moves tag and digest together. The digest is what runs, and the
  release's manifest names the digests it published.
- `cap_drop: [ALL]` and `no-new-privileges` apply to every service except four
  one-shot helpers: the AI profile's `ollama-model`, which pulls the model, and
  the demo overlay's `object-init`, `db-grants` and `demo-seed`, which prepare
  the demo volume, its grants and its data. Most services mount a read-only root
  filesystem and write only to their own volume or a tmpfs. Five long-running
  services keep a writable root: Postgres, Grafana and ClamAV because their own
  boot logic needs one, and `ollama` and `uptime-kuma` because each writes state
  outside its data volume, which their compose comments record as a known gap.
- The application images drop to a fixed numeric user at build time. Root
  appears in one-shots whose job needs it: Postgres' first-boot init, which
  re-execs as `postgres` before it accepts a connection, `pooler-auth`, which
  writes the pooler's auth file into a path only root can create, and the demo
  overlay's `object-init`, which creates and chowns the object volume. Nothing
  long-running runs as root.

Hardened pulls need no account. The images are free, and a fresh host with no
credential store pulls them like any other public image. That is Docker's
policy rather than a promise from this repository, so if a pull ever asks for
credentials, `docker login dhi.io` with a free Docker account is the fallback.

To run from a mirror, start with the four Marula images: pass
`--image-base ghcr.io/your-org/marula`, and the installer pulls them from your
prefix with the release's digests. The release's own prefix comes from
`release.json` when you leave the flag out, and each service is
`<base>-<service>`. The hardened third-party images are the other half, and they
live in the bundle's `deploy/docker-compose.yml`. Editing that file changes the
tree its `SHA256SUMS` covers, so `./install.sh` refuses the next run. If you edit
it, run the bundled `./marula install --compose-file deploy/docker-compose.yml
--images registry …` directly instead of `install.sh`, and keep the digests so a
mirror cannot substitute a different image.

## Troubleshooting

| Symptom | What it means |
|---|---|
| `sha256sum: command not found` on macOS | Use `shasum -a 256 -c SHA256SUMS-macos-arm64`. |
| `unauthorized` while pulling `dhi.io/...` | Docker asked for credentials, which the hardened images do not normally require. `docker login dhi.io` with a free Docker account is the fix. |
| `no shipped profile fits this host` | The host misses a floor in the shape's budget and the message names it. Free disk and cores are the usual ones. On a platform with no memory reading, pass `--memory-mb`. |
| `port is already allocated` | Something holds 3000, 8080, 8082, 15432 or 16379. Stop it, or move the service. |
| The API refuses to start after an install | Check the error message in `marula_compose logs --tail 100 core-api`. The API may be unable to reach Postgres, or the local model you configured may need more cores or memory than the host has. With the `ai` profile off, the shipped default fits every accepted shape. For the model case, lower `MARULA_EMBEDDING_TIER` or set `MARULA_EMBEDDING_PROVIDER=openai`. |
| `--apply` stopped partway | Read the step it named, then run `marula_compose logs --tail 100 <service>` for the service that did not come up, and `./marula doctor --dir /srv/marula`. The env file is already written, so a second `--apply` continues from the values on disk. |
| Compose rejects `!reset` | Your Compose is older than 2.24. Upgrade Docker Compose and apply again. |
| `marula.env` is lost | Restore the copy from your backup, then run `marula_compose up -d` (on `all-in-one`, `marula_compose up -d core-api web`). Without that file the eight values are gone, and a stack rebuilt with new signing keys signs everyone out. |
| `permission denied` reading `marula.env` | The install ran with `sudo` and you did not. Run the command with `sudo`, or change the file's owner once. |

`marula_compose` is the shell function from [Running commands against the
install](#running-commands-against-the-install). A completed install means every
service the shape starts is running, and `marula doctor` names the line that
failed.

For a demo install, `./marula demo reset --dir "$INSTALL"` replaces the seeded
data between showings.
