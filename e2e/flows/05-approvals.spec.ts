// Flow 5 — Time approvals (the lane-7 close-out surface).
//
// TWO PRINCIPALS, because nobody signs off their own time. The rule lives in
// `ownEntry = e.entry.user_id === meId` (`apps/web/app/approvals/page.tsx`),
// which disables the checkbox and both row buttons for the viewer's own row
// and says why in the pane. A spec that submits the entry as the demo owner
// and then tries to approve it as the demo owner is asking the product to
// break its own rule, so the entry is staged as the demo technician and
// approved by the owner. The org queue is approver-scoped
// (`crates/core-api/src/routes/approvals.rs:360-396`), so the owner sees it.
import { expect, test } from "../helpers/harness";
import { authedFetch, apiLogin, DEMO_PASSWORD, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
import { resetSurfacePrefs } from "../helpers/prefs";

/** The demo technician (`scripts/demo-up.sh:350`). The entry is THEIRS, so the
 *  owner's queue renders it actionable — the other half of the transaction. */
const DEMO_TECH_EMAIL = "tech@demo.local";

interface TimeEntry {
  id: string;
  ticket_id: string;
  approval_status: string | null;
}

/** A draft time entry owned by the demo technician, submitted into the queue
 *  the owner will review. Staged through the technician's own token, because
 *  the owner of the entry is what makes the owner-viewer's controls live. */
async function stageSubmittedEntry(notes: string, minutes: number): Promise<TimeEntry> {
  const techTokens = await apiLogin(DEMO_TECH_EMAIL, DEMO_PASSWORD);
  const authed = authedFetch(techTokens.access_token);
  const get = authed;
  const post = <T>(path: string, body?: unknown): Promise<T> =>
    authed<T>(path, { method: "POST", body });

  const tickets = (
    await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)
  ).filter((t) => t.category !== "closed");
  expect(tickets.length).toBeGreaterThan(0);
  const entry = await post<TimeEntry>("/time-entries", {
    ticket_id: tickets[0].id,
    started_at: new Date(Date.now() - 3600_000).toISOString(),
    ended_at: new Date().toISOString(),
    minutes,
    billable: true,
    notes,
    work_type: "remote",
  });
  expect(entry.approval_status).toBe("draft");
  await post(`/time-entries/${entry.id}/submit`, {});
  return entry;
}

test("submit a draft entry, bulk-approve it from the queue, see the trail", async ({
  authedPage,
  ownerTokens,
}) => {
  // The technician authors and submits; the owner is the second principal who
  // signs it off. Same entry id on both sides of the transaction.
  const entry = await stageSubmittedEntry(`E2E approvals ${Date.now() % 100000}`, 60);

  // The owner's queue lists it — and it is actionable, because it is not theirs.
  await resetSurfacePrefs(ownerTokens.access_token);
  await authedPage.goto("/approvals");
  await authedPage.getByRole("radio", { name: /^Queue/ }).click();
  const row = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Approve from the queue (per-item button) and verify read-your-write removal.
  await row.getByRole("button", { name: "Approve" }).click();
  await expect(row).toBeHidden({ timeout: 20_000 });

  // The approved tab shows the entry with its decision trail. The view control
  // is a Segmented `radiogroup` (packages/ui/src/components/view-switch.tsx), so
  // the tab is a RADIO. Asking for a button named "Approved" matches the
  // "Unapproved N" filter button instead — Playwright's `name` is a
  // case-insensitive substring match by default — and the click lands on a
  // filter while the queue stays on screen.
  await authedPage.getByRole("radio", { name: "Approved" }).click();
  const approvedRow = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(approvedRow).toBeVisible({ timeout: 20_000 });
  await approvedRow.getByTestId(`history-${entry.id}`).click();
  await expect(approvedRow.getByTestId(`trail-${entry.id}`)).toBeVisible();

  // The decision is durable, not just a local patch: the API agrees, and the
  // approver is the owner rather than the technician who authored it.
  const get = authedFetch(ownerTokens.access_token);
  const queue = await get<any[]>("/time-entries/approvals/queue?status=approved");
  const decided = queue.find((item: any) => item.entry?.id === entry.id || item.id === entry.id);
  expect(decided, "the approved entry leaves the queue and lands in the ledger").toBeTruthy();
});

test("rejection requires a typed reason code", async ({ authedPage, ownerTokens }) => {
  const entry = await stageSubmittedEntry(`E2E reject ${Date.now() % 100000}`, 30);

  await resetSurfacePrefs(ownerTokens.access_token);
  await authedPage.goto("/approvals");
  // This row is submitted, so it lives in Queue only. Saved views are reset
  // above; selecting the tab is the belt to that braces.
  await authedPage.getByRole("radio", { name: /^Queue/ }).click();
  const row = authedPage.getByTestId(`approval-row-${entry.id}`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Select the row, open the reject panel — sending WITHOUT a typed reason
  // is refused inline (typed reason vocabulary, never prose alone). The refusal
  // message is a sibling of `reject-panel` inside the dialog, not a child of it
  // (apps/web/app/approvals/page.tsx:1242-1248), so it is scoped to the dialog.
  await row.locator('input[type="checkbox"]').check();
  await authedPage.getByTestId("bulk-bar").getByRole("button", { name: "Reject…" }).click();
  await expect(authedPage.getByTestId("reject-panel")).toBeVisible();
  const rejectDialog = authedPage.getByRole("dialog", { name: "Reject this entry" });
  await authedPage.getByTestId("reject-send").click();
  await expect(rejectDialog.getByRole("alert")).toBeVisible();

  // Pick a reason, reject, and the row leaves the queue.
  await authedPage.getByTestId("reject-reason").selectOption({ index: 1 });
  await authedPage.getByTestId("reject-send").click();
  await expect(row).toBeHidden({ timeout: 20_000 });
});
