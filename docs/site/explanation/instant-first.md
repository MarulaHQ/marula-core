# Instant-first interaction

> The full technical specification lives in [`docs/instant-first.md`](../../instant-first.md)
> (doctrines #7–#9 of the project's internal engineering strategy). This page
> explains what it means for your daily work and why the product is built this way.

## What you feel in the app: no Save button, instant feedback, real undo

When you are on the phone with a customer and updating a ticket's priority or assignee, you shouldn't have to hunt for a Save button at the bottom of a form or wait for a loading spinner to see your own change.

In Marula, edits apply on screen the instant you click or press a key, and the app confirms the change with the server in the background. If you change your mind or click the wrong status, press `Ctrl+Z` (`Cmd+Z` on a Mac) to undo the change cleanly—even after the server has already saved it.

## How the server backs that up

To make instant updates safe for a team working the same board at once, the backend provides four guarantees on every write:

1. **A versioned `mutation` envelope:** Every write returns a sequence number, the record's new version, and the server timestamp so your browser reconciles immediately without polling.
2. **Structured error reasons (`IF3`):** If a change cannot be saved (or collides with a teammate's edit made a half-second earlier), the API returns a machine-readable code like `version_conflict` so the UI can show you the exact conflict instead of a generic error toast.
3. **Live push updates:** Every database write records a domain event in the transactional outbox and pushes it over the SSE stream (`GET /api/events/stream`) in under `100 ms`, so dispatchers and technicians looking at the same ticket see updates live.
4. **Server-backed undo history:** Reversible actions are tracked in the undo stack ([`v1-undo-stacks`](../../reference/read-model/v1-undo-stacks.md)) so stepping backward is a real, audited database action.

## Assistant changes always wait for human approval

While human clicks apply immediately, changes suggested by an AI assistant or background agent follow the opposite rule (doctrine #8): **they wait for a person to approve them**.

Agent suggestions land as explicit proposals (`/api/v1/agent-proposals`) that a technician or admin can approve, decline, or dismiss. Nothing silently rewrites a ticket or emails a customer behind your back.

## Your operational data stays easy to query

Doctrine #9 guarantees that your data is never trapped behind an opaque application cache. The versioned SQL views in the [read model](../reference/read-model.md) let you query your tickets, time entries, SLA outcomes, and invoices directly with standard SQL or any BI tool.
