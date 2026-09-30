# ADR 0015 — Per-user email greeting & signature, org-governed

**Status:** Accepted · 2026-09-30
**Scope:** outbound email composition — the org's own greeting and signature,
the per-user defaults beside them, the rule that connects them, and the
ledger's record of which one a message carried

## Context

Four facts in the existing code shaped this:

1. **A per-user greeting and sign-off already shipped** (Wave 12 lane 6) as
   `email.greeting` and `email.signoff`, registry settings at user scope,
   with editors on the personal preferences page.
2. **Nothing read them at send time.** The only consumer was the close-flow,
   which copied them into a textarea, and that textarea is not what the
   client receives. The requester is mailed a body the backend generates
   (`core_mail::outbound::CommentNotification`), so a personal signature had
   never actually reached an outbound message. The org's `email.signature`,
   appended unconditionally in `enqueue_outbound`, was the only signature
   that did.
3. **The close-flow's copies were device-local** (`marula.email-prefs` panel
   state), so a second device signed mail differently and neither matched
   what arrived.
4. **The board identity family already carried a signature**
   (`email.board.<uuid>.signature`), applied over the org one by
   `resolve_mail_identity`, which put a board-vs-user collision on the table
   that no code had answered.

The lane brief proposed "per-message override > user default (when the
policy allows) > board identity.signature? or org signature?" with the
question marks intact, and asked for the greeting-vs-signature policy scope
to be decided and written down rather than inherited.

## Decision

### The chain

Highest first, for the signature on a message **a person wrote**:

1. **Message override** — written for this message in a composer.
2. **User default** — the sender's own `email.signoff`.
3. **Board identity** — the board's own signature, for that board's mail.
4. **Org default** — `email.signature`.
5. **Unset** — nothing configured, and no empty block is sent.

The greeting has the same shape with the board rung dropped, because the
board identity family has never carried a greeting.

The chain itself is `core_domain::email_signature` — a pure function over
its inputs, unit-tested there, so the whole precedence table is verifiable
without a database and the API layer is left holding only reads.

### The policy gates both personal rungs

`email.signature_policy` is `allowed` or `org_locked`, org-scoped, held at
the database by a CHECK constraint as well as by the registry.

`org_locked` closes rung 1 **and** rung 2. An override is a personal
signature that was never stored, so a workspace that locked signatures
would otherwise be routable around one by composing a message with a
hand-written sign-off. Rungs 3 and 4 are org-scoped storage and are not the policy's
business: a board signature is the org speaking through a shared inbox, and
`org_locked` means "one answer from the org", not "ignore the org's own
per-board choices".

A lock REFUSES rather than ignores. `PUT /api/v1/settings/user` for
`email.signoff` under a lock is a typed 422 naming the policy, and a
per-message override on a client-visible note is a typed 422 naming it too.
Accepting a stored value and then not using it leaves a preference the admin
never agreed to, waiting for the day the lock is lifted. Accepting an
override and then dropping it sends a message the sender did not write.

### Board vs user: the user's own signature wins

Recorded because it is a real fork, not a detail. A member who has a personal signature and sends from a board that carries
one gets their **own**.

The rejected alternative is board beats user, which makes `allowed` mean
"your signature applies unless you happen to sit on a board that set one".
That makes the per-user editor unreliable rather than merely unused. The board
signature is a shared fallback, one rung above the org, and personal
identity is what the permissive policy is selling.

### The policy does NOT reach the greeting

Recorded, because the lane brief's lean was the other way and the
disagreement is the interesting part.

The brief's reasoning was "a signature is personal identity, a greeting is
house voice" — and on its own terms that argues for locking the greeting
too. We did not, for one reason: `email.greeting` is a **shipped per-user
setting** (Wave 12 lane 6) with an editor on the preferences page. Locking
it would not govern it, it would REMOVE a feature, for no compliance benefit
a greeting plausibly carries. So the greeting keeps its per-user rung; the
org gained a default (`email.org_greeting`) that stands in when a member has
none, and the org's voice is the fallback rather than the ceiling.

If a future decision wants the greeting locked too, the change is small and
the seam is in place: gate `greeting_input.user_default` on
`policy.allows_personal()` in `resolve_envelope` (the greeting is built
beside the signature input for exactly this reason), and disable the second
field in the same editor that already renders the locked state.

### Send-time resolution, one dialect

