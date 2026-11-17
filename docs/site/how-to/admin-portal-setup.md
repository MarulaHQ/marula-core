---
description: "Set up the client portal in two steps: expose the boards clients submit into, then provision each client's contacts. What portal accounts can never reach."
---

# How to set up the portal

> **Audience:** admin — provisioning portal users and deciding exposure is
> your decision, per client.

The **Client Portal** lets your customers submit support requests, view their invoices, read client-facing help articles, and book appointment slots without emailing your dispatchers. You control access from **Settings → Portal** in two steps: choose which ticket boards are visible to clients, then create portal accounts for each client's contacts.

## Prerequisites

- Admin access to **Settings → Portal** (under **Delivery**).
- A client company created in Marula. Portal settings and users are isolated per client company, so a user you create for Acme Corp can never see tickets or invoices belonging to another client.
- At least one ticket board you want clients to submit requests into (usually your `Service Desk` board).

## 1. Expose the ticket boards clients should see

Open **Settings → Portal** and look at **Board Exposure Controls**.

Every board has an explicit **Exposed to client / Hidden** toggle. Internal boards (like `Infrastructure Maintenance` or `Internal Projects`) stay hidden by default. Flip the toggle to **Exposed to client** on the board where client requests should land.

## 2. Provision portal accounts for client contacts

In the **Portal Users Directory** on the same page, you can see every portal login along with their email, display name, contact role, and status.

Click **Provision portal user** to create an account for a client contact. From each row's action menu, your service desk can also **Disable** the account, **reset password**, send a sign-in link, or **Delete** the user.

For security, there is no public self-registration form on the portal, and the internal staff password-reset form never resets client portal accounts.

## 3. Share the portal URL and sign-in instructions

The client portal has its own dedicated sign-in screen (`"Marula — client portal sign-in"`) and navigation bar (**Tickets**, **Projects**, **Invoices**, **Quotes**, **Schedule**, **Help**, and **Documentation**). Send your client contact:

- Your portal URL
- Their provisioned email address and initial password (or magic sign-in link)
- A link to [How to use the client portal](client-portal-tasks.md) or the [Portal onboarding tutorial](../tutorials/portal-onboarding.md)

## 4. What portal accounts can never access

- **Internal tools and other clients** — portal sessions are blocked at the routing and database layers from internal notes, staff settings, the password vault, unified staff search, and other clients' records.
- **Help search** — a client search reaches only articles that are **published**, marked **client-facing**, and sitting in a client-facing category. See [Portal isolation](../explanation/portal-isolation.md) for the full security design.

## What you learned

- Expose the board first, then provision the client contact under **Settings → Portal**.
- Your service desk controls portal account creation and password resets per client organization.
