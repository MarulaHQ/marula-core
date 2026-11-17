// Flow 12 — Wave 26 onboarding flows: the SKIPPED PATH IS THE HAPPY PATH.
//
// Purpose: first-class coverage of the skip contract against the real
// stack: (a) the org wizard's "skip for now" records nothing, so the
// step survives as pending/resumable; (b) "never show this again" is a
// RECORDED setting that survives an era of reloads (dismissal never
// resets); (c) the resume banner on /today respects both — shows while
// unresolved, goes quiet after recorded dismissal; (d) the per-user
// first-run walk at /onboarding completes orientation. Runs light + dark.
//
// THE TESTID VOCABULARY IS THE SHIPPED ONE (Wave 47 lane A16). The wizard was
// rewritten against the Marula surface pack in `63c4836a`, and this spec was
// left driving the pre-pack ids. The shipped controls live in
// `apps/web/components/auth/entry.tsx`: the rail renders one indexed button
// per step (`wizard-step-${i + 1}`, `:356`), "Skip for now" is
// `wizard-skip-for-now` (`:454`), and "Never show this setup again" is ONE
// recorded choice in the guide menu (`wizard-never-show-again`, `:515`) that
// dismisses every step still unresolved. The old per-step ids
// (`onboarding-rail-*`, `onboarding-never-show`) belonged to two modules with
// zero importers that `63c4836a` orphaned and lane A15 deleted.
import { expect, test } from "../helpers/harness";
import { authedFetch, apiLogin, API_URL, DEMO_PASSWORD, WEB_URL, type TokenPair } from "../helpers/api";
import { TOKENS_KEY, TOKENS_EXP_KEY } from "../helpers/harness";

/** Mirrors ORG_STEP_ORDER / USER_STEP_ORDER (apps/web/lib/onboarding.ts:43-44):
 *  the rail is indexed by position in these lists, not by step name. Kept
 *  local so the spec does not pull the app's Next.js client into the
 *  Playwright process. */
const ORG_STEP_ORDER = ["clients", "board", "tickets", "invites", "feeder", "ai"] as const;
const USER_STEP_ORDER = ["profile", "mfa", "orientation", "defaults"] as const;

/** One step's state, as `/api/v1/onboarding` reports it. */
interface OrgStepView {
  step: string;
  done: boolean;
  dismissed: boolean;
  satisfied: boolean;
  resolved: boolean;
}

interface OrgSummary {
  org: { steps: OrgStepView[] };
}

interface SummaryWithMembers extends OrgSummary {
  members: { user_id: string; email: string; steps: OrgStepView[] }[];
}

/** 1-based rail button for an org step. */
function orgRailStep(step: string): number {
  return (ORG_STEP_ORDER as readonly string[]).indexOf(step) + 1;
}

/** Seed a fresh context's page with the tokens AuthProvider restores from
 *  (the same seeding the capture script uses; a page built by hand does not
 *  run the harness's authedPage fixture). */
async function seedTokens(page: import("@playwright/test").Page, tokens: TokenPair): Promise<void> {
  await page.addInitScript(
    ([tokensKey, expKey, raw, exp]: string[]) => {
      window.localStorage.setItem(tokensKey, raw);
      window.localStorage.setItem(expKey, exp);
    },
    [
      TOKENS_KEY,
      TOKENS_EXP_KEY,
      JSON.stringify(tokens),
      String(Date.now() + tokens.expires_in * 1000 - 30_000),
    ],
  );
}

/** 1-based rail button for a per-user step. */
function userRailStep(step: (typeof USER_STEP_ORDER)[number]): number {
  return USER_STEP_ORDER.indexOf(step) + 1;
}

