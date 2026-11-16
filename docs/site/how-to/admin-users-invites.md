---
description: "Instead of creating temporary passwords and reading them over the phone, you invite teammates from Settings → Team."
---

# How to manage users and invites

> **Audience:** admin — invitations and access are settings-owner
> work, as are the account-security baselines below.

Instead of creating temporary passwords and reading them over the phone, you invite teammates from **Settings → Team**. Each person gets a single-use setup link where they can set a password or sign in with passwordless email links, enroll two-factor authentication, and set their notification hours.

## Prerequisites

- Admin access to **Settings → Team** (under **People & Access**).
- A working email address for the teammate you are inviting. Passwordless magic-link sign-ins create the exact same full session as a password login, so staff never have to manage a password if your team prefers email links.

## 1. Send the team invitation

Open **Settings → Team** and press **Invite member**. Inviting is a role
decision wearing an email form's clothes, so the dialog walks three steps:

1. **What this person will do.** Pick the intent that matches the work — the
   dialog recommends the built-in role that fits it.
2. **Give them the right role.** The recommendation is preselected and you can
   override it. Beside it, a permission preview reads the role as it will
   apply, in words. An override says so, so nobody mistakes a custom choice for
   the recommendation.
3. **Who are you inviting?** Name and work email. The invite is idempotent per
   attempt, so a double-click sends one invitation, not two.

On success, the invite is created and its email is queued in the same
transaction, so a person is never created without an email being queued for
them. Queued mail sends afterwards, and sending can still fail — if an
invitation does not arrive, the delivery ledger in
[Configure email](admin-email.md) is where to look. The dialog also shows the
setup link and its token once, for the case where email is slow or the person
is standing next to you: copy it and hand it over out of band. That link
cannot be fetched again.

The wording of the invitation itself is a template. To change it, edit the
**Workspace invitation** row in
[Email templates](admin-email-templates.md); the placeholders render the
inviter, the workspace and the expiry date at send time.

If you are still in the organisation setup wizard, its invite step renders the
same three-step flow. The words about what a role can do cannot drift between
the two surfaces, because both use the same component.

## 2. Teammate accepts the invitation

The invitation link opens `/accept-invite?token=…` as a short guided setup:

1. **Identity** — the invited address is named, and the person confirms the
   name they want to be called. A blank name is "no opinion", not an erasure of
   the name you recorded.
2. **Password** — at least the minimum length, typed twice. This is the one API
   call; every refusal is shown on this step.
3. **You're in** — confirm the timezone (saved to the account), with an
   optional card offering the desktop app on a plain browser. Then it lands on
   **Today**.

An invite that is unknown, already used or expired says so plainly and changes
nothing. If someone opens `/accept-invite` without a token, the page explains
that they need the setup link from their administrator.

Email verification links are **single-use and valid for 24 hours**, with a
one-click resend option if the link expires. Note that email verification
(confirming ownership of a mailbox) is separate from password reset (recovering
a locked account).

## 3. Password recovery for staff vs. portal clients

- **Staff accounts:** Requesting a password reset sends a single-use recovery link to the staff member's email.
- **Client portal accounts:** Portal users are excluded from the staff password-reset flow so the two login systems never mix. Your service desk resets portal passwords or sends portal sign-in links from **Settings → Portal** (see [Set up the portal](admin-portal-setup.md)).

> [!IMPORTANT]
> Removing a person ends their access, not their history. Reassign their open
> work before the last day, then remove the account.

## 4. Turn on two-factor authentication and passkeys

Have every new team member open **Settings → Security** on their first day to lock down their account:

- **Authenticator app (TOTP):** Scan the QR code to enable 6-digit verification codes and save the backup recovery codes.
- **Protected security changes:** Turning off TOTP or regenerating recovery codes requires entering the account password **and** a live 2FA code, so an unattended laptop cannot be stripped of MFA.
- **Passkeys:** Register Touch ID, Face ID, Windows Hello, or a hardware security key. Removing a passkey also requires re-verifying with password and second factor.
- **Active sessions:** Review signed-in devices and revoke any old refresh-token sessions immediately.

## 5. Set personal notification preferences and quiet hours

Each team member controls their own alert schedule under **Settings → Notifications**:

- Toggle **In-app** and **Email** delivery per event type.
- Set **Quiet hours** (start time, end time, and timezone) to hold routine email notifications overnight.
- **Urgent alerts always break through:** SLA breaches and priority escalations bypass quiet hours so an on-call lead never misses an emergency.

## Where to go next

- [Understand role permissions](admin-roles-and-permissions.md) — the matrix
  behind the recommendation, and how to fork a preset into a custom role.
- [Edit the emails Marula sends](admin-email-templates.md) — change the
  invitation wording without touching the flow.
- [Set up the portal](admin-portal-setup.md) — the separate account system for
  client contacts, which never mixes with staff sign-in.
- [Connect an IdP (SSO)](admin-sso.md) — if people should sign in through your
  identity provider instead of a Marula password.
