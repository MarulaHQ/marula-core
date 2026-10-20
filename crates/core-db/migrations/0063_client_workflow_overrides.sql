-- Per-client workflow overrides (Wave 12 lane 5, the private build tracker; migration 0063 is
-- this lane's exclusive window).
--
-- Purpose: let a CLIENT override org workflow defaults — intake board/status,
-- SLA policy, and notification/automation toggles — with a documented
-- resolution precedence: a client override wins over the org default, and an
-- explicit `disabled` row is a deny that wins over an org-enabled default.
-- Absence of a row means "inherit the org default" (defaults-first doctrine).
--
-- Client-scoping: every row is keyed by (org, client, override_key) and the
-- whole table is org-bound under RLS, exactly like the rest of the tenant
-- schema. Reads/mutations additionally verify the client is in the caller's
-- org, so one tenant can never observe or mutate another's overrides.
--
-- Customizations-as-data (upgrade-compatibility-policy §4.5/§5): rows are
-- declarative values validated against the typed override registry in
-- `core-domain::workflow_overrides` — a CLOSED key set with typed value kinds
-- (board ref, board-status ref, SLA-policy ref, toggle). No free-text keys or
-- label-driven behavior: reference constructs resolve through ids, and the
-- status construct carries its semantic `TicketStatusCategory`. The org's
-- overrides are exported verbatim by the U5 customization manifest, and a
-- migration that touches any referenced construct must transform these rows in
-- the same transaction (§4.5).
--
-- Non-purpose: no per-client copies of org constructs (rows point at the org's
-- boards/statuses/SLA policies; they never fork them), no per-ticket overrides
-- (ticket behavior already binds to semantic categories), no free-form
-- key/value bucket.

CREATE TABLE client_workflow_overrides (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
  -- Registry key from core-domain::workflow_overrides (closed set).
  override_key text NOT NULL,
  -- 'set'   → `value` is the override (client wins over the org default).
  -- 'disabled' → explicit deny/opt-out; `value` MUST be NULL (deny wins over
  --              an org-enabled default). Distinct from "no row" (inherit).
  state text NOT NULL CHECK (state IN ('set', 'disabled')),
  -- Typed payload: uuid string for reference kinds, boolean for toggles.
  value jsonb,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT client_workflow_overrides_state_value CHECK (
    (state = 'disabled' AND value IS NULL)
    OR (state = 'set' AND value IS NOT NULL)
  ),
  UNIQUE (org_id, client_id, override_key)
);

CREATE INDEX idx_client_workflow_overrides_client
  ON client_workflow_overrides (org_id, client_id);

ALTER TABLE client_workflow_overrides ENABLE ROW LEVEL SECURITY;
CREATE POLICY client_workflow_overrides_org ON client_workflow_overrides
  USING (org_id = current_setting('app.org_id', true)::uuid)
  WITH CHECK (org_id = current_setting('app.org_id', true)::uuid);
