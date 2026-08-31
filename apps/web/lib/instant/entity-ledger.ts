// Entity write ledger — the IF5 reconciliation core.
//
// Purpose: one monotonic acceptance rule per entity keyed by the IF2
// per-entity `seq` (mirrored by `version`), with an `updated_at` fallback
// for reads that carry no seq (plain GETs). Out-of-order mutation
// responses and idempotent replays resolve here instead of clobbering
// newer state in the cache.
//
// Non-purpose: no storage of entity data (the store owns data); no
// network; no React.

export type LedgerVerdict = "applied" | "duplicate" | "stale";

export interface EntityObservation {
  /** IF2 monotonic per-entity write sequence, when the source carries one. */
  seq?: number;
  /** Applied entity `updated_at` (ISO-8601). Usable alone for plain reads. */
  updatedAt?: string;
}

/** One entity's reconciliation cursor. */
interface EntityCursor {
  seq: number | null;
  updatedAt: string | null;
}

/**
 * RFC-3339 instants compare correctly through Date#parse for the UTC ("Z")
 * stamps the API emits; invalid timestamps rank oldest so they never gate
 * a write.
 */
function instantRank(iso: string): number {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t;
}

export class EntityLedger {
  private readonly cursors = new Map<string, EntityCursor>();
  private readonly pending = new Set<string>();

  /**
   * Observes one authoritative state for an entity and decides whether the
   * caller should apply it. `applied` advances the cursor; `duplicate`
   * means already-known state (safe to skip — an idempotent replay or an
   * unchanged read); `stale` means the observation is older than known
   * state and must be dropped.
   */
  observe(entityId: string, observation: EntityObservation): LedgerVerdict {
    const cursor = this.cursors.get(entityId) ?? { seq: null, updatedAt: null };
    this.cursors.set(entityId, cursor);

    let advanced = false;
    if (observation.seq !== undefined) {
      if (cursor.seq !== null) {
        if (observation.seq < cursor.seq) return "stale";
      }
      if (cursor.seq === null || observation.seq > cursor.seq) {
        cursor.seq = observation.seq;
        advanced = true;
      }
    }
    if (observation.updatedAt !== undefined) {
      const newer =
        cursor.updatedAt === null ||
        instantRank(observation.updatedAt) > instantRank(cursor.updatedAt);
      if (newer) {
        cursor.updatedAt = observation.updatedAt;
        advanced = true;
      }
    }
    return advanced ? "applied" : "duplicate";
  }

  /** The entity's last accepted IF2 version (undefined = never observed here). */
  version(entityId: string): number | undefined {
    const seq = this.cursors.get(entityId)?.seq;
    return seq === null || seq === undefined ? undefined : seq;
  }

  /**
   * Marks an optimistic write as in flight for an entity. While pending,
   * event-triggered refetches of that entity are suppressed so a slow read
   * cannot clobber the optimistic render with pre-write truth.
   */
  beginPending(entityId: string): void {
    if (entityId !== "") this.pending.add(entityId);
  }

  endPending(entityId: string): void {
    this.pending.delete(entityId);
  }

  isPending(entityId: string): boolean {
    return this.pending.has(entityId);
  }
}
