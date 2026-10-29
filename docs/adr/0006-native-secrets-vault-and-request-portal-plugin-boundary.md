# ADR 0006 — Native secrets vault: KB-integrated category trees, org-level vault, and the request-portal plugin boundary

**Status:** Accepted · 2026-09-16
**Scope:** native vault (`/passwords` + `VaultEntryView`), knowledge base
surface, plugin capability model, and the commercial secret-request portal

## Context

The shipped vault is password-centric: `PasswordEntry` rows (AES-256-GCM
sealed), metadata-only lists, audited `/passwords/{id}/reveal`, and vault
entries linked to assets. The vault was treated as a per-client browsing
surface rather than knowledgebase-class knowledge. Two requirements now shape
where the vault goes:

1. **The vault and the means to view things are base functionality of the open
   core.** Secrets are knowledgebase-class records: users browse the KB,
   select a client, and browse organized secret categories. The structure is
   seeded per secret type, admins customize defaults per client, and users add
   folders. Secrets link to other records (passwords ↔
   configurations/apps/articles) like every KB record. The **org-level vault**
   rides the same model with category/record-level RBAC, so a tech reads
   internal-process KB articles but cannot open the "MSP API secrets" category
   without the grant.
2. **Unauthenticated ingress (secret request links / client portal drop-off)
   is a commercial plugin, not core.** Architecturally it rides a *public
   vault-plugin capability interface* so open-source developers can build
   their own ingress-style plugins. The commercial Secret Request Portal
   implementation is zero-knowledge: payloads are encrypted client-side, and
   the server stores ciphertext and wrapped key material only.

## Decision

- **Category trees over flat rows.** Secret records live in client- or
  org-scoped category trees (seeded defaults: passwords / API keys /
  certificates / recovery codes / documents) following the template-registry
  doctrine (ADR 0004). Shipped defaults are immutable seed rows: orgs fork to
  customize, and child folders are first-class data.
- **Org-level scope with category-level RBAC.** The vault gains the org scope
  beside the client scope. Visibility is gated per category through the
  permission matrix (`secrets:read` module permission + per-category ACL
  grants), and preset roles update accordingly. The financial-visibility
  pattern (its own permission) applies here too: MSP admin secrets are hidden
  from regular techs by default.
- **Record links are first-class.** Secret records participate in the existing
  cross-entity link graph (passwords ↔ ConfigItem/app/KB article/ticket) via
  the same `RecordLink` seam the asset related-items panel uses.
- **Ingress is ciphertext-only, plugin-shaped.** Core exposes a *ciphertext-only*
  vault-ingress hook under a `vault:ingress` capability scope: plugins may
  submit encrypted payloads (client-side E2E, key in URL fragment, wrapped to
  the org key) into a scoped folder. The core never accepts plaintext it has
  no business seeing from unauthenticated surfaces. The commercial Secret
  Request Portal is the first-party plugin, and it holds no core privileges
  beyond this scope.
- **Vault remains self-contained.** No external vault service becomes a
  dependency of the open core (the open-adapter and one-PostgreSQL doctrines).
  External vaults (Bitwarden/1Password/Hudu) stay plugin connectors and
  deep links.

## Consequences

- Vault v1 category trees, org scope, and record links land with the knowledge
  surface work, and the migration head pairs with the KB migrations.
- The `vault:ingress` interface lands with plugin runtime v2, on which the
  commercial portal depends. Third-party ingress plugins inherit the same protections. Their ingress is
  audited and rate-limited, and noisy-neighbor quota governance is a
  platform property rather than a plugin-author obligation.
- Rotation automation and one-time share links stay tracked as vault
  follow-ups. So does the password-side link-management interface. This ADR
  governs structure first, not those features.
