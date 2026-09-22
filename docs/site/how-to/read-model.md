# How to report against the read model

> **Audience:** both — reporting authors (staff or admin) querying the stable SQL views.

Direct SQL access and external BI tools are supported — but the promise
attaches to the views, not the physical tables.

## Prerequisites

- SQL access to the deployment's Postgres; note the [role split](../reference/self-host-local-stack.md) — the app role is RLS-constrained, connect with whichever role matches the read you intend.


## Connect

Point any Postgres client or BI tool at your Meridian database with a
read-only role.

## Query the stable views

The documented read model is a stable, versioned set of SQL views over
operational data, prefixed `ops.v1_*`:

- [`v1-tickets`](../../reference/read-model/v1-tickets.md) (and the full
  [view catalog](../../reference/read-model.md))
- invoices, invoice lines, time entries, SLA outcomes, FX reconciliation,
  and more.

## Keep it supported

- Query the `ops.v1_*` views, never physical tables.
- The report builder (in-app) is a thin layer over the same field catalog;
  external BI stays parity-via-REST — no lock-in in either direction.
- Physical table layouts are explicitly not stable, ever.

## Upgrades

The read model is part of the compatibility surface: a supported view
version never breaks inside its support window. Check the release notes'
"Breaking & deprecated" section when a view version turns over.
