/**
 * Stage-1 acceptance harness (BUILD-PROMPT §7): create ticket → work it with
 * a timer → close → time entry appears. Runs against a live core-api using
 * @meridian/sdk itself — so it doubles as an end-to-end SDK test.
 *
 * Usage: pnpm accept [--url http://localhost:8080]
 * Requires: core-api running with its Postgres; Redis NOT required (the
 * outbox relay is asynchronous and not part of this API-level flow).
 *
 * Non-purpose: no UI driving (Playwright comes later), no perf measurement.
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

// Summary.
console.log(`\n${passed} checks passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.error("failed:", failures.join(", "));
  process.exit(1);
}
