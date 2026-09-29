/**
 * @module localStorage-backed {@link PanelStateStore} keyed per user id.
 *
 * Purpose: ship the default per-user persistence adapter for browser runs.
 * Keys follow `<namespace>:<userId>` (default namespace `marula.layout`) so
 * multiple apps or environments can share one origin safely. A future server
 * adapter implements the identical interface.
 * Non-purpose: no cross-tab synchronization, encryption, or quota management.
 */

import { PanelPersistError, type PanelStateStore } from "./types";

/** Minimal subset of the DOM `Storage` interface, injectable for tests. */
export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** Thrown when no usable Storage backend exists (SSR, privacy mode). */
export class StorageUnavailableError extends PanelPersistError {
  constructor() {
    super("no localStorage backend is available in this runtime");
    this.name = "StorageUnavailableError";
  }
}

function defaultStorage(): StorageLike {
  const candidate: StorageLike | undefined =
    typeof globalThis.localStorage === "undefined" ? undefined : globalThis.localStorage;
  if (
    candidate === null ||
    candidate === undefined ||
    typeof candidate.getItem !== "function"
  ) {
    throw new StorageUnavailableError();
  }
  return candidate;
}

export interface LocalStorageStoreOptions {
  /** Inject a custom backend (tests, sessionStorage). Defaults to `window.localStorage`. */
  storage?: StorageLike;
  /** Key namespace. Default `"marula.layout"`. */
  namespace?: string;
}

export class LocalStoragePanelStateStore implements PanelStateStore {
  private readonly storageOption: StorageLike | undefined;
  private readonly namespace: string;

  constructor(options: LocalStorageStoreOptions = {}) {
    this.storageOption = options.storage;
    this.namespace = options.namespace ?? "marula.layout";
  }

  async load(userId: string): Promise<string | null> {
    return this.resolveStorage().getItem(this.keyFor(userId));
  }

  async save(userId: string, payload: string): Promise<void> {
    try {
      this.resolveStorage().setItem(this.keyFor(userId), payload);
    } catch (error) {
      throw new PanelPersistError(`failed to persist layout state for user "${userId}"`, {
        cause: error,
      });
    }
  }

  async clear(userId: string): Promise<void> {
    this.resolveStorage().removeItem(this.keyFor(userId));
  }

  /** Deterministic key format: `<namespace>:<userId>`. */
  keyFor(userId: string): string {
    return `${this.namespace}:${userId}`;
  }

  private resolveStorage(): StorageLike {
    return this.storageOption ?? defaultStorage();
  }
}
