// OfflineQueue — IF6 offline replay.
//
// Purpose: writes that fail for network reasons queue locally (localStorage
// — survives reloads) and replay STRICTLY in order on reconnect. Every op
// ends accepted (with its IF2 mutation envelope) or rejected (with the
// IF3 typed reason), and each outcome is surfaced to listeners. Replay
// stops at the first op that still cannot reach the network so queue order
// — and therefore server-side apply order — is preserved across sessions.
//
// The queue belongs to the signed-in user, not the browser: one storage key
// per account, and a user change drops the previous account's queued writes
// before the new account's queue loads. A token refresh rebinds the same id
// and keeps the queue; only a different user clears it.
//
// Non-purpose: no UI (the dock renders outcomes), no retry policy beyond
// in-order single attempts per trigger, no storage beyond one key per user.

export interface QueuedWrite {
  /** Idempotency key assigned at enqueue time — stable across replays. */
  key: string;
  method: "POST" | "PATCH" | "PUT" | "DELETE";
  path: string;
  /** Full request body; already carries idempotency_key (+ version). */
  body: unknown;
  /** Human label for surfacing ("Set status Waiting on MAR-12"). */
  describe: string;
  entityId?: string;
  /**
   * Real entity ids the mutation touches, for the Wave 38 window-bus echo
   * when `entityId` is a synthetic optimistic marker (`pending-…`).
   */
  echoEntities?: string[];
  /** Entity snapshot captured before the optimistic patch (durable revert). */
  before?: unknown;
  queuedAt: string;
}

export type WriteOutcomeStatus = "accepted" | "rejected" | "queued";

export interface WriteOutcome {
  key: string;
  describe: string;
  status: WriteOutcomeStatus;
  /** IF3 machine-readable reason code when rejected. */
  reason?: string;
  message?: string;
  at: string;
}

/** Result of one replay attempt against the transport. */
export type InvokeResult =
  | { outcome: "accepted"; mutation?: unknown }
  | { outcome: "rejected"; reason: string; message?: string }
  | { outcome: "network" };

export type InvokeFn = (op: QueuedWrite) => Promise<InvokeResult>;

export interface OfflineQueueOptions {
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  /** Base key. The user id is appended so each account owns its own outbox. */
  storageKey?: string;
  /**
   * The authenticated user this queue belongs to, when one is known.
   * `undefined` leaves the instance unscoped (harnesses and suites that own
   * no session); `null` means signed out, where nothing is kept.
   */
  userId?: string | null;
  invoke: InvokeFn;
  now?: () => string;
  /** Outcomes retained for surfacing (per session). */
  outcomeHistory?: number;
}

const QUEUE_KEY = "marula-instant-outbox";

export class OfflineQueue {
  private readonly storage: OfflineQueueOptions["storage"];
  private readonly baseKey: string;
  /** `undefined` = unscoped, `null` = signed out (nothing durable). */
  private userId: string | null | undefined;
  private readonly invoke: InvokeFn;
  private readonly nowFn: () => string;
  private readonly outcomeHistory: number;
  private queue: QueuedWrite[] = [];
  private readonly outcomeListeners = new Set<(o: WriteOutcome) => void>();
  private readonly stateListeners = new Set<() => void>();
  private readonly outcomes: WriteOutcome[] = [];
  private replaying = false;
  /** Bumped whenever the owning user changes, so an in-flight replay of the
   * previous account's ops stops instead of settling them into the next one. */
  private generation = 0;

  constructor(options: OfflineQueueOptions) {
    this.storage = options.storage;
    this.baseKey = options.storageKey ?? QUEUE_KEY;
    this.userId = options.userId;
    this.invoke = options.invoke;
    this.nowFn = options.now ?? (() => new Date().toISOString());
    this.outcomeHistory = options.outcomeHistory ?? 20;
    this.queue = this.load();
  }

  /** The key this queue persists under, or null when nothing may be kept.
   * The user id is the last segment: `marula-instant-outbox:<user id>`. */
  private get storageKey(): string | null {
    if (this.userId === undefined) return this.baseKey;
    return this.userId === null ? null : `${this.baseKey}:${this.userId}`;
  }

  /**
   * Binds the durable queue to the signed-in user (`userId`) or releases it
   * on sign-out (`null`). Rebinding the same id is a no-op, so a token
   * refresh keeps queued work. A different id drops the previous account's
   * queued writes before the new account's queue loads: replay never carries
   * one user's work into another user's session.
   */
  bindTo(userId: string | null): void {
    if (this.userId === userId) return;
    const previous = this.storageKey;
    if (previous !== null) this.storage.removeItem(previous);
    this.userId = userId;
    this.generation += 1;
    this.queue = this.load();
    this.outcomes.length = 0;
    for (const fn of [...this.stateListeners]) fn();
  }

  private load(): QueuedWrite[] {
    const key = this.storageKey;
    if (key === null) return [];
    const raw = this.storage.getItem(key);
    if (raw === null) return [];
    try {
      const parsed = JSON.parse(raw) as QueuedWrite[];
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }

  private persist(): void {
    const key = this.storageKey;
    if (key !== null) {
      if (this.queue.length === 0) this.storage.removeItem(key);
      else this.storage.setItem(key, JSON.stringify(this.queue));
    }
    for (const fn of [...this.stateListeners]) fn();
  }

  onOutcome(fn: (o: WriteOutcome) => void): () => void {
    this.outcomeListeners.add(fn);
    return () => {
      this.outcomeListeners.delete(fn);
    };
  }

  onState(fn: () => void): () => void {
    this.stateListeners.add(fn);
    return () => {
      this.stateListeners.delete(fn);
    };
  }

  get pendingCount(): number {
    return this.queue.length;
  }

  list(): QueuedWrite[] {
    return [...this.queue];
  }

  recentOutcomes(): WriteOutcome[] {
    return [...this.outcomes];
  }

  /** Appends one write to the durable queue (FIFO). */
  enqueue(op: Omit<QueuedWrite, "queuedAt"> & { queuedAt?: string }): void {
    this.queue.push({ queuedAt: this.nowFn(), ...op });
    this.persist();
  }

  /**
   * Replays the queue in order. Stops at the first op that still fails for
   * network reasons (it stays at the head; later ops never overtake it).
   * Also stops when the owning user changes mid-flight: the remaining ops
   * belong to an account that is no longer signed in.
   * Accepted and rejected ops leave the queue; both record outcomes.
   */
  async replay(): Promise<void> {
    if (this.replaying) return;
    this.replaying = true;
    const generation = this.generation;
    try {
      while (this.queue.length > 0 && this.generation === generation) {
        const op = this.queue[0];
        if (op === undefined) break;
        let result: InvokeResult;
        try {
          result = await this.invoke(op);
        } catch {
          result = { outcome: "network" };
        }
        if (this.generation !== generation) return;
        if (result.outcome === "network") return;
        this.queue.shift();
        if (result.outcome === "accepted") {
          this.record({
            key: op.key,
            describe: op.describe,
            status: "accepted",
            at: this.nowFn(),
          });
        } else {
          this.record({
            key: op.key,
            describe: op.describe,
            status: "rejected",
            reason: result.reason,
            message: result.message,
            at: this.nowFn(),
          });
        }
      }
    } finally {
      this.replaying = false;
      this.persist();
    }
  }

  private record(outcome: WriteOutcome): void {
    this.outcomes.push(outcome);
    if (this.outcomes.length > this.outcomeHistory) this.outcomes.shift();
    for (const fn of [...this.outcomeListeners]) fn(outcome);
  }
}