Placeholders are expanded when a message is enqueued, against the SENDING
user's row and the org's name — never at save time. A member who edits
their display name or email address changes every message they have not sent
yet, with nothing to re-save. A test states it directly: save a signature
with `{{user.name}}`, rename the user, send again, assert the new name went
out and the old one did not.

The vocabulary is `{{user.name}}`, `{{user.email}}`, `{{org.name}}` — a
closed set, enforced on write by `validate_signature`, rendered by the same
scanner and the same error type as every other template. It is a VOCABULARY
over the existing engine (`render_with`), not a second engine, and not a
`TemplateKind`, so the `templates` table's kind CHECK is untouched and no
stored template row can claim a vocabulary it does not use.

It is exactly the three fields the `users` row can fill today. The brief
asked for `{{user.title}}` "if present" and `{{user.phone}}`. There is no
`users.title` and no `users.phone` column, so offering them would put a
token in the picker that can only ever render empty, which is the one thing
this vocabulary must not do: the editor would promise a field the send
cannot deliver. A `users.title` column adds its vocabulary entry in the
same commit that adds the column.

### The ledger remembers which rung applied

`email_messages` gained `signature_source` and `greeting_source`: nullable,
closed by CHECK, and rendered in the delivery log as "the sender", "the
board", "the organization", "this message", "a fixed Marula line" or "none".
A row written before this lane has NULL, which reads as `none` — the truth
about a row nothing was recorded against, rather than a guess.

`product` is the one value the chain cannot return. It labels a line the
product writes itself and no setting governs: the CSAT survey's neutral
"Hi," and the "Hi {first name}" salutation a notification falls back to
when the org has set no greeting. Without it the ledger would have to claim
a rung supplied a block it did not.

### Signers: a person or the product

`Signer::User` walks all five rungs. `Signer::System` skips the personal
ones, because a notification to a member, a digest, a CSAT survey and a
public booking confirmation are written by nobody — letting one borrow the
recipient's personal signature would be a signature on the org's own mail
to its own staff. A dunning statement is a `User` signer, because a member ran the sweep.
So is the admin test probe, because an admin asking "will my mail look
right" deserves the answer they will get.

## Consequences

- `email.signoff` keeps its key. Its cap moves 500 → 1000 to match the org
  signature's, and it becomes a token-bearing signature rather than a plain
  sign-off. Keeping the key stops the existing composers from orphaning a
  stored value, and migration 0118 normalizes any row the new cap would
  refuse.
- The per-message envelope is a new optional field on
  `POST /tickets/{id}/comments`. The composer sends the NOTE as the body and
  the greeting/signature as the envelope; the server assembles
  greeting + generated body + signature. The note in the timeline is a note,
  not a letter.
- The close-flow's device-local store is gone. A member's defaults come from
  the account, so a second device matches the first, and what a composer
  shows matches what the send path resolves.
- No second wrapper: `core_mail::brand` is unchanged. It already took `greeting` and `signoff`
  as `Option<&str>`, and callers now pass resolved values. The ticket path was never the wrapper; it is the outbound body
  assembler, and it now resolves through the same chain.
- The portal's own `reply` route is client-authored and sends no email, so
  there is no per-message override surface on it today. When staff-side
  portal composition is added, the envelope rides on it unchanged.
- **This is not the drawn signature pad.** The mobile Wrap-up flow's
  signature PAD draws ink and stores a drawing; this lane's signature is
  text with placeholders. The two are not coupled, share no table, no
  setting and no UI, and nothing in the resolution chain knows a drawing
  exists. A drawn mark, when it arrives, is a fourth representation the
  per-message envelope would carry rather than a rung of this chain, and
  the decision belongs to whoever builds it.

## Alternatives considered

- **One key, two scopes** (`email.signature` as both the org default and the
  personal one). Rejected: `settings` is keyed by (org, scope, key, owner),
  so the storage allows it, but a key with one declared scope plus a
  get-effective walk that falls back user → org would make the lock's
  semantics depend on which accessor a caller happened to use. The extra
  rung is a row, not an ambiguity.
- **Baking values at save time** (expanding `{{user.name}}` when the member
  types it). Rejected: a signature would outlive the name it was written
  with, and the fix would be invisible — the stored value looks fine.
- **Gating the override on the policy** vs not. Gated. An ungated override
  is a policy with a hole in it, and the hole is the easiest one to walk
  through.
- **Locking the greeting too.** Rejected for the reason above: it removes a
  shipped feature rather than governing it.
