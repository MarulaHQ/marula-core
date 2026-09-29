// Sync engine — the one place cache, offline queue, and live sync meet.
//
// Purpose: hydrate the offline cache and durable queue, keep one realtime
// socket subscribed (user channel by the hub, org + board channels
// explicitly), map every inbound contract event onto cache invalidation,
// refresh whatever is stale when the link is healthy, and run mutations
// through the replay-safe queue (optimistic render → durable enqueue →
// typed outcome). Connectivity transitions flush the queue and reconcile.
//
// Non-purpose: no React. The data layer binds this with hooks; screens
// never touch sockets, stores, or the queue directly.

import type { MeridianClient } from "@marula-core/sdk";

import { ReadCache } from "./cache";
import {
  classifyFailure,
  friendlyMutationError,
  OFFLINE_RETRY_COPY,
  type FriendlyError,
} from "./errors";
import { HttpFailure, unwrap } from "./http";
import { makeIdempotencyKey, type UuidFn } from "./idempotency";
import { MODELS } from "./models";
import {
  WriteQueue,
  type ConflictResolution,
  type QueuedOp,
  type QueueOpInput,
  type TransportFailure,
  type TransportOutcome,
} from "./queue";
import {
  RealtimeClient,
  type EventEnvelope,
  type RealtimeStatus,
  type SocketFactory,
} from "./realtime";
import type { KeyValueStore } from "./storage";
import {
  WRITE_OPERATIONS,
  type MutationBase,
  type WriteContext,
  type WriteOperation,
  type WriteOperationKind,
} from "./writes";

export interface ModelState<T = unknown> {
  status: "idle" | "loading" | "ready" | "error";
  data: T | undefined;
  error?: FriendlyError;
  stale: boolean;
  offline: boolean;
  fetchedAt?: string;
}

export interface MutationResult {
  ok: boolean;
  /** True when the write is durable but has not reached the server yet. */
  queued: boolean;
  failure?: FriendlyError;
  opId: string;
}

export interface SyncEngineDeps {
  client: MeridianClient;
  stores: {
    cache: KeyValueStore;
    queue: KeyValueStore;
    versions: KeyValueStore;
  };
  realtime: {
    url: () => string;
    token: () => string | null;
    socketFactory: SocketFactory;
  };
  isOnline: () => boolean;
  /** Refresh the access token before a wire call (single-flight upstream). */
  ensureToken?: () => Promise<void>;
  uuid: UuidFn;
  now?: () => Date;
}

interface VersionEntry {
  version: number;
  at: string;
}

const VERSION_LIMIT = 500;

export class SyncEngine {
  readonly cache: ReadCache;
  readonly queue: WriteQueue;
  readonly realtime: RealtimeClient;

  private readonly deps: SyncEngineDeps;
  private readonly states = new Map<string, ModelState>();
  private readonly fetchers = new Map<string, (client: MeridianClient) => Promise<unknown>>();
  private readonly inflight = new Map<string, Promise<void>>();
  private readonly listeners = new Set<() => void>();
  private readonly versions = new Map<string, VersionEntry>();
  private readonly opActions = new Map<
    string,
    {
      operation: WriteOperation<unknown, Record<string, unknown>>;
      args: unknown;
      base: MutationBase;
    }
  >();
  private readonly writeContext: WriteContext;
  private revision = 0;
  private started = false;
  private orgChannel: string | null = null;

