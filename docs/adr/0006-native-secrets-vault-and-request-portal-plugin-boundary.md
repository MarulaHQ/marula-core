# ADR 0006 — Native secrets vault: KB-integrated category trees, org-level vault, and the request-portal plugin boundary

**Status:** Accepted (user directive) · 2026-09-16
**Scope:** native vault (`/passwords` + `VaultEntryView`, Wave 13 lane 7), knowledge base
surface, plugin capability model, and the closed-side (Relay) secret-request portal

## Context

The shipped vault is password-centric: `PasswordEntry` rows (AES-256-GCM sealed),
metadata-only lists, audited `/passwords/{id}/reveal`, and vault entries linked to
assets. The vault was treated as a per-client browsing surface rather than
knowledgebase-class knowledge. Two directives now shape where the vault goes:

1. **The vault and the means to view things are base functionality of the open
   core.** Secrets are knowledgebase-class records: users navigate the KB, select a
   client, and browse organized secret categories; the structure is seeded per secret
   type, admins customize defaults per client, and users add folders. Secrets link to
   other records (passwords ↔ configurations/apps/articles) like every KB record. The
   **org-level vault** rides the same model with category/record-level RBAC — a tech
   reads internal-process KB articles but cannot open the "MSP API secrets" category
   without the grant.
2. **Unauthenticated ingress (secret request links / client portal drop-off) is a
   Relay plugin, not core.** Architecturally it rides a *public vault-plugin capability
   interface* so open-source developers can build their own ingress-style plugins. The
   Relay implementation (REL-V1 in planning docs) is zero-knowledge: payloads are
   encrypted client-side, the server stores ciphertext and wrapped key material only.

## Decision

- **Category trees over flat rows.** Secret records live in client- or org-scoped
  category trees (seeded defaults: passwords / API keys / certificates / recovery
  codes / documents) following the template-registry doctrine (ADR 0004): shipped
  defaults are immutable seed rows — orgs fork to customize; children folders are
  first-class data.
- **Org-level scope with category-level RBAC.** The vault gains the org scope beside
  the client scope; visibility is gated per category through the permission matrix
  (`secrets:read` module permission + per-category ACL grants), preset roles updated
  accordingly. Financial-visibility pattern (its own permission) applies: MSP admin
  secrets are hidden from regular techs by default.
- **Record links are first-class.** Secret records participate in the existing
  cross-entity link graph (passwords ↔ ConfigItem/app/KB article/ticket) via the same
  `RecordLink` seam the asset related-items panel uses.
- **Ingress is ciphertext-only, plugin-shaped.** Core exposes a *ciphertext-only*
  vault-ingress hook under a `vault:ingress` capability scope: plugins may submit
  encrypted payloads (client-side E2E, key in URL fragment, wrapped to the org key)
  into a scoped folder. The core never accepts plaintext it has no business seeing
  from unauthenticated surfaces. Relay's Secret Request Portal (relay-psa docs 03
  REL-V1) is the first-party plugin and holds no core privileges beyond this scope.
- **Vault remains self-contained.** No external vault service becomes a dependency of
  the open core (STRATEGY open-adapters doctrine + one-PostgreSQL doctrine); external
  vaults (Bitwarden/1Password/Hudu) stay plugin connectors/deep links.

## Consequences

- Vault v1 category trees + org scope + record links land with the knowledge surface
  work (tracked in the private build tracker); migration head pairs with the KB lanes.
- The `vault:ingress` interface lands with plugin runtime v2; Relay's REL-V1 portal
  (M18–M24) depends on it. Third-party ingress plugins inherit: audited, rate-limited,
  quota-governed ingress — noisy-neighbor protection is a platform property, not a
  plugin-author obligation.
- Rotation automation, one-time share links, and password-side link-management UI stay
  tracked as vault follow-ups; this ADR governs structure first, not those features.
