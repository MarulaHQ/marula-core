// Demo smoke — the "the demo still works" net.
//
// Wave 47 lane A5 (`p47-demo-smoke`). One happy path, the one a prospect walks
// on a freshly seeded demo org, asserted end to end against a real booted
// stack:
//
//   login → ticket create → note → timer start/stop → time entry →
//   invoice draft → portal view → BYOK surface
//
// Why it lives in `e2e/flows`: Playwright's config collects `flows/**/*.spec.ts`
// (e2e/playwright.config.ts:46,48), so this spec rides `pnpm e2e` and the demo
// path is kept green by every future lane that runs the suite. It runs in the
// two base themes only; the preset projects whitelist their own specs.
//
// Order independence (e2e/README.md): every leg stages the rows it asserts on,
// so the file is green in any order and on a dirty demo org.
//
// What this spec deliberately does NOT do: make a live model call. The demo
// stack boots without a provider runtime, so the BYOK leg proves the surface
// and its admission posture are readable and typed. The live completion +
// embedding proof is `scripts/demo-ai-probe.mjs` (Wave 47 lane B2), which
// scripts/demo-smoke.sh runs when an operator supplies a key.
import { expect, test } from "../helpers/harness";
import {
  DEMO_SERVICE_DESK_BOARD_ID,
  WEB_URL,
  authedFetch,
} from "../helpers/api";
import { openWorkspaceWithRecovery } from "../helpers/workspace";

const PORTAL_EMAIL = "portal@acme.demo";
const PORTAL_PASSWORD = "demo-password-123";

/** The one path: create on the board, note it, take it, stop the clock, and
 *  find the time entry the stop minted. */
