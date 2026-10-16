// Envelope mutation helper for low-frequency staff surfaces (Wave 9
// close-out: billing, BYOK, agent platform).
//
// Purpose: one flow for instant-first (IF1/IF2) mutations on screens that
// are not instant-runtime channels: optimistic local patch, idempotency
// key, If-Match version from the last observed envelope, authoritative
// reconcile from the response body, typed-rejection rollback. Written
// against the generated SDK client so path/body/response types stay
// contract-checked.
//
// Non-purpose: no durable offline queue (the instant runtime owns that
// for board/ticket writes); on network failure these surfaces surface the
// error and let the user retry with a fresh key.

import { api } from "./api";
import { newIdempotencyKey } from "./instant/instant-write";
import { friendlyMutationError } from "./error-copy";

type ApiClient = typeof api.api;

/** The openapi-fetch response tuple this helper reconciles from. */
type EnvelopeResponse<R> = {
  data?: R;
  error?: unknown;
  response: Response;
};

/** The per-entity version cursor this module tracks (mutation.version). */
export type VersionOf = (entityId: string) => number | undefined;

const versions = new Map<string, number>();

export function rememberVersion(entityId: string, version: number | null | undefined): void {
  if (typeof version === "number") versions.set(entityId, version);
}

/** Stamps a mutation body's MutationParams members from the tracked cursor. */
export function withVersion(
  entityId: string,
  body: Record<string, unknown>,
): Record<string, unknown> {
  const known = versions.get(entityId);
  return known === undefined ? body : { ...body, version: known };
}

export interface EnvelopeResult {
  ok: boolean;
  /** IF3 machine reason on rejection, when the server sent one. */
  reason?: string;
  message?: string;
  /** Friendly user copy from the central reason-code map (error-copy). */
  friendly?: string;
  /** True when the failure was offline/timeout (safe to retry later). */
  transient?: boolean;
}

function reasonOf(error: unknown): { reason?: string; message?: string } {
  if (error !== null && typeof error === "object") {
    const e = error as { reason?: unknown; message?: unknown; error?: { message?: unknown } };
    return {
      reason: typeof e.reason === "string" ? e.reason : undefined,
      message:
        typeof e.message === "string"
          ? e.message
          : typeof e.error?.message === "string"
            ? e.error.message
            : undefined,
    };
  }
  return {};
}

/** One gated mutation call — everything runEnvelopeMutation needs. */
export interface EnvelopeMutation<P, R> {
  describe: string;
  entityId?: string;
  buildBody(base: { idempotency_key: string; version?: number }): P;
  /** Optimistic local patch — runs before the wire. */
  optimistic(): void;
  /** Roll the optimistic patch back on typed rejection. */
  revert(): void;
  /** The typed client call; returns the response envelope. */
  request(client: ApiClient, body: P): Promise<EnvelopeResponse<R>>;
  /** Extract (entityId, mutation.version) pairs to remember. */
  versions?(response: R): [string, number][];
  /**
   * Reconcile from the applied snapshot the server returned (IF2
   * read-your-write). A CREATE has no local insert to correct, so this is
   * the only synchronous way the new row reaches the cache — the live
   * delta is a second path, never the only one. Runs after success only.
   */
  onApplied?(response: R): void;
}

/**
 * Runs one gated mutation: optimistic patch synchronously, wire with a
 * fresh idempotency key (+ tracked version when one is known), reconcile
 * the local patch from the response envelope, roll the patch back on a
 * typed rejection.
 */
export async function runEnvelopeMutation<P extends { idempotency_key?: string | null; version?: number | null }, R>(
  options: EnvelopeMutation<P, R>,
): Promise<EnvelopeResult> {
  options.optimistic();
  const key = newIdempotencyKey();
  const known = options.entityId !== undefined ? versions.get(options.entityId) : undefined;
  const body = options.buildBody({ idempotency_key: key, version: known });
  try {
    const result = await options.request(api.api, body);
    if (result.error !== undefined) {
      options.revert();
      const parsed = reasonOf(result.error);
      const friendly = friendlyMutationError(result.error);
      return {
        ok: false,
        reason: parsed.reason,
        message: parsed.message,
        friendly: friendly.text,
        transient: friendly.transient,
      };
    }
    if (options.versions !== undefined && result.data !== undefined) {
      for (const [id, version] of options.versions(result.data)) {
        rememberVersion(id, version);
      }
    } else if (options.entityId !== undefined && result.data !== undefined) {
      // Default echo: `mutation.version` is the token the NEXT write to the
      // same entity must carry, so a second write in one dialog (a second
      // kit member, another PO line, one more vendor link) advances the
      // cursor instead of replaying a stale one.
      const meta = (result.data as { mutation?: { version?: unknown } }).mutation;
      if (meta !== undefined && typeof meta.version === "number") {
        rememberVersion(options.entityId, meta.version);
      }
    }
    if (options.onApplied !== undefined && result.data !== undefined) {
      options.onApplied(result.data);
    }
    return { ok: true };
  } catch (error) {
    options.revert();
    const friendly = friendlyMutationError(error);
    const parsed = reasonOf(error);
    return {
      ok: false,
      reason: parsed.reason,
      message: parsed.message ?? friendly.text,
      friendly: friendly.text,
      transient: friendly.transient,
    };
  }
}

export { newIdempotencyKey };
