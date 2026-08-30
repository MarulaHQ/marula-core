# ADR 0004 — Content portability & shipped permissions

**Status:** Accepted · 2026-08-30
**Scope:** knowledge-base content model (embeds + external documents),
the template registry, and the permission model's shipped roles

## Context

Three everyday surfaces were underspecified in the build prompt:

1. **KB content stopped at "articles".** Real runbooks embed videos, and much
   company knowledge lives in SharePoint/OneDrive/Drive. An article that can't
   embed or link out gets abandoned; but raw arbitrary iframes are an injection
   surface (the widget-registry philosophy already says: registered components
   decide what may render, never free-form HTML).
2. **Templates were fragmented.** Ticket templates and project templates
   existed as unrelated features; canned responses, email templates, and KB
   article templates were unowned. N systems where one should be.
3. **RBAC had a matrix but no shipped roles**, and nothing gated financial
   data — nothing stopped a tech from seeing contract rates or margin panels.

Existing foundations: custom fields and semantic categories ("behavior binds
to category, never label") already ship; the permission matrix and RLS
backstop are in place; DocumentVersion history already names "templates" as
versioned content (docs/10-agent-platform.md §6).

## Decision

**KB content model** (planning A13 · gap-analysis B8-adjacent):

- **Allowlisted oEmbed embeds.** YouTube/Loom/Vimeo-class providers via a
  registry, extensible through the plugin/widget registration path. Raw
  arbitrary iframes are never allowed; embedding requires a registered,
  schema-validated provider.
- **First-class linked external documents.** SharePoint/OneDrive/Drive URLs
  stored as a typed resource (title, URL, permission note, last-verified
  link-check) rather than a paste-in-Markdown. Company knowledge that lives
  elsewhere is linked, not trapped; client-facing visibility flags apply to
  embeds and links exactly as to body text.

**Template registry** (planning A13):

- One `Template` entity, typed: ticket / note snippet / email / KB article /
  quote-invoice layout / project scaffold / workflow playbook.
- Typed variables (`{{client.name}}`, `{{ticket.summary}}`) resolved from
  entity context at insertion time — never string-interpolated accidentally
  into SQL or rendered content without escaping.
- Shipped defaults are **immutable seed rows**: orgs fork, never edit in
  place (defaults-first doctrine). Org-created templates are first-class,
  shareable later; all templates are customizations-as-data under the
  upgrade-compatibility policy.

**Permissions** (planning A13):

- **Preset roles ship as data**, not code: Admin, Dispatcher, Technician,
  Senior Tech, Bookkeeper, Read-only/Auditor, plus the two portal tiers —
  built on the same permission matrix as custom roles. Orgs duplicate, edit,
  and restore them; system presets are recoverable rows, not hardcoded logic.
- **`financial:read` is its own permission**, gating cost rates, contract
  rates, and margin panels. Without it those values render blank. This is the
  explicit anti-"tech sees what the client pays by accident" rule.

## Consequences

- KB embeds and linked documents land with the Phase-3 `core-kb` crate;
  the registry surface there must be the same registration path as widget
  registry, not a parallel allowlist.
- The template registry ships incrementally (ticket/project/email types in
  Phase 3, KB/quote-invoice types at Phase 4), all on one `Template` entity —
  no per-type template tables.
- Preset roles exist from the shipped permission matrix; `financial:read`
  gates are checked in the policy module, testable per existing RBAC tests.
- These are customizations-as-data: org manifests (U5) and upgrade preflight
  (U6) cover template rows and role rows like any other customization.