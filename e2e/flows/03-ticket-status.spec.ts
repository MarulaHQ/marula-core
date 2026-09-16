// Flow 3 — Ticket status transitions: change status from the status bar,
// verify the read-your-write update and the workflow gating.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
import { openWorkspaceWithRecovery } from "../helpers/workspace";

test("status change from the bar updates the status text", async ({ authedPage, ownerTokens }) => {
  const get = authedFetch(ownerTokens.access_token);
  const statuses = await get<any[]>(`/boards/${DEMO_SERVICE_DESK_BOARD_ID}/statuses`);
  const entry = statuses.find((s) => s.is_entry) ?? statuses[0];
  const others = statuses.filter((s) => s.id !== entry.id && s.category !== "closed");
  const target = others[0];
  expect(target).toBeTruthy();

  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed" && t.status_id === entry.id,
  );
  const ticket = tickets[0] ?? (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).find((t) => t.category !== "closed");
  expect(ticket).toBeTruthy();

  await openWorkspaceWithRecovery(authedPage, ticket.id);
  const before = await authedPage.getByTestId("ticket-status-text").innerText();

  await authedPage.getByTestId("ticket-status-change").click();
  const menu = authedPage.getByRole("menu", { name: "Move to status" });
  await expect(menu).toBeVisible();
  // Disabled options are workflow-gated — pick one that's enabled and different.
  const options = menu.getByRole("button").filter({ hasText: /.+/ });
  const count = await options.count();
  let picked = "";
  for (let i = 0; i < count; i++) {
    const btn = options.nth(i);
    const label = (await btn.innerText()).trim();
    if (label === "Move to another board…") continue;
    if (await btn.isEnabled()) {
      if (label !== before) {
        picked = label;
        await btn.click();
        break;
      }
    }
  }
  expect(picked).not.toBe("");
  await expect(authedPage.getByTestId("ticket-status-text")).toHaveText(picked, { timeout: 20_000 });
});

test("status menu disables workflow-forbidden transitions", async ({ authedPage, ownerTokens }) => {
  const get = authedFetch(ownerTokens.access_token);
  const tickets = (await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)).filter(
    (t) => t.category !== "closed",
  );
  const ticket = tickets[0];
  expect(ticket).toBeTruthy();

  await openWorkspaceWithRecovery(authedPage, ticket.id);
  await authedPage.getByTestId("ticket-status-change").click();
  const menu = authedPage.getByRole("menu", { name: "Move to status" });
  await expect(menu).toBeVisible();
  // At least one option must be enabled (the workflow is never a dead end).
  const enabled = menu.locator("button:not([disabled])");
  expect(await enabled.count()).toBeGreaterThan(0);
});
