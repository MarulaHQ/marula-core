# Install Marula on a machine

One engine, three ways in. The **Marula Installer** app walks five decisions
with a live preview of the config it will write. `marula install --ui` serves the
same wizard over loopback, for a machine where the app cannot be installed.
`marula install` prints the plan as text, for a terminal or a runbook. All three
measure the machine with the same function and write the same `marula.env`.

The decisions behind the shape are in ADR 0026 (the deployment target and the
plan) and ADR 0027 (the app, and the fallback). This page is the operator's side
of them.

## The app

The installer app is the primary way in. It opens on a choice that sets how much
you decide:

- **Recommended** asks seven questions. Everything else keeps its default, and
  each default states why it is the default.
- **Developer install** shows the whole surface: every parameter, grouped as the
  descriptor groups it, searchable by name or by what it changes, with what each
  one requires and what it cannot be combined with. `⌘K` reaches it from any
  screen, in either path.

Either way the flow is five screens with one decision each:

1. **How.** The two paths above, and what each one will ask.
2. **This machine.** Cores, memory, free disk at the install path, Docker, a GPU
   if one is visible — measured with the same capacity check the product runs at
   boot. The recommendation names the numbers it was chosen on.
3. **Shape.** `split` and `all-in-one` side by side, each with its floors marked
   against this host. A shape the machine cannot carry says which floor it
   misses, in that floor's own units, and the full description of a shape is one
   disclosure away rather than three lines in the reading path.
4. **Settings.** The seven decisions, or the whole catalogue. A field shows the
   default and the reason the default was chosen. A changed field shows what
   changes because of it.
5. **Review.** The plan as steps, each with the reason it exists and the
   descriptor field that justifies it. Then the config it writes, grouped by what
   it configures, with the technical lever name under each label. Then what stays
   yours to do, the apply, and the doctor's verdict.

The app has no telemetry and sends nothing off the machine. It reads the
descriptor and the host, and it works with the network down once the images are
pulled. Its window is 1440×900 by default and lays out from 1024 pixels wide.

It installs into `~/marula`. To install elsewhere, set `MARULA_INSTALL_DIR`
before launching it. The review screen always names the directory it will write
to.

## The browser fallback

`marula install --ui` serves the same wizard from loopback, for a server with no
desktop, a workstation where installing an app is not allowed, or a box reached
over SSH:

```sh
marula install --ui                  # prints one URL, opens a browser
marula install --ui --no-open        # for a headless host: print the URL only
marula install --ui --port 8899      # a fixed port, when the printer is shared
```

The URL carries a one-time token. Opening it once sets an `HttpOnly`,
`SameSite=Strict` cookie, and the token stops working. The session lasts until
the process stops. The server binds `127.0.0.1` only, refuses a `Host` header
that is not loopback, and requires the session cookie plus an installer header
on every API call. If no browser can be opened, it says so and keeps serving, and
the URL in the terminal is the way in.

`--ui-dir` points at the built front-end, which the release bundle ships beside
the binary. One directory, two hosts: the app loads the same files.

## What it will do for you

```sh
marula install --dir /srv/marula                 # print the plan, write nothing
marula install --dir /srv/marula --apply         # write the env file, start the stack
marula doctor --dir /srv/marula                  # check the result
```

The plan names every step, the reason it exists, and the descriptor field that
justifies it. It also names what stays manual, because an install that silently
leaves the backup target unset is the install nobody finishes: the TLS
certificate and hostname, the backup destination, an alert receiver.

`--apply` writes one file, `marula.env`, and runs
`docker compose --env-file … -f … up -d --wait`, adding the profile's override
file when it has one (`all-in-one` runs `docker-compose.all-in-one.yml` over the
base file, because that shape is one process rather than two). It returns when the profile's
healthchecks pass, so "the command finished" and "the install works" are the
same event. The compose file and the pooler config come from the release, and
this target supplies the environment that binds them. Where the application
images come from is a decision, and the next section is the honest version of
it.

Each line of `marula.env` sits under the sentence that explains it, taken from
the descriptor. Nothing is restated from memory and nothing is invented:

```sh
# One container instead of two: the background loops share core-api's memory
# and restart with it, so a fault split would have contained takes the API
# down too.
MARULA_ALL_IN_ONE=1
```

Only the profile's presets, your overrides and the four values the process
refuses to boot without are written. Every other lever keeps the descriptor's
default, so the file stays a deployment description instead of a transcript of
one release.

## Where the images come from, and what restarts

