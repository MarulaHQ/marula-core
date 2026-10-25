-- One provider reference settles money once per invoice (Wave 55 lane
-- `p55-stripe-settlement-fence`), closing audit record
-- `stripe-webhook/provider-ref-idempotence-not-atomic` (R3).
--
-- Why the database decides this and not the handler: `record_payment`
-- (`crates/core-api/src/routes/stripe_webhook.rs`) asked
-- `payment_exists_for_ref` — a SELECT EXISTS with no lock — and then inserted
-- the receipt. Two deliveries of one `payment_intent.succeeded` both read
-- "absent", both inserted, and both applied the amount. A partial settlement
-- leaves the invoice `issued` after the first apply, so the second apply passed
-- the payable-status check too, and `total_paid_minor` moved twice for one
-- money movement. Nothing in the schema said the reference was unique, so no
-- statement could refuse the second row.
--
-- Why (org_id, invoice_id, provider_ref) and not a key on the reference alone.
-- Provider references are minted with different multiplicity per provider, and
-- this table records all of them:
--
--   * Stripe mints `pi_*` per payment intent, and
--     `core_billing::StripePaymentProvider::charge` puts exactly one invoice id
--     in the intent's metadata, so one intent lands on one invoice
--     (`routes/invoices.rs`, `routes/portal_v2.rs`, the webhook).
--   * A credit note is applied in parts — `credit_notes.applied_minor`
--     accumulates — and may be spread across several invoices of its client
--     (`routes/credit_notes.rs`), so one note number legitimately appears once
--     per invoice.
--
-- The intersection of those shapes is "at most once per invoice", which is also
-- the question the handler asks, so `invoice_id` stays in the key. A key on
-- (org_id, provider_ref) would refuse the second application of a credit note
-- and be no stricter where it counts.
--
-- provider_ref IS NOT NULL: a receipt may carry no reference at all (a manual
-- payment recorded by staff has none). NULLs never collide in a Postgres unique
-- index; the predicate states the intent instead of relying on that.
--
-- provider <> 'none': the none-mode provider mints `local-{invoice_id}`
-- (`core_billing::NonePaymentProvider`) — a marker derived from the invoice
-- itself, not an identity a provider gave a money movement. Every staff ledger
-- payment on an invoice therefore carries the same reference by design, and the
-- record-payment form offers the second instalment. Indexing it would refuse
-- that instalment for no gain: nothing de-duplicates on it.
--
-- REPAIR, not just a guard. A deployment that already took the race this
-- closes holds two rows with the same reference, and the index cannot build
-- over them. Renaming the loser's reference to `<ref>--dup-<first 8 hex of its
-- id>` keeps the row — the money is still recorded, and the original reference
-- is still readable in it — while letting the constraint take. The winner is
-- the earliest receipt (`received_at, id`), which is the one that legitimately
-- applied; every later row is the double credit. One NOTICE per rename carries
-- org, invoice and amount so the operator can find them in the migration log.
--
-- The paid total those duplicates already inflated is data, not schema, and a
-- migration must not rewrite a ledger. The reconciliation query for it is the
-- audit's:
--
--   SELECT i.id, i.total_paid_minor, COALESCE(SUM(p.amount_minor), 0) AS receipts
--     FROM invoices i LEFT JOIN invoice_payments p ON p.invoice_id = i.id
--    WHERE i.total_paid_minor <> 0
--    GROUP BY i.id
--   HAVING i.total_paid_minor <> COALESCE(SUM(p.amount_minor), 0);
--
-- LOCK NOTE: `invoice_payments` holds one row per receipt, so it is small next
-- to the invoice tables and its writes are already serialised per invoice. The
-- index builds with ACCESS EXCLUSIVE for the length of the build; `IF NOT
-- EXISTS` makes the statement a no-op on an install that pre-built it with
-- `CREATE UNIQUE INDEX CONCURRENTLY`.

DO $$
DECLARE
  loser record;
BEGIN
  FOR loser IN
    SELECT id, org_id, invoice_id, provider_ref, amount_minor FROM (
      SELECT id, org_id, invoice_id, provider_ref, amount_minor,
             row_number() OVER (
               PARTITION BY org_id, invoice_id, provider_ref
               ORDER BY received_at, id
             ) AS rn
      FROM invoice_payments
      WHERE provider_ref IS NOT NULL AND provider <> 'none'
    ) ranked
    WHERE ranked.rn > 1
  LOOP
    UPDATE invoice_payments SET provider_ref = loser.provider_ref || '--dup-' || left(loser.id::text, 8) WHERE id = loser.id;
    RAISE NOTICE
      'invoice_payments %: duplicate provider_ref % on invoice % (org %) for % minor units renamed to keep one receipt per reference; reconcile the invoice paid total',
      loser.id, loser.provider_ref, loser.invoice_id, loser.org_id, loser.amount_minor;
  END LOOP;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_invoice_payments_provider_ref
  ON invoice_payments (org_id, invoice_id, provider_ref)
  WHERE provider_ref IS NOT NULL AND provider <> 'none';

COMMENT ON INDEX idx_invoice_payments_provider_ref IS
    'One provider reference settles money once per invoice. The webhook inserts with ON CONFLICT DO NOTHING and reads zero rows as "already applied"; API writers get a typed 409. Excludes provider none, whose reference is derived from the invoice id.';
