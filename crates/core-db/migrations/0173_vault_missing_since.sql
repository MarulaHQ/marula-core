-- 0173 — the vault key space can say its bytes are gone (F76).
--
-- The integrity pass could mark an attachment row whose object vanished and
-- could not mark a vault document's ciphertext, because `secret_records` had no
-- column to carry the fact. A reveal of a document whose bytes are gone then
-- answered from the store adapter instead of from the row, which is the state
-- the attachment side stopped having in 0146.
--
-- Idempotent and nullable: the mark is a timestamp an operator can read, not a
-- state machine, and a re-upload at the same content address clears it.
ALTER TABLE secret_records
    ADD COLUMN IF NOT EXISTS missing_since timestamptz;

COMMENT ON COLUMN secret_records.missing_since IS
    'Set by the object-integrity pass when the sealed ciphertext this row points at is absent from the store; cleared when a re-upload at the same key restores it (0173, F76).';

CREATE INDEX IF NOT EXISTS secret_records_missing_since_idx
    ON secret_records (org_id)
    WHERE missing_since IS NOT NULL;