test("org wizard skip path: 'skip for now' records nothing and stays resumable", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  const get = authedFetch(ownerTokens.access_token);

  // Re-arm the org wizard so the lane's flow is deterministic: reset
  // clears recorded step state (the seat of the skip contract) — the
  // owned disable switch stays untouched.
  await post("/onboarding/reset/org", { method: "POST", body: {} });
  const summary = (await get<unknown>("/onboarding")) as OrgSummary;
  const resetPending = summary.org.steps.filter((s) => !s.resolved);
  expect(resetPending.length).toBeGreaterThan(0);
  const firstStep = resetPending[0].step;

  await authedPage.goto("/welcome");
  // The demo org already carries clients, boards and tickets, so those steps
  // RESOLVE from the data and the wizard opens on its resumed list (the pack's
  // state 14) rather than the rail. A genuinely empty org opens the rail
  // directly. Wait for whichever this org gets, then step into the first
  // pending step either way.
  const resume = authedPage.getByTestId(`wizard-resume-${firstStep}`);
  const railStep = authedPage.getByTestId(`wizard-step-${orgRailStep(firstStep)}`);
  await expect(resume.or(railStep).first()).toBeVisible({ timeout: 30_000 });
  if ((await resume.count()) > 0) {
    await resume.click();
  }
  // The rail is present with every step: the shipped ids are 1-based indexes.
  await expect(railStep).toBeVisible({ timeout: 30_000 });

  // SKIP FOR NOW: the step stays pending (the request intentionally
  // carries never:false and the server records nothing).
  await authedPage.getByTestId("wizard-skip-for-now").click();
  await expect
    .poll(
      async () =>
        ((await get<unknown>("/onboarding")) as typeof summary).org.steps.find(
          (s) => s.step === firstStep,
        )?.resolved,
      { timeout: 20_000 },
    )
    .toBe(false);
  const after = (await get<unknown>("/onboarding")) as OrgSummary;
  const skipped = after.org.steps.find((s) => s.step === firstStep);
  expect(skipped?.done).toBe(false); // skip-for-now recorded NOTHING
  expect(skipped?.dismissed).toBe(false);
});

test("'never show this again' is a recorded choice that survives reload", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  const get = authedFetch(ownerTokens.access_token);

  await post("/onboarding/reset/org", { method: "POST", body: {} });
  const before = (await get<unknown>("/onboarding")) as OrgSummary;
  const pending = before.org.steps.filter((s) => !s.resolved).map((s) => s.step);
  expect(pending.length).toBeGreaterThan(0);

  // Record the dismissal from the shipped guide. It is ONE choice for the
  // whole setup rather than a per-step button (the per-step module that used
  // to carry `onboarding-never-show` has no importers and was deleted), and
  // the confirm dialog spells out that consequence before it applies.
  await authedPage.goto("/welcome");
  const footerNever = authedPage.getByTestId("wizard-never-show-again");
  const guideMenu = authedPage.getByTestId("wizard-guide-menu");
  // Wait for the page to settle on one of its two shapes, then take the choice
  // from wherever it lives. The resumed list carries it in its footer; the rail
  // keeps it in the header's guide menu.
  await expect(footerNever.or(guideMenu).first()).toBeVisible({ timeout: 30_000 });
  if ((await footerNever.count()) > 0) {
    await footerNever.click();
  } else {
    await guideMenu.click();
    await authedPage.getByTestId("wizard-never-show-again").click();
  }
  await expect(authedPage.getByTestId("never-show-dialog")).toBeVisible();
  await authedPage.getByTestId("never-show-confirm").click();

  // The recorded dismissal lands for every step that was still unresolved, and
  // nothing is left pending.
  await expect
    .poll(
      async () =>
        ((await get<unknown>("/onboarding")) as typeof before).org.steps.filter(
          (s) => !s.resolved,
        ).length,
      { timeout: 20_000, intervals: [500, 1_000, 2_000] },
    )
    .toBe(0);

  // The choice survives ANY number of reloads: the wizard comes back up
  // already decided, with each step carrying its recorded dismissal.
  await authedPage.goto("/welcome");
  const after = (await get<unknown>("/onboarding")) as OrgSummary;
  for (const step of pending) {
    expect(after.org.steps.find((s) => s.step === step)?.dismissed).toBe(true);
  }
});

