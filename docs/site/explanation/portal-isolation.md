# The portal isolation model

> **Audience:** both — admins provision; clients live inside a boundary
> they can and should assume is enforced by the architecture, not by
> politeness.

## The structure: separate principal, separate trust zone

The portal is not a theme on the staff product. It is a **separate
trust zone** with its own principals, its own sign-in shell, and its
own knowledge gate. The Settings→Portal banner states the boundary:
"Portal settings and users are isolated per client organization". The
design goals, in order:

1. **A portal principal is not a staff principal.** Its identity, its
   sessions, its roles live in their own domain; portal sign-in happens
   on the portal shell ("Marula — client portal sign-in"), and the
   credential lifecycle rests with the service desk ("Your service desk
   provisions accounts and can reset this password" — no self-serve
   reset, and the staff password reset flow explicitly excludes portal
   users).
2. **Per-client isolation, not per-user.** Provisioning and portal
   settings happen inside one client organization's scope, never
   org-wide. Exposure decisions bind to the client, not to a global
   switch.
3. **Exposure is a decision, not an inference.** Settings→Board Exposure
   Controls: per board, **Exposed to client / Hidden** — nothing else
   leaks through.

## What structural bar means in practice

Three facts combine into the tenant guarantee:

- Portal traffic reads from portal-scoped resolution: the portal's
  knowledge base resolves articles through its own **published +
  client-visible gate** rather than through staff KB rules.
- The staff unified search seam is structurally barred to portal
  principals — covered by a dedicated core-api test
  (`portal_principals_never_reach_the_unified_search_seam`), so cross-
  tenant document hits (or any other staff-scoped search hit) are not a
  "careful filtering" matter: they cannot arise through that seam at
  all.
- The knowledge and [workflow](../how-to/admin-workflow-categories.md)
  boundaries above (KB-class categories, ADR 0006) mean article
  exposure is an explicit flag on each article inside client-facing
  categories. Those stay exactly the subset that keeps the trust
  guarantee as internal reading lists churn.

## What clients experience (and what they should never expect)

The client's experience is four surfaces — Requests, Invoices, Knowledge,
Schedule (the [client how-to](../how-to/client-portal-tasks.md) walks
them). A client can and should assume:

- Their views cover their own tickets and their own invoices, plus
  the knowledge published for clients. Never another client's.
- Cross-border leak claims do not have a "careful with settings" escape
  valve: the protections above are in the seam and the tests, not in
  staff restraint.

What clients should **not** expect:

- Any staff surface, any cross-client search, any internal KB article
  that has not been flagged client-facing, any self-serve password
  reset, any invitation to "just try the staff login".

## Why the isolation model matters to the MSP side

- Legally/contractually, "per-client isolation" is the property that
  makes serving multiple clients from one stack defensible — the
  [portal setup how-to](../how-to/admin-portal-setup.md) turns it into
  the provisioning steps.
- Practically, isolation is why the client knowledge gate and the
  exposure toggles live in settings **under that client's scope** — a
  misconfiguration widens one client's view, never all clients'.
