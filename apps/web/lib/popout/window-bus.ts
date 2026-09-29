// Window bus — the cross-window coordination channel (BroadcastChannel
// "meridian-windows"), one instance per browser context, shared by every
// Meridian window (main + popouts).
//
// Purpose (Wave 38 lane 1):
// - window registry: which route keys / entities are open where, so the
//   notification dock routes FOCUS to an existing window instead of
//   duplicating navigation, and phantom presence can be cleaned up.
// - entity echo: the local leg of cross-window consistency — when one
//   window's optimistic write lands, siblings hear about it immediately
//   (the realtime SSE/WS hub remains the authoritative second leg).
// - session events: one logout / re-login is broadcast so EVERY window
//   reflects the session honestly — expired session signs out all, one
//   re-login restores all.
// - single-flight: two windows never double-apply one shared mutation —
//   a claim/ack handshake keyed by idempotency key.
//
// Non-purpose: no payload replication of read models — windows re-sync
// through the instant store; the bus carries keys and events only.

export type BusMessage =
  | { kind: "hello"; windowId: string; routeKey: string | null; entities: string[] }
  | { kind: "shown"; windowId: string; entities: string[] }
  | { kind: "bye"; windowId: string; windowKey: string }
  | { kind: "focus"; windowId: string; path: string }
  | { kind: "entity-changed"; entityIds: string[]; meta: unknown; from: string }
  | { kind: "session"; state: "expired" | "restored" }
  | { kind: "claim"; flightKey: string; from: string }
  | { kind: "held"; flightKey: string; to: string }
  // Desktop fleet (Wave 38 lane 2): an update restart asks EVERY window;
  // each answers dirty (unsaved edits exist — save-bar semantics) or clean.
  | { kind: "restart-check"; requestId: string; from: string }
  | { kind: "restart-answer"; requestId: string; dirty: boolean; from: string };

/** A per-JS-context id. NOT sessionStorage-backed: browsers COPY sessionStorage
 * into popups opened via window.open, which would make main + popout share
 * one id — and the self-echo guard would eat every cross-window echo. The
 * registry doesn't need id persistence: announceWindow rebuilds it on load. */
function windowId(): string {
  return `w-${Math.random().toString(36).slice(2, 10)}`;
}

type Handler = (message: BusMessage) => void;

interface BusState {
  channel: BroadcastChannel | null;
  id: string;
  handlers: Set<Handler>;
  routeKey: string | null;
  entities: string[];
  /** Other live windows: id → route key + currently shown entities. */
  peers: Map<string, { routeKey: string | null; entities: string[] }>;
}

let state: BusState | null = null;

function ensure(): BusState | null {
  if (typeof window === "undefined" || typeof BroadcastChannel === "undefined") return null;
  if (state !== null) return state;
  const channel = new BroadcastChannel("meridian-windows");
  state = {
    channel, id: windowId(), handlers: new Set(), routeKey: null, entities: [],
    peers: new Map(),
  };
  channel.onmessage = (event: MessageEvent<BusMessage>) => {
    const msg = event.data;
    if (msg === null || msg === undefined || typeof msg !== "object") return;
    if (state === null) return;
    // Registry maintenance runs before the self-echo guard: hello/shown/bye
    // from ourselves are ignored for delivery, but peers always track.
    if (msg.kind === "hello" || msg.kind === "shown") {
      if (msg.windowId !== state.id) {
        state.peers.set(msg.windowId, {
          routeKey: msg.kind === "hello" ? msg.routeKey : state.peers.get(msg.windowId)?.routeKey ?? null,
          entities: msg.entities,
        });
      }
    }
    if (msg.kind === "bye" && msg.windowId !== state.id) {
      state.peers.delete(msg.windowId);
    }
    // Never echo our own messages back to ourselves.
    if ("from" in msg && msg.from === state.id) return;
    if (msg.kind === "hello" && msg.windowId === state.id) return;
    for (const handler of state.handlers) handler(msg);
  };
  return state;
}

/** Subscribe to cross-window messages. Returns the unsubscribe function. */
export function onBusMessage(handler: Handler): () => void {
  const bus = ensure();
  if (bus === null) return () => undefined;
  bus.handlers.add(handler);
  return () => bus.handlers.delete(handler);
}

/** Announce this window to the others (route key + entities on screen). */
export function announceWindow(routeKey: string | null, entities: string[]): void {
  const bus = ensure();
  if (bus === null) return;
  bus.routeKey = routeKey;
  bus.entities = entities;
  bus.channel?.postMessage({
    kind: "hello", windowId: bus.id, routeKey, entities,
  } satisfies BusMessage);
}

/** Update the entity set this window currently shows (route changes). */
export function updateShownEntities(entities: string[]): void {
  const bus = ensure();
  if (bus === null) return;
  bus.entities = entities;
  bus.channel?.postMessage({ kind: "shown", windowId: bus.id, entities } satisfies BusMessage);
}

/** Fired on popout unload so the registry stays honest (no phantom windows). */
export function leaveWindow(windowKey: string): void {
  const bus = ensure();
  if (bus === null) return;
  bus.channel?.postMessage({ kind: "bye", windowId: bus.id, windowKey } satisfies BusMessage);
}

