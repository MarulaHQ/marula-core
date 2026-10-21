# First-party auth-provider contract (Wave 19 lane 5, Scope C)

Decided 2026-09-17 (user + lane): ONE resolution path generalizes the
Wave 13 federation pattern (edge-assertion exchange, OIDC callback)
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
   prior links. JIT only where the ORG opted in AND the provider verified
   the email. Unlinked identities get the uniform
   `edge_identity_unlinked`-class denial.
2. org MFA gate (the same `federated_mfa_gate` password login uses).
3. session issuance by the core: `handlers::issue_tokens` — auth_sessions
   + hashed refresh rotation (cookies where the client is a browser) +
   audit best-effort — shared with password login, never a parallel
   system.

## Provider discipline

* **First-party, admin-registered** like edge sources: admin CRUD, fields
  pinned and enforced byte-for-byte (issuer/audience/JWKS for edge, plus
  OIDC discovery and pinned client config).
* **Deny-by-default**: created disabled. Enabling is an explicit, audited
  admin action. Unknown and disabled are indistinguishable rejections.
* **Audited end-to-end**: source saved/deleted, link created/removed,
  login success/failure (incl. `amr`).
* **NOT in the plugin capability vocabulary**: there is no `auth:*` plugin
  scope and there never will be one (plugin-host doctrine: "no parallel
  auth system"). A provider cannot be shipped as a plugin. It is a
  first-party admin-registered source exactly like an edge source.

## Headless IdPs (updated Wave 34 lane 6)

**Keycloak — SHIPPED as an OIDC preset.** A Keycloak deployment serves
standard OIDC, so it needs no new KIND: the `keycloak` preset templates
`https://{host}/realms/{realm}` (issuer) and its discovery document, and
an admin creates the provider by naming the preset and filling the
`host` + `realm` parameters. Verification is the same RS256/JWKS path
every OIDC provider uses.

**Ory Kratos — typed NOT-BUILT (two supported paths today).**

* Kratos fronted by an OIDC server (Ory Hydra, or Ory Network's hosted
  OIDC front) speaks standard OIDC. Create a provider WITHOUT a preset
  and pin its issuer + discovery URL. Nothing Kratos-specific is needed.
* A bare Kratos deployment exposes its own session API, not OIDC
  discovery. Wiring that natively means the seam's first
  **session-introspection provider KIND**: an admin-registered source
  that verifies a Kratos session token out-of-band and resolves it into
  a `VerifiedPrincipal` through `resolve_and_issue`. That kind is not
  built. It stays inside this contract's discipline (first-party,
  deny-by-default, audited, sessions still issued by core) and never
  becomes a plugin capability.
* Escalation trigger: an operator runs bare Kratos and cannot add an
  OIDC front. The work is a new provider-kind adapter plus its audited
  admin CRUD. The seam itself (`AUTH_PROVIDER_INTERFACE_VERSION = 1`)
  already carries it.

## Directory groups (added Wave 52 lane R10)

A provider answers **who someone is**. It does not answer **what they may do**,
and until migration 0148 core had nowhere to put the second answer in the shape
a directory produces: `user_roles` holds one row per user per role, so a group
of twelve technicians had to be flattened into twelve rows and could only be
undone twelve times.

Three tables carry it: `directory_groups` (one row per group, with an optional
`external_id` that is unique per org, which is how a second sync pass finds the
row it wrote last time instead of creating a duplicate), `directory_group_members`
and `directory_group_roles`. All three are RLS-scoped to the org; the two child
tables inherit the scope from their group.

**Effective roles are the additive union** of `user_roles` and the roles reachable
through group membership. There is no deny grant, so there is no precedence rule
to write down: removing a membership takes away what that group alone granted and
leaves every other grant in place. `core_db::users::effective_roles_for_user` is
the one read that answers it, and `core_api::auth::middleware::principal_for` is
the one place a principal is assembled from it, so a group-granted role reaches
the same `Principal.permissions` a directly assigned role reaches. The direct
read stays available as `direct_roles_for_user` for the org-users editor, which
shows and edits assignments rather than effective authority.

**The sync protocol is not core.** Core ships the storage and the API
(`/api/directory-groups`, members, role mappings) and the `directory_group.*`
events; a product that speaks a directory protocol writes through that API and
keeps its protocol, its credentials and its reconciliation loop. There is no
SCIM endpoint here, no sync token, and no tier check in this code path.

## Shipped provider kinds (v1)

| Kind | Source registry | Identity claim | JIT |
|---|---|---|---|
| `oidc` (incl. presets `google` / `entra` / `okta` / `keycloak`) | `oidc_providers` (+ `oidc_accounts` links) | id-token `sub` binding | opt-in, verified-email claim required |
| `edge_assertion` | `edge_assertion_configs` (pinned iss/aud/JWKS) | proxy JWT `sub` via pinned team JWKS | never (no verified-email authority) |
| password | local | argon2 verify | — |
| magic-link (lane 5) | one-use token table, org opt-in | email ownership proof | — |

Each kind shares the same link→mfa→issue path. The ORG decides what is
enabled, and every step is typed and audited.
