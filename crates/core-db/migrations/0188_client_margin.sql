-- Migration 0188: labour cost rates + the client-margin read model.
--
-- Purpose: the repo had no cost data anywhere. Revenue and the cost of the
-- work that produced it could not meet, so "margin per client" had no
-- honest answer. This migration adds the two rates that make labour
-- costable and one view that puts revenue and cost on the same row:
--
--   * users.cost_rate_minor — what one staff member's hour costs, in the
--     org's base currency (ADR 0002: integer minor units, per HOUR,
--     matching contracts.hourly_rate_minor (0029) — the only other hourly
--     rate in the schema). NULL means "no personal rate; use the org
--     default", and that distinction survives: NULL is never read as zero.
--
--   * orgs.default_cost_rate_minor — the org-wide fallback, same unit and
--     currency. Also nullable, because an org may genuinely not know its
--     labour cost, and a fabricated zero would make every margin look
--     perfect. NULL means "cost unknown", and the view says so.
--
--   * ops.v1_client_margin — one row per client per money event, carrying
--     revenue, cost and margin for that event. The event date is
--     `period_day` (the v1 time anchor), so a reporting window turns this
--     into one row per client per period; ReportKind::ClientMargin does
--     exactly that.
--
-- WHY NOT THE SETTINGS REGISTRY for the org default: `settings` (0018) is
-- the org-level store for scalar preferences, but its value column is
-- `jsonb NOT NULL` with a CHECK admitting only string/number/boolean, and
-- `SettingValue` in core-domain has no null. "Unset" is not representable
-- there, and unset is the whole point. Rather than invent a second
-- settings table or smuggle a sentinel (-1) past a "negative is refused"
-- rule, the default is one nullable column on `orgs` — the same table that
-- already carries org-level structured configuration the settings registry
-- cannot express (`base_currency`, `ticket_prefix`, `invoice_sequence`).
--
-- DECISIONS this view encodes (each mirrors an existing precedent):
--
--   Revenue — invoices with status 'issued' or 'paid', dated by `issued_at`.
--     `draft` is not revenue yet and `void` never was; this is exactly the
--     set core-db already treats as the client's receivable
--     (core-db/src/invoices.rs: the portal lists and reads invoice rows
--     with `status IN ('issued','paid')`).
--     Amount is BASE currency: `COALESCE(base_total_minor, total_minor)` —
--     the 0065 rule that single-currency facts carry no conversion snapshot
--     and their original amount IS the base amount.
--
--   Credit notes — status 'issued', dated by `created_at`, subtracted. A
--     credit is negative revenue; ignoring it would bias margin in the
--     customer's favour. Credit notes carry no snapshot of their own
--     (0058), so a non-base credit converts at its issuing invoice's locked
--     rate when one exists, via fx_half_even_div (0067) so the arithmetic
--     stays integer and half-even like every other money conversion. A
--     non-base credit with no issuing invoice is UNATTRIBUTABLE: it is not
--     silently dropped, it lands in `unconverted_credit_minor` where the
--     report exposes it.
--
--   Time — every entry attributed to a client, billable or not, approved or
--     not. Cost is incurred by the work, not by its billing classification
--     or by an approver having clicked; filtering on approval would also
--     read zero cost for an org that never uses approvals (entries default
--     to 'draft', 0073) and would move cost between periods when an entry
--     is approved later, because the window anchors on the work day, not on
--     the decision. Approval filtering belongs to utilisation metrics
--     (core-db/src/scorecards.rs), which ask a different question.
--     Entries with no client (internal work; project time whose project has
--     no client) cannot be attributed to a client and are out of scope for
--     a per-client view — the report's totals are the sum of its rows, so
--     the two never disagree.
--
--   Cost — minutes x hourly rate, rounded ONCE PER ENTRY, half up:
--     `(minutes * rate + 30) / 60`. This is core-billing's `prorate`
--     (crates/core-billing/src/rules.rs:548-560) to the cent, so a costed
--     hour on this report and a priced hour on an invoice use one rule.
--     Rounding per entry means a client's total is the sum of the same
--     integer cents the org total is: two clients can never sum to a
--     different figure than the org, because there is no second rounding
--     step.
--
--   Missing cost — a user rate falls back to the org default; if both are
--     NULL the entry is `uncosted_minutes`, its `cost_minor` is NULL, and
--     `margin_minor` is NULL. Missing cost is never zero.
--
-- Additive only: two nullable columns and one new view. Nothing existing is
-- re-created, renamed or retyped.

ALTER TABLE users
  ADD COLUMN cost_rate_minor integer
    CONSTRAINT users_cost_rate_non_negative
    CHECK (cost_rate_minor IS NULL OR cost_rate_minor >= 0);

COMMENT ON COLUMN users.cost_rate_minor IS
  'Labour cost rate in org base-currency minor units per HOUR. NULL = fall '
  'back to orgs.default_cost_rate_minor; never read as zero.';

ALTER TABLE orgs
  ADD COLUMN default_cost_rate_minor integer
    CONSTRAINT orgs_default_cost_rate_non_negative
    CHECK (default_cost_rate_minor IS NULL OR default_cost_rate_minor >= 0);

