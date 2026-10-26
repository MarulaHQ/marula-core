// Flow — Escape closes the two overlays that once ignored it (p47 review).
//
// The ticket workbench's overlay drawer (narrow + mid widths) and the
// automation canvas's step-library dialog both shipped without an Escape
// path: keyboard users could dismiss neither. The drawer got a window-level
// Escape layer; the library got one on the dialog itself plus `aria-modal`
// and focus restore. This spec is the regression for both, and it also
// pins the focus restore so the dialog cannot eat focus on close.
import { expect, test } from "../helpers/harness";
import { authedFetch, DEMO_SERVICE_DESK_BOARD_ID } from "../helpers/api";
import { openWorkspaceWithRecovery } from "../helpers/workspace";

test("the ticket overlay drawer closes on Escape at narrow width", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const tickets = await get<any[]>(`/tickets?limit=50&board_id=${DEMO_SERVICE_DESK_BOARD_ID}`);
  const ticket = tickets.find((t) => t.category !== "closed");
  expect(ticket).toBeTruthy();

  // Narrow layout: the context panel is an overlay drawer, not persistent.
  await authedPage.setViewportSize({ width: 900, height: 800 });
  await openWorkspaceWithRecovery(authedPage, ticket.id);
  await authedPage.getByTestId("top-bar").waitFor({ state: "visible", timeout: 30_000 });

  await authedPage.getByTestId("wb-open-context").click();
  // The scrim is the drawer's backdrop; the drawer is open when it covers
  // the viewport.
  const scrim = authedPage.locator("div.fixed.inset-0.z-40");
  await expect(scrim).toBeVisible();
  await authedPage.keyboard.press("Escape");
  await expect(scrim).toHaveCount(0);
});

test("the automation step-library dialog closes on Escape and restores focus", async ({
  authedPage,
}) => {
  // The bare canvas opens the first saved workflow, or an empty document
  // when none exists — either way the editor mounts with its "Add step"
  // toggle, so this leg needs no seeded rule.
  await authedPage.goto("/automations/canvas");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });

  const addStep = authedPage.getByRole("button", { name: "Add step" }).first();
  await addStep.waitFor({ state: "visible", timeout: 30_000 });
  await addStep.click();

  const dialog = authedPage.getByRole("dialog", { name: "Add a step" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  // Autofocus moved into the dialog's search field on open.
  await expect(authedPage.getByLabel("Search steps")).toBeFocused();

  await authedPage.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  // Focus returns to the toggle that opened the dialog.
  await expect(addStep).toBeFocused();
});

test("the step-library dialog keeps Tab inside itself", async ({ authedPage }) => {
  await authedPage.goto("/automations/canvas");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  const addStep = authedPage.getByRole("button", { name: "Add step" }).first();
  await addStep.waitFor({ state: "visible", timeout: 30_000 });
  await addStep.click();

  const dialog = authedPage.getByRole("dialog", { name: "Add a step" });
  await expect(dialog).toBeVisible();
  // A full Tab lap cycles inside the dialog: focus that walks to the
  // toggle button underneath would land outside the role=dialog element.
  for (let i = 0; i < 12; i += 1) await authedPage.keyboard.press("Tab");
  await expect(
    authedPage.evaluate(() =>
      Boolean(document.activeElement?.closest('[role="dialog"]')),
    ),
  ).resolves.toBe(true);
});

test("the dispatch settings modal keeps Tab inside itself", async ({ authedPage }) => {
  await authedPage.goto("/dispatch");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByRole("button", { name: "Dispatch settings" }).click();

  const modal = authedPage.getByTestId("dispatch-settings");
  await expect(modal).toBeVisible();
  // Focus moved in on open, then a Tab lap stays inside the modal.
  await expect(
    authedPage.evaluate(() =>
      Boolean(document.activeElement?.closest('[data-testid="dispatch-settings"]')),
    ),
  ).resolves.toBe(true);
  for (let i = 0; i < 12; i += 1) await authedPage.keyboard.press("Tab");
  await expect(
    authedPage.evaluate(() =>
      Boolean(document.activeElement?.closest('[data-testid="dispatch-settings"]')),
    ),
  ).resolves.toBe(true);
  await authedPage.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
});
