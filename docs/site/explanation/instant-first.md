# Instant-first interaction

> The binding doctrine lives in [`docs/instant-first.md`](../../instant-first.md)
> (doctrines #7–#9 in [`docs/STRATEGY.md`](../../STRATEGY.md) point at it). This
> page explains what it means and why the product is built this way.

## The idea

Software for people doing real work must never make them wait to *see* their
own action. Meridian renders the intended outcome immediately (optimistic
render) and then reconciles against the authoritative response from the
server. The UI never polls to "check if it worked" — the mutation response
is the truth.

## What the core must provide (the support contract)

For the UI to be able to promise instant, the core provides:

1. A typed `mutation` envelope on every mutation — sequence, version,
   updated-at — so clients can reconcile deterministically.
2. A typed error envelope (IF3) with machine-readable reason codes, so
   failure is as renderable as success.
3. A transactional outbox emitting domain events once per mutation, with a
   live stream (`GET /api/events/stream`) for push updates.
4. Undo as the escape hatch — [v1-undo-stacks](../../reference/read-model/v1-undo-stacks.md)
   backs reversible operations.

## Agent changes default to approval

Agent-authored changes require approval by default (doctrine #8). The agent
platform is an extension surface, not a parallel write path: proposals land
as typed artifacts (`/api/v1/agent-proposals`) a human approves, declines,
or withdraws. Humans make the human decisions; AI makes the mechanical
things go faster.

## The core's data stays queryable

Doctrine #9: the core's data stays queryable. The [read
model](../reference/read-model.md) is a supported interface, not an
implementation detail — which is also why "leave any day" (the manifesto's
promise) is a property of the architecture, not a marketing line.