  constructor(deps: SyncEngineDeps) {
    this.deps = deps;
    this.cache = new ReadCache(deps.stores.cache, deps.now !== undefined ? { now: deps.now } : {});
    // Optimistic renders (and offline queues) patch the cache directly, so
    // the live view state has to follow the cache or a queued note stays
    // invisible until the next fetch. Mirror patched keys into mounted
    // models, keeping their offline/stale story intact.
    this.cache.subscribe((key) => {
      const cached = this.cache.get(key);
      const state = this.states.get(key);
      if (cached === undefined || state === undefined) return;
      this.setState(key, {
        ...state,
        status: "ready",
        data: cached.data,
        stale: cached.stale,
        ...(state.offline === true ? { offline: true } : {}),
      });
    });
    this.writeContext = {
      cache: this.cache,
      uuid: deps.uuid,
      now: deps.now ?? (() => new Date()),
    };
    this.queue = new WriteQueue({
      store: deps.stores.queue,
      send: (op) => this.sendOp(op),
      resolveConflict: (op, failure) => this.resolveConflict(op, failure),
      onSettled: (op, outcome) => this.reconcile(op, outcome),
      uuid: deps.uuid,
      ...(deps.now !== undefined ? { now: deps.now } : {}),
    });
    this.realtime = new RealtimeClient({
      url: deps.realtime.url,
      token: deps.realtime.token,
      socketFactory: deps.realtime.socketFactory,
    });
    // Queue changes are part of the engine's revision: the offline banner,
    // the More summary, and the inspector all read queue snapshots through
    // this subscription, so a state with no cache activity (offline writes
    // piling up) still re-renders.
    this.queue.subscribe(() => {
      this.revision += 1;
      for (const listener of this.listeners) listener();
    });
  }

  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    await this.cache.hydrate();
    await this.queue.hydrate();
    await this.loadVersions();
    this.realtime.onEvent((envelope) => this.handleEvent(envelope));
    this.realtime.onHello((_userId, orgId) => {
      this.orgChannel = `org:${orgId}`;
      this.realtime.subscribe(this.orgChannel);
    });
    this.realtime.onStatus((status) => this.handleRealtimeStatus(status));
    void this.realtime.connect();
  }

  stop(): void {
    this.realtime.close();
    this.started = false;
  }

  /** Session end (sign-out / org switch): drop cached reads, queued writes,
   * and version cursors. Queued work never crosses sessions. */
  async reset(): Promise<void> {
    await this.cache.clear();
    await this.queue.clear();
    this.versions.clear();
    this.states.clear();
    await this.deps.stores.versions.set(VERSIONS_KEY, "{}");
    this.revision += 1;
    for (const listener of this.listeners) listener();
  }

  // ---- read models -------------------------------------------------------

  getModelState<T>(key: string): ModelState<T> {
    const cached = this.cache.get<T>(key);
    const state = this.states.get(key) as ModelState<T> | undefined;
    if (state !== undefined) return state;
    if (cached !== undefined) {
      return {
        status: "ready",
        data: cached.data,
        stale: cached.stale,
        offline: false,
        fetchedAt: cached.fetchedAt,
      };
    }
    return { status: "idle", data: undefined, stale: false, offline: false };
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

  /** Bind a model key to its fetcher while a screen needs it. Events still
   * mark the entry stale after detach; the next mount refreshes. */
  registerModel<T>(key: string, fetcher: (client: MeridianClient) => Promise<T>): () => void {
    this.fetchers.set(key, fetcher as (client: MeridianClient) => Promise<unknown>);
    return () => {
      this.fetchers.delete(key);
    };
  }

  async refresh<T>(
    key: string,
    fetcher?: (client: MeridianClient) => Promise<T>,
    options?: { force?: boolean; maxAgeMs?: number },
  ): Promise<void> {
    const resolved = (fetcher as ((client: MeridianClient) => Promise<unknown>) | undefined) ??
      this.fetchers.get(key);
    if (resolved === undefined) return;
    if (options?.force !== true && options?.maxAgeMs !== undefined) {
      const state = this.states.get(key);
      const fetchedAt = state?.fetchedAt ?? this.cache.get(key)?.fetchedAt;
      if (fetchedAt !== undefined && Date.now() - Date.parse(fetchedAt) < options.maxAgeMs) {
        return;
      }
    }
    const existing = this.inflight.get(key);
    if (existing !== undefined) return existing;
    const load = this.doRefresh(key, resolved).finally(() => {
      this.inflight.delete(key);
    });
    this.inflight.set(key, load);
    return load;
  }

  /** Event-driven invalidation: mark stale, then reconcile when healthy. */
  async invalidate(keys: string[], options?: { refresh?: boolean }): Promise<void> {
    await this.cache.markStaleMatching((key) => keys.includes(key));
    for (const key of keys) {
      const state = this.states.get(key);
      if (state !== undefined) this.setState(key, { ...state, stale: true });
    }
    if (options?.refresh !== false && this.deps.isOnline()) {
      for (const key of keys) {
        if (this.fetchers.has(key)) void this.refresh(key, undefined, { force: true });
      }
    }
  }

  async invalidatePrefixes(
    prefixes: string[],
    options?: { refresh?: boolean },
  ): Promise<void> {
    const matched = await this.cache.markStaleMatching((key) =>
      prefixes.some((prefix) => key.startsWith(prefix)),
    );
    for (const key of matched) {
      const state = this.states.get(key);
      if (state !== undefined) this.setState(key, { ...state, stale: true });
    }
    if (options?.refresh !== false && this.deps.isOnline()) {
      for (const key of matched) {
        if (this.fetchers.has(key)) void this.refresh(key, undefined, { force: true });
      }
    }
  }

  private async doRefresh(
    key: string,
    fetcher: (client: MeridianClient) => Promise<unknown>,
  ): Promise<void> {
    if (!this.deps.isOnline()) {
      const cached = this.cache.get(key);
      this.setState(key, {
        status: cached !== undefined ? "ready" : "error",
        data: cached?.data,
        stale: cached !== undefined,
        offline: true,
        ...(cached !== undefined ? { fetchedAt: cached.fetchedAt } : {}),
        error: { text: OFFLINE_RETRY_COPY, transient: true },
      });
      return;
    }
    const current = this.states.get(key);
    if (current === undefined || current.data === undefined) {
      this.setState(key, {
        status: "loading",
        data: current?.data,
        stale: false,
        offline: false,
      });
    }
    try {
      await this.deps.ensureToken?.();
      const data = await fetcher(this.deps.client);
      await this.cache.set(key, data, "server");
      this.setState(key, {
        status: "ready",
        data,
        stale: false,
        offline: false,
        fetchedAt: new Date().toISOString(),
      });
      this.afterModelRefresh(key, data);
    } catch (error) {
      const failure = classifyFailure(error, { offline: !this.deps.isOnline() });
      const cached = this.cache.get(key);
      const friendly = friendlyMutationError(error, { fallback: failure.message });
      this.setState(key, {
        status: cached !== undefined ? "ready" : "error",
        data: cached?.data,
        stale: cached !== undefined,
        offline: failure.kind === "offline",
        ...(cached !== undefined ? { fetchedAt: cached.fetchedAt } : {}),
        error: friendly,
      });
    }
  }

  private afterModelRefresh(key: string, data: unknown): void {
    if (key !== MODELS.myWork) return;
    if (!Array.isArray(data)) return;
    for (const row of data) {
      const boardId = (row as { board_id?: unknown }).board_id;
      if (typeof boardId === "string") this.realtime.subscribe(`board:${boardId}`);
    }
  }

  // ---- mutations ---------------------------------------------------------

  async mutate<A, B extends Record<string, unknown>>(
    operation: WriteOperation<A, B>,
    args: A,
  ): Promise<MutationResult> {
    const idempotencyKey = makeIdempotencyKey(this.deps.uuid);
    const versionKey = operation.entityId?.(args) ?? operation.entityKey(args);
    const version = this.versions.get(versionKey)?.version;
    const base: MutationBase = {
      idempotencyKey,
      ...(version !== undefined ? { version } : {}),
    };
    await operation.optimistic?.(args, base, this.writeContext);
    const body = operation.build(args, base);
    const input: QueueOpInput = {
      kind: operation.kind,
      entityKey: operation.entityKey(args),
      versionKey,
      label: operation.label(args),
      method: operation.method,
      path: operation.path(args),
      body,
      idempotencyKey,
      ...(version !== undefined ? { version } : {}),
      invalidate: operation.invalidate(args),
    };
    const op = await this.queue.enqueue(input);
    this.opActions.set(op.id, {
      operation: operation as unknown as WriteOperation<unknown, Record<string, unknown>>,
      args,
      base,
    });
    if (this.deps.isOnline()) {
      await this.queue.flush();
    }
    const settled = this.queue.snapshot().find((candidate) => candidate.id === op.id);
    if (settled === undefined || settled.state === "applied") {
      // Applied — the authoritative reconcile ran (or runs) on settle.
      this.opActions.delete(op.id);
      return { ok: true, queued: false, opId: op.id };
    }
    if (settled.state === "rejected") {
      await operation.revert?.(args, base, this.writeContext);
      this.opActions.delete(op.id);
      const failure = settled.failure;
      return {
        ok: false,
        queued: false,
        opId: op.id,
        failure: {
          text: failure?.message ?? "That change couldn't be saved.",
          ...(failure?.reason !== undefined ? { reason: failure.reason } : {}),
        },
      };
    }
    return {
      ok: true,
      queued: true,
      opId: op.id,
      failure: {
        text: this.deps.isOnline()
          ? "Queued — it will sync when the server is reachable again."
          : "You're offline — your change is saved on this device and will sync automatically when the connection returns.",
        transient: true,
      },
    };
  }

  private async sendOp(op: QueuedOp): Promise<TransportOutcome> {
    const operation = WRITE_OPERATIONS[op.kind as WriteOperationKind] as
      | WriteOperation<unknown, Record<string, unknown>>
      | undefined;
    if (operation === undefined) {
      return {
        ok: false,
        kind: "http",
        reason: "unsupported_operation",
        message: "This queued change was made by a different app version and can't be replayed.",
        transient: false,
        conflict: false,
      };
    }
    try {
      await this.deps.ensureToken?.();
      const data = unwrap(await operation.send(this.deps.client, op.body));
      const version = operation.versionOf?.(data);
      if (version !== undefined) await this.rememberVersion(op.versionKey, version);
      return { ok: true, data };
    } catch (error) {
      const failure = classifyFailure(error, {
        status: error instanceof HttpFailure ? error.status : undefined,
        offline: !this.deps.isOnline(),
      });
      return {
        ok: false,
        kind: failure.kind,
        ...(failure.reason !== undefined ? { reason: failure.reason } : {}),
        message: failure.message,
        ...(failure.status !== undefined ? { status: failure.status } : {}),
        transient: failure.transient,
        conflict: failure.conflict,
        ...(failure.details !== undefined ? { details: failure.details } : {}),
      };
    }
  }

  /** Last-write resolution: the 409 envelope carries the server's current
   * version (`expected`); adopt it and re-apply the user's write once. The
   * queue audits the adoption; a second conflict is a typed rejection. */
  private async resolveConflict(
    _op: QueuedOp,
    failure: TransportFailure,
  ): Promise<ConflictResolution> {
    if (!this.deps.isOnline()) {
      return { retry: false, detail: "offline — conflict resolution deferred" };
    }
    const expected = failure.details?.["expected"];
    if (failure.reason === "version_conflict" && typeof expected === "number") {
      return {
        retry: true,
        version: expected,
        detail: "adopted the server's current version (last-write)",
      };
    }
    return { retry: false, detail: failure.message };
  }

  /** After an op lands: reconcile the optimistic render from the server
   * snapshot, then invalidate the caches it touched. */
  private async reconcile(op: QueuedOp, outcome: TransportOutcome): Promise<void> {
    const action = this.opActions.get(op.id);
    if (action !== undefined) {
      this.opActions.delete(op.id);
      if (outcome.ok) {
        await action.operation.apply?.(
          outcome.data,
          action.args,
          action.base,
          this.writeContext,
        );
      }
    }
    await this.invalidate(op.invalidate, { refresh: true });
  }

  // ---- connectivity / link lifecycle ------------------------------------

  /** Call on every connectivity transition. */
  async handleConnectivityChange(online: boolean): Promise<void> {
    if (online) {
      await this.queue.flush();
      await this.refreshStale();
      return;
    }
    for (const [key, state] of this.states) {
      this.setState(key, { ...state, offline: true, stale: state.data !== undefined });
    }
  }

  /** Flush pending writes, then reconcile everything the UI already knows. */
  async catchUp(): Promise<void> {
    if (!this.deps.isOnline()) return;
    await this.queue.flush();
    await this.refreshStale();
  }

  private async refreshStale(): Promise<void> {
    for (const key of this.cache.keys()) {
      const entry = this.cache.get(key);
      if (entry === undefined || !entry.stale) continue;
      if (!this.fetchers.has(key)) continue;
      void this.refresh(key, undefined, { force: true });
    }
  }

  private handleRealtimeStatus(status: RealtimeStatus): void {
    if (status === "open") {
      if (this.orgChannel !== null) this.realtime.subscribe(this.orgChannel);
      void this.catchUp();
    }
  }

  private handleEvent(envelope: EventEnvelope): void {
    const { keys, prefixes } = invalidationFor(envelope);
    if (keys.length > 0) void this.invalidate(keys, { refresh: true });
    if (prefixes.length > 0) void this.invalidatePrefixes(prefixes, { refresh: true });
  }

  // ---- version registry --------------------------------------------------

  private async loadVersions(): Promise<void> {
    const raw = await this.deps.stores.versions.get(VERSIONS_KEY);
    if (raw === null) return;
    try {
      const parsed = JSON.parse(raw) as Record<string, VersionEntry>;
      for (const [key, entry] of Object.entries(parsed)) {
        if (typeof entry?.version === "number") this.versions.set(key, entry);
      }
    } catch {
      this.versions.clear();
    }
  }

  private async rememberVersion(key: string, version: number): Promise<void> {
    this.versions.set(key, { version, at: new Date().toISOString() });
    if (this.versions.size > VERSION_LIMIT) {
      const oldest = [...this.versions.entries()].sort((a, b) => a[1].at.localeCompare(b[1].at))[0];
      if (oldest !== undefined) this.versions.delete(oldest[0]);
    }
    const payload: Record<string, VersionEntry> = {};
    for (const [entryKey, entry] of this.versions) payload[entryKey] = entry;
    await this.deps.stores.versions.set(VERSIONS_KEY, JSON.stringify(payload));
  }

  knownVersion(key: string): number | undefined {
    return this.versions.get(key)?.version;
  }

  // ---- internals ---------------------------------------------------------

  private setState(key: string, next: ModelState): void {
    this.states.set(key, next);
    this.revision += 1;
    for (const listener of this.listeners) listener();
  }
}

