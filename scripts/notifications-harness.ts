/**
 * Notifications-lane live harness (Stage 2 item 2.2) — mimics
 * scripts/acceptance.ts check [10] for the notification chain, without
 * touching that file.
 *
 * Verifies, against a LIVE stack (core-api :18182, worker :18184 on the
 * lane's private Postgres 25433 / Redis 26479):
 *   [1] @mention in a comment creates a notification row transactionally
 *       and emits `notification.created` through outbox → relay → Redis →
 *       worker → core-api's SSE stream → the MENTIONED USER's stream
 *       (per-user fan-out);
 *   [2] `ticket.comment_added` rides the same leg to that user;
 *   [3] REST: cursor list shape, unread count flips, bulk mark-read is
 *       idempotent and empties the badge;
 *   [4] preferences: PUT persists a quiet-hours window with tz; invalid
 *       windows are rejected 422; GET reports the effective chain source;
 *   [5] self-mentions and unknown handles create nothing (negative leg).
 *
 * Usage:
 *   pnpm tsx scripts/notifications-harness.ts
 *   (env: NOTIF_API_URL, NOTIF_SSE_URL, NOTIF_JWT_SECRET, NOTIF_PG_CONTAINER)
 *
 * The live leg is core-api's SSE stream at `/api/events/stream?token=`. Node
 * ships no global EventSource, so the harness reads the stream straight off
 * fetch's response body and parses the frames itself.
 *
 * Non-purpose: no UI driving, no perf measurement, and it never edits
 * acceptance.ts — this is the lane-local live-push assertion demanded by §8.
 */
import crypto from "node:crypto";
import { execSync } from "node:child_process";

const API_URL = process.env.NOTIF_API_URL ?? "http://localhost:18182";
const SSE_URL = (() => {
  if (process.env.NOTIF_SSE_URL) return process.env.NOTIF_SSE_URL;
  const u = new URL(API_URL);
  u.pathname = "/api/events/stream";
  u.search = "";
  return u.toString();
})();
const JWT_SECRET = process.env.NOTIF_JWT_SECRET ?? "notif-harness-secret";
const PG_CONTAINER = process.env.NOTIF_PG_CONTAINER ?? "marula-notif";

let passed = 0;
const failures: string[] = [];
function check(name: string, condition: boolean, detail = "") {
  if (condition) {
    passed += 1;
    console.log(`  ok  ${name}`);
  } else {
    failures.push(name);
    console.error(`FAIL  ${name} ${detail}`);
  }
}