Two decisions are not environment variables, so they are not in `marula.env`.
They change the compose invocation, and the plan writes each as a generated file
beside the env so the install says what it did.

**`--images build|registry`.** The default, `build`, compiles this checkout: the
compose file's four application services carry a `build:` context, so a first
install needs the source, a Rust toolchain in the image build, and a Node
toolchain for the web bundle. It is the shape a developer install uses and the
only shape that works before a registry has published anything.

`--images registry` stops Compose from building and pins each application image
to the release's digest:

```yaml
services:
  core-api:
    image: ghcr.io/marulagh/marula-core-api@sha256:…
    pull_policy: always
    build: !reset null
```

`build: !reset null` is what removes the base file's `build:`. Without it Compose
builds the service and ignores the image. `!reset` needs Compose 2.24 or newer.
A registry install refuses to plan without a digest for every service it starts
(`--image-digest core-api=sha256:…`), because a tag can be moved and a digest is
what the release signed.

Third-party images are already pinned by digest in the compose file, so they are
pulled either way.

**`--restart unless-stopped|always|no`.** The default follows Docker's usual
posture: a service comes back after a crash and after the daemon restarts, and a
deliberate stop stays stopped. `always` also comes back after a deliberate stop,
which surprises people. `no` leaves the failure in front of you, which is right
on a box you are working on and wrong on one that serves a team. The plan writes
`marula.restart.yml` for the long-running services only. A one-shot
(`migrate`, `app-role-password`) is never given a policy, because restarting it
repeats work or fails forever.

None of these policies starts anything at boot on its own. A container only
comes back with the machine if the Docker daemon starts with the machine, which
is a setting of the host's service manager and not of this installer.

## The eight values it generates

The compose stack refuses to render without eight values. Four are parameters
the descriptor describes (the two signing keys and the two seam tokens); four are
infrastructure the descriptor does not (the owner and app database passwords, the
object store's RPC secret and the error tracker's key).

`--apply` resolves all eight in this order: your `--set` value, then what an
existing `marula.env` already holds, then a freshly generated 32-byte hex value.
The order matters. A re-run must not rotate a signing key, which signs everyone
out, or the database password the running volume was initialised with. The same
file is therefore safe to apply twice.

The values are written resolved, never as templates. Compose expands variables in
the YAML but not in `--env-file` values, so a `postgres://…:${MARULA_APP_PASSWORD}@…`
left in the file would reach Postgres as a literal password.

`--dry-run` prints the whole file with every secret masked as `****`, including
where one appears inside a URL. A key in a scrollback or a CI log is a leaked
key.

## Choosing a profile

The installer scores the host against each profile's floors and picks the
highest one it meets, preferring `split` because it is the shipped topology. A
host that misses a floor is told which one, in its own units:

```text
no shipped profile fits this host. What each one needs beyond it:
  split: 8192 MB memory (this platform publishes no reading)
```

On a platform that publishes no memory reading, the refusal names the fix:
pass `--memory-mb`, or set `MARULA_HOST_MEMORY_MB`, and the same host is scored
again. The value is the operator's declaration, not a guess by the installer.

If the profile's local embedding model does not fit the host, the plan says so
and names the cloud path: set `MARULA_EMBEDDING_PROVIDER=openai` and
`MARULA_EMBEDDING_API_KEY`. The installer will not quietly pick a smaller local
model, because that trades recall for memory without telling you.

## Overrides

`--set NAME=value` changes one lever, repeatable:

```sh
marula install --dir /srv/marula --set MARULA_DB_POOL_MAX=40 --set EMAIL_TRANSPORT=smtp
```

A name the descriptor does not publish is refused before anything is written.
That is the same catalogue `marula deploy describe --json` emits, so a typo is
an argument error rather than a setting that silently does nothing.

## What it does not do

No artifact packaging and no updates (W60-4 and W60-5 own those). No uninstall.
It does not create the TLS certificate, the backup target or the alert receiver,
and it does not start a second machine.

## Checking an install

`marula doctor` reads `marula.env` when it exists and the process environment
otherwise, so it answers for an install it did not create. Each check says what
it looked at and what it found:

| Check | Fails when |
|---|---|
| host floors | the machine has less than the profile needs |
| docker | the daemon does not answer |
| required values | one of the four boot-critical values is empty |
| port | something already holds a port the profile publishes |
| embedding model | the profile's local model needs more memory than the host reports |

It exits non-zero when any check fails, and it ends with the compose services it
can see.
