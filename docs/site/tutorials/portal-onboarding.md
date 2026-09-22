# Tutorial: portal onboarding

> **Audience:** both — steps 1–3 are admin (provisioning and exposure);
> steps 4–5 are the client's first minutes and can be handed to them.

The client portal is a separate trust zone with its own sign-in shell,
separate auth, and per-client isolation. There is no public self-serve
entry — your service desk provisions accounts, and that is
deliberate. What a portal user can reach (the exposed boards,
published knowledge, the invoice view) is the MSP's provisioning
decision, made via Settings→Portal.

## Prerequisites

- Admin access for steps 1–3, plus a second browser profile for step 4.
- A client organization on the seeded demo org to isolate against.


## 1. Expose the boards you want visible

Sign in as an admin and open **Settings→Board Exposure Controls** (Section:
Portal). Exposed to client / Hidden per board, explicitly: a board is
either client-visible or not — nothing in between is inferable from the
surface. Expose the Service Desk board you want the client to create
requests on.

## 2. Provision the portal user

Still under Settings→Portal: **Portal Users Directory** shows email,
display name, contact role, and status per portal user. Use **Provision
portal user** to create an account. Directory row actions: Disable, reset
password, Delete.

Isolation applies here too: portal settings and users are isolated
per client organization — you provision inside a specific client's scope,
never org-wide.

## 3. Hand over the sign-in

The portal lives at its own shell: "Meridian — client portal sign-in",
with its own nav (Requests / Invoices / Knowledge / Schedule) and a footer
stating **"Support access provided by your service desk"**. Credential
facts to hand over honestly:

- Your service desk provisions accounts and can reset this password.
- Self-serve password reset on the portal is deliberately not offered
  (the staff password-reset flow explicitly excludes portal users).
- Portal sign-in is separate from staff sign-in — a client user never
  reaches staff surfaces and vice versa.

## 4. The client's first request

The client signs in and lands in the Requests view. From there they can
open a request against the exposed board. It becomes an ordinary ticket
in your Service Desk board, attributed to the portal channel. They can
follow it on Requests, see their **Invoices**, self-serve answers from
**Knowledge** (client-facing articles — staff can flag articles
client-facing under Knowledge), and book time on **Schedule**.

## 5. What the client cannot reach (and why)

Everything on the staff side: settings, other clients, cross-client
search, the vault, provenance. Portal search resolves from the portal
knowledge base with its own published + client-visible gate, and portal
principals are structurally barred from the staff unified search. The
model behind this is explained in
[portal isolation](../explanation/portal-isolation.md).

## What you learned

- Provisioning is MSP-side, per client: exposure first, then users.
- The portal governs its own sign-in, its own credential recovery, and
  its own search domains by design.
- The first minutes for a client are four nav items: Requests, Invoices,
  Knowledge, Schedule.
