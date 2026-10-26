---
description: "Because you own your PostgreSQL database, you can connect Metabase, Grafana, Power BI, or psql directly to Marula for custom reporting."
---

# How to report against the read model

> **Audience:** both — reporting authors (staff or admin) querying the stable SQL views.

Because you own your PostgreSQL database, you can connect Metabase, Grafana, Power BI, or `psql` directly to Marula for custom reporting. To make sure your SQL queries never break when you upgrade Marula, always query the versioned **`ops.v1_*` reporting views** rather than internal application tables.

## Prerequisites

- Read-only SQL access to your Marula PostgreSQL database. Check the [Self-host local stack reference](../reference/self-host-local-stack.md) for how database roles and row-level security (`RLS`) are configured.

## 1. Connect your BI tool or SQL client

Connect your reporting tool to PostgreSQL using a read-only database credential scoped appropriately for your reporting needs.

## 2. Query the `ops.v1_*` views

Marula publishes a versioned set of SQL views under the `ops` schema (`ops.v1_*`) covering your daily operations and financials:

- [`v1-tickets`](../../reference/read-model/v1-tickets.md)
- `v1-time-entries`, `v1-invoices`, `v1-invoice-lines`, `v1-sla-outcomes`, `v1-fx-reconciliation`, `v1-clients`, `v1-contacts`, `v1-sites`, `v1-board-statuses`, `v1-first-response`, `v1-gap-days`, and `v1-undo-stacks`
- Browse every column and type in the full [Read-model view catalog](../../reference/read-model.md)

The in-app **Report Builder** (`/reports`) queries these exact same `ops.v1_*` views, so numbers on your internal dashboards and external BI tools always match.

## 3. Why you should query views instead of raw tables

- **Guaranteed upgrade stability:** Internal tables can gain columns, split, or normalize across releases. The `ops.v1_*` views are part of Marula's public compatibility contract and keep their column names and semantics across upgrades.
- **Clear deprecation notice:** If a future major version ever introduces `ops.v2_*`, the change will be announced in the release notes under **"Breaking & deprecated"** with a full support window ([see Upgrade policy](../explanation/upgrade-policy.md)).
