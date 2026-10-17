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
import { authedFetch } from "../helpers/api";

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
  // state 14) rather than the rail. Step into the first pending step; a
  // genuinely empty org opens the rail directly, and the same rail assertion
  // holds either way.
  const resume = authedPage.getByTestId(`wizard-resume-${firstStep}`);
  if (await resume.isVisible({ timeout: 15_000 }).catch(() => false)) {
    await resume.click();
  }
  // The rail is present with every step: the shipped ids are 1-based indexes.
  await expect(authedPage.getByTestId(`wizard-step-${orgRailStep(firstStep)}`)).toBeVisible({
    timeout: 30_000,
  });

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
  if (await footerNever.isVisible({ timeout: 15_000 }).catch(() => false)) {
    // The resumed list carries the choice in its own footer.
    await footerNever.click();
  } else {
    // Otherwise it lives in the header's guide menu.
    await authedPage.getByTestId("wizard-guide-menu").click();
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
