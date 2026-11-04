# ADR 0029: The refresh gate is keyed twice — IP outside, session inside

Date: 2026-10-27 · Status: Accepted; implemented (`handlers::refresh`, `hardening::record_session_failure`) · Ruling: Wave 62 register triage

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
   request is checked again against that session. A failure that reaches this point
   is charged to the session key; the shared IP key is charged only by failures that
   resolve no session at all — a caller with no valid token, which is the case an
   attacker controls. Both keys are still written on the durable attempt row, and
   both are cleared by a success. (Amended 2026-11-05, while implementing: charging
   the IP counter on a session-bound failure reproduces the incident. The two
   counters would arm and expire in lockstep, so the office ladder still climbs to
   an hour and the session key buys nothing.)
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

Implemented. `handlers::refresh` checks the outer IP key before any credential work,
then checks again against the resolved session before it classifies the token
(`GateSubject::Session`), records a session-bound failure on that session alone
(`hardening::record_session_failure`), and clears both keys after a rotation
commits. `resolve_refresh` still returns no session id, so the id the pair was
presented with is the one used — but only after the bridge has confirmed the
pair, which is what keeps an unvalidated id from naming a bucket.

## Consequences

- The refresh path gains one read after token resolution. It is the same shape the
  MFA legs already pay after their bridge resolves.
- Two counters must be cleared on success, so a change that clears only the IP
  key re-opens the ratchet this ADR exists to close.
- A refresh request is checked against two keys now, so the durable-gate tests that
  pin one bucket for refresh assert two: the outer IP key and the session key.
