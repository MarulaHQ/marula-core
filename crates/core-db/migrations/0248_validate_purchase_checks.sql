-- Validates the purchase CHECK constraints migration 0235 added NOT VALID.
-- 0247 is the e-waste correction; this purchase validation is the next free
-- number after it.
--
-- Adding them NOT VALID kept that migration's exclusive locks off a scan of
-- every purchase order, line and receipt: new and changed rows were checked from
-- the moment each constraint existed, and this transaction proves the rows that
-- predate them. `VALIDATE CONSTRAINT` takes a share-update lock, so reads and
-- writes continue while it scans.
--
-- The two tables 0235 created are not here: their constraints were declared with
-- their first rows, so there is nothing to validate.

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_acquired_for_check;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_delivery_mode_check;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_ownership_check;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_client_shape;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_drop_ship_needs_client;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_client_drop_ship_shape;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_vendor_rental_shape;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_recharge_rule_shape;

ALTER TABLE purchase_orders
  VALIDATE CONSTRAINT purchase_orders_recharge_needs_client;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_line_kind_check;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_destination_class_check;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_weight_check;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_sell_price_check;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_serial_shape;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_serialized_expects_serial;

ALTER TABLE purchase_order_lines
  VALIDATE CONSTRAINT purchase_order_lines_pass_through_shape;

ALTER TABLE purchase_receipt_lines
  VALIDATE CONSTRAINT purchase_receipt_lines_destination_class_check;
