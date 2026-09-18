# First-party auth-provider contract (Wave 19 lane 5, Scope C)

Decided 2026-09-17 (user + lane): ONE resolution path generalizes the
Wave 13 federation pattern — edge-assertion exchange, OIDC callback —
into a provider seam whose ONLY job is to resolve a credential/assertion
into a principal. **Sessions and tokens never cross it.**

## The seam (modeled on FEDERATION_BUNDLE_INTERFACE_VERSION)

`core_api::auth::provider` pins:

```
AUTH_PROVIDER_INTERFACE_VERSION = 1
```

Types:

* `VerifiedPrincipal` — what a provider VERIFIED and nothing more:
  `{ subject, email, email_verified, display_name, amr }`.
* `ProviderContext` — `{ namespace (LinkNamespace: oidc provider id or
  bundle kind+source id), org_id, display_source, jit_allowed }`.
* `ProviderLoginOutcome` — tokens or the org MFA answer, identical to
  password login's shapes.

`resolve_and_issue(state, ctx, verified)` is the ONE login path every
first-party provider routes through:

1. conservative link resolution (`federation::find_link_user`) — explicit
   prior links; JIT only where the ORG opted in AND the provider verified
   the email; unlinked identities get the uniform
   `edge_identity_unlinked`-class denial.
2. org MFA gate (the same `federated_mfa_gate` password login uses).
3. session issuance by the core: `handlers::issue_tokens` — auth_sessions
   + hashed refresh rotation (cookies where the client is a browser) +
   audit best-effort — shared with password login, never a parallel
   system.

## Provider discipline

* **First-party, admin-registered** like edge sources: admin CRUD, fields
  pinned and enforced byte-for-byte (issuer/audience/JWKS for edge; OIDC
  discovery + pinned client config).
* **Deny-by-default**: created disabled; enabling is an explicit, audited
  admin action; unknown and disabled are indistinguishable rejections.
* **Audited end-to-end**: source saved/deleted, link created/removed,
  login success/failure (incl. `amr`).
* **NOT in the plugin capability vocabulary**: there is no `auth:*` plugin
  scope and there never will be one (plugin-host doctrine: "no parallel
  auth system"). A provider cannot be shipped as a plugin; it is a
  first-party admin-registered source exactly like an edge source.

## Future option: commercial headless IdP (NOT built in this lane)

A commercial headless IdP — **Ory Kratos** or **Keycloak** — would assert
through this seam as a future provider kind, the same way a new edge
source kind would: an admin registers the source (pinned JWKS/issuer/base
URL), the provider adapter verifies credentials/assertions, and
`resolve_and_issue` resolves it into the SAME session machinery. Nothing
about the plug-in system changes; the integration rides at the provider
registry with an audited enable, not inside a plugin sandbox. That keeps
the non-pluggable guarantee: a plugin can never mint, rotate, or observe
sessions regardless of what it can otherwise touch.

## Shipped provider kinds (v1)

| Kind | Source registry | Identity claim | JIT |
|---|---|---|---|
| `oidc` (incl. presets) | `oidc_providers` (+ `oidc_accounts` links) | id-token `sub` binding | opt-in, verified-email claim required |
| `edge_assertion` | `edge_assertion_configs` (pinned iss/aud/JWKS) | proxy JWT `sub` via pinned team JWKS | never (no verified-email authority) |
| password | local | argon2 verify | — |
| magic-link (lane 5) | one-use token table, org opt-in | email ownership proof | — |

Each kind shares the same link→mfa→issue path; the ORG decides what is
enabled, and every step is typed and audited.
