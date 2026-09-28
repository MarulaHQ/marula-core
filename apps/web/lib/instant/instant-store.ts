// InstantStore — the reload-the-cache-not-the-page client model (IF4/IF5).
//
// Purpose: one normalized cache of per-collection read models (board,
// ticket detail, list, calendar). Hydration is once-per-session per
// collection: already-known data never refetches and never flashes a
// loading state. Live WS events arrive as deltas — a coalesced, scoped
// revalidation of the affected slice, not a page reload. Mutation
// responses reconcile through the entity ledger keyed by the IF2 seq, so
// out-of-order responses and idempotent replays cannot regress state.
//
// Non-purpose: no transport (socket.ts), no persistence (the outbox is
// the durable part; the cache refetches snapshots on reload), no React
// (use-instant.ts binds with useSyncExternalStore).

import { EntityLedger } from "./entity-ledger";
import type { InstantEvent } from "./events";

export interface ReadModelContext {
  /** True while an optimistic write is in flight for this entity. */
  isPending(entityId: string): boolean;
}

export interface MutationMetaLike {
  after: unknown;
  before: unknown;
  seq: number;
  updated_at: string;
  version: number;
  idempotency_key?: string | null;
}

/**
 * One per-collection read model. `hydrate` is the cheap snapshot;
 * `relevant` routes live events; `revalidate` applies one event as a
 * delta (a scoped refetch or a payload merge) and returns the next data.
 */
export interface ReadModelSpec<T> {
  hydrate(): Promise<T>;
  relevant(event: InstantEvent): boolean;
  revalidate(data: T, event: InstantEvent, ctx: ReadModelContext): Promise<T>;
  /**
   * Merges one authoritative entity snapshot (from a mutation envelope)
   * into the collection. Optional: collections without it only reconcile
   * through explicit patches and revalidation.
   */
  applyEntity?(data: T, entity: unknown): T;
  /** The entity ids a snapshot currently contains, for mutation routing. */
  entityIds(data: T): string[];
}

interface Entry {
  spec: ReadModelSpec<unknown>;
  data: unknown;
  hydrated: boolean;
  inflight: Promise<unknown> | null;
  queuedEvents: InstantEvent[];
  draining: boolean;
  dirtyAgain: boolean;
}

/** Bounded identity LRU: at-least-once delivery (org + ticket scope
 * overlap, Last-Event-ID replay racing live frames) dedupes by frame
 * identity instead of revalidating twice. */
const DEDUPE_CAPACITY = 256;

function frameKey(event: InstantEvent): string {
  return `${event.type}|${event.entity}|${event.entity_id ?? ""}|${event.seq ?? ""}|${event.occurred_at}`;
}

export class InstantStore {
  readonly ledger = new EntityLedger();
  private readonly entries = new Map<string, Entry>();
  private readonly listeners = new Set<() => void>();
  private readonly context: ReadModelContext = {
    isPending: (entityId) => this.ledger.isPending(entityId),
  };

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  /** Synchronous cache read for render — undefined until first hydration. */
  peek<T>(key: string): T | undefined {
    return this.entries.get(key)?.data as T | undefined;
  }

  isHydrated(key: string): boolean {
    return this.entries.get(key)?.hydrated ?? false;
  }

  /**
   * Hydrate-once: the first caller per key fetches the snapshot; everyone
   * else (including later mounts of the same view) reads the cache with
   * no network. Concurrent first calls share the in-flight fetch.
   */
  async ensure<T>(key: string, spec: ReadModelSpec<T>): Promise<T> {
    const entry = this.entryFor(key, spec as ReadModelSpec<unknown>);
    if (entry.hydrated) return entry.data as T;
    if (entry.inflight !== null) return (await entry.inflight) as T;
    entry.inflight = spec
      .hydrate()
      .then((data) => {
        entry.data = data;
        entry.hydrated = true;
        this.notify();
        return data;
      })
      .finally(() => {
        entry.inflight = null;
      });
    return (await entry.inflight) as T;
  }

  /** Explicit re-snapshot (user-driven refresh); bypasses hydrate-once. */
  async refresh<T>(key: string, spec: ReadModelSpec<T>): Promise<T> {
    const entry = this.entryFor(key, spec as ReadModelSpec<unknown>);
    if (entry.inflight !== null) return (await entry.inflight) as T;
    entry.inflight = spec
      .hydrate()
      .then((data) => {
        entry.data = data;
        entry.hydrated = true;
        this.notify();
        return data;
      })
      .finally(() => {
        entry.inflight = null;
      });
    return (await entry.inflight) as T;
  }

  /**
   * Wave 38 cross-window leg: re-snapshot every hydrated read model that
   * currently holds `entityId` — the receiving side of a window-bus
   * entity-changed ping when the mutation envelope alone cannot merge the
   * change (comment posts, structured payloads). Concurrent pings dedupe
   * through each entry's inflight promise; failures leave the next SSE
   * frame to reconcile.
   */
  async refreshHolding(entityId: string): Promise<void> {
    for (const [key, entry] of this.entries.entries()) {
      if (!entry.hydrated || entry.data === undefined) continue;
      if (!entry.spec.entityIds(entry.data).includes(entityId)) continue;
      try {
        await this.refresh(key, entry.spec);
      } catch {
        // The next event (or a user action) reconciles — never wedged.
      }
    }
  }

