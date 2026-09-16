// Flow 4 — Notes + tasks on the ticket workspace: post a client-visible
// note through the composer and complete a task (tri-state).
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
import { openWorkspaceWithRecovery } from "../helpers/workspace";

async function openTicket(authedPage: any, ownerTokens: any) {
  const get = authedFetch(ownerTokens.access_token);
  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed",
  );
  const ticket = tickets[0];
  expect(ticket).toBeTruthy();
  await openWorkspaceWithRecovery(authedPage, ticket.id);
  return ticket;
}

test("post a client-visible note and see it on the timeline", async ({ authedPage, ownerTokens }) => {
  const ticket = await openTicket(authedPage, ownerTokens);
  await authedPage.getByTestId("detail-tab-notes").click();
  await expect(authedPage.getByTestId("notes-composer")).toBeVisible({ timeout: 15_000 });

  const body = `E2E note ${Date.now() % 100000} — diagnosis captured.`;
  await authedPage.getByTestId("notes-composer-body").fill(body);
  await authedPage.getByTestId("note-kind-client_visible").click();
  await authedPage.getByTestId("note-post").click();
  await expect(authedPage.getByText(body).first()).toBeVisible({ timeout: 20_000 });
});

test("add a task and check it off (tri-state)", async ({ authedPage, ownerTokens }) => {
  const ticket = await openTicket(authedPage, ownerTokens);
  await authedPage.getByTestId("detail-tab-tasks").click();
  const input = authedPage.getByTestId("tab-task-input");
  await expect(input).toBeVisible({ timeout: 15_000 });

  const task = `E2E task ${Date.now() % 100000}`;
  await input.fill(task);
  await input.press("Enter");

  // The row appears (optimistic or server-confirmed — id is unstable during
  // the envelope round-trip, so anchor on text + aria-labels, never ids).
  const row = authedPage.locator("li").filter({ hasText: task }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });

  // Check it off: the ✓ button's aria-label flips Check → Uncheck on
  // completion. Anchor on the check button via its testid PREFIX (ids are
  // unstable during the envelope round-trip) — the ✗ button's aria-label
  // is always "Uncheck <task>", so the ✓ button's label is the state signal.
  const checkBtn = row.locator('button[data-testid^="tab-task-check-"]');
  await checkBtn.click();
  await expect(checkBtn).toHaveAttribute("aria-label", `Uncheck ${task}`, {
    timeout: 15_000,
  });
});
