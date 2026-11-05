-- A report definition may carry its card (F103).
--
-- Why: `CardSpec` says a card is stored beside a `ReportSpec` by both surfaces,
-- but this table had one spec column and no card. An accepted authoring draft
-- therefore kept the query and dropped the window, the refresh and the size.
-- The card is the surface's contract for how the report is drawn, and the run
-- route reads its window as the default span when a caller asks for none.
--
-- Additive, and no backfill: a definition without a card is legitimate and
-- stays valid. The CHECK is the same floor `spec` carries — the column is
-- either absent or a JSON object; `CardSpec::validate` remains the Rust gate.
--
-- Non-purpose: no execution state (runs compile on demand), no per-user
-- placement (dashboards persist that separately).

ALTER TABLE report_definitions
  ADD COLUMN card jsonb
  CHECK (card IS NULL OR jsonb_typeof(card) = 'object');
