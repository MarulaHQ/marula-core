# How to set up the portal

> **Audience:** admin — provisioning portal users and deciding exposure is
> your decision, per client.

## Prerequisites

- Admin access to **Settings→Portal** (DELIVERY group).
- The client organization that will be the trust boundary in mind:
  "Portal settings and users are isolated per client organization" —
  everything here happens inside one client's scope.
- Boards that make sense to expose (usually the client's Service Desk
  board).

## 1. Expose boards first

**Settings→Board Exposure Controls**: per board, **Exposed to client /
Hidden** — a direct toggle, nothing inferred. Exposure governs what the
client's Requests view can land on. Boards stay hidden until you say
otherwise.

## 2. Provision the user

**Portal Users Directory** lists existing portal users (email, display
name, contact role, status). **Provision portal user** creates a new one.
Row actions: Disable, reset password, Delete — the service desk owns the
credentials lifecycle, deliberately:

> "Your service desk provisions accounts and can reset this password."

There is no self-serve password reset in the portal shell, and the
staff password-reset flow excludes portal users too.

## 3. Hand over first sign-in

The portal shell is separate staff-side by design ("Marula — client
portal sign-in", nav Requests / Invoices / Knowledge / Schedule, footer
"Support access provided by your service desk"). Hand over:

- the portal URL,
- their provisioned email and initial password,
- the one-line expectation: requests, invoices, published knowledge,
  and scheduling — and nothing else.

A complete first-session walkthrough for the client themselves lives in
the [portal onboarding tutorial](../tutorials/portal-onboarding.md),
and the running tasks are cataloged in [client-side portal tasks](client-portal-tasks.md).

## 4. What provisioning does NOT do

- It does not grant staff search, settings, or cross-client visibility —
  portal principals are structurally barred from the staff unified
  search, and the portal knowledge base resolves through its own
  published + client-visible gate.
- It does not touch other clients: isolation is per client organization.
  The design rationale: [portal isolation](../explanation/portal-isolation.md).

## What you learned

- Exposure first, users second — the client's world is exactly the
  exposed boards plus their records.
- Provisioning and credential recovery are MSP-side by design.
- The same page drives both controls. No other surface decides portal
  scope.
