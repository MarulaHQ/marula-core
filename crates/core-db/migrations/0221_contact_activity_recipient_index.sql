-- Contact activity: the outbound email leg can use an index (migration 0221).
--
-- The read in `core_db::contact_activity` joins email twice now — inbound on
-- `from_address`, outbound on `to_addresses` — instead of one OR that neither
-- index could serve. The inbound half already has its index: 0199 created
-- `idx_email_messages_org_from_lower (org_id, lower(from_address))`, and it
-- starts working the moment the OR stops hiding it. Nothing is added for it
-- here; a second, partial copy of the same expression would be a duplicate to
-- keep in sync for a narrower scan.
--
-- The recipient half needs a new shape. `to_addresses` is `text[]`, and
-- `lower()` over `unnest` cannot use a btree at all. The array operator class
-- CAN answer "does this array contain this address" (`@>`) from a GIN index,
-- but only for exact elements — so stored elements have to already be
-- lowercase. The ingest path guarantees that from this commit on
-- (`emails::enqueue_outbound` folds every recipient; the inbound path already
-- folded both sides), and the backfill below brings existing outbound rows to
-- the same form so history keeps matching. Direction is a partial predicate:
-- only outbound mail is addressed to a contact.
--
-- The backfill's WHERE keeps the write to rows whose stored form actually
-- differs, and the index is built after it, over the normalized values.

UPDATE email_messages m
   SET to_addresses = n.addresses
  FROM (
        SELECT id,
               ARRAY(SELECT lower(addr) FROM unnest(to_addresses) AS addr) AS addresses
          FROM email_messages
         WHERE direction = 'outbound'
       ) AS n
 WHERE m.id = n.id
   AND m.to_addresses IS DISTINCT FROM n.addresses;

CREATE INDEX idx_email_messages_outbound_recipients
  ON email_messages USING gin (to_addresses)
  WHERE direction = 'outbound';
