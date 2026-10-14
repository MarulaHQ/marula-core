# Tutorial: your first ticket

> **Audience:** developer — the API edition of the first-ticket lesson.

If you prefer exploring a system from the terminal or want to build an integration script, this tutorial walks through the exact same workflow as [Your first ticket (in the UI)](first-ticket.md) using `curl` against the REST API. It takes about 15 minutes.

## What you'll build

You will boot a local Marula stack, register an organization, sign in for a bearer token, create a client and a high-priority ticket, log work, and read the resulting domain events from `/api/v1/events`.

## Prerequisites

- Docker (runs PostgreSQL with `pgvector`, Valkey, and local object storage)
- Rust toolchain (pinned in `rust-toolchain.toml`) and Node.js with `pnpm`

## 1. Start the local stack

```sh
git clone https://github.com/Marlin-Hofmeyer/marula && cd marula
docker compose -f deploy/docker-compose.yml up -d
cargo install --path crates/marula-core-cli
marula dev
```

`marula dev` runs all database migrations, starts `core-api` and `worker`, and serves the web workspace.

## 2. Register an organization

Send a `POST` request to `/api/v1/register`. Each organization has a unique URL slug; if your chosen slug is already taken, the API returns `409 Conflict` with reason `org_slug_taken` and a verified-available `suggestion` field you can use right away.

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

Next, create a ticket using the returned client ID:

```sh
curl -s -X POST localhost:8080/api/v1/tickets \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"client_id":"…","title":"Printer on fire","priority":"high"}'
```

## 5. Start and stop the timer

In the web UI, a technician starts the work-bar timer, types their troubleshooting notes, and stops the timer on a single screen. Stopping the timer creates the billable time entry automatically.

> "A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job."
> — [MANIFESTO.md](../../../MANIFESTO.md), quoted verbatim

## 6. Inspect the emitted domain events

Every write in `core-api` commits a versioned event to the transactional outbox in the same database transaction:

```sh
curl -s "localhost:8080/api/v1/events?limit=5" \
  -H "authorization: Bearer $TOKEN"
```

You will see `ticket.created` along with the timer and time-entry events ready for webhooks, automation rules, and the WebSocket hub.

## What you learned

- `core-api` is the single writer for the database; the web app, mobile app, CLI, and TypeScript SDK all call the same endpoints.
- Every write returns an instant-first response envelope and records a domain event in the outbox.

Next: Read [How to call the API](../how-to/call-the-api.md) to use the typed `@marula-core/sdk` client, or browse the full [API reference](../reference/api.md).
