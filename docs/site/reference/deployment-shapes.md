---
description: "The two shapes Marula ships: what each one runs, the floors it asks of a host, and how to choose when the machine is close to the line."
---

# Deployment shapes

> **Audience:** site operator — you are deciding how many processes Marula runs
> on the box, and you own the disk that decision costs.

A shape is a promise about processes. Marula ships two, and they run the same
product: the same API, the same worker loops, the same database. What differs is
how much of it shares one container, and therefore what a fault takes down with
it.

Both shapes are described in the descriptor that ships in the installer, so the
numbers below are the same numbers the plan checks.

## The two shapes

| | `split` | `all-in-one` | `demo` |
|---|---|---|---|
| Application processes | core-api and worker, separate | one core-api process, worker loops inside | core-api and worker, separate |
| Containers | 7 | 5 | 9 |
| Cores | 4 | 4 | 4 |
| Memory | 8192 MB | 8192 MB | 8192 MB |
| Free disk | 40 GB | 30 GB | 30 GB |
| Ports | 8080 (API), 8082 (worker health) | 8080 and 8082 on the one process | 8080 and 8082, **loopback only** |

Containers counts the whole stack: the application, Postgres, Valkey, the
migration one-shot and the web console. `split` adds a worker and PgBouncer.

## split

The default. core-api serves the API and the live-push stream; the worker runs
the background loops in its own container. PgBouncer in transaction mode sits
between them and Postgres, so a burst of API requests queues at the pooler
instead of exhausting the database's connections.

Pick it because a stalled worker should restart without interrupting the API
somebody is talking to, and because the two processes then scale separately.
Pay for it with a second copy of the application image and a second set of
resource limits.

The disk floor is 10 GB higher than `all-in-one` for exactly that reason.

## all-in-one

One application container. The background loops live inside core-api, which
means a fault in a loop takes the API down with it. It exists for a demo box, a
laptop, or a single small server where a second container is more moving parts
than the team can justify.

It sets eleven values that `split` leaves alone: the pooler is out of the path,
so the API talks to Postgres directly and the statement cache has to be off, and
the worker's health endpoint moves into the API process.

## demo

The only shape with a published password. It runs the split stack with the demo
overlay: a seeded organisation, a one-shot seeder that runs before the API, and
the surface that replaces the demo data between showings.

Two things make it different rather than convenient:

- **The API is pinned to loopback.** The reset surface that shape carries has no
  authentication, so it must not face a network. Reach it through an SSH tunnel
  or a proxy on the same host. The public edge in front of a demo answers 404 for
  those routes as a second lock, not the only one.
- **It is never recommended.** The installer will not pick it because the machine
  happens to fit, and no scoring function should hand somebody an instance whose
  sign-in is in the documentation. It is chosen by name: `--profile demo`, or the
  card in the wizard.

Outbound mail is off. The demo data includes webhook endpoints and an alert
source the seed creates, so nothing a visitor triggers reaches a real address.

Resetting it is `marula demo reset`, which re-runs the overlay's seeder with
`--force`. That replaces the demo organisation's data; it does not snapshot it
first. Use the documented reset ritual when you need the snapshot, the BYOK
restore and the time-travel step.

## Where the floors come from

The four cores are the shipped embedding model's floor. Marula runs retrieval
locally by default, and a model that cannot load turns every search into an error
rather than a slower answer. If you would rather use cloud embeddings, set
`MARULA_EMBEDDING_PROVIDER=openai` and `MARULA_EMBEDDING_API_KEY`, and the hybrid
memory refusal in the plan stops applying. The installer will not quietly pick a
smaller local model for you.

The memory floor covers the model, Postgres and Valkey on one box. The disk floor
covers the images and the database with room to grow.

## How the installer picks

With no `--profile`, the installer scores the host against both shapes and takes
the highest one it meets, preferring `split`. "Highest" means the one that runs
more separate processes, not the one with the largest footprint.

A host that misses a floor is told which one, in its own units:

```text
Error: profile split needs more machine than this host has: 40 GB free disk (has 36)
```

If the platform publishes no memory reading, the installer treats the floor as
unmet rather than assuming it fits, and names the fix: declare the value with
`--memory-mb`, or set `MARULA_HOST_MEMORY_MB`. That is your declaration, not a
measurement, and the plan says so.

## When the machine is short

Developer mode in the app, and `--allow-unfit` on the command line, will plan a
shape the host misses. The plan runs and repeats the numbers it missed, at the
top of the review and in the terminal output.

This is a decision to proceed with less than the shape asks for. Nothing in the
plan prevents the stack failing later: an out-of-memory kill looks like a
container that stops, and a full disk looks like a failed migration. Recommended
mode never offers it.

## Choosing

| Your box | Take |
|---|---|
| A server that serves a team | `split` |
| A demo, a laptop, or one small box | `all-in-one` |
| A box you show the product on, reset between showings | `demo` |
| 30–40 GB free disk and nothing else spare | `all-in-one`, then grow the disk before moving to `split` |
| No local model, cloud embeddings configured | either, with the memory floor still applying to Postgres and Valkey |

The next step is the install itself: [from a terminal](../how-to/install-on-a-server.md),
or [with the app](../how-to/install-with-the-app.md).
