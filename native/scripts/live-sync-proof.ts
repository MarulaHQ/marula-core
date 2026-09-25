/**
 * Live-stack proof for the mobile foundation's sync legs.
 *
 * Runs the REAL app core (the same SyncEngine / WriteQueue / RealtimeClient
 * the app ships) against the e2e stack (scripts/e2e-stack.sh up):
 *
 *   [1] offline queue replay — a ticket note enqueued while offline replays
 *       on reconnect and lands exactly ONCE (idempotency key).
 *   [2] live sync over the realtime hub —
 *       (a) a mentioned teammate's socket receives `ticket_comment` when the
 *           note is written from another client (the phone -> their pane leg),
 *       (b) a second authed socket subscribed to the ticket's board channel
 *           receives `ticket.updated` for a ticket mutation.
 *   [3] ordered timer replay — start/stop through the queue materialize a
 *       time entry; an unknown session id is a typed rejection, never a
 *       silent apply.
 *   [4] consumption replay — an offline stock consume replays to exactly one
 *       ledger movement.
 *   [5] tool checkout replay (when the seed has a serialized unit).
 *
 * Usage:
 *   pnpm tsx native/scripts/live-sync-proof.ts
 * Env: E2E_API_URL (default http://localhost:28480),
 *      E2E_WS_URL  (default ws://localhost:29481/ws)
 *
 * Non-purpose: no UI driving (Maestro flows own that) and no server edits.
 */

import { randomUUID } from "node:crypto";

import { MeridianClient } from "@meridian/sdk";

import { fetchStockItems, fetchStockSerials } from "../src/core/models";
import { SyncEngine } from "../src/core/sync";
import { MemoryStore } from "../src/core/storage";
import {
  addTicketNote,
  checkoutTools,
  checkinTools,
  consumeStock,
  startTimer,
  stopTimer,
} from "../src/core/writes";

const API_URL = (process.env.E2E_API_URL ?? "http://localhost:28480").replace(/\/$/, "");
const WS_URL = process.env.E2E_WS_URL ?? "ws://localhost:29481/ws";
const PASSWORD = "demo-password-123";

let passed = 0;
const failures: string[] = [];
function check(name: string, condition: boolean, detail = ""): void {
  if (condition) {
    passed += 1;
    console.log(`  ok  ${name}`);
  } else {
    failures.push(name);
    console.error(`FAIL  ${name}${detail !== "" ? ` — ${detail}` : ""}`);
  }
}

async function rest<T>(
  path: string,
  options: { method?: string; body?: unknown; token: string },
): Promise<T> {
  const response = await fetch(`${API_URL}${path}`, {
    method: options.method ?? "GET",
    headers: {
      ...(options.body !== undefined ? { "content-type": "application/json" } : {}),
      authorization: `Bearer ${options.token}`,
    },
    ...(options.body !== undefined ? { body: JSON.stringify(options.body) } : {}),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `${options.method ?? "GET"} ${path} -> ${response.status}: ${text.slice(0, 200)}`,
    );
  }
  return (text === "" ? (null as T) : (JSON.parse(text) as T));
}

interface SocketLog {
  frames: { type: string; channel?: string; event?: { entity?: string; payload?: Record<string, unknown> } }[];
  socket: WebSocket;
}

async function openSocket(token: string, channels: string[] = []): Promise<SocketLog> {
  const socket = new WebSocket(`${WS_URL}?token=${encodeURIComponent(token)}`);
  const frames: SocketLog["frames"] = [];
  socket.addEventListener("message", (message) => {
    const parsed = JSON.parse(String(message.data)) as SocketLog["frames"][number];
    frames.push(parsed);
    if (parsed.type === "hello") {
      for (const channel of channels) {
        socket.send(JSON.stringify({ type: "subscribe", channel }));
      }
    }
  });
  await new Promise<void>((resolve, reject) => {
    const deadline = setTimeout(() => reject(new Error("socket did not open")), 5000);
    socket.addEventListener("open", () => {
      clearTimeout(deadline);
      resolve();
    });
  });
  // Give the hub a beat to deliver hello + process subscriptions.
  await sleep(400);
  return { frames, socket };
}

