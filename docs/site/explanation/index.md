# Explanation

These pages explain **why** Marula works the way it does: why renaming a board column never breaks an SLA timer, why billing corrections create credit notes instead of editing sent invoices, and why AI features ship turned off by default.

If you want a step-by-step lesson, start in [Tutorials](../tutorials/index.md). If you want a quick checklist for a daily job, go to [How-to guides](../how-to/index.md). For schemas and endpoints, see [Reference](../reference/index.md). When something is broken rather than unexplained, [Troubleshooting](../troubleshooting.md) starts from the symptom.

- [Why we're building Marula](why-marula.md) — Our product manifesto, quoted word for word from `MANIFESTO.md`. Audience: both.
- [Instant-first interaction](instant-first.md) — Why there is no Save button to chase, how optimistic updates and `Ctrl/Cmd+Z` undo work, and why assistant suggestions wait for human approval. Audience: both.
- [Customizations are data](customizations-are-data.md) — Why custom fields, board workflows, and automation rules are stored as structured data rows so software updates never break your setup. Audience: both.
- [Contract relationship types](contract-relationship-types.md) — How Marula models the six MSP agreement types (recurring, retainer, time & materials, fixed fee, per unit, and per incident), plus prepaid blocks, caps, minimums, escalators, change orders, and SLA credits. Audience: both.
- [Billing corrections](billing-corrections.md) — Why issued invoices lock permanently, how credit notes and pre-bill verification adjustments work, and why sales tax is calculated in your accounting software. Audience: both.
- [FX additivity](fx-additivity.md) — How multi-currency billing anchors to one base currency, why the exchange-rate book is append-only, and why reconciliation totals are grouped by financial document type. Audience: both.
- [Provenance vs audit](provenance-vs-audit.md) — The difference between **Provenance** (who changed a specific field on a record, from what value to what value) and **Audit** (security and administrative events across your organization). Audience: both.
- [Knowledge-base categories and the secret vault](kb-class-categories.md) — How folder permissions work for help articles and passwords, why restricted items are completely hidden rather than shown as locked rows, and how every password reveal is logged. Audience: both.
- [The portal isolation model](portal-isolation.md) — How client portal accounts, ticket boards, and Knowledge Base searches are fenced off per client company. Audience: both.
- [AI: deny-by-default and the decision harness](ai-deny-by-default.md) — Why the master AI switch ships **OFF**, how per-role surface grants work, and how triage/routing models record their probability scores for inspection. Audience: both.
- [The upgrade compatibility policy](upgrade-policy.md) — How the N-2 upgrade guarantee and expand-then-contract database migrations keep self-hosted upgrades predictable. Audience: admin.
