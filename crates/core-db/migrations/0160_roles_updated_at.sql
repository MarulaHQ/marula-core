-- Role edits become auditable.
--
-- `update_custom_role` had been writing `updated_at = now()` since the module
-- existed, and `roles` never had the column: every PATCH answered 500 against a
-- real database, which no test covered until Wave 54's S3c lane added one.
-- Wave 54 removed the clause to make the write land. This adds the column so a
-- role edit carries the time it happened, and the write sets it again.
--
-- `NOT NULL DEFAULT now()` backfills existing rows with the migration time,
-- which is the honest answer: nobody recorded when those rows last changed.

ALTER TABLE roles ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

COMMENT ON COLUMN roles.updated_at IS
    'Last write to this row. Set by update_custom_role; presets are immutable (builtin IS NULL predicate) so this moves only for custom roles.';
