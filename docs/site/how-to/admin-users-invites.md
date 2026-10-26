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

Open **Settings → Team** and invite the new team member by email. Marula generates a unique **setup link** (`/accept-invite?token=…`). If someone opens `/accept-invite` without a valid token in the URL, the page explains that they need the setup link from their administrator.

## 2. Teammate accepts and verifies their email

When the invitee opens their setup link, they choose their sign-in method (password or magic link) and land directly in the workspace.

Email verification links are **single-use and valid for 24 hours**, with a one-click resend option if the link expires. Note that email verification (confirming ownership of a mailbox) is separate from password reset (recovering a locked account).

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