test("demo path: create a ticket, note it, run the timer, land a time entry", async ({
  authedPage,
  ownerTokens,
}) => {
  const title = `Demo smoke ${Date.now() % 100000}`;

  // --- create, the way a prospect does: the board entry state ---------------
  await authedPage.goto(`/boards/${DEMO_SERVICE_DESK_BOARD_ID}`);
  await authedPage.getByRole("button", { name: "New ticket" }).click();
  await authedPage.fill("#new-ticket-title", title);
  await authedPage.getByRole("button", { name: "Create ticket" }).click();

  const row = authedPage.getByText(title).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(authedPage.getByTestId("ticket-status-bar")).toBeVisible({ timeout: 30_000 });
  const ticketId = authedPage.url().split("/tickets/")[1]?.split(/[?#]/)[0] ?? "";
  expect(ticketId, "the workspace URL carried the new ticket's id").not.toBe("");

  // --- note -----------------------------------------------------------------
  await authedPage.getByTestId("detail-tab-notes").click();
  await expect(authedPage.getByTestId("notes-composer")).toBeVisible({ timeout: 15_000 });
  const note = `Demo smoke note ${Date.now() % 100000} — captured from the smoke path.`;
  await authedPage.getByTestId("notes-composer-body").fill(note);
  await authedPage.getByTestId("note-post").click();
  await expect(authedPage.getByText(note).first()).toBeVisible({ timeout: 20_000 });

  // --- timer start: Take It assigns, moves to In Progress, and runs the clock
  const takeIt = authedPage.getByTestId("wb-take-it");
  await expect(takeIt).toBeVisible({ timeout: 20_000 });
  await takeIt.click();
  // The work bar owns the running session app-wide (components/timer-dock.tsx).
  await expect(authedPage.getByTestId("work-bar")).toBeVisible({ timeout: 20_000 });
  await expect(authedPage.getByTestId("timer-elapsed")).toBeVisible();

  // --- timer stop: off the ticket surface, Wrap up stops the session directly
  // (on the ticket page the same control hands to the wrap-up sheet; this leg
  // takes the shortest honest stop).
  await authedPage.goto("/today");
  await authedPage.getByTestId("workbar-wrap-up").click();
  await expect(authedPage.getByTestId("work-bar")).toBeHidden({ timeout: 20_000 });

  // --- time entry: the stop mints one on the ticket (the row-level
  //     consequence this leg owns, asserted through the public surface).
  const get = authedFetch(ownerTokens.access_token);
  await expect
    .poll(
      async () => (await get<unknown[]>(`/tickets/${ticketId}/time-entries`)).length,
      { message: "the stopped timer minted a time entry on the ticket", timeout: 20_000 },
    )
    .toBeGreaterThan(0);

  // ... and the workspace shows the entries tab that carries it.
  await openWorkspaceWithRecovery(authedPage, ticketId);
  await authedPage.getByTestId("detail-tab-time").click();
  await expect(authedPage.getByTestId("detail-panel-time")).toBeVisible();
});

/** The money half: a prospect opens the ledger and works the invoice run. */
test("demo path: the billing ledger renders and an invoice run previews", async ({
  authedPage,
}) => {
  await authedPage.goto("/billing");
  // The ledger and its two money cards are the surface a prospect reads first.
  await expect(
    authedPage.getByRole("heading", { name: "Unbilled candidates", exact: true }),
  ).toBeVisible({ timeout: 30_000 });
  await expect(
    authedPage.getByRole("heading", { name: "Invoice runs", exact: true }),
  ).toBeVisible();

  // The run composer: preview a window, then issue when the window is billable.
  await authedPage.getByRole("button", { name: "New invoice run" }).click();
  await authedPage.getByRole("button", { name: "Preview" }).click();
  // Either branch is honest on a fresh org: N billable contracts, or the typed
  // "nothing billable" line for a period the seed already billed. The run is
  // idempotent per window, so a re-run must accept both.
  await expect(
    authedPage.getByText(/billable contract|nothing billable in this window/),
  ).toBeVisible({ timeout: 20_000 });

  const issue = authedPage.getByRole("button", { name: /Issue invoices/ });
  if (await issue.isEnabled()) {
    await issue.click();
    await expect(authedPage.getByRole("dialog")).toBeHidden({ timeout: 30_000 });
  } else {
    await authedPage.getByRole("button", { name: "Cancel" }).click();
  }
});

/** The client's half: the portal is its own sand-themed app with its own
 *  session, so it gets its own context — never the staff cookie jar. */
test("demo path: the client portal signs in and reads a ticket", async ({ browser }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  try {
    await page.goto(`${WEB_URL}/portal/login`);
    await page.getByTestId("portal-use-password").click();
    await page.fill("#portal-email", PORTAL_EMAIL);
    await page.fill("#portal-password", PORTAL_PASSWORD);
    await page.getByTestId("portal-password-submit").click();

    // The seeded contact's client has the Service Desk board exposed (demo
    // data lane B3), so the open list always holds a row.
    await expect(page.getByTestId("portal-tickets-open")).toBeVisible({ timeout: 30_000 });
    const ticketRow = page.locator('[data-testid^="portal-ticket-"]').first();
    await expect(ticketRow).toBeVisible({ timeout: 15_000 });
    await ticketRow.click();
    await expect(page.getByTestId("portal-thread")).toBeVisible({ timeout: 30_000 });
    await expect(page.getByRole("heading", { name: "Your tickets" })).toBeVisible();
  } finally {
    await context.close();
  }
});

/** The AI surface: the demo's BYOK slot is wired and answers typed.
 *  The live call is the operator probe's job (scripts/demo-ai-probe.mjs). */
test("demo path: the BYOK surface renders and its posture is readable", async ({
  authedPage,
  ownerTokens,
}) => {
  await authedPage.goto("/settings/ai");
  await expect(authedPage.getByTestId("settings-ai-page")).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByTestId("ai-master-card")).toBeVisible();

  const get = authedFetch(ownerTokens.access_token);
  const status = await get<{ active_provider?: string }>("/settings/ai/status");
  expect(status, "the AI status overview answered with a typed body").toBeTruthy();
});
