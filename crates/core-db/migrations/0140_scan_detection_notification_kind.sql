-- Scan-detection notifications reach the notification center (Wave 52 lane
-- P6b, `p52-scan-outcome`).
--
-- The upload scan gate (P6a) refuses an infected upload and holds the bytes
-- for an operator. This lane tells the uploader and the org's scan-detection
-- recipients about that refusal through the notification center, using the
-- `scan_detection` kind — the same kind the inbound-email quarantine notice
-- (Wave 36 lane 3) writes.
--
-- That kind has existed in `NotificationKind` since Wave 36, but no migration
-- ever added it to the column CHECKs. 0102 was the last migration to rewrite
-- them, and its list stops at `stock_low`, so an INSERT with
-- kind = 'scan_detection' violates `notifications_kind_check` on a database
-- built from these migrations. The three tables had also drifted apart:
-- 0101/0102 widened `notifications` and `notification_preferences` but left
-- `notification_org_defaults` at the 0096 list, so an org-level default for
-- `dedupe_report` or `stock_low` was equally unrepresentable.
--
-- This migration rewrites all three constraints to one list so the three
-- links of the preference chain accept the same kinds — a user override, an
-- org default, and a builtin default must not disagree about what a kind is.
-- Additive in the schema sense (widening a CHECK), and no existing row can
-- violate the wider predicate.
--
-- NUMBERING: 0140 is the next free slot (main's head was 0139 at lane start).
-- A board claim is a reservation at claim time, not a guarantee at merge
-- time — re-check main's head immediately before merge and renumber on a
-- collision.

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report','stock_low','scan_detection'));

ALTER TABLE notification_preferences DROP CONSTRAINT notification_preferences_kind_check;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report','stock_low','scan_detection'));

ALTER TABLE notification_org_defaults DROP CONSTRAINT notification_org_defaults_kind_check;
ALTER TABLE notification_org_defaults ADD CONSTRAINT notification_org_defaults_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation',
                  'missing_time','ticket_update','report_delivery','ai_cap_warn',
                  'secret_rotation','dedupe_report','stock_low','scan_detection'));
