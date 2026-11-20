---
description: "If you prefer exploring a system from the terminal or want to build an integration script, this tutorial walks through the exact same workflow as Your first…"
---

# Tutorial: your first ticket

> **Audience:** developer — the API edition of the first-ticket lesson.

If you prefer exploring a system from the terminal or want to build an integration script, this tutorial walks through the exact same workflow as [Your first ticket (in the UI)](first-ticket.md) using `curl` against the REST API. It takes about 15 minutes.

## What you'll build

You will boot a local Marula stack, register an organization, sign in for a bearer token, open the live stream, and then create a client and a ticket and watch the events arrive while you log work.

## Prerequisites

- Docker, with Compose 2.24 or newer. Nothing else is built on your machine:
  the images compile the Rust workspace and the web bundle themselves.

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

Keep the access token in the shell you are working in, so the calls below read
from it:

```sh
export TOKEN=…        # the access_token field from the response
```

## 4. Watch the live stream

The outbox feeds a server-sent event stream. Replay is opt-in: a fresh
connection carries only what commits after it opens, because the resume cursor is
a `Last-Event-ID` header that a browser sends on reconnect and `curl` does not.
Start this now, in a second terminal, with the same token, and leave it running:

```sh
export TOKEN=…        # the same value as above
curl -N localhost:8080/api/v1/events/stream \
  -H "authorization: Bearer $TOKEN"
```

`-N` keeps `curl` from buffering, and a `: ping` comment every 15 seconds proves
the connection is alive while nothing is committing. Nothing else arrives until
the next step creates something.

## 5. Create a client and open a ticket

First, create the client company:

```sh
curl -s -X POST localhost:8080/api/v1/clients \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"Acme Corp"}'
```

A ticket belongs to a board, so list the boards first and take an `id` from the
response:

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

The call returns an instant-first envelope, and `ticket.created` lands in the
terminal from step 4 as it commits.

## 6. Start and stop the timer

In the web UI, a technician starts the work-bar timer, types their troubleshooting notes, and stops the timer on a single screen. Stopping the timer creates the billable time entry automatically.

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](https://github.com/MarulaHQ/marula-core/blob/main/MANIFESTO.md), quoted verbatim

## 7. What the stream carries

Every entity write in `core-api`, whether a ticket, a client or a time entry,
commits a versioned event to the transactional outbox in the same database
transaction. The stream from step 4 is that outbox. Registration and sign-in are
not among them: they write an audit row and return tokens, which is why your
first frame was the ticket.

## What you learned

- `core-api` is the single writer for the database; the web app, the mobile app
  and the TypeScript SDK all call the same endpoints.
- An entity write returns an instant-first response envelope and records a domain
  event in the outbox.

Next: Read [How to call the API](../how-to/call-the-api.md) to use the typed `@marula-core/sdk` client, or browse the full [API reference](../reference/api.md).
