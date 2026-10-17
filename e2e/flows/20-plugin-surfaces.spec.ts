// Flow 20 — §4.6 plugin surfaces (Wave 34 lane 4): the declarative
// chain over the REAL demo stack. Catalog chain via API (publish →
// install → enable) for the widget + validator plugins; the zip staging
// path drives the REAL consent UI for the automation plugin (the
// consent screen must show the surface declarations before an admin
// grants the capability). UI assertions cover the ticket-pane widget,
// the automations action picker, and the board field-editor validator
// picker. Captures land in the RUNNING LANE's evidence dir (`captures/` under
// design/qa-<branch>/, see e2e/helpers/evidence.ts) so a sweep cannot rewrite a
// neighbouring lane's committed record (Wave 47 lane A17); the lane that owns
// design/qa-p34-plugin-surface-closeout still lands there, because the branch
// name is the directory name.
import { expect, test } from "../helpers/harness";
import { authedFetch, apiLogin, API_URL } from "../helpers/api";
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { evidenceDir } from "../helpers/evidence";

// Playwright transpiles specs outside their dir; resolve repo paths from cwd.
const REPO = process.cwd();

const CAPTURES = evidenceDir("captures");

/** The reference automation plugin's bundle, built on demand.
 *
 *  A plugin's `dist` directory is gitignored (`.gitignore:107`), so no checkout
 *  and no CI job carries the zip: the spec used to `readFileSync` an artifact
 *  that nothing in the gate path produced. The repo owns the packer
 *  (`packages/plugin-kit/bin/build.mjs`), so build it here when it is absent,
 *  and hand back `null` when the build cannot produce it — the caller then
 *  skips with a typed reason naming the command rather than an ENOENT. */
function ensureTriageZip(): string | null {
  const pluginDir = join(REPO, "plugins/wasm/triage-note-action");
  const zipPath = join(pluginDir, "dist/triage-note-action-1.0.1.zip");
  if (existsSync(zipPath)) return zipPath;
  try {
    execFileSync("node", [join(REPO, "packages/plugin-kit/bin/build.mjs"), pluginDir], {
      cwd: REPO,
      stdio: "pipe",
    });
  } catch (error) {
    console.warn(`[20-plugin-surfaces] plugin-kit build failed: ${String(error)}`);
  }
  return existsSync(zipPath) ? zipPath : null;
}

async function api(token: string) {
  return authedFetch(token);
}

test("plugin widget renders on the classic ticket pane", async ({ authedPage, ownerTokens }) => {
  const f = await api(ownerTokens.access_token);
  const slug = `e2e-escalation-${Date.now() % 100000}`;
  // Publish → install (ui.widgets) → enable, all through the catalog chain.
  await f("/plugins/catalog", {
    method: "POST",
    body: {
      slug,
      name: "Escalation Policy Widget",
      description: "e2e widget plugin",
      publisher: "marula",
      version: "1.0.0",
      manifest: {
        description: "e2e widget plugin",
        capabilities: ["ui.widgets"],
        surface_profile: {
          widgets: [
            {
              id: "escalation_policy",
              title: "Escalation policy",
              slot: "ticket_pane",
              kind: "list",
              props_schema: {
                type: "object",
                properties: { "Tier 1": { type: "string", maxLength: 80 } },
              },
              default_props: { "Tier 1": "Board owner, within 2 hours" },
            },
          ],
        },
      },
    },
  });
  await f("/plugins/installed", {
    method: "POST",
    body: { slug, granted_capabilities: ["ui.widgets"] },
  });
  const installs = await f<any[]>("/plugins/installed");
  const install = installs.find((i: any) => i.slug === slug);
  expect(install).toBeTruthy();
  await f(`/plugins/installed/${install!.id}`, {
    method: "PATCH",
    body: { status: "enabled" },
  });

  const tickets = await f<any[]>("/tickets");
  expect(tickets.length).toBeGreaterThan(0);
  await authedPage.goto(`/tickets/classic/${tickets[0].id}`);
  // Dismiss the first-run help tour if it is up, then open the Details
  // tab — the ticket-pane plugin slot renders in the details column.
  const done = authedPage.getByRole("button", { name: "Done" });
  if (await done.isVisible({ timeout: 5_000 }).catch(() => false)) {
    await done.click();
  }
  await authedPage.getByText("Details", { exact: true }).first().click();
  const section = authedPage.getByTestId("plugin-widgets-ticket_pane");
  await expect(section).toBeVisible({ timeout: 30_000 });
  await expect(section.getByText("Escalation policy").first()).toBeVisible();
  await authedPage.screenshot({
    path: join(CAPTURES, "ticket-pane-plugin-widget.png"),
    fullPage: false,
  });
});

