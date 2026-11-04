# ADR 0029: The refresh gate is keyed twice — IP outside, session inside

Date: 2026-10-27 · Status: Accepted; implementation open · Ruling: Wave 62 register triage

## Context

Every throttled auth surface takes a durable gate in Postgres (migration 0088), so
a restart never resets a lock. The gate holds one key per IP and, where an actor is
known, one key per email or user (`crates/core-api/src/auth/hardening.rs`).

`POST /api/auth/refresh` is the one surface with no actor to key on before the
token is validated. It therefore runs on the IP key alone
(`GateSubject::IpOnly`). Wave 47 lane A26 fixed the worst part of that: a
successful rotation now clears the counters, so the ladder is a reaction to a run
of failures rather than a permanent penalty. What it did not change is the key
set. A stale cookie from one laptop still arms a bucket shared by everyone behind
the same egress, and the refresh arm is the one that bounces a user back to the
login screen.

The choice is not free in either direction:

- IP only punishes an office NAT, a CI runner, and a long end-to-end sweep for one
  client's mistake.
- Session only would let an unauthenticated caller skip the gate entirely by not
  presenting a session, which is the case an attacker controls.

## Decision

1. The IP key stays, and stays first. It is the outer bound: it runs before any
   credential work, so it costs one indexed read and cannot be avoided by an
   attacker who never has a valid token.
2. A session key joins it. Once `resolve_refresh` has produced a session, the same
   request is checked again against that session, and a failure is recorded on
   both keys.
3. A successful rotation clears both counters, exactly as it clears the IP counter
   today.
4. The pre-auth refusal keeps its current shape. The typed 429 answers only after
   the IP gate; the session-bound answer adds no new oracle about whether a
   session exists, because the caller already presented the token that resolves
   it.

## Why both

Each key covers the failure the other cannot. The IP bound stops a caller who
rotates nothing. The session bound stops one session's failures from charging a
whole office. Taking only one of them leaves the incident that produced this
ruling — one stale cookie, one armed hour.

## Implementation status

Not implemented at HEAD. `handlers::refresh` still passes `GateSubject::IpOnly`
(`crates/core-api/src/auth/handlers.rs:803`), and `GateSubject` has no session
variant (`hardening.rs:44-48`). The owning follow-up is the A26 bullet in the live
tracker. Until it lands, this record is the decision, not a description of the
code.

## Consequences

- The refresh path gains one read after token resolution. It is the same shape the
  MFA legs already pay after their bridge resolves.
- Two counters must be cleared on success, so a change that clears only the IP
  key re-opens the ratchet this ADR exists to close.
- Tests that assert one armed bucket for refresh now assert two, and the
  end-to-end gate's arming counts move with them.