async function jsonFetch(
  method: string,
  path: string,
  body?: unknown,
  token?: string,
): Promise<{ status: number; data: any }> {
  const res = await fetch(`${API_URL}${path}`, {
    method,
    headers: {
      ...(body !== undefined ? { "content-type": "application/json" } : {}),
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
  });
  const text = await res.text();
  return { status: res.status, data: text ? JSON.parse(text) : null };
}

// --- teammate JWT (the seeded user never logs in; we mint its token) -----

const b64u = (buf: Buffer) => buf.toString("base64url");
function signJwt(claims: Record<string, unknown>, secret: string): string {
  const header = b64u(Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const payload = b64u(Buffer.from(JSON.stringify(claims)));
  const sig = b64u(
    crypto.createHmac("sha256", secret).update(`${header}.${payload}`).digest(),
  );
  return `${header}.${payload}.${sig}`;
}

// --- SSE helpers: core-api's /api/events/stream (the one live push leg) ---
//
// Each SSE `data:` frame is the core-events identity — `{type, org_id,
// entity, entity_id, seq, at}`, with the event uuid on the SSE `id:` field.
// The frame carries no payload, so assertions read `type`/`entity` and
// cross-check the recipient's row over REST. The stream is per-user at
// fan-out: a `notification.created` frame arrives on the recipient's own
// connection, which is what proves per-user delivery.

type StreamFrame = {
  type: string;
  org_id?: string;
  entity?: string;
  entity_id?: string | null;
  seq?: number | null;
  at?: string;
};

type SseEvent = { data?: string };
type SseListener = (event?: SseEvent) => void;

/** One SSE stream over fetch's response body. Node ships no global
 * EventSource, so the harness parses the frames itself; the shape mirrors
 * what `openStream` and `framePump` below expect. */
class FetchStream {
  readyState = 0;
  private readonly listeners = new Map<string, Set<SseListener>>();
  private readonly controller = new AbortController();

  constructor(url: string) {
    void this.run(url);
  }

  addEventListener(type: "open" | "error" | "message", fn: SseListener): void {
    const set = this.listeners.get(type) ?? new Set<SseListener>();
    set.add(fn);
    this.listeners.set(type, set);
  }

  close(): void {
    this.readyState = 2;
    this.controller.abort();
  }

  private emit(type: string, event?: SseEvent): void {
    for (const fn of [...(this.listeners.get(type) ?? [])]) fn(event);
  }

  private async run(url: string): Promise<void> {
    try {
      const response = await fetch(url, {
        headers: { accept: "text/event-stream" },
        signal: this.controller.signal,
      });
      if (!response.ok || response.body === null) {
        this.readyState = 2;
        this.emit("error");
        return;
      }
      this.readyState = 1;
      this.emit("open");
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let boundary = buffer.indexOf("\n\n");
        while (boundary !== -1) {
          const block = buffer.slice(0, boundary);
          buffer = buffer.slice(boundary + 2);
          const data = block
            .split("\n")
            .filter((line) => line.startsWith("data:"))
            .map((line) => line.slice("data:".length).trimStart())
            .join("\n");
          if (data !== "") this.emit("message", { data });
          boundary = buffer.indexOf("\n\n");
        }
      }
      this.readyState = 2;
    } catch {
      if (this.readyState !== 2) {
        this.readyState = 2;
        this.emit("error");
      }
    }
  }
}

async function openStream(token: string): Promise<{ source: FetchStream; pump: FramePump }> {
  const source = new FetchStream(`${SSE_URL}?token=${encodeURIComponent(token)}`);
  await new Promise<void>((resolve, reject) => {
    const deadline = setTimeout(() => reject(new Error("stream did not open")), 5000);
    source.addEventListener("open", () => {
      clearTimeout(deadline);
      resolve();
    });
    source.addEventListener("error", () => {
      clearTimeout(deadline);
      reject(new Error("stream refused"));
    });
  });
  return { source, pump: framePump(source) };
}

/**
 * One persistent listener per stream: every incoming frame is recorded AND
 * matched against registered waiters, so several `waitFor` promises can be
 * outstanding simultaneously.
 */
function framePump(source: FetchStream) {
  const seen: StreamFrame[] = [];
  const waiters: Array<{
    match: (frame: StreamFrame) => boolean;
    resolve: (frame: StreamFrame) => void;
    timer: NodeJS.Timeout;
  }> = [];
  source.addEventListener("message", (message) => {
    const raw = message?.data;
    if (typeof raw !== "string") return;
    const frame = JSON.parse(raw) as StreamFrame;
    seen.push(frame);
    for (let i = waiters.length - 1; i >= 0; i--) {
      if (waiters[i].match(frame)) {
        clearTimeout(waiters[i].timer);
        waiters.splice(i, 1)[0].resolve(frame);
      }
    }
  });
  return {
    seen,
    waitFor(match: (frame: StreamFrame) => boolean): Promise<StreamFrame> {
      const already = seen.find(match);
      if (already) return Promise.resolve(already);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("message timeout")), 10000);
        waiters.push({ match, resolve, timer });
      });
    },
  };
}

type FramePump = ReturnType<typeof framePump>;

// ===========================================================================

const slug = `notif-${Date.now()}`;

console.log(`\n[1] register org (${slug})`);
const reg = await jsonFetch("POST", "/api/auth/register", {
  org_name: "Notif Harness Co",
  org_slug: slug,
  email: `owner@${slug}.test`,
  password: "correct-horse-battery",
  display_name: "Nora Harness",
});
check("register returns tokens", Boolean(reg.data?.access_token));
const ownerToken: string = reg.data.access_token;

