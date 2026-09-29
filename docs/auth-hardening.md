# Auth Hardening (Stage 2 — TASKLIST 2.3)

Implementation notes and verification guide for: audit-log completion + query
API, TOTP **and** WebAuthn MFA with single-use recovery codes, generic OIDC
SSO, and refresh-token reuse detection.

## What shipped

| Area | Detail |
|---|---|
| Migration | `crates/core-db/migrations/0013_auth_hardening.sql` — `totp_secrets` (AES-256-GCM at rest), `webauthn_credentials`, `recovery_codes` (argon2id hashes), `oidc_providers` / `oidc_accounts`, `mfa_challenges` (ceremony state + SSO one-time codes), refresh-token chain columns on `auth_sessions`, RLS policies on every new table, three SECURITY DEFINER bridges (`app_resolve_refresh`, `app_resolve_oidc_provider`, `app_list_enabled_oidc_providers`, `app_resolve_sso_code`) |
| Refresh reuse detection | Rotation now inserts a NEW session row (`chain_id`/`rotated_from`) instead of mutating in place. Presenting any consumed token resolves through `app_resolve_refresh` flagged inactive ⇒ the whole chain is revoked and `auth.refresh_reuse_detected` is audited |
| TOTP | RFC 6238 via `totp-rs`; ±1 window. Per-user replay watermark (`last_used_step`). Enroll begin → confirm (first code) → recovery batch issued once. Disable requires password + live factor |
| Recovery codes | 8 × argon2id-hashed, single-use enforced in SQL (`used_at IS NULL` guard), regenerate invalidates prior batch |
| WebAuthn | Passkey registration/authentication ceremonies via `webauthn-rs`. Challenge state persisted server-side (`mfa_challenges`), strictly single-use. Passkeys stored as public JSONB |
| OIDC SSO | Generic provider config (org-scoped). Authorization-code flow with signed state JWT (provider-bound), nonce echo check, PKCE S256 always on, RS256 id_token verification against provider JWKS, issuer pinning. Conservative linking (explicit authenticated link flow only), JIT provisioning default OFF and requiring a verified email claim |
| Audit query API | `GET /api/audit-events` gains actor / action-prefix / from–to filters plus opaque keyset cursor (`next_cursor`). Legacy `before` param still works |
| Audit emission | auth.register, login (+failures), mfa_challenge_issued, mfa_verify_failed (+reasons incl. replay), mfa_login_success (method-tagged), totp enabled/disabled/enroll-started, recovery regenerated, webauthn credential added/removed, logout, session_revoked, refresh_reuse_detected, oidc_provider_saved/deleted, oidc_login_success/failed, oidc_account_linked, oidc_jit_provisioned |
| Secrets rule | Provider client secrets and TOTP secrets sealed with AES-256-GCM (12-byte nonce prefix). Never returned after write (verified by test + e2e). Key: `MARULA_SECRET_KEY` env (64 hex chars) |
| Web UI | `/mfa` challenge screen (code or passkey), `/settings/security` (TOTP wizard, recovery display-once, passkey register/remove, session list + revoke), SSO buttons on `/login` driven by enabled providers, `/sso` handoff landing |

### Known gaps (honest)

- **At-rest encryption is app-level, not envelope encryption.** One AES-256
  key from `MARULA_SECRET_KEY` (dev fallback derives it from
  `JWT_SECRET`, loudly logged). Per-org data keys under a rotating master
  key remain the documented upgrade path. Call sites won't change.
- **JIT-provisioned users get the org's builtin Technician role** — role
  mapping per provider is not configurable yet.
- **No rate limiting** on login/MFA endpoints yet (Stage-4 hardening).
- **TOTP setup shows secret + otpauth URI for manual entry. No QR image**
  rendered in-app (avoids an extra dependency. Authenticator apps accept
  manual entry or pasting the URI).

## Environment

```sh
docker run -d --name marula-auth -e POSTGRES_USER=marula \
  -e POSTGRES_PASSWORD=marula -e POSTGRES_DB=marula \
  -p 25434:5432 pgvector/pgvector:pg17

export DATABASE_URL=postgres://marula:marula@localhost:25434/marula
cd crates/core-db && sqlx migrate run && cd ../..

# core-api (compose parity: owner role)
env DATABASE_URL=$DATABASE_URL BIND_ADDR=0.0.0.0:18185 \
    JWT_SECRET=smoke-secret-18185 WEB_BASE_URL=http://localhost:3000 \
    ./target/debug/core-api
```

## Automated evidence

