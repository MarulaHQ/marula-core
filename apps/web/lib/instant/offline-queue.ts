// OfflineQueue — IF6 offline replay.
//
// Purpose: writes that fail for network reasons queue locally (localStorage
// — survives reloads) and replay STRICTLY in order on reconnect. Every op
// ends accepted (with its IF2 mutation envelope) or rejected (with the
// IF3 typed reason), and each outcome is surfaced to listeners. Replay
// stops at the first op that still cannot reach the network so queue order
// — and therefore server-side apply order — is preserved across sessions.
//
// Non-purpose: no UI (the dock renders outcomes), no retry policy beyond
// in-order single attempts per trigger, no storage beyond one key.

export interface QueuedWrite {
  /** Idempotency key assigned at enqueue time — stable across replays. */
  key: string;
  method: "POST" | "PATCH" | "PUT" | "DELETE";
  path: string;
  /** Full request body; already carries idempotency_key (+ version). */
  body: unknown;
  /** Human label for surfacing ("Set status Waiting on MER-12"). */
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
  storageKey?: string;
  invoke: InvokeFn;
  now?: () => string;
  /** Outcomes retained for surfacing (per session). */
  outcomeHistory?: number;
}

const QUEUE_KEY = "marula-instant-outbox";

export class OfflineQueue {
  private readonly storage: OfflineQueueOptions["storage"];
  private readonly storageKey: string;
  private readonly invoke: InvokeFn;
  private readonly nowFn: () => string;
  private readonly outcomeHistory: number;
  private queue: QueuedWrite[] = [];
  private readonly outcomeListeners = new Set<(o: WriteOutcome) => void>();
  private readonly stateListeners = new Set<() => void>();
  private readonly outcomes: WriteOutcome[] = [];
  private replaying = false;

  constructor(options: OfflineQueueOptions) {
    this.storage = options.storage;
    this.storageKey = options.storageKey ?? QUEUE_KEY;
    this.invoke = options.invoke;
    this.nowFn = options.now ?? (() => new Date().toISOString());
    this.outcomeHistory = options.outcomeHistory ?? 20;
    this.queue = this.load();
  }

  private load(): QueuedWrite[] {
    const raw = this.storage.getItem(this.storageKey);
    if (raw === null) return [];
    try {
      const parsed = JSON.parse(raw) as QueuedWrite[];
      return Array.isArray(parsed) ? parsed : [];
    } catch {
      return [];
    }
  }

  private persist(): void {
    if (this.queue.length === 0) this.storage.removeItem(this.storageKey);
    else this.storage.setItem(this.storageKey, JSON.stringify(this.queue));
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
   * Accepted and rejected ops leave the queue; both record outcomes.
   */
  async replay(): Promise<void> {
    if (this.replaying) return;
    this.replaying = true;
    try {
      while (this.queue.length > 0) {
        const op = this.queue[0];
        if (op === undefined) break;
        let result: InvokeResult;
        try {
          result = await this.invoke(op);
        } catch {
          result = { outcome: "network" };
        }
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