  /**
   * Synchronous optimistic/authoritative patch. No-op when the collection
   * is not hydrated yet (hydration will carry the truth).
   */
  patch<T>(key: string, updater: (data: T) => T): void {
    const entry = this.entries.get(key);
    if (!entry || !entry.hydrated) return;
    const next = updater(entry.data as T);
    if (next === entry.data) return;
    entry.data = next;
    this.notify();
  }

  /** Routes one live event to interested collections as a delta. */
  applyEvent(event: InstantEvent): void {
    const key = frameKey(event);
    if (this.recentFrames.has(key)) return;
    this.recentFrames.add(key);
    if (this.recentFrames.size > DEDUPE_CAPACITY) {
      const oldest = this.recentOrder.shift();
      if (oldest !== undefined) this.recentFrames.delete(oldest);
    }
    this.recentOrder.push(key);
    for (const [key, entry] of this.entries) {
      if (!entry.hydrated) continue;
      if (!entry.spec.relevant(event)) continue;
      entry.queuedEvents.push(event);
      this.scheduleDrain(key, entry);
    }
  }

  private readonly recentFrames = new Set<string>();
  private readonly recentOrder: string[] = [];

  /**
   * The reconnect fallback sweep (instant-first live leg): re-snapshots
   * every HYDRATED collection once. Used when a stream reconnect carried
   * a Last-Event-ID cursor — replay covers the bounded gap, and past the
   * replay buffer this sweep is the documented recovery; the seq ledger
   * makes the two mutually idempotent. Never-hydrated entries stay put
   * (their first `ensure` will fetch fresh anyway).
   */
  async rehydrateAll(): Promise<void> {
    const sweeps: Promise<unknown>[] = [];
    for (const entry of this.entries.values()) {
      if (!entry.hydrated || entry.inflight !== null) continue;
      entry.inflight = entry.spec
        .hydrate()
        .then((data) => {
          if (entry.hydrated) {
            entry.data = data;
            this.notify();
          }
        })
        .finally(() => {
          entry.inflight = null;
        });
      sweeps.push(entry.inflight);
    }
    await Promise.all(sweeps);
  }

  /**
   * Reconciles one authoritative mutation envelope (IF2): the ledger
   * decides applied/duplicate/stale by seq, and applied snapshots merge
   * into every hydrated collection currently holding that entity.
   */
  applyMutation(
    entityId: string,
    meta: MutationMetaLike,
    opts: { skipApply?: boolean } = {},
  ): "applied" | "duplicate" | "stale" {
    const verdict = this.ledger.observe(entityId, {
      seq: meta.seq,
      updatedAt: meta.updated_at,
    });
    if (verdict !== "applied" || opts.skipApply || meta.after === null) {
      return verdict;
    }
    const entity = meta.after;
    for (const entry of this.entries.values()) {
      if (!entry.hydrated || entry.spec.applyEntity === undefined) continue;
      if (entry.data === undefined) continue;
      const holds = entry.spec.entityIds(entry.data).includes(entityId);
      if (!holds) continue;
      const next = entry.spec.applyEntity(entry.data, entity);
      if (next !== entry.data) {
        entry.data = next;
        this.notify();
      }
    }
    return verdict;
  }

  /**
   * Routes a pre-write entity snapshot back into the holding collections
   * WITHOUT touching the ledger — used when an offline-replayed op is
   * rejected after its optimistic patch was already rendered. The ledger
   * never observes reverts: they restore rendered truth, not write order.
   */
  revertEntity(entityId: string, before: unknown): void {
    if (before === null || before === undefined) return;
    for (const entry of this.entries.values()) {
      if (!entry.hydrated || entry.spec.applyEntity === undefined) continue;
      if (entry.data === undefined) continue;
      const holds = entry.spec.entityIds(entry.data).includes(entityId);
      if (!holds) continue;
      const next = entry.spec.applyEntity(entry.data, before);
      if (next !== entry.data) {
        entry.data = next;
        this.notify();
      }
    }
  }

  private entryFor(key: string, spec: ReadModelSpec<unknown>): Entry {
    let entry = this.entries.get(key);
    if (entry === undefined) {
      entry = {
        spec,
        data: undefined,
        hydrated: false,
        inflight: null,
        queuedEvents: [],
        draining: false,
        dirtyAgain: false,
      };
      this.entries.set(key, entry);
    }
    return entry;
  }

  private scheduleDrain(key: string, entry: Entry): void {
    if (entry.draining) {
      entry.dirtyAgain = true;
      return;
    }
    entry.draining = true;
    void this.drain(key, entry);
  }

  /** One revalidation per queued event, in arrival order, off the hot path. */
  private async drain(key: string, entry: Entry): Promise<void> {
    try {
      do {
        entry.dirtyAgain = false;
        while (entry.queuedEvents.length > 0) {
          const event = entry.queuedEvents.shift();
          if (event === undefined || entry.data === undefined) continue;
          const next = await entry.spec.revalidate(
            entry.data,
            event,
            this.context,
          );
          if (next !== entry.data) {
            entry.data = next;
            this.notify();
          }
        }
      } while (entry.dirtyAgain);
    } finally {
      entry.draining = false;
    }
  }

  private notify(): void {
    for (const fn of [...this.listeners]) fn();
  }
}
