-- Wave 19 lane 1 `p19-vault-scale`: vault scale-out (ADR 0006 hygiene).
--
-- Durable admission-control state for the vault, keyed per org. Both
-- tables are OPERATOR-MACHINE state (written by the API under its own
-- org-bound session; no user-owned rows), so each carries exactly one
-- org-scoped RLS policy like 0078's secret trees — reads/writes ride the
-- same `app.org_id` GUC posture as every other tenant table
-- (RLS-coverage gate: every user-data table must carry a policy).
--
--   1. `vault_write_budget_windows` — the cross-replica side of the A1
--      per-org vault write-budget (Wave 16 lane 9 kept an in-memory
--      fixed-window bucket as the SINGLE-REPLICA fast path; this table
--      makes the budget hold ACROSS replicas on multi-replica
--      deployments). One row per org; `seats` is the count of admitted
--      seats inside the fixed 60-second window anchored at
--      `window_started` (DB clock — same tumbling-window semantics as
--      the in-memory bucket, now shared).
--
--   2. `vault_bulk_pacers` — durable token-bucket state for the A2
--      bulk-ingestion pacer (`vault_bulk_rows_per_sec` ceiling). One row
--      per org; `tokens` is the fractional refill bucket (capacity =
--      rate, refilled by elapsed DB-clock seconds since `last_refill`).
--      Durability closes the Wave 16 pacing gap: a restarted bulk
--      DRIVER must not rebound a fresh pace at restart — the bucket
--      state survives, so pace continues exactly where it left off,
--      cross-replica too.

-- ---------------------------------------------------------------------------
-- 1. Per-org write-budget windows.
-- ---------------------------------------------------------------------------

CREATE TABLE vault_write_budget_windows (
  org_id uuid PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
  window_started timestamptz NOT NULL,
  seats bigint NOT NULL CHECK (seats >= 0),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- 2. Per-org bulk-ingestion token buckets.
-- ---------------------------------------------------------------------------

CREATE TABLE vault_bulk_pacers (
  org_id uuid PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
  tokens double precision NOT NULL CHECK (tokens >= 0),
  last_refill timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- 3. RLS — same single org policy shape as 0078.
-- ---------------------------------------------------------------------------

ALTER TABLE vault_write_budget_windows ENABLE ROW LEVEL SECURITY;
CREATE POLICY vault_write_budget_windows_org ON vault_write_budget_windows
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE vault_bulk_pacers ENABLE ROW LEVEL SECURITY;
CREATE POLICY vault_bulk_pacers_org ON vault_bulk_pacers
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
