// Flow — the MCP capability catalog on the agent-platform settings surface
// (row A29). The page renders the DERIVED catalog (the committed artifact):
// a family list with grant checkboxes, and a sensitive-reads section that
// starts disabled (documents, ticket correspondence). This spec pins the
// catalog rendering and one grant round-trip so the admin surface cannot
// regress into a free-text-only registry.
import { expect, test } from "../helpers/harness";

test("the catalog renders families and the sensitive reads start disabled", async ({
  authedPage,
}) => {
  await authedPage.goto("/settings/agent-platform");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });

  const families = authedPage.getByTestId("mcp-catalog-families");
  await expect(families).toBeVisible({ timeout: 30_000 });
  await expect(families).toContainText("tickets");
  await expect(families).toContainText("invoices");
  await expect(families).toContainText("tickets:read");

  // Sensitive reads are their own section and are never implied by a family
  // grant: documents and ticket correspondence appear here, unchecked.
  const sensitive = authedPage.getByTestId("mcp-sensitive-reads");
  await expect(sensitive).toContainText("documents");
  await expect(sensitive).toContainText("tickets_correspondence:read");
  await expect(
    authedPage.getByRole("checkbox", { name: "Grant documents reads (sensitive)" }),
  ).not.toBeChecked();
});

test("granting a family from the catalog writes the grant and revoking removes it", async ({
  authedPage,
}) => {
  await authedPage.goto("/settings/agent-platform");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByTestId("mcp-catalog-families")).toBeVisible({ timeout: 30_000 });

  const toggle = authedPage.getByRole("checkbox", { name: "Grant vendors reads" });
  const registry = authedPage.getByTestId("mcp-capability-registry");

  try {
    await toggle.check();
    await expect(toggle).toBeChecked({ timeout: 15_000 });
    await expect(
      registry.getByRole("button", { name: "Revoke" }).first(),
    ).toBeVisible({ timeout: 15_000 });
    await expect(registry).toContainText("vendors:read");
  } finally {
    // Leave the demo org as found even when an assertion above failed.
    if (await toggle.isChecked()) {
      await toggle.uncheck();
      await expect(toggle).not.toBeChecked({ timeout: 15_000 });
    }
  }
});
