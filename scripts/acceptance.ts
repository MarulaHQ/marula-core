/**
 * Stage-1 acceptance harness (BUILD-PROMPT §7): create ticket → work it with
 * a timer → close → time entry appears. Runs against a live core-api using
 * @meridian/sdk itself — so it doubles as an end-to-end SDK test.
 *
 * Usage: pnpm accept [--url http://localhost:8080] [--ws-url ws://localhost:8081] [--skip-perf]
 * Requires: core-api running with its Postgres. Checks [1]–[9] need ONLY that.
 * Check [10] exercises the full push chain (api → outbox → relay → Redis →
 * hub → socket) and needs Redis + worker + realtime up; if the hub is not
 * reachable it reports SKIP instead of FAIL so API-only runs stay green.
 * Check [11] measures p95 latency of UI-backed calls against the <100 ms
 * budget (BUILD-PROMPT §Stage-1); --skip-perf opts out.
 *
 * Non-purpose: no UI driving (that was verified via Playwright separately).
 */
import { MeridianClient } from "../packages/sdk/src/index.js";

const API_URL = process.argv.includes("--url")
  ? process.argv[process.argv.indexOf("--url") + 1]
  : process.env.ACCEPTANCE_API_URL ?? "http://localhost:8080";

const slug = `accept-${Date.now()}`;
let passed = 0;
const failures = [];

function check(name, condition, detail = "") {
  if (condition) {
    passed += 1;
    console.log(`  ok  ${name}`);
  } else {
    failures.push(name);
    console.error(`FAIL  ${name} ${detail}`);
  }
}

async function typed(call, name) {
  const { data, error } = await call;
  if (error) throw new Error(`${name} failed: ${JSON.stringify(error)}`);
  return data;
}

const owner = new MeridianClient({ baseUrl: API_URL });

// 1. Register bootstraps org + Owner + defaults.
console.log(`\n[1] register org (${slug})`);
const tokens = await owner.register({
  org_name: "Acceptance Co",
  org_slug: slug,
  email: `owner@${slug}.test`,
  password: "correct-horse-battery",
  display_name: "Acceptance Owner",
});
check("register returns tokens", Boolean(tokens.access_token && tokens.refresh_token));

// 2. Authenticated identity carries the full permission matrix.
console.log("\n[2] /api/me");
const me = await typed(owner.api.GET("/api/me"), "me");
check("me is the registered owner", me.email === `owner@${slug}.test`);
check("owner has tickets:create", me.permissions.includes("tickets:create"));

// 3. Default board seeded with four semantic statuses.
console.log("\n[3] default board + statuses");
const boards = await typed(owner.api.GET("/api/boards"), "boards");
check("one default board seeded", boards.length === 1 && boards[0].is_default);
const statuses = await typed(
  owner.api.GET("/api/boards/{id}/statuses", { params: { path: { id: boards[0].id } } }),
  "statuses",
);
check("four semantic statuses", statuses.length === 4);
const entry = statuses.find((s) => s.is_entry);
const closed = statuses.find((s) => s.category === "closed");
check("entry + closed statuses exist", Boolean(entry && closed));

// 4. Ticket lifecycle: create with server-allocated number.
console.log("\n[4] ticket create");
const ticket = await typed(
  owner.api.POST("/api/tickets", {
    body: {
      board_id: boards[0].id,
      title: "Acceptance: printer on fire",
      body: "Stage-1 acceptance flow",
      priority: "p1",
    },
  }),
  "create ticket",
);
check("ticket number allocated", /^MER-\d+$/.test(ticket.number));

// 5. Timer: start → working set shows it → stop materializes an entry.
console.log("\n[5] timer start/stop");
const started = await typed(
  owner.api.POST("/api/timers/start", { body: { ticket_id: ticket.id } }),
  "timer start",
);
check("timer running", started.stopped_at === null);
await new Promise((r) => setTimeout(r, 1100)); // ensure >0 elapsed for rounding
const stopped = await typed(
  owner.api.POST("/api/timers/stop", { body: { reason: "acceptance" } }),
  "timer stop",
);
check("time entry created from timer", stopped.minutes >= 1);

// 6. Close the ticket via semantic status change.
console.log("\n[6] close ticket");
const closedTicket = await typed(
  owner.api.POST(`/api/tickets/{id}/status`, {
    params: { path: { id: ticket.id } },
    body: { status_id: closed.id },
  }),
  "status change",
);
check("ticket closed with resolved_at", closedTicket.category === "closed" && closedTicket.resolved_at !== null);

// 7. The time entry appears in listings.
console.log("\n[7] time entry listing");
const entries = await typed(
  owner.api.GET("/api/time-entries", { params: { query: { ticket_id: ticket.id } } }),
  "time entries",
);
check("entry listed for ticket", entries.length === 1 && entries[0].timer_session_id === started.id);

// 8. Refresh rotation works; old refresh token dies with it.
console.log("\n[8] refresh rotation");
const rotated = await owner.refresh();
check("refresh returns new tokens", rotated.access_token.length > 0 && rotated.refresh_token !== tokens.refresh_token);

