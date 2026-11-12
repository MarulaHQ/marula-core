---
description: "Install Marula from a terminal: read the plan first, choose a shape the machine can carry, apply it, and check the result with the doctor."
---

# Install Marula on a server from the terminal

> **Audience:** site operator — you own the box Marula will run on, and you
> reach it over SSH rather than through a desktop.

`marula install` prints a plan and writes nothing. That is the whole design: you
read what it intends to do to the machine, then you tell it to do it. The same
command carries the machine's measurements, so the plan is about your server and
not about an ideal one.

If you would rather click through the decisions, [install with the app](install-with-the-app.md)
runs the same engine behind a window. Both of them end at the same
`marula.env` and the same Compose invocation.

## What the machine needs first

- Docker with Compose 2.24 or newer. A registry install uses `!reset` in an
  override file, and older Compose ignores it.
- A directory for the deployment's files. Anywhere works; `/srv/marula` is a
  common choice and the rest of this page uses it.
- Free disk for the shape you pick. See [deployment shapes](../reference/deployment-shapes.md)
  before you buy more: the floors are real and the installer checks them.

Nothing else has to be installed. The binary carries the descriptor, which is
the catalogue of every setting, so the plan works with the network down and no
checkout on disk.

## Read the plan before you run it

```sh
marula install --dir /srv/marula
```

The output has three parts. The host facts come first — cores, memory, free
disk, whether Docker answers. Then each step, with the reason it exists and the
part of the descriptor that justifies it. Then the values it stays manual about,
because a server missing a backup destination is a server nobody finished.

No profile is named, so the installer scores this host against each shape and
takes the highest one it meets, preferring `split`. Naming one is an override:

```sh
marula install --dir /srv/marula --profile all-in-one
marula install --dir /srv/demo   --profile demo       # the seeded demo tier
```

`demo` is the one shape never chosen for you: it carries a published sign-in and
an unauthenticated reset surface pinned to loopback, so it is picked by name. See
[deployment shapes](../reference/deployment-shapes.md#demo).

A shape the machine cannot carry is refused with the number it misses, in that
number's own units:

```text
Error: profile split needs more machine than this host has: 40 GB free disk (has 36)
```

Some platforms publish no memory reading. That is treated as "cannot prove it
fits" rather than "probably fine", so the plan refuses and names the way out:
declare the value with `--memory-mb 16384`, or set `MARULA_HOST_MEMORY_MB`, and
the same host is scored again.

## Apply it

```sh
marula install --dir /srv/marula --apply
marula demo reset --dir /srv/demo     # replace the demo data between showings
```

`--apply` writes one environment file and runs Compose with it, adding the
shape's override file and a restart file it generates beside it. It returns when
the health checks pass, so a successful exit means the stack is answering rather
than merely started.

Two things are worth knowing about the file it writes:

- Every value sits under the sentence from the descriptor that explains it. The
  file is a description of your deployment, not a transcript of a release.
- Eight values have no safe default and are generated: the two signing keys, the
  two seam tokens, the owner and app database passwords, the object store secret
  and the error tracker's key. Re-running keeps them. Rotating a signing key
  signs everyone out, and rotating the database password under a live volume
  breaks the next migration, so `--apply` is safe to repeat.

If an earlier install left a Postgres volume behind, the plan stops and says so
instead of generating a second password that the volume will refuse:

```text
Error: the docker volume marula_pgdata already holds an owner password from an
earlier install, and this plan would generate a new one.
```

Either remove the volume if its data is disposable, or pass the password it
holds with `--set POSTGRES_PASSWORD=<value>`.

## Pull published images instead of building

By default the plan compiles the four services from the checkout, which needs
Rust, Node and the sources on the target. A released bundle ships a
`release.json` that pins each service by digest instead:

```sh
marula install --dir /srv/marula --images registry \
  --image-base ghcr.io/marulagh/marula \
  --image-manifest ./release.json
```

`--image-digest core-api=sha256:…` pins one service by hand and wins over the
manifest. A registry install refuses to plan without a digest for every service
it starts, because a tag can move and a digest is what the release signed. The
manifest beside `--dir` is found without the flag, so a bundle usually needs
only `--images registry`.

## Decide what happens when a service dies

```sh
marula install --dir /srv/marula --restart always
```

The default, `unless-stopped`, brings a service back after a crash and after the
Docker daemon restarts, and leaves a deliberate `compose stop` stopped. `always`
comes back even after that, which surprises people. `no` leaves the failure in
front of you, which is right on a box you are working on. One-shots never get a
policy: restarting a migration repeats work.

None of these starts anything at boot by itself. A container only comes back with
the machine if the Docker daemon does, and that is a setting of your service
manager.

## Change a setting

```sh
marula install --dir /srv/marula --set MARULA_DB_POOL_MAX=40 --set EMAIL_TRANSPORT=smtp
```

A name the descriptor does not publish is refused before anything is written, so
a typo is an argument error rather than a setting that silently does nothing.
`marula deploy describe --json` is the same catalogue, if you want to read it
before choosing. Every lever, its default and what moves when you change it are
in the descriptor, and [the settings screen](install-with-the-app.md#the-settings-screen)
explains how to read one.

## Check the result

```sh
marula doctor --dir /srv/marula
```

The doctor reads `marula.env` when it exists and the process environment
otherwise, so it also answers for an install it did not create. It checks the
host floors, the Docker daemon, the four boot-critical values, the ports the
shape publishes and whether the embedding model fits. It exits non-zero when a
check fails and ends with the Compose services it can see.

## If it will not plan

| Symptom | What it means |
|---|---|
| `no shipped profile fits this host` | every shape misses a floor; the message lists each one |
| `this platform publishes no reading` | declare memory with `--memory-mb` and score the host again |
| `the Docker server did not answer` | the daemon is down, or your user cannot reach the socket |
| `already holds an owner password` | a volume from an earlier install; see above |
| `refuses to plan without a digest` | a registry install with a service the release did not pin |

A developer installing on a machine that is genuinely too small can plan past
the floors with `--allow-unfit`. The plan runs and repeats the numbers it misses.
Nothing in it prevents the stack failing later, which is why it is not the
default and not offered in the app's recommended mode.

The server is half the story. The desktop client on each workstation is the
other half, and it points back at this install at first run:
[install the desktop app](install-the-desktop-app.md). Not sure which download
each machine wants? [Which installer do I want?](choose-your-installer.md)
sorts the release page in one table.
