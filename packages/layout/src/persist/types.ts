/**
 * @module Pluggable panel-state persistence contract.
 *
 * Purpose: define {@link PanelStateStore}, the async seam every storage
 * backend implements (localStorage today, a server API later) so layout state
 * survives reloads and follows a user across devices when a server adapter
 * arrives. Payloads are opaque serialized envelopes produced by
 * `encodeLayoutState`; adapters never interpret them.
 * Non-purpose: no cache invalidation, sync/merge strategy, or multi-tab
 * coordination — those are host-app concerns layered above this interface.
 */

/** Thrown for storage-level failures (quota exceeded, unavailable backend). */
export class PanelPersistError extends Error {
  constructor(
    message: string,
    options?: { cause?: unknown },
  ) {
    super(message, options);
    this.name = "PanelPersistError";
  }
}

export interface PanelStateStore {
  /**
   * Load the persisted payload for one user, or `null` when nothing was ever
   * saved. Raw bytes/strings are passed through untouched; validation happens
   * in the engine's decoder.
   */
  load(userId: string): Promise<string | null>;
  /** Persist the payload for one user. Implementations must overwrite atomically. */
  save(userId: string, payload: string): Promise<void>;
  /** Remove persisted state for one user. Idempotent. */
  clear(userId: string): Promise<void>;
}
