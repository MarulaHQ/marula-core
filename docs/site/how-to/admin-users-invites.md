# How to manage users and invites

> **Audience:** admin — invitations and access are settings-owner
> work, as are the account-security baselines below.

## Prerequisites

- Admin access to **Settings→Team** (PEOPLE & ACCESS).
- A sign-up email the invitee can reach, **or** a magic-link-capable
  mailbox. Marula's passwordless parity means you can invite without
  choosing anyone's password: magic-link sign-ins are "the same full
  sessions a password sign-in issues" (the auth surface says this
  explicitly — never treat passwordless accounts as second-class).

## 1. Invite the member

From Settings→Team, invite by email. The member receives a
**setup link**,
the acceptance page is gated on it ("This page needs the setup link your
administrator sent you (…/accept-invite?token=…)") and direct navigation
explains the precondition honestly.

## 2. Member completes setup

They open the setup link, set their method (password or the
magic-link/email path), and land signed in. Email verification is a
separate concern with its own lifetime: verification links are
**single-use and valid 24 hours**, with a resend flow. Verification
(confirms the mailbox) is not password recovery (resets access) — the two
flows answer different prompts.

## 3. Give each account its own recovery story

- Password reset issues a **single-use reset link**.
- Portal users are explicitly excluded from staff reset — portal
  passwords are reset by the service desk per the portal surface's own
  copy ("Your service desk provisions accounts and can reset this
  password"). See [portal setup](admin-portal-setup.md).

## 4. Harden the account

Lives in each user's own **Settings→Security**, and admins should direct
every member there at onboarding:

- **TOTP** two-factor setup (start setup from the card).
- **Sensitive changes gate**: disabling TOTP or regenerating recovery
  codes requires the account password **plus** the current code — even
  the account owner cannot silence MFA without the second factor.
- **Passkeys** (Touch ID / Windows Hello / security keys). Removal
  also requires the password + second factor.
- **Sessions**: the page lists live refresh-token chains, so stale
  devices are visible and actable.

## 5. Notifications per member, not per org

Per-user event matrix at **Settings→Notifications**: in-app/email toggles
with quiet-from/until/timezone rows. Quiet hours hold non-critical email
during the window — **SLA breaches and escalations always break through**.
Known rough edge, so you are not surprised: a few shipped events render as
raw slugs (`report_delivery`, `ai_cap_warn`, `secret_rotation`) without
friendly labels yet. The semantic names are honest even when the labels
are bare: report delivery, AI capacity warnings, secret rotation.
