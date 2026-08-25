/**
 * @module In-memory {@link PanelStateStore}.
 *
 * Purpose: dependency-free store for unit tests, sandboxes, and SSR
 * prerenders where no durable backend should be touched.
 * Non-purpose: no eviction, size limits, or change events — data lives exactly
 * as long as the store instance does.
 */

import type { PanelStateStore } from "./types";

export class MemoryPanelStateStore implements PanelStateStore {
  private readonly entries = new Map<string, string>();

  async load(userId: string): Promise<string | null> {
    return this.entries.get(this.keyFor(userId)) ?? null;
  }

  async save(userId: string, payload: string): Promise<void> {
    this.entries.set(this.keyFor(userId), payload);
  }

  async clear(userId: string): Promise<void> {
    this.entries.delete(this.keyFor(userId));
  }

  keyFor(userId: string): string {
    return userId;
  }
}
