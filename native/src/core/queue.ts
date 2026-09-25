// Typed offline write queue.
//
// Purpose: every mutation the app makes is durable and replayable. Ops are
// per-entity FIFO (a ticket's note can never overtake an earlier note on
// the same ticket), carry a client-minted idempotency key so replay lands
// exactly once, and record an append-only audit trail per op. A version
// conflict (HTTP 409 / version_conflict) resolves as last-write: the engine
// re-reads the server's current version, re-applies the user's write with
// the fresh version, and records the resolution in the audit — never a
// silent merge. A typed business rejection is surfaced, never dropped.
//
// Non-purpose: no transport knowledge (the engine injects `send`) and no
// UI. This module is pure TypeScript so the replay invariants are unit
// tested without a device.

import type { FailureKind } from "./errors";
import type { KeyValueStore } from "./storage";
import type { UuidFn } from "./idempotency";

export type QueueAuditKind =
  | "enqueued"
  | "sent"
  | "applied"
  | "retry"
  | "conflict_resolved"
  | "rejected"
  | "dropped";

export interface QueueAuditEntry {
  at: string;
  kind: QueueAuditKind;
  detail?: string;
}

export type OpMethod = "POST" | "PATCH" | "PUT" | "DELETE";

export interface QueueOpInput {
  /** Write-operation discriminator; replay looks up the typed sender. */
  kind: string;
  /** Ordering scope — all ops for one entity replay in enqueue order. */
  entityKey: string;
  /** Version-registry key (defaults to entityKey at the engine). */
  versionKey: string;
  label: string;
  method: OpMethod;
  /** Canonical /api/v1 path (the SDK's canonical surface). */
  path: string;
  body: Record<string, unknown>;
  idempotencyKey: string;
  /** Entity version the write was built against (IF2 If-Match). */
  version?: number;
  /** Cache keys reconciled after the op applies (persisted, so a replay
   * after relaunch still triggers the right refetches). */
  invalidate: string[];
}

export type QueueOpState = "pending" | "inflight" | "applied" | "rejected";

export interface QueuedOp extends QueueOpInput {
  id: string;
  enqueuedAt: string;
  state: QueueOpState;
  attempts: number;
  failure?: { kind: FailureKind["kind"]; reason?: string; message: string; status?: number };
  audit: QueueAuditEntry[];
}

export interface TransportSuccess {
  ok: true;
  data: unknown;
}

export interface TransportFailure {
  ok: false;
  kind: FailureKind["kind"];
  reason?: string;
  message: string;
  status?: number;
  transient: boolean;
  conflict: boolean;
  details?: Record<string, unknown>;
}

export type TransportOutcome = TransportSuccess | TransportFailure;
export type OpTransport = (op: QueuedOp) => Promise<TransportOutcome>;

export interface ConflictResolution {
  retry: boolean;
  /** Version to stamp on the retried body (last-write resolution). */
  version?: number;
  detail: string;
}

export interface WriteQueueDeps {
  store: KeyValueStore;
  send: OpTransport;
  /** Reads the server's current entity version and decides how to resolve. */
  resolveConflict: (op: QueuedOp, failure: TransportFailure) => Promise<ConflictResolution>;
  /** Called after an op applies (or dedupes) — refresh the entity there. */
  onSettled?: (op: QueuedOp, outcome: TransportOutcome) => Promise<void> | void;
  now?: () => Date;
  uuid: UuidFn;
  maxTransientAttempts?: number;
  prefix?: string;
}

export interface FlushReport {
  attempted: number;
  applied: number;
  rejected: number;
  deferred: number;
  offline: boolean;
}

const DEFAULT_PREFIX = "meridian.rn.queue.";
const DEFAULT_MAX_ATTEMPTS = 5;

export class WriteQueue {
  private readonly store: KeyValueStore;
  private readonly send: OpTransport;
  private readonly resolveConflict: WriteQueueDeps["resolveConflict"];
  private readonly onSettled: WriteQueueDeps["onSettled"];
  private readonly now: () => Date;
  private readonly uuid: UuidFn;
  private readonly maxTransientAttempts: number;
  private readonly prefix: string;

