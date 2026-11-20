---
description: "If you prefer exploring a system from the terminal or want to build an integration script, this tutorial walks through the exact same workflow as Your first…"
---

# Tutorial: your first ticket

> **Audience:** developer — the API edition of the first-ticket lesson.

If you prefer exploring a system from the terminal or want to build an integration script, this tutorial walks through the exact same workflow as [Your first ticket (in the UI)](first-ticket.md) using `curl` against the REST API. It takes about 15 minutes.

## What you'll build

You will boot a local Marula stack, register an organization, sign in for a bearer token, create a client and a P2 ticket, log work, and watch the resulting domain events on the live stream.

## Prerequisites

- Docker (runs PostgreSQL with `pgvector`, Valkey, and local object storage)
- Rust toolchain (pinned in `rust-toolchain.toml`) and Node.js with `pnpm`

## 1. Start the local stack

```sh
git clone https://github.com/MarulaHQ/marula-core && cd marula-core
docker compose -f deploy/docker-compose.yml --env-file deploy/dev.env up -d --build
```

The first run builds the Rust workspace and the web bundle into images, so give
it minutes; later runs reuse the layers. Migrations run before the API starts,
and the workspace comes up on <http://localhost:3000>. [`deploy/dev.env`](https://github.com/MarulaHQ/marula-core/blob/main/deploy/dev.env)
holds published development secrets: it belongs on a machine you own. [Work on
the code](../how-to/install-from-source.md) covers the same two commands and
what each URL serves.

## 2. Register an organization

Send a `POST` request to `/api/v1/auth/register`. Each organization has a unique URL slug; if your chosen slug is already taken, the API returns `409 Conflict` with reason `org_slug_taken` and a verified-available `suggestion` field you can use right away.

```sh
curl -s -X POST localhost:8080/api/v1/auth/register \
  -H 'content-type: application/json' \
  -d '{"org_name":"Acme","org_slug":"acme","email":"you@example.com","password":"a-long-password","display_name":"You"}'
```

The response carries the access token, so this call signs you in as well.

## 3. Sign in for an access token

```sh
curl -s -X POST localhost:8080/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"you@example.com","password":"…"}'
```

Copy the access token from the response and pass it as `Authorization: Bearer $TOKEN` on subsequent calls.

## 4. Create a client and open a ticket

First, create the client company:

```sh
curl -s -X POST localhost:8080/api/v1/clients \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"Acme Corp"}'
```

A ticket belongs to a board, so list the boards first and take an `id` from the response:

```sh
curl -s localhost:8080/api/v1/boards -H "authorization: Bearer $TOKEN"
```

Then create the ticket with that board, the client id from the call above, and a
priority of `p1` to `p4`:

```sh
curl -s -X POST localhost:8080/api/v1/tickets \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"board_id":"…","client_id":"…","title":"Printer on fire","priority":"p2"}'
```

## 5. Start and stop the timer

In the web UI, a technician starts the work-bar timer, types their troubleshooting notes, and stops the timer on a single screen. Stopping the timer creates the billable time entry automatically.

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](https://github.com/MarulaHQ/marula-core/blob/main/MANIFESTO.md), quoted verbatim

## 6. Inspect the emitted domain events

Every write in `core-api` commits a versioned event to the transactional outbox in the same database transaction:

The outbox feeds a server-sent event stream. `-N` keeps `curl` from buffering, so
each event prints as it commits, starting with `ticket.created` from the call
above:

```sh
curl -N localhost:8080/api/v1/events/stream \
  -H "authorization: Bearer $TOKEN"
```

The same events are what webhooks and automation rules consume, so this stream is
the shortest way to see them without a receiver.

## What you learned

- `core-api` is the single writer for the database; the web app, mobile app, CLI, and TypeScript SDK all call the same endpoints.
- Every write returns an instant-first response envelope and records a domain event in the outbox.

Next: Read [How to call the API](../how-to/call-the-api.md) to use the typed `@marula-core/sdk` client, or browse the full [API reference](../reference/api.md).
