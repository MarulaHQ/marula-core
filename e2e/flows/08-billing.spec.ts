// Flow 8 — Billing: unbilled candidates card, invoice list, credit notes.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

test("billing page renders unbilled candidates + invoices", async ({ authedPage }) => {
  await authedPage.goto("/billing");
  // The heading and the empty-state copy share a stem, so name the role.
  await expect(
    authedPage.getByRole("heading", { name: "Unbilled candidates", exact: true }),
  ).toBeVisible({ timeout: 30_000 });
});

test("credit notes page renders and issues a credit note against demo data", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const invoices = await get<any[]>("/invoices");
  const client = (await get<any[]>("/clients"))[0];
  test.skip(!client, "demo seed has no clients");

  await authedPage.goto("/billing/credit-notes");
  await expect(authedPage.getByTestId("issue-credit-note")).toBeVisible({ timeout: 30_000 });

  if (invoices.some((i) => i.status === "issued")) {
    await authedPage.getByTestId("issue-credit-note").click();
    await expect(authedPage.locator("#cn-total")).toBeVisible({ timeout: 15_000 });
    await authedPage.getByRole("dialog").getByRole("combobox").first().click();
    await authedPage.getByRole("option", { name: new RegExp(client.name.slice(0, 12)) }).first().click();
    await authedPage.fill("#cn-total", "5.00");
    await authedPage.fill("#cn-reason", "E2E verification credit");
    await authedPage.getByRole("button", { name: "Issue", exact: true }).click();
    await expect(authedPage.getByText(/CRN-/).first()).toBeVisible({ timeout: 20_000 });
  }
});
