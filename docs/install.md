# Install Marula on a machine

Two verbs, one source of truth. `marula install` reads the machine, picks a
profile from the descriptor's floors, and prints a plan. `marula doctor` reads
the same facts back as a verdict. Neither writes anything until you pass
`--apply`, and `--apply` runs exactly the steps the plan printed.

The decisions behind the shape are in ADR 0026. This page is the operator's
side of them.

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
same event. The compose file, the pooler config and the images come from the
release; this target only supplies the environment that binds them.

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

No interactive wizard (that is W60-3, over this same data). No artifact
packaging, no updates, no uninstall. It does not create the TLS certificate, the
backup target or the alert receiver, and it does not start a second machine.

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
