// Flow 15 — Purchasing close-out (Wave 32 lane 4): vendor + order raised in
// the browser, send, receiving with the over-receipt acknowledgement, an
// order portal deeplink, and the vendor settings editor. Scratch data is
// staged through the API per test; the flows never depend on each other.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";
import type { Page } from "@playwright/test";

async function pickSelect(page: Page, testid: string, option: string): Promise<void> {
  await page.getByTestId(testid).click();
  await page.getByRole("option", { name: option }).click();
}

interface Vendor {
  id: string;
  name: string;
}

interface StockItem {
  id: string;
  name: string;
  unit: string;
}

interface Location {
  id: string;
  name: string;
}

test("raise an order in the browser, send it, receive with an over-receipt acknowledgement", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 1000000;

  const vendor = await post<Vendor>("/vendors", { name: `E2E Vendor ${run}`, default_currency: "USD" });
  const item = await post<StockItem>("/stock-items", {
    name: `E2E Cable ${run}`,
    kind: "consumable",
    unit: "each",
    currency: "USD",
    is_tool: false,
  });
  const location = await post<Location>("/stock-locations", {
    name: `E2E Dock ${run}`,
    kind: "warehouse",
  });

  // The order is raised through the browser dialog, one line in.
  await authedPage.goto("/purchasing");
  await authedPage.getByTestId("purchasing-new").click();
  await pickSelect(authedPage, "purchasing-order-vendor", vendor.name);
  await authedPage.getByTestId("purchasing-line-0-description").fill("Cat6 patch panel");
  await authedPage.getByTestId("purchasing-line-0-qty").fill("2");
  await authedPage.getByTestId("purchasing-line-0-cost").fill("42.50");
  // Receiving posts ledger rows, so the line must be stock-linked.
  await pickSelect(authedPage, "purchasing-line-0-item", item.name);
  await authedPage.getByTestId("purchasing-order-create").click();

  await expect(authedPage).toHaveURL(/\/purchasing\/[0-9a-f-]{36}$/);
  await expect(authedPage.getByTestId("purchasing-order-number")).toBeVisible();

  // Send flips the status; the notice confirms the write landed before we
  // navigate away (an early goto cancels the in-flight mutation).
  await authedPage.getByTestId("purchasing-send").click();
  await expect(authedPage.getByTestId("purchasing-action-notice")).toBeVisible({ timeout: 20_000 });
  await authedPage.goto("/purchasing");
  const orderRow = authedPage
    .locator('[data-testid^="purchasing-row-"]')
    .filter({ hasText: vendor.name });
  await expect(orderRow).toContainText("ordered", { timeout: 20_000 });
  await orderRow.locator('[data-testid^="purchasing-open-"]').click();
  await expect(authedPage.getByTestId("purchasing-order-number")).toBeVisible();

  // Receiving: five in against two ordered is refused until acknowledged.
  await authedPage.getByTestId("purchasing-receive").click();
  await pickSelect(authedPage, "purchasing-receive-location", location.name);
  await authedPage.locator('[data-testid^="purchasing-receive-qty-"]').first().fill("5");
  await authedPage.getByTestId("purchasing-receive-carrier").fill("UPS");
  await authedPage.getByTestId("purchasing-receive-tracking").fill(`1Z-${run}`);
  await authedPage.getByTestId("purchasing-receive-submit").click();
  await expect(authedPage.getByTestId("purchasing-receive-error")).toBeVisible({ timeout: 20_000 });

  await authedPage.getByTestId("purchasing-over-receipt").click();
  await authedPage.getByTestId("purchasing-receive-submit").click();
  await expect(authedPage.getByTestId("purchasing-receipts")).toContainText("5 each", { timeout: 20_000 });

  // A portal link becomes a one-click deeplink button. With no links yet
  // the form is already open; the toggle only exists once links exist.
  const linkToggle = authedPage.getByTestId("purchasing-link-form-toggle");
  if (await linkToggle.isVisible()) await linkToggle.click();
  await pickSelect(authedPage, "purchasing-link-kind", "Order portal");
  await authedPage.getByTestId("purchasing-link-url").fill(`https://portal.example.test/orders/${run}`);
  await authedPage.getByTestId("purchasing-link-label").fill("Vendor portal");
  await authedPage.getByTestId("purchasing-link-add").click();
  const deeplink = authedPage.locator('[data-testid^="purchasing-deeplink-"]').first();
  await expect(deeplink).toBeVisible({ timeout: 20_000 });
  await expect(deeplink).toHaveAttribute("href", `https://portal.example.test/orders/${run}`);
});

test("vendor settings: create, replace contacts, add a portal link", async ({ authedPage, ownerTokens }) => {
  const run = Date.now() % 1000000;
  const vendorName = `E2E Vendor Co ${run}`;

  await authedPage.goto("/purchasing/vendors");
  await authedPage.getByTestId("purchasing-vendors-new").click();
  await authedPage.getByTestId("purchasing-vendor-name").fill(vendorName);
  await authedPage.getByTestId("purchasing-vendor-currency").fill("EUR");
  await authedPage.getByTestId("purchasing-vendor-save").click();

  const row = authedPage
    .locator('[data-testid^="purchasing-vendor-row-"]')
    .filter({ hasText: vendorName });
  await expect(row).toBeVisible({ timeout: 20_000 });

  // The panel auto-expands after create: replace the contact list in one write.
  await expect(authedPage.locator('div[data-testid^="purchasing-vendor-panel-"]')).toBeVisible({ timeout: 20_000 });
  await authedPage.getByTestId("purchasing-contact-add").click();
  await authedPage.getByTestId("purchasing-contact-0-name").fill("Dana Ops");
  await authedPage.getByTestId("purchasing-contact-0-email").fill("dana@vendor.example");
  await authedPage.getByTestId("purchasing-contacts-save").click();
  await expect(authedPage.getByTestId("purchasing-vendors-notice")).toBeVisible({ timeout: 20_000 });

  // The portal link lands in the vendor's link list.
  await authedPage.getByTestId("purchasing-vendor-link-url").fill(`https://portal.example.test/v/${run}`);
  await authedPage.getByTestId("purchasing-vendor-link-add").click();
  await expect(authedPage.locator('[data-testid^="purchasing-vendor-link-"]').first()).toBeVisible({
    timeout: 20_000,
  });
});
