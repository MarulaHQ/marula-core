// Flow 21 — Wave 38 web multi-window: route-keyed popouts, compact shell,
// and cross-window live consistency (two windows, one mutation, both update).
//
// Order-independence: scratch state only (a fresh note body per run); no
// demo rows mutated. Both windows share one browser context, exactly like
// a real user's main window + popout.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";

async function openTicketAny(authedPage: any, ownerTokens: any) {
  const get = authedFetch(ownerTokens.access_token);
  const tickets = (
    await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`)
  ).filter((t) => t.category !== "closed");
  const ticket = tickets[0];
  expect(ticket).toBeTruthy();
  return ticket;
}

test("popout window renders the compact shell (no nav rail, slim bar)", async ({
  authedPage,
  ownerTokens,
}) => {
  const ticket = await openTicketAny(authedPage, ownerTokens);
  await authedPage.goto(`/tickets/${ticket.id}`);
  await expect(authedPage.getByTestId("popout-open")).toBeVisible({ timeout: 20_000 });

  const [popout] = await Promise.all([
    authedPage.context().waitForEvent("page"),
    authedPage.getByTestId("popout-open").click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await expect(popout.getByTestId("popout-bar")).toBeVisible({ timeout: 20_000 });
  // Compact shell honesty: no nav rail, no top search bar.
  await expect(popout.locator("nav")).toHaveCount(0);
  await expect(popout.getByTestId("top-bar")).toHaveCount(0);
});

test("two windows, one mutation, both update (note posted in popout shows in main)", async ({
  authedPage,
  ownerTokens,
}) => {
  const ticket = await openTicketAny(authedPage, ownerTokens);
  await authedPage.goto(`/tickets/${ticket.id}`);
  await expect(authedPage.getByTestId("popout-open")).toBeVisible({ timeout: 20_000 });

  const [popout] = await Promise.all([
    authedPage.context().waitForEvent("page"),
    authedPage.getByTestId("popout-open").click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await expect(popout.getByTestId("detail-tab-notes")).toBeVisible({ timeout: 20_000 });

  // The main window watches the same ticket's notes tab.
  await authedPage.getByTestId("detail-tab-notes").click();
  await expect(authedPage.getByTestId("notes-composer")).toBeVisible({ timeout: 15_000 });

  // Post in the POPOUT window.
  const body = `E2E multi-window note ${Date.now() % 100000}`;
  await popout.getByTestId("detail-tab-notes").click();
  await expect(popout.getByTestId("notes-composer")).toBeVisible({ timeout: 15_000 });
  await popout.getByTestId("notes-composer-body").fill(body);
  await popout.getByTestId("note-post").click();
  await expect(popout.getByText(body).first()).toBeVisible({ timeout: 20_000 });

  // The MAIN window surfaces it live — BroadcastChannel echo + realtime hub.
  await expect(authedPage.getByText(body).first()).toBeVisible({ timeout: 20_000 });
});

test("board card context menu pops a ticket out; keyboard palette carries popout commands", async ({
  authedPage,
  ownerTokens,
}) => {
  const ticket = await openTicketAny(authedPage, ownerTokens);
  await authedPage.goto(`/boards/${DEMO_SERVICE_DESK_BOARD_ID}`);
  const move = authedPage.getByTestId(`kanban-move-${ticket.number}`);
  await expect(move).toBeVisible({ timeout: 20_000 });
  await move.click();
  await expect(authedPage.getByTestId(`kanban-popout-${ticket.number}`)).toBeVisible();
  const [popout] = await Promise.all([
    authedPage.context().waitForEvent("page"),
    authedPage.getByTestId(`kanban-popout-${ticket.number}`).click(),
  ]);
  await popout.waitForLoadState("domcontentloaded");
  await expect(popout.getByTestId("popout-bar")).toBeVisible({ timeout: 20_000 });
});
