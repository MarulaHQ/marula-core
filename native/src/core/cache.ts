// Offline-first read cache over the public read models.
//
// Purpose: hydrate once from a snapshot read model, keep entries in memory
// and in the durable store, and let WebSocket events mark them stale. A
// screen renders the cached entity immediately, shows an offline state when
// the server cannot be reached, and reconciles when a refresh lands — no
// loading flash on data the client already knows (docs/instant-first.md).
//
// Non-purpose: no fetching (the sync engine owns transport) and no React
// (a subscription feeds useSyncExternalStore in src/data).

import type { KeyValueStore } from "./storage";

export interface CacheEntry<T = unknown> {
  key: string;
  data: T;
  fetchedAt: string;
  /** An event (or a failed refresh) says the server may be ahead. */
  stale: boolean;
  source: "server" | "cache" | "optimistic";
}

export interface CacheOptions {
  prefix?: string;
  now?: () => Date;
}

const DEFAULT_PREFIX = "marula.rn.cache.";

interface PersistedEntry {
  data: unknown;
  fetchedAt: string;
  stale: boolean;
  source: CacheEntry["source"];
}

export class ReadCache {
  private readonly entries = new Map<string, CacheEntry>();
  private readonly listeners = new Set<(key: string) => void>();
  private readonly store: KeyValueStore;
  private readonly prefix: string;
  private readonly now: () => Date;
  private revision = 0;

  constructor(store: KeyValueStore, options: CacheOptions = {}) {
    this.store = store;
    this.prefix = options.prefix ?? DEFAULT_PREFIX;
    this.now = options.now ?? (() => new Date());
  }

  /** Restores every persisted entry. Call once at app start. */
  async hydrate(): Promise<void> {
    const keys = await this.store.keys();
    for (const storageKey of keys) {
      if (!storageKey.startsWith(this.prefix)) continue;
      const raw = await this.store.get(storageKey);
      if (raw === null) continue;
      try {
        const parsed = JSON.parse(raw) as PersistedEntry;
        this.entries.set(storageKey.slice(this.prefix.length), {
          key: storageKey.slice(this.prefix.length),
          data: parsed.data,
          fetchedAt: parsed.fetchedAt,
          stale: parsed.stale,
          source: parsed.source,
        });
      } catch {
        await this.store.remove(storageKey);
      }
    }
    this.emitAll();
  }

  get<T>(key: string): CacheEntry<T> | undefined {
    return this.entries.get(key) as CacheEntry<T> | undefined;
  }

  /** Snapshot revision for useSyncExternalStore. */
  getRevision(): number {
    return this.revision;
  }

  keys(): string[] {
    return [...this.entries.keys()];
  }

  async set<T>(key: string, data: T, source: CacheEntry["source"] = "server"): Promise<void> {
    this.entries.set(key, {
      key,
      data,
      fetchedAt: this.now().toISOString(),
      stale: false,
      source,
    });
    await this.persist(key);
    this.emit(key);
  }

  /** Optimistic local patch — the entry stays marked optimistic until the
   * authoritative snapshot reconciles it. */
  async patch<T>(
    key: string,
    patcher: (current: T | undefined) => T,
    source: CacheEntry["source"] = "optimistic",
  ): Promise<void> {
    const current = this.entries.get(key);
    const next = patcher(current?.data as T | undefined);
    this.entries.set(key, {
      key,
      data: next,
      fetchedAt: current?.fetchedAt ?? this.now().toISOString(),
      stale: current?.stale ?? source !== "server",
      source,
    });
    await this.persist(key);
    this.emit(key);
  }

  /** Event-driven invalidation: keep showing the data, flag it may be old. */
  async markStale(key: string): Promise<void> {
    const entry = this.entries.get(key);
    if (entry === undefined) return;
    if (entry.stale) return;
    this.entries.set(key, { ...entry, stale: true });
    await this.persist(key);
    this.emit(key);
  }

  async markStaleMatching(predicate: (key: string) => boolean): Promise<string[]> {
    const matched: string[] = [];
    for (const key of this.entries.keys()) {
      if (predicate(key)) matched.push(key);
    }
    for (const key of matched) await this.markStale(key);
    return matched;
  }

  async remove(key: string): Promise<void> {
    if (!this.entries.delete(key)) return;
    await this.store.remove(this.prefix + key);
    this.emit(key);
  }

  /** Logout / org switch: drop everything, durable included. */
  async clear(): Promise<void> {
    const keys = [...this.entries.keys()];
    this.entries.clear();
    for (const key of keys) await this.store.remove(this.prefix + key);
    this.emitAll();
  }

  subscribe(listener: (key: string) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private async persist(key: string): Promise<void> {
    const entry = this.entries.get(key);
    if (entry === undefined) return;
    const payload: PersistedEntry = {
      data: entry.data,
      fetchedAt: entry.fetchedAt,
      stale: entry.stale,
      source: entry.source,
    };
    await this.store.set(this.prefix + key, JSON.stringify(payload));
  }

  private emit(key: string): void {
    this.revision += 1;
    for (const listener of this.listeners) listener(key);
  }

  private emitAll(): void {
    this.revision += 1;
    for (const listener of this.listeners) listener("*");
  }
}