  private ops: QueuedOp[] = [];
  private readonly recentApplied: QueuedOp[] = [];
  private listeners = new Set<() => void>();
  private revision = 0;
  private flushing: Promise<FlushReport> | null = null;

  constructor(deps: WriteQueueDeps) {
    this.store = deps.store;
    this.send = deps.send;
    this.resolveConflict = deps.resolveConflict;
    this.onSettled = deps.onSettled;
    this.now = deps.now ?? (() => new Date());
    this.uuid = deps.uuid;
    this.maxTransientAttempts = deps.maxTransientAttempts ?? DEFAULT_MAX_ATTEMPTS;
    this.prefix = deps.prefix ?? DEFAULT_PREFIX;
  }

  async hydrate(): Promise<void> {
    const raw = await this.store.get(this.opsKey);
    if (raw === null) return;
    try {
      const parsed = JSON.parse(raw) as QueuedOp[];
      this.ops = parsed
        .filter((op) => op.state !== "applied")
        .map((op) => ({ ...op, state: op.state === "inflight" ? "pending" : op.state }));
    } catch {
      this.ops = [];
    }
    this.revision += 1;
  }

  async enqueue(input: QueueOpInput): Promise<QueuedOp> {
    const op: QueuedOp = {
      ...input,
      id: this.uuid(),
      enqueuedAt: this.now().toISOString(),
      state: "pending",
      attempts: 0,
      audit: [{ at: this.now().toISOString(), kind: "enqueued" }],
    };
    this.ops.push(op);
    await this.persist();
    this.emit();
    return op;
  }

  /** Serially drains pending ops. Entities are ordered; one entity's
   * failure blocks only its own later ops. Single-flight per queue. */
  flush(): Promise<FlushReport> {
    if (this.flushing !== null) return this.flushing;
    this.flushing = this.flushInner().finally(() => {
      this.flushing = null;
    });
    return this.flushing;
  }

  private async flushInner(): Promise<FlushReport> {
    const report: FlushReport = {
      attempted: 0,
      applied: 0,
      rejected: 0,
      deferred: 0,
      offline: false,
    };
    const blocked = new Set<string>();
    for (const op of [...this.ops]) {
      if (op.state !== "pending") continue;
      if (blocked.has(op.entityKey)) {
        report.deferred += 1;
        continue;
      }
      report.attempted += 1;
      op.state = "inflight";
      op.attempts += 1;
      this.audit(op, "sent", `attempt ${op.attempts}`);
      await this.persist();
      this.emit();
      const outcome = await this.send(op);
      if (outcome.ok) {
        op.state = "applied";
        this.audit(op, "applied");
        report.applied += 1;
        this.recentApplied.push(op);
        this.ops = this.ops.filter((candidate) => candidate.id !== op.id);
        await this.persist();
        this.emit();
        await this.onSettled?.(op, outcome);
        continue;
      }
      if (outcome.reason === "idempotency_key_reused") {
        // The server already applied this exact key — replay is a no-op.
        op.state = "applied";
        this.audit(op, "applied", "already applied (idempotent replay)");
        report.applied += 1;
        this.recentApplied.push(op);
        this.ops = this.ops.filter((candidate) => candidate.id !== op.id);
        await this.persist();
        this.emit();
        await this.onSettled?.(op, outcome);
        continue;
      }
      if (outcome.conflict) {
        const resolution = await this.tryResolveConflict(op, outcome);
        if (resolution) {
          report.applied += 1;
          continue;
        }
        report.rejected += 1;
        blocked.add(op.entityKey);
        continue;
      }
      if (outcome.kind === "offline" || outcome.kind === "timeout" || outcome.transient) {
        if (op.attempts >= this.maxTransientAttempts) {
          op.state = "rejected";
          op.failure = failureOf(outcome);
          this.audit(op, "rejected", "retry budget exhausted while offline");
          report.rejected += 1;
        } else {
          op.state = "pending";
          op.failure = failureOf(outcome);
          this.audit(op, "retry", outcome.message);
          report.deferred += 1;
          report.offline = true;
          blocked.add(op.entityKey);
        }
        await this.persist();
        this.emit();
        continue;
      }
      op.state = "rejected";
      op.failure = failureOf(outcome);
      this.audit(op, "rejected", outcome.message);
      report.rejected += 1;
      blocked.add(op.entityKey);
      await this.persist();
      this.emit();
    }
    return report;
  }