const VERSIONS_KEY = "meridian.rn.versions";

/** Entity → cache-key map for the event trail the hub delivers. Mirrors
 * crates/realtime/src/channel.rs routing plus the envelope entities each
 * core-api route emits; unknown entities simply mark nothing. */
export function invalidationFor(envelope: EventEnvelope): {
  keys: string[];
  prefixes: string[];
} {
  const payload = envelope.payload ?? {};
  const keys = new Set<string>();
  const prefixes = new Set<string>();
  const ticketId = str(payload["ticket_id"]) ?? envelope.entity_id ?? undefined;
  switch (envelope.entity) {
    case "ticket": {
      keys.add(MODELS.myWork);
      if (ticketId !== undefined) keys.add(MODELS.ticket(ticketId));
      break;
    }
    case "ticket_comment": {
      keys.add(MODELS.myWork);
      if (ticketId !== undefined) {
        keys.add(MODELS.ticket(ticketId));
        keys.add(MODELS.ticketComments(ticketId));
      }
      break;
    }
    case "timer": {
      keys.add(MODELS.activeTimer);
      keys.add(MODELS.myWork);
      if (ticketId !== undefined) keys.add(MODELS.ticket(ticketId));
      break;
    }
    case "time_entry": {
      keys.add(MODELS.activeTimer);
      keys.add(MODELS.myWork);
      if (ticketId !== undefined) keys.add(MODELS.ticket(ticketId));
      break;
    }
    case "notification": {
      keys.add(MODELS.notifications);
      keys.add(MODELS.unreadCount);
      break;
    }
    case "appointment": {
      keys.add(MODELS.myWork);
      prefixes.add("dispatch.day:");
      break;
    }
    case "stock_movement":
    case "stock_item": {
      keys.add(MODELS.onHand);
      keys.add(MODELS.stockItems);
      const itemId = str(payload["stock_item_id"]) ?? str(payload["item_id"]);
      if (itemId !== undefined) {
        keys.add(MODELS.stockItem(itemId));
        keys.add(MODELS.stockMovements(itemId));
      }
      break;
    }
    case "tool_checkout":
    case "tool_kit": {
      keys.add(MODELS.toolCheckouts);
      keys.add(MODELS.onHand);
      keys.add(MODELS.stockSerials);
      break;
    }
    case "asset": {
      const assetId = str(payload["asset_id"]) ?? envelope.entity_id ?? undefined;
      if (assetId !== undefined) keys.add(MODELS.asset(assetId));
      break;
    }
    default:
      break;
  }
  return { keys: [...keys], prefixes: [...prefixes] };
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}