- `cargo test --workspace` (126 tests) includes:
  - RFC 6238 Appendix B vectors (SHA-1) through the verification primitive,
    drift-window acceptance, malformed-code rejection.
  - TOTP replay watermark rejection at the SQL layer.
  - recovery-code single-use IN SQL + regeneration wipe.
  - ceremony one-shot semantics (second finish refused).
  - RLS isolation on every new table (virgin pool = zero rows. Cross-org invisible).
  - audit filter/cursor walk without overlap. Cross-org events never visible.
  - OIDC provider metadata never exposing ciphertext. Link uniqueness.
  - state codec tamper/expiry/type checks. Conservative decision matrix.
  - full HTTP loop against a real in-process mock IdP:
    `cargo test -p core-api --test oidc_flow`
    (explicit link → SSO login → tokens via single-use handoff code. Unlinked subject ⇒ `link_required`. JIT-off refusal vs JIT-on provisioning. Forged state ⇒ redirect w/ error. Nonce mismatch ⇒ 401. Secret never in responses).

## Manual verification steps

### TOTP end-to-end (curl)

1. Register: `POST /api/auth/register` → tokens.
2. `POST /api/auth/mfa/totp/enroll-begin` (Bearer) → `secret_base32`.
3. Compute the current code (any RFC 6238 app):
   `oathtool --totp -b <SECRET>` or paste the `otpauth_url` into an
   authenticator app.
4. `POST /api/auth/mfa/totp/enroll-verify {"code":"<code>"}` → 8 recovery
   codes (displayed exactly once).
5. `POST /api/auth/login` → `{"mfa_required":true,"mfa_token":…}`.
6. Wrong code ⇒ 401. Right code ⇒ TokenPair. Reusing the SAME code again
   ⇒ 401 (replay watermark) even within its window.

### WebAuthn passkeys (browser required)

Hardware-bound ceremonies cannot be driven by curl. With `apps/web` running:

1. Sign in, open **Settings → Security**, enter a label, click
   **Register passkey** and approve the platform-authenticator prompt.
2. The passkey appears in the list. Sign out, sign in with password ⇒ the
   MFA challenge offers **Use a passkey**. Approve the prompt ⇒ signed in.
3. Removal requires password re-entry.

Endpoint-level behavior without hardware is covered by the automated smoke:
begin returns browser-ready CreationOptions. A bogus attestation is refused
and the ceremony state is consumed (finishing twice fails even with valid
hardware material).

### OIDC against a local mock IdP (manual replica of the automated e2e)

The automated suite spins its own IdP. To reproduce manually:

1. Run any compliant stub IdP on `127.0.0.1:<port>` exposing discovery,
   `/authorize`, `/token`, JWKS (RS256). A ready-made one runs inside
   `cargo test -p core-api --test oidc_flow` — set `RUST_LOG=debug` and copy
   the printed endpoints if you want to poke it with curl.
2. As an admin: `PUT /api/auth/oidc/providers/{name}` with
   `{issuer, discovery_url, client_id, client_secret, jit_provisioning:false}`.
3. `GET http://localhost:18185/api/auth/oidc/{name}/login` → 303 to the IdP.
4. Authenticate. The provider redirects back to
   `/api/auth/oidc/{name}/callback?code=…&state=…`.
5. Callback validates state signature/expiry/provider binding + nonce echo,
   then 303s to `{WEB_BASE_URL}/sso?code=<one-time>`.
6. `POST /api/sso/exchange {"code": …}` → TokenPair (single use with a 60 s TTL).
7. Unlinked identity (JIT off) lands on `/sso?error=link_required` instead.

### Refresh reuse detection (curl)

1. Login → `(T₀, S₀)`. Refresh → `(T₁, S₁)`. Refresh again → `(T₂, S₂)`.
2. Re-present `(T₀, S₀)` ⇒ 401 AND `auth.refresh_reuse_detected` audited.
3. `(T₂, S₂)` is now dead too — the whole chain was revoked.

## Identity federation bundles (planned — the seam for community auth)

What shipped above (generic OIDC + conservative linking + JIT-off-by-default)
is the foundation of a public, versioned **identity-federation bundle
interface**. Plan of record:

- **First-party OIDC presets** (Google, Microsoft Entra ID/M365, Okta,
  generic): declarative config bundles — issuer, scopes, claim mapping,
  redirect guidance. Picking a preset is configuration, not code.
- **Edge-assertion method for ZTNA proxies** (Cloudflare Access pattern):
  validate the proxy's signed JWT (`Cf-Access-Jwt-Assertion`) against the
  team JWKS with issuer/audience pinned, then resolve the asserted identity
  to a user under the same conservative linking rules as OIDC (verified
  email, explicit link flow, JIT default OFF). Raw header trust without
  signature verification is never valid — this hard line is what makes
  "self-hosted behind Cloudflare Tunnel + Access" a supported topology
  instead of a header-spoofing risk.
- **Everything else is a bundle.** Community or commercial bundles cover
  the rest (SAML/SCIM brokers, directory connectors, edge-identity proxies),
  admin-enabled and audit-logged, off by default.

Invariants every bundle inherits (non-negotiable):

1. Account linking requires a verified email claim. Explicit link flow. JIT provisioning defaults off.
2. The org's MFA policy applies regardless of identity source.
3. Logins land in the same session model (short-lived JWT + rotating
   refresh with reuse detection) — a bundle adds an identity source,
   never a parallel session system.
4. Every bundle login emits the standard auth audit events.
