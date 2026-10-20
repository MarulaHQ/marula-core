# Owner tasks

> **Audience:** owner — you sign off on contracts and invoices, and you decide
> who has access. You do not need the settings detail to start; each link goes
> to it.

You are accountable for the numbers and for who can see what. These are the
jobs that land on your desk, in the order they usually arrive. When something is
broken rather than unowned, [Troubleshooting](../troubleshooting.md) starts from
the symptom.

## "I need to close the month"

1. [Verify a billing period](../how-to/admin-billing-verification.md) — walk every
   logged hour in the window, write down anything that should not be billed, and
   sign off before invoices go out.
2. [Your first invoice](../tutorials/first-invoice.md) — the whole path from
   approved hours to a sent invoice, once, end to end.
3. [Billing corrections](../explanation/billing-corrections.md) — what happens
   when the invoice already went out. Issued invoices lock; corrections are credit
   notes, and the page says why.
4. [Record FX rates and read reconciliation](../how-to/admin-fx-rates.md) —
   multi-currency totals grouped by document type rather than added together.

## "A client says the invoice is wrong"

Start with [billing corrections](../explanation/billing-corrections.md) for the
policy, then [provenance vs audit](../explanation/provenance-vs-audit.md) to
answer "who changed this rate, and when". Provenance shows the field, the old
value, the new value and the actor: a person, an agent run or an import.

## "I need to know what we sold"

- [Price a service contract](../how-to/admin-contract-types.md) — recurring
  retainers, time and materials, prepaid blocks, caps, minimums, escalators,
  change orders, service credits.
- [Contract relationship types](../explanation/contract-relationship-types.md) —
  the six agreement types and how each one bills.
- [Bill project work](../how-to/project-billing.md) — phase budgets, milestones
  released into invoice runs, project hours without a dummy ticket.
- [Buy: vendors, purchase orders, receiving, bills](../how-to/admin-purchasing.md) —
  what you spend against what you bill.

## "Show me where the business stands"

- [Build dashboards and report schedules](../how-to/admin-dashboards-reports.md) —
  widgets over the read model, plus scheduled email digests.
- [Report against the read model](../how-to/read-model.md) — point a BI tool or a
  SQL client at the stable `ops.v1_*` views when the built-in dashboards are not
  the question.
- [The read model view catalogue](../reference/read-model.md) — every view and its
  columns, so you know what can be asked.

## "Who has access, and to what"

- [Manage users and invites](../how-to/admin-users-invites.md) — invitations,
  two-factor and passkeys, quiet hours.
- [Connect an IdP (SSO)](../how-to/admin-sso.md) — sign-in through your identity
  provider, and exactly what SSO does not change.
- [Knowledge-base categories and the secret vault](../explanation/kb-class-categories.md) —
  why restricted items are hidden rather than shown locked, and why every password
  reveal is logged.
- [The portal isolation model](../explanation/portal-isolation.md) — what a client
  account can never reach.

## "What is the AI allowed to do"

- [AI: deny-by-default](../explanation/ai-deny-by-default.md) — why the master
  switch ships off and what a grant changes.
- [Manage AI access](../how-to/admin-ai-access.md) — grant a helper to a role and
  set a usage cap.
- [Your first agent run](../tutorials/first-agent-run.md) — the plan-first
  behaviour you can hold an agent to.

There is no site page for custom roles and the permission model beyond what the
settings screens say. If you need that before you grant access, walk **Settings
→ People** with your admin and read the role descriptions together. **Not
documented yet.**

## "I am handing over, or leaving"

[Export your organization's data](../how-to/export-data.md) builds a versioned
JSON bundle and tracks past bundles. The manifest is also what an upgrade
preflight reads, so a current export doubles as documentation of every
customization you made.

## Not documented yet

These are owner tasks with no page. They are recorded here so the gap is visible
rather than filled with invented behaviour.

- Contract renewal: which contracts are up for renewal, and what changed last
  term. The contract pages cover setup, not the renewal cycle.
- Margin per client or per ticket: labour cost is not a documented input, so no
  page claims a margin figure.
- Quoting: [approving a quote](../how-to/client-quote-approval.md) is documented
  from the client side. Authoring a quote, and the path from an accepted quote to
  an invoice, has no page.
- Data retention and deletion: no page states how long records are kept or what
  an erasure request removes.