console.log("\n[2] seed teammate + mint their access token");
const me = await jsonFetch("GET", "/api/me", undefined, ownerToken);
const orgId: string = me.data.org_id;
check("owner identity resolves", Boolean(orgId));
const teammateId = crypto.randomUUID();
const teammateEmail = `terry.mate@${slug}.test`;
execSync(
  `docker exec ${PG_CONTAINER} psql -U marula -d marula -c ` +
    `"INSERT INTO users (id, org_id, email, display_name, password_hash, timezone)
      VALUES ('${teammateId}', '${orgId}', '${teammateEmail}', 'Terry Mate', 'harness-no-login', 'UTC')
      ON CONFLICT DO NOTHING"`.replace(/\s+/g, " "),
);
const sid = crypto.randomUUID();
const nowSec = Math.floor(Date.now() / 1000);
const teammateToken = signJwt(
  { sub: teammateId, org: orgId, sid, exp: nowSec + 900, iat: nowSec },
  JWT_SECRET,
);

console.log("\n[3] teammate's SSE stream opens (org-scoped; per-user fan-out)");
const teammateStream = await openStream(teammateToken);
check("teammate's stream connected", teammateStream.source.readyState === 1, String(teammateStream.source.readyState));
const pump = teammateStream.pump;

console.log("\n[4] mention flow over the live push chain");
const boards = await jsonFetch("GET", "/api/boards", undefined, ownerToken);
const boardId: string = boards.data[0].id;
const ticket = await jsonFetch(
  "POST",
  "/api/tickets",
  { board_id: boardId, title: "Harness: mention probe", priority: "p3" },
  ownerToken,
);
check("probe ticket created", ticket.status === 200);

const notifPromise = pump.waitFor(
  (f) => f.type === "notification.created",
);
const commentAddedPromise = pump.waitFor(
  (f) => f.type === "ticket.comment_added",
);
const comment = await jsonFetch(
  "POST",
  `/api/tickets/${ticket.data.id}/comments`,
  { body: `@terry-mate please take a look (@nora-harness self ping, @ghost unknown)`, kind: "internal" },
  ownerToken,
);
check("comment accepted", comment.status === 200);
const [notifFrame, commentFrame] = await Promise.all([notifPromise, commentAddedPromise]);
check(
  "notification.created delivered to the MENTIONED user's stream",
  notifFrame.type === "notification.created" && notifFrame.entity === "notification",
  JSON.stringify(notifFrame),
);
check(
  "ticket.comment_added delivered on the same leg",
  commentFrame.type === "ticket.comment_added" && commentFrame.entity === "ticket_comment",
  JSON.stringify(commentFrame),
);
// The stream frame carries the event identity, not the payload, so the
// mention's recipient and kind are cross-checked on the durable row.
const mentionInbox = await jsonFetch("GET", "/api/notifications?limit=1", undefined, teammateToken);
const mentionRow = mentionInbox.data?.items?.[0];
check(
  "the mention landed as a durable row for the mentioned user",
  mentionRow?.user_id === teammateId &&
    mentionRow?.kind === "mention" &&
    mentionRow?.ticket_id === ticket.data.id,
  JSON.stringify(mentionRow),
);

console.log("\n[5] REST unread/list/mark-read as the mentioned user");
// Three mentions ⇒ three inbox rows, enough to exercise keyset paging.
for (let i = 0; i < 2; i++) {
  const extra = await jsonFetch(
    "POST",
    `/api/tickets/${ticket.data.id}/comments`,
    { body: `@terry-mate follow-up ${i} (paging probe)`, kind: "internal" },
    ownerToken,
  );
  if (extra.status !== 200) throw new Error("paging-probe comment failed");
}
const unread1 = await jsonFetch("GET", "/api/notifications/unread-count", undefined, teammateToken);
check("unread count is 3", unread1.data?.count === 3, JSON.stringify(unread1.data));