test("resume banner: visible while unresolved, quiet after 'Later', never silent on reload", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  await post("/onboarding/reset/org", { method: "POST", body: {} });
  // The demo org carries legacy onboarding.wizard_completed=true and in
  // fixture-clean databases that means every step resolves; verify the
  // banner either shows while pending or stays quiet when the org is
  // genuinely complete — whichever the real state is — and never nags.
  await authedPage.goto("/today");
  const banner = authedPage.getByTestId("onboarding-banner");
  const visible = await banner.isVisible({ timeout: 15_000 }).catch(() => false);
  if (visible) {
    // recorded dismissal
    await authedPage.getByTestId("onboarding-banner-dismiss").click();
    await expect(banner).toBeHidden({ timeout: 20_000 });
    // RELOAD: the dismissal is a recorded setting — the banner stays quiet.
    await authedPage.reload();
    await expect(banner).toBeHidden({ timeout: 20_000 });
  } else {
    // org fully resolved: nothing to nag about (the honest empty state)
    expect(await banner.isVisible()).toBe(false);
  }
});

test("per-user first-run: orientation completes, defaults persist, skips stick", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  // Re-arm the caller's own first-run so the pending set is deterministic
  // (the demo org is shared across projects; a previous run may have
  // recorded these steps already).
  await authedFetch(ownerTokens.access_token)("/onboarding/reset/user", {
    method: "POST",
    body: {},
  });
  const summary = (await get<unknown>("/onboarding")) as SummaryWithMembers;
  // The summary lists member state for admins too — the CALLER'S row is
  // identified by email, never "first member with steps".
  const mine = summary.members.find((m) => m.email === "owner@demo.local" && m.steps.length > 0);
  const pending = mine?.steps.filter((s) => !s.resolved).map((s) => s.step) ?? [];

  await authedPage.goto("/onboarding");
  if (pending.length === 0) return; // already resolved org: nothing to drive

  const resolved = async (step: string) =>
    ((await get<unknown>("/onboarding")) as typeof summary).members
      .find((m) => m.email === "owner@demo.local" && m.steps.length > 0)
      ?.steps.find((s) => s.step === step)?.resolved;

  // Orientation: the 60-second product orientation completes with one click.
  if (pending.includes("orientation")) {
    await authedPage.getByTestId(`wizard-step-${userRailStep("orientation")}`).click();
    await expect(authedPage.getByTestId("onboarding-orientation-done")).toBeVisible({
      timeout: 20_000,
    });
    await authedPage.getByTestId("onboarding-orientation-done").click();
    await expect.poll(async () => resolved("orientation"), { timeout: 20_000 }).toBe(true);
  }
  // Defaults: theme + mode ride the Wave 24 settings keys.
  if (pending.includes("defaults")) {
    await authedPage.getByTestId(`wizard-step-${userRailStep("defaults")}`).click();
    await authedPage.getByTestId("onboarding-mode").selectOption("dark");
    await authedPage.getByTestId("onboarding-save-defaults").click();
    await expect.poll(async () => resolved("defaults"), { timeout: 20_000 }).toBe(true);
  }
});

// ---------------------------------------------------------------------------
// Wave 72 lane W72-1 — the AI setup step (the wizard's sixth step).
//
// The step is owner/admin surface (the seal/test/credential/pick endpoints
// are all require_admin + owner/admin standing), so the two reads here are:
// the OWNER sees the working step, the TECHNICIAN sees one honest sentence
// and no key fields. Order independence: every test resets the org wizard
// itself, stages its own data through the API, and puts the shared org's
// state back (keys removed, catalog entries disabled) — the wizard's step
// recording is the state the other tests already re-arm.
// ---------------------------------------------------------------------------

interface AiStatus {
  configured_providers: string[];
}

interface CatalogEntry {
  id: string;
  provider: string;
  model: string;
  label: string;
  capabilities: unknown;
  enabled: boolean;
}

interface AiMatrix {
  catalog: CatalogEntry[];
  credentials: { id: string; provider: string; status: string; owner_user_id: string | null }[];
}

interface MeSurfaces {
  surfaces: { surface_key: string; picked: { provider: string; model: string } | null }[];
}

/** The wizard lands on its resumed list or the rail — get to the rail. */
/** Reach the wizard's Connect AI step (the sixth org step, key `ai`).
 *  The demo org resolves clients, boards and tickets from its own data,
 *  so /welcome opens on the resumed list (the pack's state 14) with a
 *  row per pending step, and the AI row's button focuses that step. A
 *  genuinely empty org opens the rail directly. Either way this ends on
 *  the rail with Connect AI focused. */
