// select-a11y.spec.ts — the shared Select popup, audited while it is OPEN
// (Wave 47 lane A22).
//
// Why a dedicated spec: e2e/flows/axe-audit.spec.ts sweeps the surfaces, and a
// popup is not on any surface until someone opens it. The kit's Radix Select
// renders `listbox > viewport (scrolls) > items`, so two rules live on the
// viewport and only one of them can pass at a time:
//   - `scrollable-region-focusable` (serious) when the viewport has no tab stop
//   - `aria-required-children` (critical) when the tab stop sits on a generic
//     div, which a listbox may not own
// packages/ui/src/components/select.tsx carries the fix (role="group" +
// tabIndex={0}); this spec is its browser proof and fails without it. It runs
// in BOTH theme projects because the popup is themed.
//
// The trigger list is the node registry's, so it is long enough to scroll; the
// viewport height is then squeezed so overflow is guaranteed rather than
// incidental, and the spec asserts the overflow before scanning. Without that
// precondition a shrinking registry would let the test pass by measuring a
// popup that never scrolls.
import { AxeBuilder } from "@axe-core/playwright";
import { expect, test } from "../helpers/harness";

const RULES = ["scrollable-region-focusable", "aria-required-children"];

test.describe("Select popup axe audit", () => {
  test("an open, scrolling popup trips neither scroll nor ownership rule", async ({
    authedPage,
  }, testInfo) => {
    // Short viewport on purpose: the rule editor's trigger select is
    // registry-driven, and this makes its list overflow its box.
    await authedPage.setViewportSize({ width: 1280, height: 420 });
    await authedPage.goto("/automations", { waitUntil: "networkidle" });
    await authedPage.getByRole("button", { name: /new rule/i }).first().click();

    const trigger = authedPage.locator("#ar-event");
    await trigger.scrollIntoViewIfNeeded();
    await trigger.click();

    const listbox = authedPage.getByRole("listbox");
    await expect(listbox).toBeVisible();

    const viewport = authedPage.locator("[data-radix-select-viewport]");
    const metrics = await viewport.evaluate((el) => ({
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
    }));
    expect(
      metrics.scrollHeight,
      "the popup must actually scroll, or this scan proves nothing",
    ).toBeGreaterThan(metrics.clientHeight);

    const scan = await new AxeBuilder({ page: authedPage })
      .include('[role="listbox"]')
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();

    await testInfo.attach("select-popup-axe.json", {
      body: JSON.stringify(
        {
          project: testInfo.project.name,
          scroll: metrics,
          violations: scan.violations.map((v) => ({
            id: v.id,
            impact: v.impact,
            targets: v.nodes.map((n) => n.target),
          })),
        },
        null,
        2,
      ),
      contentType: "application/json",
    });

    for (const id of RULES) {
      const hit = scan.violations.find((v) => v.id === id);
      expect(hit, `${id}: ${hit?.nodes.map((n) => n.html).join(" | ") ?? ""}`).toBeUndefined();
    }
  });
});
