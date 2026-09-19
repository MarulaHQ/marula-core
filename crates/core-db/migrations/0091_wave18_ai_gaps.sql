-- Wave 18 lane 2b `p18-ai-gaps`: the AI-wave honest-gaps collector —
-- additive riders on 0081's AI policy schema. (0090 stays RESERVED for
-- Wave 23 lane 2's decision harness.)
--
-- Three riders, each idempotent (IF NOT EXISTS / drop+re-add with the
-- superset of values):
--
--   1. Ledger idempotence — a per-resolution-edge partial unique index on
--      `ai_usage_ledger.request_id` so a repeated edge write dedupes at
--      the edge (typed conflict in the API), not runner-side. The runner
--      keeps its never-reuse discipline; the database now enforces it
--      too. Partial on request_id IS NOT NULL: tool rows and legacy rows
--      with no request id are unaffected. Org + surface are in the key
--      because "edge" is (org, surface, step) — request ids are only
--      promised unique within a surface's resolution edge.
--
--   2. Cost-weighted metering — the ledger row carries the catalog's
--      `cost_weight` at call time and a STORED generated column with the
--      weighted total. Raw token columns stay untouched (reconstruction
--      remains exact); every consumer (caps enforcement, progress read
--      models) switches to the weighted sum, so `cost_weight` stops
--      being display-only. Existing rows default to weight 1 — their
--      weighted total equals their raw total.
--
--      Capabilities become load-bearing in the same lane, so pre-existing
--      catalog rows (created before the tags meant anything) are
--      backfilled with the `completion` tag: every entry curated under
--      the display-only regime was a completion model. `tools` /
--      `decision` are NEVER backfilled — those edges stay closed until
--      an admin tags the entry explicitly.
--
--   3. Caps periods — the 'month'-only CHECK widens to
--      week/quarter/year (superset; additive). The progress read models
--      become caps-driven: each cap row is paired with usage since THAT
--      period's start (date_trunc on the configured period), instead of
--      every cap being measured against the current month.
--
-- Non-purpose: no capability data changes (item 4 of the lane is a code
-- seam over the existing `ai_model_catalog.capabilities` column), no
-- plugin-surface table (plugin registrations ARE ai_surfaces rows), no
-- data backfill beyond the default weight.

-- ---------------------------------------------------------------------------
-- 1. Ledger idempotence: per-edge request_id uniqueness.
-- ---------------------------------------------------------------------------

CREATE UNIQUE INDEX IF NOT EXISTS uq_ai_usage_ledger_edge_request
  ON ai_usage_ledger (org_id, surface, request_id)
  WHERE request_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- 2. Cost-weighted metering: the weight snapshot + the weighted total.
-- ---------------------------------------------------------------------------

ALTER TABLE ai_usage_ledger
  ADD COLUMN IF NOT EXISTS cost_weight integer NOT NULL DEFAULT 1
    CHECK (cost_weight BETWEEN 1 AND 5);

-- STORED generated: kept in the row (caps checks read it per call);
-- expression matches the raw reconstruction (prompt + completion) x weight.
ALTER TABLE ai_usage_ledger
  ADD COLUMN IF NOT EXISTS weighted_tokens bigint GENERATED ALWAYS AS
    ((prompt_tokens::bigint + completion_tokens) * cost_weight) STORED;

-- Backfill: display-era rows were all completion models.
UPDATE ai_model_catalog
   SET capabilities = '["completion"]'::jsonb
 WHERE capabilities = '[]'::jsonb;

-- ---------------------------------------------------------------------------
-- 3. Caps periods: widen the period CHECK (superset — no data rewrite).
-- ---------------------------------------------------------------------------

ALTER TABLE ai_caps DROP CONSTRAINT ai_caps_period_check;
ALTER TABLE ai_caps ADD CONSTRAINT ai_caps_period_check
  CHECK (period IN ('week', 'month', 'quarter', 'year'));

-- ---------------------------------------------------------------------------
-- Progress read models, rebuilt caps-driven: one row per cap scope, with
-- usage summed over THAT cap's period and weighted tokens. `used_tokens`
-- keeps its name (now the weighted figure); `period` changes from the
-- month timestamptz to the configured period key and `period_start`
-- carries the window start — the caps-driven shape item 3 calls for. The
-- views are DROPPED and recreated (not CREATE OR REPLACE) because the
-- `period` column TYPE changes.
-- ---------------------------------------------------------------------------

DROP VIEW IF EXISTS ops.v1_ai_usage_user_progress;
DROP VIEW IF EXISTS ops.v1_ai_usage_org_progress;