COMMENT ON COLUMN orgs.default_cost_rate_minor IS
  'Fallback labour cost rate in org base-currency minor units per HOUR, '
  'used when users.cost_rate_minor is NULL. NULL = cost unknown; the '
  'margin read model reports those minutes as uncosted instead of costed at '
  'zero.';

-- ---------------------------------------------------------------------------
-- ops.v1_client_margin — revenue and costed time, per client per event.
--
-- Grain: one row per client per money event. `period_day` is the event's
-- date (invoice issue date, credit-note creation date, or the time entry's
-- UTC work day) and is the view's time anchor; a reporting window over it
-- yields one row per client per period.
--
-- Columns (v1):
--   org_id uuid                  owning org (RLS seam)
--   client_id uuid               client the money belongs to (nullable: only
--                                for future unattributed rows; today every
--                                row carries a client)
--   client_name text             denormalized client name
--   period_day date              event date — the window anchor
--   revenue_minor bigint         base-currency revenue for this event
--                                (negative for credit notes)
--   cost_minor bigint            costed labour for this event, NULL when no
--                                rate could be resolved
--   costed_minutes integer       minutes priced into cost_minor
--   uncosted_minutes integer     minutes with no rate; never priced at zero
--   margin_minor bigint          revenue_minor - COALESCE(cost_minor, 0);
--                                NULL exactly when cost_minor is NULL
--   unconverted_credit_minor bigint  credit notes in a non-base currency with
--                                no issuing invoice to borrow a rate from —
--                                reported, never silently dropped
--
-- security_invoker: the caller's RLS applies to invoices, credit_notes,
-- time_entries, users, orgs and clients exactly as if they were queried
-- directly (0026/0059 doctrine).
CREATE VIEW ops.v1_client_margin WITH (security_invoker = true) AS
SELECT f.org_id,
       f.client_id,
       cl.name AS client_name,
       f.period_day,
       f.revenue_minor,
       f.cost_minor,
       f.costed_minutes,
       f.uncosted_minutes,
       f.revenue_minor - COALESCE(f.cost_minor, 0) AS margin_minor,
       f.unconverted_credit_minor
FROM (
  -- Invoiced revenue: issued or paid documents, base-currency amount.
  SELECT i.org_id,
         i.client_id,
         i.issued_at AS period_day,
         COALESCE(i.base_total_minor, i.total_minor) AS revenue_minor,
         0::bigint AS cost_minor,
         0 AS costed_minutes,
         0 AS uncosted_minutes,
         0::bigint AS unconverted_credit_minor
  FROM invoices i
  WHERE i.status IN ('issued', 'paid')

  UNION ALL

  -- Credit notes: negative revenue. Convert only through a snapshot or the
  -- base currency itself; anything else is surfaced as unconverted.
  SELECT cn.org_id,
         cn.client_id,
         CAST(cn.created_at AT TIME ZONE 'UTC' AS date) AS period_day,
         -COALESCE(cv.converted_minor, 0::bigint) AS revenue_minor,
         0::bigint AS cost_minor,
         0 AS costed_minutes,
         0 AS uncosted_minutes,
         CASE WHEN cv.converted_minor IS NULL THEN cn.total_minor ELSE 0::bigint END
           AS unconverted_credit_minor
  FROM credit_notes cn
  JOIN orgs o ON o.id = cn.org_id
  LEFT JOIN invoices inv ON inv.id = cn.issued_against_invoice_id
  LEFT JOIN LATERAL (
    SELECT CASE
             WHEN cn.currency = o.base_currency THEN cn.total_minor
             WHEN inv.fx_rate_used IS NOT NULL
               THEN fx_half_even_div(cn.total_minor::numeric * inv.fx_rate_used, 1)
           END AS converted_minor
  ) cv ON TRUE
  WHERE cn.status = 'issued'

  UNION ALL

  -- Costed time: every client-attributed entry, at the user rate or the org
  -- default. Rounded once per entry, half up, exactly like core-billing
  -- prorate: (minutes * rate + 30) / 60.
  SELECT te.org_id,
         te.client_id,
         te.day_utc AS period_day,
         0::bigint AS revenue_minor,
         CASE WHEN r.rate_minor IS NOT NULL
              THEN ((te.minutes::bigint * r.rate_minor) + 30) / 60
         END AS cost_minor,
         CASE WHEN r.rate_minor IS NOT NULL THEN te.minutes ELSE 0 END AS costed_minutes,
         CASE WHEN r.rate_minor IS NULL THEN te.minutes ELSE 0 END AS uncosted_minutes,
         0::bigint AS unconverted_credit_minor
  FROM ops.v1_time_entries te
  LEFT JOIN users u ON u.id = te.user_id
  LEFT JOIN orgs o ON o.id = te.org_id
  CROSS JOIN LATERAL (
    SELECT COALESCE(u.cost_rate_minor, o.default_cost_rate_minor)::bigint AS rate_minor
  ) r
  WHERE te.client_id IS NOT NULL
) f
LEFT JOIN clients cl ON cl.id = f.client_id;

GRANT SELECT ON ops.v1_client_margin TO meridian_app;
