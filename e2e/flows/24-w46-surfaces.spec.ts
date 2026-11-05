// Flow 24 — Wave 46 surfaces: dashboard card sizing, and the report authoring
// draft row. Both are new in the same group, and neither had a browser pass.
//
// What this proves, and what it does not. A card's span and height are controls
// read from the server's catalogue, and the choice survives a reload through the
// per-user settings row — that is the whole persistence loop, not a DOM
// assertion. The authoring page is text only, and a generate call reaches the
// real endpoint: this stack configures no provider, so the honest outcome is a
// typed "not available" answer rather than a draft, and the row must say which.
import { expect, test } from "../helpers/harness";

test("dashboards: a card's size is a control and it survives a reload", async ({ authedPage }) => {
  await authedPage.goto("/dashboards");

  const grid = authedPage.getByTestId("dashboard-grid");
  await expect(grid).toBeVisible({ timeout: 30_000 });

  // The read-only notice is the one state where there is nothing to size.
  await expect(authedPage.getByTestId("dashboard-readonly")).toHaveCount(0);

  const card = grid.locator("[data-widget-id]").first();
  await expect(card).toBeVisible();

  const title = await card.getByRole("heading").first().innerText();
  const before = await card.getAttribute("data-card-height");

  await card.getByRole("button", { name: `Size and position for ${title}` }).click();
  const panel = card.getByRole("radiogroup", { name: `${title} card height` });
  await expect(panel).toBeVisible();

  // "Tall" is the tallest height the catalogue publishes today. Choosing it by
  // its visible label keeps this independent of the slug behind it.
  const tall = panel.getByRole("radio", { name: "Tall" });
  await expect(tall).toBeVisible();

  // The write is asynchronous: wait for the settings PUT rather than racing the
  // reload against it, and fail loudly if the server refused the arrangement.
  const put = authedPage.waitForResponse(
    (res) =>
      res.url().includes("/api/v1/settings/user") && res.request().method() === "PUT",
  );
  await tall.click();
  expect((await put).status()).toBeLessThan(400);

  await expect(card).toHaveAttribute("data-card-height", "tall");
  expect(before).not.toBe("tall");
  await expect(authedPage.getByTestId("dashboard-save-error")).toHaveCount(0);

  // The stored arrangement, not the in-memory one: a reload re-reads settings.
  await authedPage.reload();
  const reloaded = authedPage.getByTestId("dashboard-grid").locator("[data-widget-id]").first();
  await expect(reloaded).toHaveAttribute("data-card-height", "tall", { timeout: 30_000 });

  // Put it back so the run leaves the demo org as it found it.
  await reloaded.getByRole("button", { name: `Size and position for ${title}` }).click();
  await reloaded
    .getByRole("radiogroup", { name: `${title} card height` })
    .getByRole("radio", { name: "Regular" })
    .click();
  await expect(reloaded).toHaveAttribute("data-card-height", "regular");
});

test("report authoring: text only, and a refusal is stated rather than faked", async ({
  authedPage,
}) => {
  await authedPage.goto("/reports/author");

  const box = authedPage.getByRole("textbox").first();
  await expect(box).toBeVisible({ timeout: 30_000 });

  // No capture affordance, by design: voice-note capture is its own lane.
  await expect(authedPage.locator("audio")).toHaveCount(0);
  await expect(authedPage.locator("input[type=file]")).toHaveCount(0);

  const generate = authedPage.getByRole("button", { name: /Generate/i }).first();
  await expect(generate).toBeDisabled();

  await box.fill("Tickets per client this quarter, as a bar chart, refreshed daily.");
  await expect(generate).toBeEnabled();
  await generate.click();

  // One of two honest endings: a draft row with its status, or a typed refusal.
  // This stack holds no provider credential, so the refusal is the expected one.
  const row = authedPage.getByTestId("report-draft-row");
  // The row appears while the call is in flight ("drafting"); wait for a
  // terminal status before reading it, or the assertion races the answer.
  await expect(row).toHaveAttribute(
    "data-status",
    /^(refused|ready|needs_fixes)$/,
    { timeout: 60_000 },
  );

  const status = await row.getAttribute("data-status");
  if (status === "refused") {
    // The admission path answered: the row says why, and there is no draft.
    const refusal = row.getByRole("alert");
    await expect(refusal).toBeVisible();
    await expect(refusal).not.toBeEmpty();
  } else {
    expect(status).toMatch(/^(ready|needs_fixes)$/);
    await expect(row.getByRole("heading").first()).toBeVisible();
  }
});

test("a described schedule becomes fields, and a cadence the route cannot express is refused", async ({
  authedPage,
}) => {
  await authedPage.goto("/reports");
  await authedPage.getByRole("tab", { name: /Schedules/ }).click();

  const phrase = authedPage.getByLabel("In your own words");
  await expect(phrase).toBeVisible({ timeout: 30_000 });

  // A phrase the wire shape cannot carry is refused with a reason, not bent
  // into the nearest cadence: weekly is one weekday, never "every other".
  await phrase.fill("every other Tuesday at 09:00");
  await authedPage.getByRole("button", { name: "Read it" }).click();
  await expect(authedPage.getByText(/never every other week/i)).toBeVisible();

  // A phrase it can read becomes an editable sentence.
  await phrase.fill("every Monday at 09:00");
  await authedPage.getByRole("button", { name: "Read it" }).click();
  await expect(authedPage.getByText("Every Monday at 09:00").first()).toBeVisible();

  // Taking the proposal fills the existing form — the one route that saves.
  await authedPage.getByRole("button", { name: "Use this schedule" }).click();
  // The dialog is the writer, and the proposal also has a Weekday field: scope
  // the assertion to the dialog or it matches both.
  const dialog = authedPage.getByRole("dialog");
  await expect(dialog.getByLabel("Weekday")).toBeVisible({ timeout: 15_000 });
  await expect(dialog.getByLabel("Time of day")).toHaveValue("09:00");
});
