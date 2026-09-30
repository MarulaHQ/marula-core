// Wave 41 lane 8 (`p41-ui-money`) — seed the approvals queue so the surface
// can be SEEN, not just typechecked.
//
// Purpose: the demo org ships with an empty approver queue, which renders the
// empty state and nothing else. Every state the pack actually draws — the
// flagged strip, a mark with its fix in the row, the bulk bar, the entry in
// context — needs rows to exist. This mirrors the pack's own fixture
// (source/approvals.html `E(...)`): twelve entries across a few technicians,
// including one vague description, one that overlaps a colleague's, and one far
// longer than that person's usual.
//
// It is a capture aid, not a test fixture: it writes real rows into whatever
// stack CAPTURE_API points at, and it says so on stdout.
const API = process.env.CAPTURE_API ?? "http://localhost:28490";
const EMAIL = process.env.CAPTURE_EMAIL ?? "owner@demo.local";
const PASSWORD = process.env.CAPTURE_PASSWORD ?? "demo-password-123";

/** Minutes from local midnight, so the windows land on "today" for a reader. */
function todayAt(hour, minute) {
  const now = new Date();
  const d = new Date(now.getFullYear(), now.getMonth(), now.getDate(), hour, minute, 0, 0);
  return d.toISOString();
}

async function api(pathname, { method = "GET", token, body } = {}) {
  const res = await fetch(`${API}${pathname}`, {
    method,
    headers: {
      "content-type": "application/json",
      ...(token === undefined ? {} : { authorization: `Bearer ${token}` }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  if (!res.ok) {
    throw new Error(`${method} ${pathname} -> ${res.status} ${await res.text()}`);
  }
  return res.status === 204 ? null : res.json();
}

async function main() {
  const login = await api("/api/v1/auth/login", {
    method: "POST",
    body: { email: EMAIL, password: PASSWORD },
  });
  const token = login.access_token;
  console.log("logged in as", EMAIL);

  const tickets = await api("/api/v1/tickets?limit=200", { token });
  const list = Array.isArray(tickets) ? tickets : (tickets.items ?? []);
  if (list.length === 0) throw new Error("the demo org has no tickets to bill against");
  const users = await api("/api/v1/org/users?limit=50", { token });
  const staff = (users.users ?? []).map((u) => u.id);
  if (staff.length < 2) throw new Error("need at least two staff to build an overlap");
  console.log(`${list.length} tickets, ${staff.length} staff`);

  // The pack's fixture, in its own shape: who, which ticket, what they wrote,
  // and the window. `long` entries are 4x that person's usual so the
  // longer-than-usual verdict fires; the overlap pair share a boundary.
  // Each person's rows need their OWN session: a time entry is always created
  // for the caller, so one owner's session produces a queue of one person's
  // work (and every row overlapping). The demo staff share the demo password.
  const sessions = new Map();
  for (const user of users.users ?? []) {
    if (sessions.has(user.id)) continue;
    try {
      const auth = await api("/api/v1/auth/login", {
        method: "POST",
        body: { email: user.email, password: PASSWORD },
      });
      sessions.set(user.id, { token: auth.access_token, name: user.display_name });
    } catch {
      // Not every staff account can sign in with the demo password; skip it.
    }
  }
  const actors = [...sessions.entries()];
  if (actors.length < 2) {
    throw new Error(
      `need two signable staff to build a real queue; got ${actors.length} of ${(users.users ?? []).length}`,
    );
  }
  console.log(`${actors.length} signable staff: ${actors.map(([, v]) => v.name).join(", ")}`);
  const PLAN = [
    { notes: "Disabled the three leaver accounts and removed their licences.", from: [8, 15], to: [9, 0], wt: "remote" },
    { notes: "Mounted and configured two access points in the second-floor meeting rooms.", from: [8, 0], to: [9, 30], wt: "on_site" },
    // The vague one: two words, so "Vague description" has something to say.
    { notes: "Onsite support.", from: [8, 0], to: [9, 30], wt: "on_site" },
    { notes: "Enrolled six users in MFA and walked two through the app.", from: [8, 0], to: [9, 30], wt: "remote" },
    // Overlaps the row above by 30 minutes.
    { notes: "Fixed calendar sharing for two users and noted the setting.", from: [9, 0], to: [10, 0], wt: "remote" },
    { notes: "Ran the follow-up session and sent the recap to the partners.", from: [9, 0], to: [10, 0], wt: "remote" },
    { notes: "Surveyed all four floors and mapped every access point and cable run.", from: [8, 0], to: [10, 0], wt: "on_site" },
    // Four hours for someone whose usual is 45 minutes.
    { notes: "Imaged and set up the laptops.", from: [9, 0], to: [13, 0], wt: "on_site" },
    { notes: "Installed the phones and tested inbound and outbound calls.", from: [9, 0], to: [10, 30], wt: "on_site" },
    { notes: "Cut over the file server, then checked shares and permissions.", from: [8, 30], to: [11, 30], wt: "on_site" },
    { notes: "Ran and labelled 12 cable drops on floor two.", from: [8, 30], to: [11, 0], wt: "on_site" },
    { notes: "Reviewed the password policy against the client's requirements.", from: [9, 30], to: [10, 30], wt: "remote", billable: false, reason: "internal" },
  ];

  // Spread each person's rows across the day instead of stacking them: a queue
  // where every row overlaps tells you the overlap verdict works and nothing
  // else. One deliberate overlap per person is enough to prove it.
  const shiftHours = (i, entryIndex) => (entryIndex % 3) * 3;
  let made = 0;
  const perActor = new Map();
  for (const [i, row] of PLAN.entries()) {
    const ticket = list[i % list.length];
    const [actorId, actor] = actors[i % actors.length];
    const n = perActor.get(actorId) ?? 0;
    perActor.set(actorId, n + 1);
    const shift = n === 0 ? 0 : shiftHours(i, n);
    let entry;
    try {
      entry = await api("/api/v1/time-entries", {
      method: "POST",
      token: actor.token,
      body: {
        ticket_id: ticket.id,
        billable: row.billable ?? true,
        started_at: todayAt(row.from[0] + shift, row.from[1]),
        ended_at: todayAt(row.to[0] + shift, row.to[1]),
        notes: row.notes,
        work_type: row.wt,
        ...(row.billable === false ? { unbillable_reason: row.reason } : {}),
      },
      });
    } catch (e) {
      // Not every staff account may log time (a bookkeeper cannot). Skip the
      // row rather than failing the whole seed — the point is a queue with
      // more than one person's work in it, not a complete org chart.
      if (String(e).includes("403")) continue;
      throw e;
    }
    await api(`/api/v1/time-entries/${entry.id}/submit`, {
      method: "POST",
      token: actor.token,
      body: {},
    });
    made += 1;
  }
  console.log(`submitted ${made} entries into the approver queue`);
}

await main();