function makeEngine(client: MeridianClient, state: { online: boolean }): SyncEngine {
  const engine = new SyncEngine({
    client,
    stores: {
      cache: new MemoryStore(),
      queue: new MemoryStore(),
      versions: new MemoryStore(),
    },
    realtime: {
      url: () => WS_URL,
      token: () => client.tokens.accessToken,
      socketFactory: (url) => new WebSocket(url) as never,
    },
    isOnline: () => state.online,
    uuid: () => randomUUID(),
  });
  return engine;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor<T>(
  fn: () => T | undefined | Promise<T | undefined>,
  timeoutMs = 8000,
  intervalMs = 100,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await fn();
    if (value !== undefined) return value;
    if (Date.now() > deadline) throw new Error("timed out waiting for condition");
    await sleep(intervalMs);
  }
}

async function main(): Promise<void> {
  console.log(`[proof] api ${API_URL} · ws ${WS_URL}`);
  const owner = new MeridianClient({ baseUrl: API_URL });
  await owner.login({ email: "owner@demo.local", password: PASSWORD });
  const ownerToken = owner.tokens.accessToken ?? "";
  const tech = new MeridianClient({ baseUrl: API_URL });
  await tech.login({ email: "tech@demo.local", password: PASSWORD });
  const techToken = tech.tokens.accessToken ?? "";

  const work = await rest<{ id: string; number: string; title: string; board_id: string }[]>(
    "/api/v1/tickets/my-work",
    { token: ownerToken },
  );
  const all =
    work.length > 0
      ? work
      : await rest<{ id: string; number: string; title: string; board_id: string }[]>(
          "/api/v1/tickets",
          { token: ownerToken },
        );
  const ticket = all[0];
  if (ticket === undefined) throw new Error("no seeded ticket available");
  console.log(`[proof] using ticket ${ticket.number} (board ${ticket.board_id})`);

  // --- fixtures: deterministic stock + tool set (idempotent) --------------
  const locations = await rest<{ id: string; name: string }[]>("/api/v1/stock-locations", {
    token: ownerToken,
  });
  let bin = locations.find((location) => location.name === "Proof bin");
  if (bin === undefined) {
    bin = await rest<{ id: string; name: string }>("/api/v1/stock-locations", {
      method: "POST",
      token: ownerToken,
      body: { name: "Proof bin", kind: "shelf", code: "PROOF-BIN", idempotency_key: randomUUID() },
    });
  }
  const items = await fetchStockItems(owner);
  let consumable = items.find((item) => item.name === "Proof consumable");
  if (consumable === undefined) {
    consumable = await rest<typeof consumable>("/api/v1/stock-items", {
      method: "POST",
      token: ownerToken,
      body: {
        name: "Proof consumable",
        sku: "PROOF-CONSUMABLE",
        kind: "consumable",
        currency: "USD",
        idempotency_key: randomUUID(),
      },
    });
  }
  let tool = items.find((item) => item.name === "Proof torque wrench");
  if (tool === undefined) {
    tool = await rest<typeof tool>("/api/v1/stock-items", {
      method: "POST",
      token: ownerToken,
      body: {
        name: "Proof torque wrench",
        sku: "PROOF-TOOL",
        kind: "serialized",
        is_tool: true,
        currency: "USD",
        idempotency_key: randomUUID(),
      },
    });
  }
  const onHandBefore = await rest<
    { stock_item_id: string; stock_location_id: string; qty_milli: number }[]
  >("/api/v1/stock/on-hand", { token: ownerToken });
  const consumableStock = onHandBefore.find(
    (row) => row.stock_item_id === consumable?.id && row.stock_location_id === bin?.id,
  );
  if (consumableStock === undefined && consumable !== undefined && bin !== undefined) {
    await rest("/api/v1/stock/receive", {
      method: "POST",
      token: ownerToken,
      body: {
        item_id: consumable.id,
        location_id: bin.id,
        qty_milli: 10_000,
        idempotency_key: randomUUID(),
      },
    });
  }
  const serialsBefore = await fetchStockSerials(owner);
  if (!serialsBefore.some((serial) => serial.serial_number === "PROOF-SN-1") && tool !== undefined && bin !== undefined) {
    await rest("/api/v1/stock/receive", {
      method: "POST",
      token: ownerToken,
      body: {
        item_id: tool.id,
        location_id: bin.id,
        qty_milli: 1000,
        serial_numbers: ["PROOF-SN-1"],
        idempotency_key: randomUUID(),
      },
    });
  }
  console.log(`[proof] fixtures ready (bin ${bin?.id ?? "?"}, consumable ${consumable?.id ?? "?"}, tool ${tool?.id ?? "?"})`);

  const state = { online: true };
  const engine = makeEngine(owner, state);
  await engine.start();

  // --- [1] offline note replay lands exactly once -------------------------
  state.online = false;
  await engine.handleConnectivityChange(false);
  const mark = `proof-${randomUUID().slice(0, 8)}`;
  const queued = await engine.mutate(addTicketNote, {
    ticketId: ticket.id,
    ticketNumber: ticket.number,
    body: `Offline note ${mark}`,
  });
  check("offline mutation reports queued", queued.queued === true && queued.ok === true);
  check("queue holds one pending op", engine.queue.pendingCount() === 1);
  await sleep(250);

  state.online = true;
  await engine.handleConnectivityChange(true);
  await waitFor(() => (engine.queue.pendingCount() === 0 ? true : undefined), 10000);
  check("queue drained after reconnect", engine.queue.pendingCount() === 0);

  const comments = await rest<{ body: string }[]>(`/api/v1/tickets/${ticket.id}/comments`, {
    token: ownerToken,
  });
  const matches = comments.filter((comment) => comment.body.includes(mark));
  check("offline note landed exactly once", matches.length === 1, `found ${matches.length}`);

  // --- [2a] mentioned teammate receives the comment live ------------------
  const techSocket = await openSocket(techToken);
  const mentionMark = `proof-mention-${randomUUID().slice(0, 8)}`;
  const mention = await engine.mutate(addTicketNote, {
    ticketId: ticket.id,
    ticketNumber: ticket.number,
    body: `@tech ${mentionMark}`,
  });
  check("mentioning note applied", mention.ok === true);
  const techComment = await waitFor(
    () =>
      techSocket.frames.find(
        (frame) =>
          frame.type === "event" &&
          frame.event?.entity === "ticket_comment" &&
          JSON.stringify(frame.event.payload ?? {}).includes(mentionMark),
      ),
    8000,
  ).catch(() => undefined);
  check("mentioned teammate's socket received the ticket_comment event", techComment !== undefined);
  const techNotification = await waitFor(
    () =>
      techSocket.frames.find(
        (frame) =>
          frame.type === "event" &&
          frame.event?.entity === "notification" &&
          JSON.stringify(frame.event.payload ?? {}).includes("mention"),
      ),
    8000,
  ).catch(() => undefined);
  check("mentioned teammate's socket received the notification event", techNotification !== undefined);
  techSocket.socket.close();

  // --- [2b] board channel receives ticket mutations ----------------------
  const boardSocket = await openSocket(ownerToken, [`board:${ticket.board_id}`]);
  const patchMark = `proof-patch-${randomUUID().slice(0, 8)}`;
  await rest(`/api/v1/tickets/${ticket.id}`, {
    method: "PATCH",
    token: ownerToken,
    body: { body: `Harness body ${patchMark}`, idempotency_key: randomUUID() },
  });
  const boardTicketEvent = await waitFor(
    () =>
      boardSocket.frames.find(
        (frame) =>
          frame.type === "event" &&
          frame.event?.entity === "ticket" &&
          JSON.stringify(frame.event.payload ?? {}).includes(ticket.id),
      ),
    8000,
  ).catch(() => undefined);
  check("board-subscribed second client received ticket.updated", boardTicketEvent !== undefined);
  boardSocket.socket.close();

  // --- [3] timer: typed rejection + ordered replay ------------------------
  state.online = false;
  await engine.handleConnectivityChange(false);
  const bogusStop = await engine.mutate(stopTimer, {
    sessionId: randomUUID(),
    ticketId: ticket.id,
  });
  check("bogus stop queued offline", bogusStop.queued === true);
  state.online = true;
  await engine.handleConnectivityChange(true);
  await waitFor(() => (engine.queue.pendingCount() === 0 ? true : undefined), 10000).catch(
    () => undefined,
  );
  const rejected = engine.queue.rejected();
  check(
    "bogus stop rejected with typed copy, never applied blind",
    rejected.length >= 1,
    `rejected=${rejected.length}`,
  );
  for (const op of rejected) await engine.queue.drop(op.id);

  const started = await engine.mutate(startTimer, {
    ticketId: ticket.id,
    ticketNumber: ticket.number,
  });
  check(
    "online timer start applied",
    started.ok === true && started.queued === false,
    `ok=${started.ok} queued=${started.queued} failure=${started.failure?.text ?? ""}`,
  );
  const entriesBefore = (await rest<unknown[]>(`/api/v1/tickets/${ticket.id}/time-entries`, {
    token: ownerToken,
  })).length;
  const active = await waitFor(async () => {
    const response = await rest<{ id: string } | null>("/api/v1/timers/active", {
      token: ownerToken,
    });
    return response !== null ? response : undefined;
  }, 8000).catch(() => undefined);
  check("active timer read model shows the session", active !== undefined);
  if (active !== undefined) {
    // The server rejects a zero-second entry ("invalid duration"); a real
    // stop follows a worked interval.
    await sleep(1100);
    const stopped = await engine.mutate(stopTimer, {
      sessionId: active.id,
      ticketId: ticket.id,
    });
    check(
      "timer stop applied",
      stopped.ok === true,
      `ok=${stopped.ok} queued=${stopped.queued} failure=${stopped.failure?.text ?? ""}`,
    );
    const entries = await waitFor(async () => {
      const rows = await rest<unknown[]>(`/api/v1/tickets/${ticket.id}/time-entries`, {
        token: ownerToken,
      });
      return rows.length > entriesBefore ? rows : undefined;
    }, 8000).catch(() => undefined);
    check(
      "stop materialized a time entry",
      entries !== undefined,
      `before=${entriesBefore} after=${entries?.length}`,
    );
  }

  // --- [4] consumption replay ---------------------------------------------
  if (consumable !== undefined && bin !== undefined) {
    state.online = false;
    await engine.handleConnectivityChange(false);
    const consumeMark = `proof-consume-${randomUUID().slice(0, 8)}`;
    const consume = await engine.mutate(consumeStock, {
      itemId: consumable.id,
      itemName: consumable.name,
      locationId: bin.id,
      qtyMilli: 1000,
      ticketId: ticket.id,
      note: consumeMark,
    });
    check("offline consumption queued", consume.queued === true);
    state.online = true;
    await engine.handleConnectivityChange(true);
    await waitFor(() => (engine.queue.pendingCount() === 0 ? true : undefined), 10000);
    const movements = await rest<{ note?: string | null }[]>(
      `/api/v1/stock-items/${consumable.id}/movements`,
      { token: ownerToken },
    );
    const landed = movements.filter((movement) => movement.note === consumeMark);
    check(
      "consumption replay recorded exactly one movement",
      landed.length === 1,
      `found ${landed.length}`,
    );
  }

  // --- [5] tool checkout replay -------------------------------------------
  const serials = await fetchStockSerials(owner);
  const inStock = serials.find((serial) => serial.serial_number === "PROOF-SN-1");
  if (inStock !== undefined) {
    if (inStock.status === "checked_out" && inStock.checkout_id != null) {
      // A previous run left it out; return it first so the leg starts clean.
      await engine.mutate(checkinTools, { checkoutId: inStock.checkout_id });
    }
    const me = await rest<{ user_id: string }>("/api/v1/me", { token: ownerToken });
    state.online = false;
    await engine.handleConnectivityChange(false);
    const checkout = await engine.mutate(checkoutTools, {
      holderUserId: me.user_id,
      fromLocationId: bin?.id ?? inStock.location_id ?? "",
      dueAt: new Date(Date.now() + 86400000).toISOString(),
      items: [{ itemId: inStock.stock_item_id, serialId: inStock.id }],
    });
    check("offline tool checkout queued", checkout.queued === true);
    state.online = true;
    await engine.handleConnectivityChange(true);
    await waitFor(() => (engine.queue.pendingCount() === 0 ? true : undefined), 10000);
    const after = await rest<
      { id: string; status: string; holder_user_id: string | null; checkout_id: string | null }[]
    >("/api/v1/stock-serials", { token: ownerToken });
    const unit = after.find((candidate) => candidate.id === inStock.id);
    check(
      "checkout replay moved the unit to the holder",
      unit?.status === "checked_out" && unit.holder_user_id === me.user_id,
      `status=${unit?.status}`,
    );
    if (unit?.checkout_id != null) {
      const returned = await engine.mutate(checkinTools, { checkoutId: unit.checkout_id });
      check("checkin applied", returned.ok === true);
      const back = await waitFor(async () => {
        const rows = await rest<{ id: string; status: string }[]>("/api/v1/stock-serials", {
          token: ownerToken,
        });
        const rowAfter = rows.find((candidate) => candidate.id === inStock.id);
        return rowAfter?.status === "in_stock" ? true : undefined;
      }, 8000).catch(() => undefined);
      check("checkin returned the unit to stock", back === true);
    }
  }

  engine.stop();
  console.log(`\n[proof] ${passed} passed, ${failures.length} failed`);
  if (failures.length > 0) process.exit(1);
}

main().catch((error) => {
  console.error("[proof] fatal:", error);
  process.exit(1);
});