CREATE VIEW ops.v1_ai_usage_user_progress WITH (security_invoker = true) AS
SELECT
  c.org_id,
  c.user_id,
  c.period,
  date_trunc(c.period, now()) AS period_start,
  COALESCE((
    SELECT SUM(l.weighted_tokens)
    FROM ai_usage_ledger l
    WHERE l.org_id = c.org_id AND l.user_id = c.user_id
      AND l.created_at >= date_trunc(c.period, now())
  ), 0) AS used_tokens,
  c.soft_token_limit AS user_soft_limit,
  c.hard_token_limit AS user_hard_limit,
  oc.soft_token_limit AS org_soft_limit,
  oc.hard_token_limit AS org_hard_limit,
  -- effective limits: per-user cap when present, else org-wide
  COALESCE(c.hard_token_limit, oc.hard_token_limit) AS effective_hard_limit,
  COALESCE(c.soft_token_limit, oc.soft_token_limit) AS effective_soft_limit,
  CASE WHEN COALESCE(c.hard_token_limit, oc.hard_token_limit) IS NOT NULL
       THEN ROUND(COALESCE((
              SELECT SUM(l.weighted_tokens)
              FROM ai_usage_ledger l
              WHERE l.org_id = c.org_id AND l.user_id = c.user_id
                AND l.created_at >= date_trunc(c.period, now())
            ), 0)::numeric
            / COALESCE(c.hard_token_limit, oc.hard_token_limit) * 100, 2)
       ELSE NULL END AS pct_hard,
  CASE WHEN COALESCE(c.soft_token_limit, oc.soft_token_limit) IS NOT NULL
       THEN ROUND(COALESCE((
              SELECT SUM(l.weighted_tokens)
              FROM ai_usage_ledger l
              WHERE l.org_id = c.org_id AND l.user_id = c.user_id
                AND l.created_at >= date_trunc(c.period, now())
            ), 0)::numeric
            / COALESCE(c.soft_token_limit, oc.soft_token_limit) * 100, 2)
       ELSE NULL END AS pct_soft
FROM ai_caps c
LEFT JOIN ai_caps oc
  ON oc.org_id = c.org_id AND oc.user_id IS NULL AND oc.period = c.period
WHERE c.user_id IS NOT NULL;

CREATE VIEW ops.v1_ai_usage_org_progress WITH (security_invoker = true) AS
SELECT
  c.org_id,
  c.period,
  date_trunc(c.period, now()) AS period_start,
  COALESCE((
    SELECT SUM(l.weighted_tokens)
    FROM ai_usage_ledger l
    WHERE l.org_id = c.org_id
      AND l.created_at >= date_trunc(c.period, now())
  ), 0) AS used_tokens,
  COALESCE((
    SELECT COUNT(DISTINCT l.user_id)
    FROM ai_usage_ledger l
    WHERE l.org_id = c.org_id
      AND l.created_at >= date_trunc(c.period, now())
  ), 0) AS active_users,
  c.soft_token_limit AS org_soft_limit,
  c.hard_token_limit AS org_hard_limit,
  CASE WHEN c.hard_token_limit IS NOT NULL
       THEN ROUND(COALESCE((
              SELECT SUM(l.weighted_tokens)
              FROM ai_usage_ledger l
              WHERE l.org_id = c.org_id
                AND l.created_at >= date_trunc(c.period, now())
            ), 0)::numeric / c.hard_token_limit * 100, 2)
       ELSE NULL END AS pct_hard,
  CASE WHEN c.soft_token_limit IS NOT NULL
       THEN ROUND(COALESCE((
              SELECT SUM(l.weighted_tokens)
              FROM ai_usage_ledger l
              WHERE l.org_id = c.org_id
                AND l.created_at >= date_trunc(c.period, now())
            ), 0)::numeric / c.soft_token_limit * 100, 2)
       ELSE NULL END AS pct_soft,
  COALESCE(
    (SELECT jsonb_object_agg(s.surface, s.surface_tokens)
     FROM (
       SELECT l.surface, SUM(l.weighted_tokens) AS surface_tokens
       FROM ai_usage_ledger l
       WHERE l.org_id = c.org_id
         AND l.created_at >= date_trunc(c.period, now())
       GROUP BY l.surface
     ) s),
    '{}'::jsonb
  ) AS tokens_by_surface
FROM ai_caps c
WHERE c.user_id IS NULL;

GRANT SELECT ON ops.v1_ai_usage_user_progress TO meridian_app;
GRANT SELECT ON ops.v1_ai_usage_org_progress TO meridian_app;
