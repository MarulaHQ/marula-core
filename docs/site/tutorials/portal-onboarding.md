# Tutorial: portal onboarding

> **Audience:** both — steps 1–3 are admin (provisioning and exposure);
> steps 4–5 are the client's first minutes and can be handed to them.

Your clients shouldn't have to email your helpdesk just to check if a ticket is being worked on, download last month's invoice, or approve a hardware quote. Marula includes a built-in **Client Portal** where your customer's contacts can log requests, read client-facing help articles, pay or review invoices, and book appointment slots.

To prevent accidental data leaks, the portal runs in its own isolated sign-in zone. There is no public "sign up" button: you decide which ticket boards are visible and explicitly invite each client contact under **Settings → Portal**.

## Prerequisites

- Admin access for steps 1–3, plus a private browser window (or second browser profile) to test signing in as the client in step 4.
- A client company in the seeded demo org.

## 1. Choose which ticket boards clients can see

Sign in as an admin and open **Settings → Portal** to view **Board Exposure Controls**.

Every ticket board is marked either **Exposed to client** or **Hidden**. Boards you use for internal infrastructure work or private projects stay hidden until you explicitly expose them. Turn on exposure for the **Service Desk** board where you want clients to submit new support requests.

## 2. Create a login for your client contact

Stay in **Settings → Portal** and scroll to the **Portal Users Directory**. Here you see every portal account along with their email, display name, contact role, and active status.

Click **Provision portal user** to create a login for a contact at a specific client company. Portal accounts always belong to one client organization; they can never see records from your other clients. From the directory row menu, your service desk can also reset a user's password, send a sign-in link, disable their access, or delete the account.

## 3. Share the portal sign-in details with your client

Open the portal sign-in page (`Marula — client portal sign-in`). It uses a dedicated navigation bar (**Requests**, **Invoices**, **Knowledge**, and **Schedule**) and displays **"Support access provided by your service desk"** in the footer.

Three things to know when handing logins to a customer:

- **Your team controls account access.** You provision accounts and manage password resets or magic sign-in links from the admin directory.
- **Staff password recovery stays separate.** The internal staff password-reset endpoint rejects portal email addresses so the two login realms never mix.
- **Strict boundary between staff and client logins.** A portal session can never open internal staff pages (`/today`, `/tickets`, `/settings`), and a staff session is not used on the client portal.

## 4. Walk through the client's first request

Sign in as the portal user you created. You land on **Requests**:

1. Click to submit a new request on the exposed Service Desk board. On your team's side, this immediately appears as a normal ticket tagged with the `portal` channel.
2. Reply on the request thread. Note that internal notes written by your technicians never appear here; the client only sees client-visible replies.
3. Click **Invoices** to view issued bills, **Knowledge** to search help articles your team marked as client-visible, or **Schedule** to request an appointment window.

## 5. What the client can never reach

Portal users are fenced off at the database and API routing layers from your internal notes, other clients' tickets, staff settings, the password vault, and audit logs. Even the search bar in the portal runs against a separate query path restricted to published, client-visible articles for that client. Read [Portal isolation](../explanation/portal-isolation.md) for the security architecture behind this boundary.

## What you learned

- You control portal access in two steps under **Settings → Portal**: expose the board, then provision the contact inside their client organization.
- Portal logins and sessions, along with portal search queries, are completely separated from your internal staff workspace.
- Clients get four clean self-service tabs: **Requests**, **Invoices**, **Knowledge**, and **Schedule**.
