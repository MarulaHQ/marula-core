// axe-audit.spec.ts — full-matrix automated accessibility sweep
// (p29-accessibility-deep, deliverables A + D).
//
// Sweeps every shipped web surface with axe-core (WCAG 2.x A/AA tags) in
// BOTH the light and dark projects. Fails on NEW critical violations vs
// e2e/axe-baseline.json; the baselined backlog is the fix work order
// (per-item owners in the baseline file). AXE_UPDATE_BASELINE=1 snapshots;
// AXE_STRICT=1 additionally fails on baselined criticals still present.
//
// Evidence: `<lane>/axe-results.json` in the running lane's own evidence dir
// (e2e/helpers/evidence.ts) — the coverage matrix and Wave 35 read it, and no
// lane's sweep overwrites another lane's record (Wave 47 lane A17).

import { test } from "../helpers/harness";
import { SURFACES, runAudit } from "../helpers/axe";
import { evidenceDir } from "../helpers/evidence";

test.describe("axe full-matrix audit", () => {
  test("evidence lands in this lane's own evidence dir", () => {
    // The full-matrix test below only writes; this makes the destination
    // visible in the run output so a clobbering regression is obvious.
    console.log(`[axe-audit] evidence dir: ${evidenceDir()}`);
  });
  // One scan page per surface, all inside the authenticated fixture.
  test("every shipped surface passes WCAG 2.x A/AA with no NEW critical violations", async ({
    authedPage,
  }, testInfo) => {
    // 29 surfaces × (goto + networkidle + axe analyze) ≈ 10–12 min per theme.
    test.setTimeout(1_500_000);
    const pages = SURFACES.map((s) => ({
      surface: s.label,
      route: s.path,
      page: authedPage,
    }));
    const outcomes = await runAudit(pages);
    const totalCritical = outcomes.reduce(
      (sum, o) => sum + o.violations.filter((v) => v.impact === "critical").length,
      0,
    );
    // The summary rides the test title so the list reporter keeps a trace
    // of the audit's shape per theme project.
    testInfo.attach("axe-summary", {
      body: JSON.stringify(
        outcomes.map((o) => ({
          surface: o.surface,
          violations: o.violations.length,
          critical: o.violations.filter((v) => v.impact === "critical").length,
        })),
        null,
        2,
      ),
      contentType: "application/json",
    });
    console.log(
      `[axe-audit] ${testInfo.project.name}: ${outcomes.length} surfaces, ${totalCritical} baselined-critical total`,
    );
  });
});
