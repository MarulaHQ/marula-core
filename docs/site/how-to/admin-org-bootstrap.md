# How to bootstrap your first organization

> **Audience:** admin — you are creating the org you will administer.

When you launch a fresh Marula install, you create your organization and first administrator account right from the sign-in screen. Once inside, you set your company name, ticket numbering prefix, default timezone, and base currency before inviting the rest of your team.

## Prerequisites

- A reachable Marula server in your browser (or the desktop app — see [Self-host local stack reference](../reference/self-host-local-stack.md)).
- Each user account belongs to one organization.

## 1. Switch to Create org on the sign-in page

Open `/login`. Beneath the heading *"Sign in to your service desk, or create your organization"*, switch the segmented toggle from **Sign in** to **Create org**.

## 2. Register your organization and owner account

Fill in your organization name, desired URL slug, and owner login details. Organization slugs must be unique; if a slug is already taken, the server returns `409 Conflict` (`org_slug_taken`) along with an available `suggestion` you can accept with one click. (You can also register via `POST /api/v1/register` as shown in the [API tutorial](../tutorials/first-ticket-api.md).)

## 3. Set your company identity and base currency

Sign in and open **Settings → General**. Set these four values first, before your team starts logging tickets or invoices:

- **Organization name**
- **Ticket prefix** (for example, `INC` or `MSP`)
- **Workspace slug**
- **Base currency** (ISO 4217 code such as `USD`, `GBP`, `EUR`, or `ZAR`)

Two safeguards to keep in mind:

- Every change takes effect immediately and writes an entry to the audit log.
- Once your organization has recorded foreign-exchange conversion snapshots on invoices or stock receipts, Marula blocks changing the **base currency** so historical financial totals stay accurate. Set your base currency first (see [Manage FX rates](admin-fx-rates.md)).

Further down **Settings → General**, set your **end-of-day hour** (which controls when end-of-day missing-time reminders trigger), **default timezone**, and **first day of the week**.

## 4. Choose your theme and invite your team

- **Theme and density:** Open **Settings → Appearance** to pick your organization's default theme preset (`Marula` neutral, `Marula Warm`, or `High contrast`). Individual team members can still pick their own theme or density under **Preferences**.
- **Single sign-on (optional):** Connect Google Workspace, Microsoft Entra ID, Okta, or Keycloak using [Connect an IdP (SSO)](admin-sso.md).
- **Invite staff:** Send setup links to your technicians and dispatchers (plus your bookkeepers) via [Manage users and invites](admin-users-invites.md).

## Related

- If you are just exploring locally, the preloaded demo organization (`owner@demo.local` / `demo-password-123`) already has these steps completed.
- Before opening your boards to live customer traffic, configure your business hours in [Configure SLA calendars & escalation ladders](admin-sla-calendars.md).