  /** One conflict resolution attempt; on success the op is applied, on a
   * second conflict it is rejected. Returns true when applied. */
  private async tryResolveConflict(op: QueuedOp, failure: TransportFailure): Promise<boolean> {
    const resolution = await this.resolveConflict(op, failure);
    if (!resolution.retry) {
      op.state = "rejected";
      op.failure = failureOf(failure);
      this.audit(op, "rejected", resolution.detail);
      await this.persist();
      this.emit();
      return false;
    }
    const before = op.version;
    if (resolution.version !== undefined) {
      op.version = resolution.version;
      op.body = { ...op.body, version: resolution.version };
    }
    this.audit(
      op,
      "conflict_resolved",
      `${resolution.detail} (version ${before ?? "none"} -> ${resolution.version ?? "unknown"})`,
    );
    op.state = "inflight";
    op.attempts += 1;
    await this.persist();
    this.emit();
    const retry = await this.send(op);
    if (retry.ok) {
      op.state = "applied";
      this.audit(op, "applied", "after conflict resolution");
      this.recentApplied.push(op);
      this.ops = this.ops.filter((candidate) => candidate.id !== op.id);
      await this.persist();
      this.emit();
      await this.onSettled?.(op, retry);
      return true;
    }
    op.state = "rejected";
    op.failure = failureOf(retry);
    this.audit(op, "rejected", retry.message);
    await this.persist();
    this.emit();
    return false;
  }

  /** Re-arm a rejected op after the user fixes the cause. */
  async retryRejected(id: string): Promise<void> {
    const op = this.ops.find((candidate) => candidate.id === id);
    if (op === undefined || op.state !== "rejected") return;
    op.state = "pending";
    op.attempts = 0;
    op.failure = undefined;
    this.audit(op, "retry", "re-armed by user");
    await this.persist();
    this.emit();
  }

  async drop(id: string): Promise<void> {
    const op = this.ops.find((candidate) => candidate.id === id);
    if (op === undefined) return;
    this.audit(op, "dropped", "discarded by user");
    this.ops = this.ops.filter((candidate) => candidate.id !== id);
    await this.persist();
    this.emit();
  }

  /** Logout/org-switch: queued work never crosses sessions. */
  async clear(): Promise<void> {
    for (const op of this.ops) this.audit(op, "dropped", "session ended");
    this.ops = [];
    this.recentApplied.length = 0;
    await this.persist();
    this.emit();
  }

  pending(): QueuedOp[] {
    return this.ops.filter((op) => op.state === "pending" || op.state === "inflight");
  }

  rejected(): QueuedOp[] {
    return this.ops.filter((op) => op.state === "rejected");
  }

  pendingCount(): number {
    return this.pending().length;
  }

  appliedThisSession(): QueuedOp[] {
    return [...this.recentApplied];
  }

  snapshot(): QueuedOp[] {
    return [...this.recentApplied, ...this.ops];
  }

  getRevision(): number {
    return this.revision;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private audit(op: QueuedOp, kind: QueueAuditKind, detail?: string): void {
    op.audit.push({
      at: this.now().toISOString(),
      kind,
      ...(detail !== undefined ? { detail } : {}),
    });
  }

  private async persist(): Promise<void> {
    const durable = this.ops.filter((op) => op.state !== "applied");
    await this.store.set(this.opsKey, JSON.stringify(durable));
  }

  private emit(): void {
    this.revision += 1;
    for (const listener of this.listeners) listener();
  }

  private get opsKey(): string {
    return `${this.prefix}ops`;
  }
}

function failureOf(outcome: TransportFailure): QueuedOp["failure"] {
  return {
    kind: outcome.kind,
    ...(outcome.reason !== undefined ? { reason: outcome.reason } : {}),
    message: outcome.message,
    ...(outcome.status !== undefined ? { status: outcome.status } : {}),
  };
}