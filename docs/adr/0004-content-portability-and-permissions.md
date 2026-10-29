# ADR 0004 — Content portability & shipped permissions

**Status:** Accepted · 2026-08-30
**Scope:** knowledge-base content model (embeds + external documents),
the template registry, and the permission model's shipped roles

## Context

Three everyday surfaces were underspecified in the original build scope:

1. **KB content stopped at "articles".** Real runbooks embed videos, and much
   company knowledge lives in SharePoint/OneDrive/Drive. An article that can't
   embed or link out gets abandoned. But raw arbitrary iframes are an injection
   surface, and the widget-registry philosophy already settles who decides that:
   registered components decide what may render, never free-form HTML.
2. **Templates were fragmented.** Ticket templates and project templates
   existed as unrelated features. Canned responses, email templates, and KB
   article templates were unowned. N systems where one should be.
3. **RBAC had a matrix but no shipped roles**, and nothing gated financial
   data. Nothing stopped a tech from seeing contract rates or margin panels.

Existing foundations: custom fields and semantic categories ("behavior binds
to category, never label") already ship, and the permission matrix and RLS
backstop are in place. DocumentVersion history already names "templates" as
versioned content (docs/10-agent-platform.md §6).

## Decision

**KB content model:**

- **Allowlisted oEmbed embeds.** YouTube/Loom/Vimeo-class providers via a
  registry, extensible through the plugin/widget registration path. Raw
  arbitrary iframes are never allowed; embedding requires a registered,
  schema-validated provider.
- **First-class linked external documents.** SharePoint/OneDrive/Drive URLs
  stored as a typed resource (title, URL, permission note, last-verified
  link-check) rather than a paste-in-Markdown. Company knowledge that lives
  elsewhere is linked, not trapped. Client-facing visibility flags apply to
  embeds and links exactly as to body text.

**Template registry:**

- One `Template` entity, typed: ticket / note snippet / email / KB article /
  quote-invoice layout / project scaffold / workflow playbook.
- Typed variables (`{{client.name}}`, `{{ticket.summary}}`) resolved from
  entity context at insertion time, never string-interpolated into SQL or
  rendered content without escaping.
- Shipped defaults are **immutable seed rows**: orgs fork, never edit in
  place (the defaults-first doctrine). Org-created templates are first-class
  and shareable later. Every template row is stored as org data under the
  upgrade-compatibility policy, which migrates it like any other
  customisation.

**Permissions:**

- **Preset roles ship as data**, not code: Admin, Dispatcher, Technician,
  Senior Tech, Bookkeeper, Read-only/Auditor, plus the two portal tiers,
  built on the same permission matrix as custom roles. Orgs duplicate and
  restore them, and edit the copies. System presets stay recoverable rows,
  never hardcoded logic.
- **`financial:read` is its own permission**, gating cost rates, contract
  rates, and margin panels. Without it those values render blank. This is the
  explicit rule against a tech seeing what the client pays by accident.

## Consequences

- KB embeds and linked documents land with the Phase-3 `core-kb` crate. The
  registry surface there must be the same registration path as the widget
  registry, not a parallel allowlist.
- The template registry ships incrementally (ticket/project/email types in
  Phase 3, KB/quote-invoice types at Phase 4), all on one `Template` entity,
  with no per-type template tables.
- Preset roles exist from the shipped permission matrix, and `financial:read`
  gates are checked in the policy module, testable per existing RBAC tests.
- Template rows and role rows are stored customizations: org manifests and
  upgrade preflight cover them like any other customisation.
