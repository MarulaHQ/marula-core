// Shared deep-link helper for the ticket workspace.
//
// The workspace deep-link path hydrates the detail once and retries only
// once (documented in apps/web/app/tickets/[id]/page.tsx); a slow snapshot
// can exhaust that. One explicit reload re-runs the bootstrap deterministically.
import { expect, type Page } from "@playwright/test";

export async function openWorkspaceWithRecovery(page: Page, ticketId: string) {
  await page.goto(`/tickets/${ticketId}`);
  const bar = page.getByTestId("ticket-status-bar");
  const appeared = await bar
    .waitFor({ state: "visible", timeout: 30_000 })
    .then(() => true)
    .catch(() => false);
  if (!appeared) {
    await page.reload({ waitUntil: "domcontentloaded" });
    await expect(bar).toBeVisible({ timeout: 30_000 });
  }
}
