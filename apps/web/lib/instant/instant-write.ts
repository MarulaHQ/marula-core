// Instant write executor — the optimistic write path (IF1 client side).
//
// Purpose: one flow for every autosave-grade mutation: generate an
// idempotency key, attach the If-Match version from the entity ledger,
// render the optimistic patch synchronously, then either reconcile from
// the IF2 read-your-write envelope, revert on a typed IF3 rejection, or
// hand the serialized op to the durable offline queue (IF6) on network
// failure. The same idempotency key rides the queued op, so replay can
// never double-apply.
//
// Non-purpose: no transport (runtime.ts adapts the SDK client), no UI.

import type { InstantStore, MutationMetaLike } from "./instant-store";
import type { OfflineQueue } from "./offline-queue";

/** Transport seam the runtime binds to the SDK client. */
export interface InstantTransportResult {
  outcome: "accepted" | "rejected" | "network";
  /** IF2 mutation envelope on accepted writes (when the endpoint gates). */
  mutation?: MutationMetaLike | null;
  /** IF3 machine-readable reason on rejected writes. */
  reason?: string;
  message?: string;
}

export interface InstantTransport {
  request(
    method: "POST" | "PATCH" | "PUT" | "DELETE",
    path: string,
    body: unknown,
  ): Promise<InstantTransportResult>;
}

export interface WriteSpec {
  /** Human label surfaced in the outbox ("Set status Waiting on MER-12"). */
  describe: string;
  /** The written entity's id, when known before the response (updates). */
  entityId?: string;
  method: "POST" | "PATCH" | "PUT";
  path: string;
  /**
   * Builds the request body from the IF1 flattened params. The executor
   * fills `idempotency_key` (fresh per edit episode) and `version` (the
   * last observed entity seq, when known).
   */
  buildBody(base: { idempotency_key: string; version?: number }): unknown;
  /** Optimistic render — runs synchronously, before the wire. */
  optimistic?(): void;
  /** Typed-rejection rollback for the optimistic render. */
  revert?(): void;
  /** Entity snapshot BEFORE the optimistic patch, for durable reverts. */
  captureBefore?(): unknown;
}

export type WriteResult =
  | { status: "accepted"; mutation: MutationMetaLike | null }
  | { status: "rejected"; reason: string; message?: string }
  | { status: "queued"; key: string };

export function newIdempotencyKey(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c !== undefined && typeof c.randomUUID === "function") return c.randomUUID();
  return `key-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function executeInstantWrite(
  store: InstantStore,
  queue: OfflineQueue,
  transport: InstantTransport,
  spec: WriteSpec,
): Promise<WriteResult> {
  const key = newIdempotencyKey();
  const version =
    spec.entityId !== undefined ? store.ledger.version(spec.entityId) : undefined;
  const body = spec.buildBody({ idempotency_key: key, version });
  const before = spec.captureBefore?.();

  spec.optimistic?.();
  if (spec.entityId !== undefined) store.ledger.beginPending(spec.entityId);

  return transport.request(spec.method, spec.path, body).then((result) => {
    if (result.outcome === "network") {
      queue.enqueue({
        key,
        method: spec.method,
        path: spec.path,
        body,
        describe: spec.describe,
        entityId: spec.entityId,
        before,
      });
      return { status: "queued", key } as WriteResult;
    }
    if (spec.entityId !== undefined) store.ledger.endPending(spec.entityId);
    if (result.outcome === "rejected") {
      spec.revert?.();
      return {
        status: "rejected",
        reason: result.reason ?? "conflict",
        message: result.message,
      } as WriteResult;
    }
    // Accepted: reconcile the authoritative snapshot through the ledger.
    if (spec.entityId !== undefined && result.mutation !== undefined && result.mutation !== null) {
      store.applyMutation(spec.entityId, result.mutation);
    }
    return { status: "accepted", mutation: result.mutation ?? null } as WriteResult;
  });
}
