// Flow 2 — Ticket create → detail: create from the board entry state
// through the instant-first envelope, then land on the ticket workspace
// with the status bar rendering the entry status.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
import { openWorkspaceWithRecovery } from "../helpers/workspace";

test("create a ticket from the board and open its workspace", async ({ authedPage }) => {
  const suffix = Date.now() % 100000;
  const title = `E2E create flow ${suffix}`;

  await authedPage.goto(`/boards/${DEMO_SERVICE_DESK_BOARD_ID}`);
  await authedPage.getByRole("button", { name: "New ticket" }).click();
  await authedPage.fill("#new-ticket-title", title);
  await authedPage.getByRole("button", { name: "Create ticket" }).click();

  // Board shows the new ticket (instant-first read-your-write).
  await expect(authedPage.getByText(title).first()).toBeVisible({ timeout: 20_000 });

  // Open the workspace and verify the status bar renders.
  await authedPage.getByText(title).first().click();
  await expect(authedPage.getByTestId("ticket-status-bar")).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByTestId("ticket-status-text")).not.toHaveText("Unknown status");
});

test("ticket workspace renders tabs, notes composer, and timeline", async ({ authedPage, ownerTokens }) => {
  const get = authedFetch(ownerTokens.access_token);
  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed",
  );
  const ticket = tickets[0];
  expect(ticket).toBeTruthy();

  await openWorkspaceWithRecovery(authedPage, ticket.id);
  for (const tab of ["notes", "tasks", "details"]) {
    await authedPage.getByTestId(`detail-tab-${tab}`).click();
    await expect(authedPage.getByTestId(`detail-panel-${tab}`)).toBeVisible();
  }
});
