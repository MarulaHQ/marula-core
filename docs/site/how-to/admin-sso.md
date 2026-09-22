# How to connect an IdP (SSO)

> **Audience:** admin — IdP wiring is a settings-owner decision; every
> staff and portal account experiences the round-trip you configure here.

## Prerequisites

- Admin access to the authentication provider configuration (Settings,
  under the auth surface family) and a worked relationship with your
  chosen IdP.
- The
  [auth provider contract](../../reference/auth-provider-contract.md) —
  this page is the operator walk-through. The contract supplies the
  wire-level truth.

## What the round-trip actually is

The SSO surface itself demonstrates the loop's failure mode honestly:
deep-linking the SSO page without an IdP handoff renders the explainable
error **"Missing handoff code"**. There is no shortcut SSO — the flow is
always:

1. **Initiate at login.** The user starts sign-in at Meridian's login
   surface, which redirects to your IdP.
2. **IdP authenticates.** Whatever your IdP requires (password, passkey,
   its own MFA) happens on its site, not Meridian's.
3. **Handoff back.** The IdP returns a handoff code to Meridian, and
   the SSO completion exchanges that code for a Meridian session.

## 1. Configure the provider

Wire the provider settings in the auth configuration surface: client
credentials or metadata your IdP defines, redirect URIs pointing back at
your deployment. Deployments behind a reverse proxy should keep the
redirect URIs consistent with the [TLS runbook's](../reference/reverse-proxy-tls.md)
public URL — redirect mismatches fail at the IdP, not at Meridian.

## 2. Verify the whole loop, not half of it

Test with a real account:

- sign-in start from the login page (not a deep-linked SSO URL — that is
  the "Missing handoff code" path),
- IdP completes,
- the redirected handoff lands the user back signed in with the same
  session behavior as normal staff sign-ins.

## 3. What SSO does NOT change

- **MFA is not bypassed by other means**: sensitive changes keep the
  password + current-code gate inside [Settings→Security]
  (admin-users-invites.md#4-harden-the-account) regardless of SSO.
- **Passwordless parity holds**: magic-link and SSO both issue
  "the same full sessions a password sign-in issues" — no second-class
  sessions on either path.
- **Portal auth stays separate**: IdP wiring for staff sign-in is not a
  portal identity migration — portal provisioning and resets remain
  service-desk-side ([portal setup](admin-portal-setup.md)).

## What you learned

- SSO is a redirect round-trip and nothing else in Meridian's posture.
  Handoff codes are the proof, and their absence fails closed.
- The provider contract document is authoritative for anything wire-level.