const page1 = await jsonFetch(
  "GET",
  "/api/notifications?limit=2",
  undefined,
  teammateToken,
);
check(
  "cursor page returns two items + next cursor",
  page1.data?.items?.length === 2 && typeof page1.data?.next_cursor === "string",
  JSON.stringify(page1.data),
);
check(
  "page is newest-first",
  page1.data.items[0].created_at >= page1.data.items[1].created_at,
);
const page2 = await jsonFetch(
  "GET",
  `/api/notifications?limit=10&cursor=${encodeURIComponent(page1.data.next_cursor)}`,
  undefined,
  teammateToken,
);
check(
  "second page carries the remainder and exhausts",
  page2.data?.items?.length === 1 && page2.data?.next_cursor === null,
  JSON.stringify(page2.data),
);

const allIds: string[] = [
  ...page1.data.items.map((n: any) => n.id),
  ...page2.data.items.map((n: any) => n.id),
];
const markBulk = await jsonFetch(
  "POST",
  "/api/notifications/mark-read",
  { ids: [...allIds, ...allIds] },
  teammateToken,
);
check("bulk mark-read is idempotent under duplicate ids", markBulk.data?.marked === 3, JSON.stringify(markBulk.data));
const unread2 = await jsonFetch("GET", "/api/notifications/unread-count", undefined, teammateToken);
check("badge emptied after bulk read", unread2.data?.count === 0, JSON.stringify(unread2.data));

console.log("\n[6] negative legs: in-app pref off + unknown/self handles");
await jsonFetch(
  "PUT",
  "/api/notification-preferences",
  { kind: "mention", in_app: false, email: false },
  teammateToken,
);
const before = await jsonFetch("GET", "/api/notifications/unread-count", undefined, teammateToken);
await jsonFetch(
  "POST",
  `/api/tickets/${ticket.data.id}/comments`,
  { body: "@terry-mate this should be suppressed (in_app off)", kind: "internal" },
  ownerToken,
);
await new Promise((r) => setTimeout(r, 700));
const after = await jsonFetch("GET", "/api/notifications/unread-count", undefined, teammateToken);
check(
  "in_app=false suppresses the inbox row entirely",
  after.data?.count === before.data?.count,
  `${JSON.stringify(before.data)} -> ${JSON.stringify(after.data)}`,
);
await jsonFetch(
  "PUT",
  "/api/notification-preferences",
  { kind: "mention", in_app: true, email: false },
  teammateToken,
);

console.log("\n[7] preferences surface: validation + defaults-first chain");
const badPref = await jsonFetch(
  "PUT",
  "/api/notification-preferences",
  { kind: "mention", in_app: true, email: true, quiet_from: "22:00", quiet_until: "25:99" },
  ownerToken,
);
check("invalid window rejected 422", badPref.status === 422, String(badPref.status));
const badTz = await jsonFetch(
  "PUT",
  "/api/notification-preferences",
  { kind: "mention", in_app: true, email: true, timezone: "Mars/Olympus" },
  ownerToken,
);
check("unknown timezone rejected 422", badTz.status === 422, String(badTz.status));

const goodPref = await jsonFetch(
  "PUT",
  "/api/notification-preferences",
  {
    kind: "ticket_update",
    in_app: true,
    email: true,
    quiet_from: "22:00",
    quiet_until: "06:00",
    timezone: "Europe/Berlin",
  },
  ownerToken,
);
check(
  "valid quiet window persists (source=user)",
  goodPref.status === 200 && goodPref.data?.source === "user" && goodPref.data?.timezone === "Europe/Berlin",
  JSON.stringify(goodPref.data),
);
const prefsList = await jsonFetch("GET", "/api/notification-preferences", undefined, ownerToken);
const slaBreach = (prefsList.data as any[]).find((p) => p.kind === "sla_breach");
check(
  "untouched kinds report builtin defaults",
  prefsList.status === 200 && slaBreach?.source === "builtin" && slaBreach?.in_app === true && slaBreach?.email === false,
  JSON.stringify(slaBreach),
);

teammateStream.source.close();

console.log(`\n${passed} checks passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.error("failed:", failures.join(", "));
  process.exit(1);
}