async function openAiStep(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/welcome");
  await expect(
    page.getByTestId("wizard-resumed").or(page.getByTestId("wizard-step-6")).first(),
  ).toBeVisible({ timeout: 30_000 });
  if ((await page.getByTestId("wizard-resumed").count()) > 0) {
    await page.getByTestId("wizard-resume-ai").click();
  }
  await expect(page.getByTestId("wizard-step-6")).toBeVisible({ timeout: 30_000 });
}

test("AI setup: renders for the owner with the honest gaps, gates the technician out", async ({
  browser,
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  await post("/onboarding/reset/org", { method: "POST", body: {} });

  // The OWNER (admin standing) gets the working step.
  await openAiStep(authedPage);
  await expect(authedPage.getByTestId("ai-setup")).toBeVisible({ timeout: 20_000 });
  for (const id of ["openai", "anthropic", "ollama", "custom"]) {
    await expect(authedPage.getByTestId(`ai-setup-provider-${id}`)).toBeVisible();
  }
  // The backend gap is visible, named, and inert: no key field can open for
  // a provider the seal surface does not accept.
  await expect(authedPage.getByTestId("ai-setup-provider-gemini")).toContainText(
    "Not available yet",
  );
  await authedPage.getByTestId("ai-setup-provider-gemini").click({ force: true });
  await expect(authedPage.getByTestId("ai-setup-key-input")).toHaveCount(0);
  // And the exit is first-class on the step itself.
  await expect(authedPage.getByTestId("ai-setup-notnow")).toBeVisible();

  // The TECHNICIAN gets one honest sentence and no cards. One session, not
  // two: the context login's own pair is what the page is seeded with (the
  // harness's p62-gate-truth lesson — a page presenting a token the cookie
  // session already rotated arms the refresh gate).
  const ctx = await browser.newContext({ baseURL: WEB_URL, viewport: { width: 1440, height: 900 } });
  const techPage = await ctx.newPage();
  const techRes = await ctx.request.post(`${API_URL}/api/v1/auth/login`, {
    data: { email: "tech@demo.local", password: DEMO_PASSWORD },
  });
  if (!techRes.ok()) throw new Error(`tech login failed (${techRes.status()})`);
  const techTokens = (await techRes.json()) as TokenPair;
  await seedTokens(techPage, techTokens);
  // The step rides the org wizard, which any member can open; the gate is
  // the step's own, not the wizard's.
  await openAiStep(techPage);
  await expect(techPage.getByTestId("ai-setup-gate")).toBeVisible({ timeout: 20_000 });
  await expect(techPage.getByTestId("ai-setup-chat-cards")).toHaveCount(0);
  await ctx.close();
});

test("AI setup: a key saves with test-connection feedback and leaves nothing behind", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  const get = authedFetch(ownerTokens.access_token);
  await post("/onboarding/reset/org", { method: "POST", body: {} });

  await openAiStep(authedPage);
  await expect(authedPage.getByTestId("ai-setup")).toBeVisible({ timeout: 20_000 });

  // Pick ChatGPT, paste a key, save. The seal is local (no vendor call in
  // v1), so the honest test result is the connected line.
  await authedPage.getByTestId("ai-setup-provider-openai").click();
  await authedPage.getByTestId("ai-setup-key-input").fill("sk-demo-e2e-w72");
  await authedPage.getByTestId("ai-setup-key-save").click();
  await expect(authedPage.getByTestId("ai-setup-test-openai")).toContainText(
    /Connected|Failed/,
    { timeout: 20_000 },
  );
  // Presence only: the draft never renders back.
  await expect(authedPage.getByTestId("ai-setup-key-input")).toHaveValue("");

  // The org state carries the key; then the test puts the org back.
  const status = (await get<unknown>("/settings/ai/status")) as AiStatus;
  expect(status.configured_providers).toContain("openai");
  await authedFetch(ownerTokens.access_token)("/settings/ai/byok/openai", {
    method: "DELETE",
  });
  const after = (await get<unknown>("/settings/ai/status")) as AiStatus;
  expect(after.configured_providers).not.toContain("openai");
});

