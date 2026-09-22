# Knowledge-base categories and the secret vault share one grant model

> **Audience:** both — admins administer grants; every staff member
> experiences the hidden-vs-dead distinction as security, not cosmetics.

## Why they share a model: KB-class categories

ADR 0006 introduced the pattern these surfaces both apply — **KB-class
categories**: categories that carry grants per category, and where the
position of a record within a category tree is the access decision. The
knowledge base (articles) and the secret vault (secret categories) both
lean on the same model, and that is why a grant learned in one place
predicts the other.

"The vault record browser, per-category" and "the knowledge base,
category-granted" are the same design problem seen twice: knowledge and
credentials are sensitive exactly in proportion to which **chain** in the
tree their record belongs to.

How the surfaces render it:

- **Knowledge base** (/kb): categories carry articles, and a
  **client-facing flag** on articles; "articles flagged client-facing in
  a client-facing category appear in the portal" — the portal sees only
  the visible subset (and only that). The [portal-isolation design](portal-isolation.md)
  picks up the portal-side story.
- **Secret vault** (/passwords): "Secret categories are KB-class: org
  level or per client, with restricted chains grant-gated behind the
  scenes. Every reveal is audited." (The vault's own banner.)

## The one distinction you must keep: hidden, never dead

The vault's empty-state copy is the whole discipline of ADR 0006's
grants:

> "you cannot access are hidden entirely, never rendered as dead entries"

It matters at both ends of the grant decision:

- **Where you lack access, absence is not an error.** Restricted chains
  "hidden entirely" means a record simply does not render in your list —
  there is no locked/dead row, no counting signal that something was
  there and you were denied. Enumeration attempts are answered with
  invisibility, not with a denial screen that confirms existence.
- **Where you hold access, the reveal is a signed event.** "Every reveal
  is audited" — reading a secret is itself a write to the audit ledger,
  so the access surface and the audit surface never disagree on whether
  a record was opened, and who did.

## The editor side: one grant decision governs both reading and writing

Because the grant boundary is the **category chain**, the authoring
story stays single-decision: you compose an article or a secret into a
chain whose grant you already hold, and nothing about the outline
changes your rights. There is no separate "write access" or "read
access" wobble for a chain — the chain's grant is the decision, and
(quoting the client-facing flag's limits) portal exposure is a separate,
explicit opt-in on the article.

## Related doctrine

- The same chain boundaries mirror the semantic-category discipline of
  [workflow categories](../how-to/admin-workflow-categories.md).
- The vault's brutish reveal auditing means a vault audit sweep is a
  first-class record — [provenance vs audit](provenance-vs-audit.md)
  ties the two together.
