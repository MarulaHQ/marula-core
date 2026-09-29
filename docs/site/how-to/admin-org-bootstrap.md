# How to bootstrap your first organization

> **Audience:** admin — you are creating the org you will administer.

## Prerequisites

- A reachable Marula deployment (self-hosted console or the desktop app
  — see [self-host reference](../reference/self-host-local-stack.md)).
- No existing session in the target deployment, or awareness that one
  account belongs to one org.

## 1. Find the create-org toggle

The landing page at `/login` carries both entry modes side by side:
"Sign in to your service desk, or create your organization" with a
**Sign in / Create org segmented toggle**. Choose **Create org**.

## 2. Register

Registration creates both the organization and its owning principal.
Slugs are unique: if the org slug is taken, the API returns a typed `409`
with reason `org_slug_taken` and a verified-available `suggestion` you can
accept. (The same registration flow works via
`POST /api/v1/register` — see the [API edition tutorial](../tutorials/first-ticket-api.md).)

## 3. Set the tenant identity

Sign in and open **Settings→General**. Here you fix the things that are
painful to change later: org name, ticket prefix, workspace slug, and the
**base currency** (ISO 4217). Two warnings from the surface itself:

- Changes apply immediately after saving and are recorded in the audit
  log.
- Changing the base currency is **refused** while FX conversion snapshots
  exist — record rates first, or reset rate history deliberately (see
  [FX](admin-fx-rates.md)).

Org defaults down the page: end-of-day hour (drives missing-time
detection and EOD nudges), default timezone, first day of week.

## 4. Shape identity plumbing deliberately

- Theme and appearance: **Settings→Appearance** carries the theme presets
  (Tangerine default) org-wide, with per-user override on Preferences.
- Optional SSO via IdP: [connect an IdP](admin-sso.md).
- Every later user is an invite: [users & invites](admin-users-invites.md).

## Related

- The seeded demo org (`owner@demo.local` / `demo-password-123`) exists so
  tutorials skip this step — this page is how you leave the demo behind.
- [SLA calendars](admin-sla-calendars.md) before your first ticket wave,
  so clocks run on the right calendar.