// 9. A second client cannot see the first org's data (RLS backstop).
console.log("\n[9] tenant isolation");
const outsider = new MeridianClient({ baseUrl: API_URL });
await outsider.register({
  org_name: "Outsider Co",
  org_slug: `${slug}-out`,
  email: `owner@${slug}-out.test`,
  password: "correct-horse-battery",
  display_name: "Outsider",
});
const outsideBoardList = await typed(outsider.api.GET("/api/boards"), "outsider boards");
check("other org's boards invisible", !outsideBoardList.some((b) => b.id === boards[0].id));
const crossRead = await outsider.api.GET("/api/tickets/{id}", {
  params: { path: { id: ticket.id } },
});
check("cross-org ticket read rejected", crossRead.error !== undefined);

// 10. Push chain: api mutation → outbox → relay → Redis → hub → this socket.
// SKIPs (does not fail) when the realtime hub isn't running, so API-only
// environments still pass; a connected socket that never sees its event FAILS.
console.log("\n[10] websocket event delivery");
const wsUrl = (() => {
  if (process.argv.includes("--ws-url")) return process.argv[process.argv.indexOf("--ws-url") + 1];
  const env = process.env.ACCEPTANCE_WS_URL;
  if (env) return env;
  const u = new URL(API_URL);
  u.protocol = "ws";
  u.port = "8081";
  u.pathname = "/ws";
  return u.toString();
})();
let sawEvent = false;
try {
  const ws = await new Promise<WebSocket>((resolve, reject) => {
    const sock = new WebSocket(`${wsUrl}?token=${rotated.access_token}`);
    const t = setTimeout(() => {
      sock.close();
      reject(new Error("connect timeout"));
    }, 4000);
    sock.onopen = () => {
      clearTimeout(t);
      resolve(sock);
    };
    sock.onerror = () => {
      clearTimeout(t);
      reject(new Error("socket error"));
    };
  });
  const nextMessage = <T = unknown>(timeoutMs: number): Promise<T> =>
    new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("message timeout")), timeoutMs);
      ws.onmessage = (m) => {
        clearTimeout(t);
        resolve(JSON.parse(String(m.data)));
      };
    });
  type ServerFrame = { type: string; channel?: string; event?: { type: string; entity_id?: string } };
  const hello = await nextMessage<ServerFrame>(4000);
  check("hub authenticates jwt and says hello", hello.type === "hello");
  ws.send(JSON.stringify({ type: "subscribe", channel: `board:${boards[0].id}` }));
  const subbed = await nextMessage<ServerFrame>(4000);
  check("board subscribe confirmed", subbed.type === "subscribed" && subbed.channel === `board:${boards[0].id}`);
  const probeTicket = await typed(
    owner.api.POST("/api/tickets", {
      body: { board_id: boards[0].id, title: "Acceptance: WS probe", priority: "p3" },
    }),
    "create ws-probe ticket",
  );
  while (true) {
    const frame = await nextMessage<ServerFrame>(10000);
    if (frame.type === "event" && frame.event?.type === "ticket.created" && frame.event?.entity_id === probeTicket.id) {
      sawEvent = true;
      break;
    }
    if (frame.type === "error") throw new Error(`hub error: ${JSON.stringify(frame)}`);
  }
  ws.close();
} catch (err) {
  console.log(`  skip  websocket delivery (${(err as Error).message}; full push chain needs Redis + worker + realtime up)`);
}
check("ticket.created pushed over ws", sawEvent);

// 11. p95 latency of UI-backed interactions against the <100 ms budget.
if (!process.argv.includes("--skip-perf")) {
  console.log("\n[11] p95 latency (budget: 100 ms)");
  const SAMPLES = 12;
  const percentile = (xs: number[], p: number) => {
    const s = [...xs].sort((a, b) => a - b);
    return s[Math.min(s.length - 1, Math.ceil(p * s.length) - 1)];
  };
  const measure = async (name: string, call: () => Promise<unknown>) => {
    const times: number[] = [];
    for (let i = 0; i <= SAMPLES; i++) {
      const t0 = performance.now();
      await call();
      if (i > 0) times.push(performance.now() - t0); // discard warmup
    }
    return { name, p50: percentile(times, 0.5), p95: percentile(times, 0.95) };
  };
  let n = 0;
  const results = [
    await measure("GET /api/tickets", () => typed(owner.api.GET("/api/tickets"), "list")),
    await measure("GET /api/tickets/{id}", () =>
      typed(owner.api.GET("/api/tickets/{id}", { params: { path: { id: ticket.id } } }), "get"),
    ),
    await measure("POST comment", async () => {
      n += 1;
      await typed(
        owner.api.POST("/api/tickets/{id}/comments", {
          params: { path: { id: ticket.id } },
          body: { body: `perf probe ${n}`, kind: "internal" },
        }),
        "comment",
      );
    }),
  ];
  for (const r of results) console.log(`  ${r.name.padEnd(22)} p50 ${r.p50.toFixed(1)} ms · p95 ${r.p95.toFixed(1)} ms`);
  check("all p95 under 100 ms", results.every((r) => r.p95 < 100));
}

// Summary.
console.log(`\n${passed} checks passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.error("failed:", failures.join(", "));
  process.exit(1);
}
