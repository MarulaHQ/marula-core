-- Migration 0224: one reservation per host address.
--
-- `knowledge_ip_reservations.address` is `inet`, and the unique key is
-- `UNIQUE (subnet_id, address)`. An `inet` value carries its mask, so
-- `10.0.0.5/24` and `10.0.0.5/32` are two different values and two rows for one
-- host both satisfy the key. Scope's own operator (`address <<= cidr`) treats
-- them as the same host, which is the inconsistency this migration removes.
--
-- Two steps:
--
--   1. Canonicalise what is stored. A reservation is a host, never a network,
--      so every existing row keeps its address and drops its mask.
--      `insert_reservation` now stores `host(…)` itself, so this is the only
--      rewrite and it is filtered to the rows that carry a mask.
--   2. A unique index on the host form, which is the backstop the old key
--      lacked for any writer that bypasses the query module.
--
-- An install that already holds both `10.0.0.5/24` and `10.0.0.5/32` for one
-- subnet cannot be repaired by guessing which row is real: step 1 raises the
-- uniqueness error that names the pair, and an operator deletes or re-keys one
-- before re-running. Refusing is deliberate — silently dropping a reservation
-- would hide an address someone believes is accounted for.
--
-- Additive otherwise: nothing is dropped, and no row is removed.

UPDATE knowledge_ip_reservations
   SET address = host(address)::inet
 WHERE address <> host(address)::inet;

CREATE UNIQUE INDEX IF NOT EXISTS uq_knowledge_ip_reservations_host
  ON knowledge_ip_reservations (subnet_id, host(address));
