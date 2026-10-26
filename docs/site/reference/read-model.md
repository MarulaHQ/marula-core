---
description: "The documented read model is a stable, versioned set of SQL views over operational data — the supported interface for direct SQL access and external…"
---

# Read model view catalog

The documented read model is a stable, versioned set of SQL views over
operational data — the supported interface for direct SQL access and
external reporting tools. "Your database" is a promise, but the promise
attaches to the views, not the physical tables.

Authoritative per-view docs live in
[`docs/reference/read-model.md`](../../reference/read-model.md):

- [v1-board-statuses](../../reference/read-model/v1-board-statuses.md)
- [v1-clients](../../reference/read-model/v1-clients.md)
- [v1-contacts](../../reference/read-model/v1-contacts.md)
- [v1-first-response](../../reference/read-model/v1-first-response.md)
- [v1-fx-reconciliation](../../reference/read-model/v1-fx-reconciliation.md)
- [v1-gap-days](../../reference/read-model/v1-gap-days.md)
- [v1-invoice-lines](../../reference/read-model/v1-invoice-lines.md)
- [v1-invoices](../../reference/read-model/v1-invoices.md)
- [v1-sites](../../reference/read-model/v1-sites.md)
- [v1-sla-outcomes](../../reference/read-model/v1-sla-outcomes.md)
- [v1-tickets](../../reference/read-model/v1-tickets.md)
- [v1-time-entries](../../reference/read-model/v1-time-entries.md)
- [v1-undo-stacks](../../reference/read-model/v1-undo-stacks.md)

The in-app report builder draws its field catalog from the same `ops.v1_*`
views; external BI stays parity-via-REST (ADR 0001).
