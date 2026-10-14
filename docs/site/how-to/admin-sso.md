# How to connect an IdP (SSO)

> **Audience:** admin — IdP wiring is a settings-owner decision; every
> staff and portal account experiences the round-trip you configure here.

Single Sign-On (SSO) lets your staff sign in to Marula through your company's identity provider (such as Google Workspace, Microsoft Entra ID, Okta, or Keycloak) using the same credentials and security keys they already use for email.

## Prerequisites

- Admin access to **Settings** (authentication provider configuration) and admin permissions in your identity provider (IdP) to register an OIDC client.
- If you want the wire-level OIDC specification and claim rules, see the [auth provider contract](../../reference/auth-provider-contract.md).

## How the SSO sign-in round-trip works

Every SSO login follows a three-step redirect handshake:

1. **Start at Marula's login page:** The staff member clicks your SSO provider on `/login`, and Marula redirects their browser to your IdP. (Bookmarking or opening the internal SSO callback URL directly without starting at login shows `"Missing handoff code"` because no sign-in state was started.)
2. **Authenticate at your IdP:** Your IdP verifies the user's password or passkey along with corporate MFA policies.
3. **Return with a one-time handoff code:** Your IdP redirects back to Marula with a short-lived code, which Marula exchanges for a full staff session.

## 1. Configure your identity provider

Open the authentication provider settings in Marula and enter the client ID, client secret, and redirect URI registered in your IdP. If you run Marula behind a reverse proxy, make sure your public hostname matches the redirect URI in your IdP (see the [Reverse proxy & TLS reference](../reference/reverse-proxy-tls.md)).

### Use a built-in provider preset

Instead of copying OIDC discovery and issuer URLs by hand, pick one of the built-in presets:

- **Google Workspace**
- **Microsoft Entra ID** (enter your tenant ID)
- **Okta** (enter your Okta domain)
- **Keycloak** (enter your host URL and `realm`)

Marula derives the discovery endpoints automatically and pins the issuer URL for token verification.

If you use another standard OpenID Connect provider (such as Ory Hydra, Ory Network, or Authentik), choose the custom OIDC option and supply the issuer and discovery URLs directly. (Note that a bare Ory Kratos instance without an OIDC layer does not publish an OIDC discovery document; see the [auth provider contract](../../reference/auth-provider-contract.md).)

## 2. Test the full sign-in loop

Open a private browser window and verify the entire flow:

1. Start from `/login` and click your SSO button.
2. Complete sign-in at your IdP.
3. Confirm you land in the Marula workspace with your expected staff role.

## 3. What SSO does and does not change

- **Full session parity:** Sessions started via SSO or passwordless magic links have the exact same capabilities as password sign-ins.
- **In-app security changes still require verification:** Changing two-factor settings or recovery codes inside [Settings → Security](admin-users-invites.md#4-turn-on-two-factor-authentication-and-passkeys) stays protected.
- **Client portal accounts remain separate:** Staff SSO applies to your internal team. Client portal logins stay isolated per client organization under [Settings → Portal](admin-portal-setup.md).

## What you learned

- Built-in presets for Google Workspace, Entra ID, Okta, and Keycloak configure your OIDC endpoints automatically.
- Always start SSO sign-ins from `/login` so Marula can verify the round-trip handoff code.
