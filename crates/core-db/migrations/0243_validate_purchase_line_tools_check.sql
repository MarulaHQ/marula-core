-- Validates the constraint migration 0242 added NOT VALID.
--
-- Adding it NOT VALID kept 0242's exclusive lock off a scan of every purchase
-- line: new and changed rows were checked from the moment it existed, and this
-- transaction proves the rows that predate it. A row that names the `tools`
-- destination would fail here, which is the point — the destination is reached
-- by the serial insert trigger from the item's tool flag, never by a purchase.

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_no_tools_destination;
