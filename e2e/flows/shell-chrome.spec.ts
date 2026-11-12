// Flow — the shell chrome: top bar composition, connection presence, counters.
//
// The top bar and the rail are drawn from the mockup pack, and three parts of
// that composition are easy to lose silently: the rail's shell-owned width
// (64px icons-only / 188px expanded), the 20px rail glyph on the 24 grid, and
// the count chip's rules (nothing when the number is unknown or zero, a cap on
// large counts, one accessible name carrying the number).
//
// This flow measures the rendered chrome rather than trusting the class names,
// in both themes, and leaves a screenshot pair in the lane's evidence dir.
import { expect, test } from "../helpers/harness";
import { evidenceDir } from "../helpers/evidence";
import { join } from "node:path";

const CAPTURES = evidenceDir("captures");

test("the top bar and rail hold the pack's composition", async ({ authedPage }, testInfo) => {
  const theme = testInfo.project.name === "dark" ? "dark" : "light";
  await authedPage.goto("/tickets");
  const bar = authedPage.getByTestId("top-bar");
  await expect(bar).toBeVisible({ timeout: 30_000 });

  // 56px bar, one hairline border.
  const barBox = await bar.boundingBox();
  expect(barBox?.height).toBe(56);

  // The brand keeps a fixed column, so the search field starts at the same x
  // on every surface instead of sliding with the lockup's own width.
  const launcher = authedPage.getByTestId("search-launcher");
  await expect(launcher).toBeVisible();
  const launcherBox = await launcher.boundingBox();
  expect(launcherBox?.width).toBeGreaterThan(300);
  expect(Math.round(launcherBox?.height ?? 0)).toBe(40);

  // Presence is a word, not a colour swatch: the stack is healthy here, so the
  // line must say Online.
  const presence = authedPage.getByTestId("connection-presence");
  await expect(presence).toBeVisible();
  await expect(presence).toContainText("Online");
  await expect(presence).toHaveAttribute("data-state", "online");

  // The dock controls are the pack's 40px bordered squares, and the avatar is
  // the one unframed edge.
  const themeButton = authedPage.getByRole("button", { name: /switch to (dark|light) mode/i });
  const themeBox = await themeButton.boundingBox();
  expect(Math.round(themeBox?.width ?? 0)).toBe(40);
  expect(Math.round(themeBox?.height ?? 0)).toBe(40);
  const themeStyle = await themeButton.evaluate((el) => {
    const s = getComputedStyle(el);
    return { border: s.borderTopWidth, radius: s.borderTopLeftRadius };
  });
  expect(themeStyle.border).toBe("1px");
  expect(themeStyle.radius).toBe("10px");

  const bell = authedPage.getByRole("button", { name: /^Notifications/ });
  await expect(bell).toBeVisible();
  // The count reaches assistive tech through the button's name, not a second
  // node: "Notifications" or "Notifications (3 unread)".
  await expect(bell).toHaveAttribute("aria-label", /^Notifications( \(\d+ unread\))?$/);

  // The rail is the shell's width, not its tiles': 64px icons-only.
  const rail = authedPage.getByTestId("nav-rail");
  await expect(rail).toBeVisible();
  const railBox = await rail.boundingBox();
  expect(Math.round(railBox?.width ?? 0)).toBe(64);

  // A tile is 44px and carries a 20px glyph (the pack's `#rail a svg`), which
  // is what makes the icon set's 1.75px stroke read at its drawn weight.
  const ticketsLink = rail.getByRole("link", { name: /^Tickets(, \d+ open)?$/ });
  await expect(ticketsLink).toBeVisible();
  const tileBox = await ticketsLink.boundingBox();
  expect(Math.round(tileBox?.width ?? 0)).toBe(44);
  expect(Math.round(tileBox?.height ?? 0)).toBe(44);
  const glyphBox = await ticketsLink.locator("svg").first().boundingBox();
  expect(Math.round(glyphBox?.width ?? 0)).toBe(20);

  // The dock cluster is pinned to the right edge and must stay inside the
  // window: a cluster that overflows silently loses the avatar off-screen, and
  // a screenshot clip narrower than the viewport hides exactly that.
  const viewport = authedPage.viewportSize();
  const cluster = authedPage.getByTestId("top-bar").locator("div").last();
  const clusterBox = await cluster.boundingBox();
  expect(Math.round(clusterBox?.x ?? 0) + Math.round(clusterBox?.width ?? 0)).toBeLessThanOrEqual(
    (viewport?.width ?? 1440) - 8,
  );

  await authedPage.screenshot({
    path: join(CAPTURES, `shell-chrome-${theme}.png`),
    clip: { x: 0, y: 0, width: viewport?.width ?? 1440, height: 240 },
  });

  // The launcher is the search field: it takes the person to the search page.
  await launcher.click();
  await expect(authedPage).toHaveURL(/\/search(\?.*)?$/);
  await expect(authedPage.getByTestId("top-bar")).toBeVisible();
});
