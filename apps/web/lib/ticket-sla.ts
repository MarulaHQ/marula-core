// SLA clock reads, shared by every ticket surface.
//
// Purpose: the resolution clock a customer is promised lives behind
// `GET /api/v1/tickets/{id}/sla`, which is per ticket. A ledger page therefore
// needs one call per row, and three surfaces asking for the same ticket must
// not each pay for it. This module is the one place that knows that, so:
//
//   1. RESULTS ARE CACHED BY TICKET ID for the session. Opening the ledger,
//      then Today, then a ticket in the workbench costs one read of that
//      ticket's clock, not three.
//   2. THE FAN-OUT IS CAPPED. Fifty rows means fifty calls, not fifty
//      SIMULTANEOUS ones — an uncapped Promise.all over a page is a
//      self-inflicted thundering herd against our own API.
//   3. IN-FLIGHT READS ARE SHARED. Two surfaces that want the same ticket
//      while its read is still open attach to that read instead of opening a
//      second one.
//
// A cached clock goes stale as time passes, which is why the cache stores the
// SERVER'S ANSWER, not a computed label: the label is derived per render from
// the cached deadline, so a row that was 12 minutes out reads 2 minutes out
// without a refetch. Only `applied`, `breached` and the state's `resolution_due`
// are cached, and those are the facts — everything else is derived.
//
// Non-purpose: this is not a bulk endpoint, and it does not pretend to be. The
// read is still per ticket; what changed is that it happens once per ticket
// per session rather than once per surface per render. Killing the per-ticket
// read entirely needs a batched API route, which is backend work this lane
// does not own.
"use client";

import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState } from "react";
import { api } from "./api";

export type TicketSlaStatus = components["schemas"]["TicketSlaStatus"];

/** How many clock reads may be in flight at once, across every surface. */
const CONCURRENCY = 6;

interface Waiter {
  id: string;
  resolve: (value: TicketSlaStatus | null) => void;
}

/** ticket id -> the server's answer. A cached `null` is a real answer (no
 *  policy applies, or the caller may not read it), not a cache miss. */
const cache = new Map<string, TicketSlaStatus | null>();
/** ticket id -> the promise every concurrent caller for that id is holding. */
const inFlight = new Map<string, Promise<TicketSlaStatus | null>>();
const waiters: Waiter[] = [];
let active = 0;

async function readOne(id: string): Promise<TicketSlaStatus | null> {
  try {
    const res = await api.api.GET("/api/v1/tickets/{id}/sla", {
      params: { path: { id } },
    });
    const value = res.data ?? null;
    cache.set(id, value);
    return value;
  } catch {
    // A failed read is cached as null on purpose: a ticket whose clock the
    // caller may not read must not become a fresh retry on every page view.
    // The countdown falls back to the ticket's own due date, which is honest.
    cache.set(id, null);
    return null;
  }
}

/**
 * Start queued reads, at most `CONCURRENCY` at a time.
 *
 * The waiter is settled HERE rather than by a later pass over the queue: once a
 * waiter is shifted off, nothing else holds a reference to its resolver, so
 * resolving by lookup would silently drop it.
 */
function pump(): void {
  while (active < CONCURRENCY && waiters.length > 0) {
    const waiter = waiters.shift();
    if (waiter === undefined) return;
    active += 1;
    const finish = (value: TicketSlaStatus | null): void => {
      active -= 1;
      inFlight.delete(waiter.id);
      waiter.resolve(value);
      pump();
    };
    void readOne(waiter.id).then(finish, () => finish(null));
  }
}

/**
 * Read one ticket's clock. Joins an in-flight read for the same id rather than
 * opening a second one, and returns the cached answer without any call at all.
 */
export function readTicketSla(id: string): Promise<TicketSlaStatus | null> {
  const cached = cache.get(id);
  if (cached !== undefined) return Promise.resolve(cached);
  const running = inFlight.get(id);
  if (running !== undefined) return running;

  let resolveWith: (value: TicketSlaStatus | null) => void = () => {};
  const promise = new Promise<TicketSlaStatus | null>((resolve) => {
    resolveWith = resolve;
  });
  inFlight.set(id, promise);
  waiters.push({ id, resolve: resolveWith });
  pump();
  return promise;
}

/** Drop one ticket's cached clock — call after a change that resets it. */
export function invalidateTicketSla(id: string): void {
  cache.delete(id);
}

/** Drop every cached clock. Used by tests and by sign-out. */
export function clearTicketSlaCache(): void {
  cache.clear();
  inFlight.clear();
  waiters.length = 0;
  active = 0;
}

/**
 * Resolve the clocks for a set of tickets, returning a map keyed by id.
 *
 * Seeded from the cache on every run, so revisiting a surface paints from
 * memory immediately and only reaches the network for ids this session has
 * never read.
 */
export function useTicketSlaMap(ids: readonly string[]): {
  clocks: Record<string, TicketSlaStatus | null>;
  pending: boolean;
} {
  // The joined key is the stable identity of the id SET. `ids` is a fresh array
  // on every render and must not itself be the dependency.
  const key = ids.join("|");
  const wanted = useMemo(() => (key === "" ? [] : key.split("|")), [key]);
  const [clocks, setClocks] = useState<Record<string, TicketSlaStatus | null>>({});
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (wanted.length === 0) {
      setClocks({});
      setPending(false);
      return;
    }
    let cancelled = false;

    const seeded: Record<string, TicketSlaStatus | null> = {};
    let missing = 0;
    for (const id of wanted) {
      const cached = cache.get(id);
      if (cached === undefined) missing += 1;
      else seeded[id] = cached;
    }
    setClocks(seeded);
    setPending(missing > 0);

    void Promise.all(wanted.map(async (id) => [id, await readTicketSla(id)] as const)).then(
      (settledPairs) => {
        if (cancelled) return;
        const next: Record<string, TicketSlaStatus | null> = {};
        for (const [id, value] of settledPairs) next[id] = value;
        setClocks(next);
        setPending(false);
      },
    );

    return () => {
      cancelled = true;
    };
  }, [wanted]);

  return { clocks, pending };
}