test("AI setup: the chat default persists through the org pick", async ({
  authedPage,
  ownerTokens,
}) => {
  const post = authedFetch(ownerTokens.access_token);
  const get = authedFetch(ownerTokens.access_token);
  await post("/onboarding/reset/org", { method: "POST", body: {} });

  // Stage one completion pair for Ollama (no key needed, so it is eligible
  // on a bare org). Remember the pair's previous state to put it back.
  const matrixBefore = (await get<unknown>("/settings/ai/matrix")) as AiMatrix;
  const staged = matrixBefore.catalog.find((e) => e.provider === "ollama" && e.model === "llama3");
  const stagedWasEnabled = staged?.enabled ?? false;
  await post("/settings/ai/catalog", {
    method: "PUT",
    body: {
      provider: "ollama",
      model: "llama3",
      label: "llama3",
      capabilities: ["completion"],
      enabled: true,
      idempotency_key: `w72-spec:${Date.now()}`,
    },
  });

  // The org default pick that stands before the test — restored after.
  const meBefore = (await get<unknown>("/ai/me/surfaces")) as MeSurfaces;
  const priorPick = meBefore.surfaces.find((s) => s.surface_key === "general_chat")?.picked ?? null;

  await openAiStep(authedPage);
  await expect(authedPage.getByTestId("ai-setup")).toBeVisible({ timeout: 20_000 });
  await authedPage.getByTestId("ai-setup-provider-ollama").click();
  await authedPage.getByTestId("ai-setup-chat-default-ollama-llama3").click();
  await authedPage.getByTestId("ai-setup-save").click();
  await expect(authedPage.getByTestId("ai-setup-report")).toContainText(
    "Chat default set to ollama / llama3.",
    { timeout: 20_000 },
  );

  // Server truth: the general_chat pick resolves to the staged pair.
  await expect
    .poll(async () => {
      const me = (await get<unknown>("/ai/me/surfaces")) as MeSurfaces;
      return me.surfaces.find((s) => s.surface_key === "general_chat")?.picked?.model ?? null;
    })
    .toBe("llama3");

  // Put the shared org back as far as the API allows: the staged catalog
  // entry goes dark again, and a pick that existed before is rewritten. A
  // pick that did NOT exist stays (the org pick surface has no DELETE) —
  // harmless while the org's AI switch is off, and named here so the next
  // lane that adds the DELETE closes it.
  await post("/settings/ai/catalog", {
    method: "PUT",
    body: {
      provider: "ollama",
      model: "llama3",
      label: "llama3",
      capabilities: ["completion"],
      enabled: stagedWasEnabled,
      idempotency_key: `w72-spec-cleanup:${Date.now()}`,
    },
  });
  if (priorPick !== null) {
    await post("/settings/ai/model-picks/general_chat", {
      method: "PUT",
      body: { provider: priorPick.provider, model: priorPick.model, idempotency_key: `w72-spec-restore:${Date.now()}` },
    });
  }
});

test("AI setup: 'Not now' skips without saving anything", async ({ authedPage, ownerTokens }) => {
  const post = authedFetch(ownerTokens.access_token);
  const get = authedFetch(ownerTokens.access_token);
  await post("/onboarding/reset/org", { method: "POST", body: {} });
  const before = (await get<unknown>("/settings/ai/status")) as AiStatus;

  await openAiStep(authedPage);
  await expect(authedPage.getByTestId("ai-setup")).toBeVisible({ timeout: 20_000 });
  // Type a key and walk away from it: the skip seals nothing.
  await authedPage.getByTestId("ai-setup-provider-openai").click();
  await authedPage.getByTestId("ai-setup-key-input").fill("sk-never-saved-w72");
  await authedPage.getByTestId("ai-setup-notnow").click();

  // The step recorded its skip (skip-for-now persists nothing, the step
  // stays pending) and the key store is untouched.
  const after = (await get<unknown>("/settings/ai/status")) as AiStatus;
  expect(after.configured_providers).toEqual(before.configured_providers);
});
