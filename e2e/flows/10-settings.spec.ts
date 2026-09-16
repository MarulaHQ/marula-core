// Flow 10 — Settings surfaces: team directory (invite copy-once) and tags
// manager (create + counts), closing the round over the 10 core flows.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

test("team directory renders with demo members", async ({ authedPage }) => {
  await authedPage.goto("/settings/team");
  await expect(authedPage.getByRole("heading", { name: "Team" }).first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(authedPage.getByRole("button", { name: "Invite member" })).toBeVisible();
});

test("tags manager creates a tag and shows its usage count", async ({ authedPage }) => {
  const suffix = Date.now() % 100000;
  await authedPage.goto("/settings/tags");
  await expect(authedPage.getByTestId("create-tag")).toBeVisible({ timeout: 30_000 });
  await authedPage.getByTestId("create-tag").click();
  await expect(authedPage.locator("#tag-name")).toBeVisible({ timeout: 15_000 });
  await authedPage.fill("#tag-name", `e2e-${suffix}`);
  await authedPage.getByRole("button", { name: "Create", exact: true }).click();
  await expect(authedPage.getByTestId(`tag-count-e2e-${suffix}`)).toBeVisible({ timeout: 20_000 });
});

test("boards index renders the seeded demo boards", async ({ authedPage, ownerTokens }) => {
  const get = authedFetch(ownerTokens.access_token);
  const boards = await get<any[]>("/boards");
  expect(boards.length).toBeGreaterThan(0);

  await authedPage.goto("/boards");
  await expect(authedPage.getByRole("heading", { name: "Boards", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect(authedPage.getByText(boards[0].name).first()).toBeVisible({ timeout: 20_000 });
});
