// Flow 5 — Time approvals (the lane-7 close-out surface): submit a draft
// entry via API, approve it from the /approvals queue, verify the decision
// trail. This exercises the final close-out UI, not a moving target.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";

interface TimeEntry {
  id: string;
  ticket_id: string;
  approval_status: string | null;
}

test("submit a draft entry, bulk-approve it from the queue, see the trail", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });

  // Stage: a fresh draft entry on an open demo ticket, submitted into the queue.
  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed",
  );
  expect(tickets.length).toBeGreaterThan(0);
  const started = new Date(Date.now() - 3600_000).toISOString();
  const ended = new Date().toISOString();
  const entry = await post<TimeEntry>("/time-entries", {
    ticket_id: tickets[0].id,
    started_at: started,
    ended_at: ended,
    minutes: 60,
    billable: true,
    notes: `E2E approvals ${Date.now() % 100000}`,
    work_type: "remote",
  });
  expect(entry.approval_status).toBe("draft");
  await post(`/time-entries/${entry.id}/submit`, {});

  // The queue lists it.
  await authedPage.goto("/approvals");
  const row = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Approve from the queue (per-item button) and verify read-your-write removal.
  await row.getByRole("button", { name: "Approve" }).click();
  await expect(row).toBeHidden({ timeout: 20_000 });

  // The approved tab shows the entry with its decision trail.
  await authedPage.getByRole("button", { name: "Approved" }).click();
  const approvedRow = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(approvedRow).toBeVisible({ timeout: 20_000 });
  await approvedRow.getByTestId(`history-${entry.id}`).click();
  await expect(approvedRow.getByTestId(`trail-${entry.id}`)).toBeVisible();
});

test("rejection requires a typed reason code", async ({ authedPage, ownerTokens }) => {
  const get = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });

  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed",
  );
  const started = new Date(Date.now() - 7200_000).toISOString();
  const entry = await post<TimeEntry>("/time-entries", {
    ticket_id: tickets[0].id,
    started_at: started,
    ended_at: new Date().toISOString(),
    minutes: 30,
    billable: true,
    notes: `E2E reject ${Date.now() % 100000}`,
    work_type: "remote",
  });
  await post(`/time-entries/${entry.id}/submit`, {});

  await authedPage.goto("/approvals");
  const row = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Select the row, open the reject panel — sending WITHOUT a typed reason
  // is refused inline (typed reason vocabulary, never prose alone).
  await row.locator('input[type="checkbox"]').check();
  await authedPage.getByTestId("bulk-bar").getByRole("button", { name: "Reject…" }).click();
  await expect(authedPage.getByTestId("reject-panel")).toBeVisible();
  await authedPage.getByTestId("reject-send").click();
  await expect(authedPage.getByTestId("reject-panel").getByRole("alert")).toBeVisible();

  // Pick a reason, reject, and the row leaves the queue.
  await authedPage.getByTestId("reject-reason").selectOption({ index: 1 });
  await authedPage.getByTestId("reject-send").click();
  await expect(row).toBeHidden({ timeout: 20_000 });
});