/** Ask the window already showing this path's entity to focus + navigate
 * to it. True when a peer claimed the route (caller skips local nav). */
export function requestFocus(path: string): boolean {
  const bus = ensure();
  if (bus === null) return false;
  const seg = path.split("/").filter(Boolean);
  if (seg.length < 2) return false;
  const entity = `${seg[0]}/${seg[1]}`;
  for (const [peerId, peer] of bus.peers) {
    if (peer.entities.includes(entity)) {
      bus.channel?.postMessage({ kind: "focus", windowId: peerId, path } satisfies BusMessage);
      return true;
    }
  }
  return false;
}

/** The receiving side of requestFocus: subscribe for focus requests aimed
 * at THIS window and navigate. */
export function onFocusRequest(navigate: (path: string) => void): () => void {
  return onBusMessage((msg) => {
    if (msg.kind === "focus" && msg.windowId === ensure()?.id) navigate(msg.path);
  });
}

/**
 * Broadcast that entities changed so sibling windows re-sync instantly.
 * `meta` is the IF2 mutation envelope (MutationMetaLike shape) when the
 * change came from an accepted write — siblings apply it through the same
 * optimistic path (ledger dedupes; their SSE stream reconciles after).
 */
export function broadcastEntityChange(entityIds: string[], meta: unknown = null): void {
  const bus = ensure();
  if (bus === null || entityIds.length === 0) return;
  bus.channel?.postMessage({ kind: "entity-changed", entityIds, meta, from: bus.id } satisfies BusMessage);
}

/** Session honesty: broadcast expiry / restore to every window. */
export function broadcastSession(state: "expired" | "restored"): void {
  const bus = ensure();
  if (bus === null) return;
  bus.channel?.postMessage({ kind: "session", state } satisfies BusMessage);
}

/**
 * Single-flight guard for a shared mutation keyed `flightKey` (the write's
 * idempotency key): broadcast a claim; if another window reports it is
 * already applying that exact write, resolve `false` and the caller skips
 * instead of double-applying. Returns `true` when this window owns the
 * flight. The ack handshake is one microtask — honest for co-located
 * windows on the same origin, and the server-side idempotency key remains
 * the hard guarantee underneath.
 */
export async function claimFlight(flightKey: string): Promise<boolean> {
  const bus = ensure();
  if (bus === null || flightKey === "") return true;
  let held = false;
  const off = onBusMessage((msg) => {
    if (msg.kind === "held" && msg.flightKey === flightKey && msg.to === bus.id) held = true;
  });
  bus.channel?.postMessage({ kind: "claim", flightKey, from: bus.id } satisfies BusMessage);
  // One macrotask lets sibling windows answer on the same channel.
  await new Promise((resolve) => setTimeout(resolve, 0));
  off();
  return !held;
}

/** Answer sibling claims when THIS window is already flying the same key. */
export function answerClaims(isFlying: (flightKey: string) => boolean): () => void {
  return onBusMessage((msg) => {
    if (msg.kind !== "claim") return;
    if (!isFlying(msg.flightKey)) return;
    const bus = ensure();
    bus?.channel?.postMessage({ kind: "held", flightKey: msg.flightKey, to: msg.from } satisfies BusMessage);
  });
}

/**
 * Desktop fleet restart handshake (Wave 38 lane 2): before the updater
 * restarts the app, the initiating window asks every sibling "anything
 * unsaved?". Each window that registers an isDirty resolver answers;
 * `true` means some window still has unsaved edits.
 */
export async function anyWindowDirty(
  requestId: string = `restart-${Date.now()}`,
): Promise<{ dirty: boolean; peers: number }> {
  const bus = ensure();
  if (bus === null) return { dirty: false, peers: 0 };
  let dirty = false;
  const seen = new Set<string>();
  const off = onBusMessage((msg) => {
    if (msg.kind !== "restart-answer" || msg.requestId !== requestId) return;
    seen.add(msg.from);
    if (msg.dirty) dirty = true;
  });
  bus.channel?.postMessage({ kind: "restart-check", requestId, from: bus.id } satisfies BusMessage);
  // Fleet survey: a real round-trip window for co-located windows.
  await new Promise((resolve) => setTimeout(resolve, 150));
  off();
  return { dirty, peers: seen.size };
}

/** Register THIS window's dirtiness for restart surveys (the same state
 * the close guard uses — one truth for "unsaved edits exist here"). */
export function answerRestartChecks(isDirty: () => boolean): () => void {
  return onBusMessage((msg) => {
    if (msg.kind !== "restart-check" || msg.from === ensure()?.id) return;
    const bus = ensure();
    bus?.channel?.postMessage({
      kind: "restart-answer",
      requestId: msg.requestId,
      dirty: isDirty(),
      from: bus.id,
    } satisfies BusMessage);
  });
}

/** This window's bus id (identity for logs and tests). */
export function busId(): string {
  return ensure()?.id ?? "w-anon";
}

/** Test seam. */
export function __resetWindowBusForTests(): void {
  state?.channel?.close();
  state = null;
}
