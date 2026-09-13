# Tutorial: your first ticket

This lesson walks you from an empty machine to a worked ticket with a time
entry, using the real API. It takes about 20 minutes.

## What you'll build

A running Meridian stack, an organization with one client, one ticket, a
timer that turns into a time entry, and the domain events those actions
emitted.

## Prerequisites

- Docker (for Postgres+pgvector, Redis, MinIO)
- Rust toolchain (see `rust-toolchain.toml`) and Node + pnpm

## 1. Start the stack

```sh
git clone https://github.com/Marlin-Hofmeyer/meridian && cd meridian
docker compose -f deploy/docker-compose.yml up -d
cargo install --path crates/meridian-cli
meridian dev
```

`meridian dev` runs migrations, starts the core API and worker, and serves
the web console.

## 2. Register an organization

Registration is `POST /api/v1/register`. The org slug must be unique; if it
is taken the API returns a typed `409` with reason `org_slug_taken` and a
verified-available `suggestion` you can accept.

## 3. Log in

```sh
curl -s -X POST localhost:8080/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"you@example.com","password":"…"}'
```

The response is a token pair; send the access token as `Authorization:
Bearer …` on every subsequent call.

## 4. Create a client and a ticket

```sh
curl -s -X POST localhost:8080/api/v1/clients \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"name":"Acme Corp"}'
```

Then create the ticket:

```sh
curl -s -X POST localhost:8080/api/v1/tickets \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"client_id":"…","title":"Printer on fire","priority":"high"}'
```

## 5. Work it

In the web console, the ticket is one screen: notes, timer, conversation,
and calendar live together. Start the timer, add a note, stop the timer.
The rough notes become a proper time entry — no separate time-entry screen
hunt.

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](../../../MANIFESTO.md), quoted verbatim

## 6. Watch the events

Every mutation emits a domain event from the transactional outbox:

```sh
curl -s "localhost:8080/api/v1/events?limit=5" \
  -H "authorization: Bearer $TOKEN"
```

You should see `ticket.created` plus a timer/time-entry event.

## What you learned

- The API is the single write surface; the console and SDK are clients of it.
- Mutations flow through the instant-first envelope (optimistic render,
  authoritative reconcile).
- Domain events are the audit trail — and the integration seam.

Next: [How to call the API](../how-to/call-the-api.md) for the typed client,
or the [API reference](../reference/api.md) for the full surface.
