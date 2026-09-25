// Storage seam for the offline cache and write queue.
//
// Purpose: a tiny async key/value contract the core depends on, so every
// core module is unit-testable without React Native. The app binds
// AsyncStorage in src/data/storage.ts; tests bind MemoryStore.
//
// Non-purpose: no namespaces, TTLs, or JSON handling — callers own that.

export interface KeyValueStore {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  remove(key: string): Promise<void>;
  /** Every key this store currently holds (used to hydrate caches). */
  keys(): Promise<string[]>;
}

export class MemoryStore implements KeyValueStore {
  private readonly map = new Map<string, string>();

  async get(key: string): Promise<string | null> {
    return this.map.get(key) ?? null;
  }

  async set(key: string, value: string): Promise<void> {
    this.map.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.map.delete(key);
  }

  async keys(): Promise<string[]> {
    return [...this.map.keys()];
  }
}