test("consent shows surface declarations; approving registers the action kind", async ({
  authedPage,
  ownerTokens,
}) => {
  const f = await api(ownerTokens.access_token);
  // Stage the reference automation plugin from its built zip. The bundle is a
  // gitignored artifact, so build it on demand; a build the repo's own packer
  // refuses is a typed skip, never an ENOENT that reads like drift.
  const zipPath = ensureTriageZip();
  if (zipPath === null) {
    test.skip(
      true,
      "plugin bundle not built: node packages/plugin-kit/bin/build.mjs plugins/wasm/triage-note-action (dist/ is gitignored)",
    );
    return;
  }
  const zip = readFileSync(zipPath);
  const stagedRes = await fetch(`${API_URL}/api/v1/plugins/distribution/staged/zip`, {
    method: "POST",
    headers: {
      "content-type": "application/zip",
      authorization: `Bearer ${ownerTokens.access_token}`,
    },
    body: new Uint8Array(zip),
  });
  if (stagedRes.status === 200 || stagedRes.status === 409) {
    // Digest-dedup: an earlier run staged the identical bundle — reuse it.
    const list = await f<any[]>("/plugins/distribution/staged");
    const found = list.find((i: any) => i.manifest?.name?.includes("Triage Note Action"));
    expect(found).toBeTruthy();
    var staged = found;
  } else {
    expect(stagedRes.status).toBe(201);
    var staged = (await stagedRes.json()) as any;
  }

  // Approve happens ONCE per digest — a re-run meets an already-approved
  // row and skips straight to the registered-kind assertion.
  const enableAndAssertPicker = async () => {
    const installs = await f<any[]>("/plugins/installed");
    const install = installs.find((i: any) => i.slug === "triage-note-action");
    expect(install).toBeTruthy();
    if (install!.status !== "enabled") {
      await f(`/plugins/installed/${install!.id}`, {
        method: "PATCH",
        body: { status: "enabled" },
      });
    }
    await authedPage.goto("/automations");
    await authedPage.getByRole("button", { name: /new rule|create rule|add rule/i }).first().click();
    const trigger = authedPage.getByLabel("Add action");
    await trigger.scrollIntoViewIfNeeded();
    await trigger.click();
    await expect(authedPage.getByRole("option", { name: "Triage note" })).toBeVisible({
      timeout: 15_000,
    });
    await authedPage.screenshot({
      path: join(CAPTURES, "automations-plugin-action-picker.png"),
      fullPage: false,
    });
  };
  if (staged.status !== "pending") {
    await enableAndAssertPicker();
    return;
  }

  // The REAL consent surface: review, capture, grant automation.actions, approve.
  await authedPage.goto("/settings/plugins");
  await authedPage.getByRole("button", { name: "Approvals" }).click();
  const row = authedPage.getByTestId(`staged-row-${staged.id}`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.getByRole("button", { name: "Review consent" }).click();
  const surfaces = authedPage.getByText("Triage note");
  await expect(surfaces.first()).toBeVisible({ timeout: 30_000 });
  // The pane's heading is "Declared surfaces" and it lists one row per surface
  // kind the plugin contributes (apps/web/app/settings/plugins/consent.tsx:257-306).
  // This plugin declares an automation action and no widget, so the row an
  // approver reads is the action one.
  await expect(authedPage.getByText("Declared surfaces")).toBeVisible();
  await expect(authedPage.getByText("Automation actions").first()).toBeVisible();
  await authedPage.screenshot({
    path: join(CAPTURES, "consent-surface-declarations.png"),
    fullPage: true,
  });
  const grantBox = authedPage.locator('input[aria-label="Grant automation.actions"]');
  await grantBox.waitFor({ state: "visible", timeout: 15_000 });
  await grantBox.check();
  await authedPage.getByRole("button", { name: "Approve — installs disabled" }).click();
  // The decision races nothing: either the row flips off "pending" here,
  // or a prior run already minted this exact version (the approve then
  // 409s `plugin_version_taken` and the enable below still proceeds).
  await expect
    .poll(
      async () =>
        (await f<any[]>("/plugins/installed")).some((i) => i.slug === "triage-note-action"),
      { timeout: 30_000, intervals: [1_000, 2_000, 5_000] },
    )
    .toBe(true);

  // Approve lands DISABLED — enable, then the automations editor lists the kind.
  await enableAndAssertPicker();
});

test("field editor lists plugin validators", async ({ authedPage, ownerTokens }) => {
  const f = await api(ownerTokens.access_token);
  const slug = `e2e-validator-${Date.now() % 100000}`;
  await f("/plugins/catalog", {
    method: "POST",
    body: {
      slug,
      name: "Asset Tag Validator",
      description: "e2e validator plugin",
      publisher: "marula",
      version: "1.0.0",
      manifest: {
        description: "e2e validator plugin",
        capabilities: ["fields.validate"],
        surface_profile: {
          field_validators: [
            {
              id: "asset_tag",
              label: "Asset tag",
              schema: {
                type: "object",
                properties: { value: { type: "string", minLength: 6, maxLength: 32 } },
              },
            },
          ],
        },
      },
    },
  });
  await f("/plugins/installed", {
    method: "POST",
    body: { slug, granted_capabilities: ["fields.validate"] },
  });
  const installs = await f<any[]>("/plugins/installed");
  const install = installs.find((i: any) => i.slug === slug);
  await f(`/plugins/installed/${install!.id}`, {
    method: "PATCH",
    body: { status: "enabled" },
  });

  const boards = await f<any[]>("/boards");
  expect(boards.length).toBeGreaterThan(0);
  await authedPage.goto(`/boards/${boards[0].id}/fields`);
  await authedPage.getByRole("button", { name: "Add field" }).first().click();
  const picker = authedPage.getByTestId("field-validator");
  await expect(picker).toBeVisible({ timeout: 30_000 });
  await picker.selectOption({ label: "Asset tag" });
  await expect(picker).toHaveValue(/plugin\.[a-z0-9-]+\.asset_tag/);
  await authedPage.screenshot({
    path: join(CAPTURES, "field-editor-plugin-validator.png"),
    fullPage: false,
  });
});
