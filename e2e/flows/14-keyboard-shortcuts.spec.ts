// Flow 14 — keyboard shortcuts (Wave 29 lane 1 `p29-keyboard-shortcuts`).
//
// The hold-Alt overlay (render while held, dismiss on release, per-surface
// inventory), the ?-cheat sheet (full registry), and the core command
// paths (nav jumps, palette, new-record click parity).
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

test("hold-Alt overlay renders while held and dismisses on release", async ({ authedPage }) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();

  const overlay = authedPage.getByTestId("shortcut-overlay");
  await expect(overlay).toHaveCount(0);
  await authedPage.keyboard.down("Alt");
  await expect(overlay).toBeVisible();
  // Rows carry typed labels + keys + scope badges.
  await expect(authedPage.getByTestId("shortcut-overlay-rows")).toContainText("Search & commands");
  await expect(authedPage.getByTestId("shortcut-overlay-rows")).toContainText("Go to Clients");
  await authedPage.keyboard.up("Alt");
  await expect(overlay).toHaveCount(0);
});

test("overlay inventory changes with the surface (new rows on /clients, none on /reports)", async ({
  authedPage,
}) => {
  await authedPage.goto("/clients");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.down("Alt");
  await expect(authedPage.getByTestId("shortcut-overlay")).toBeVisible();
  await expect(authedPage.getByTestId("shortcut-overlay")).toContainText("New client");
  await authedPage.keyboard.up("Alt");
  await expect(authedPage.getByTestId("shortcut-overlay")).toHaveCount(0);

  await authedPage.goto("/reports");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.down("Alt");
  await expect(authedPage.getByTestId("shortcut-overlay")).toBeVisible();
  await expect(authedPage.getByTestId("shortcut-overlay")).not.toContainText("New client");
  await authedPage.keyboard.up("Alt");
});

test("? opens the full cheat-sheet dialog; Escape closes", async ({ authedPage }) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.press("?");
  await expect(authedPage.getByTestId("shortcut-cheatsheet")).toBeVisible();
  // The full registry: global commands AND per-surface commands listed.
  await expect(authedPage.getByTestId("shortcut-cheatsheet")).toContainText("New client");
  await expect(authedPage.getByTestId("shortcut-cheatsheet")).toContainText("Copy link");
  await expect(authedPage.getByTestId("shortcut-cheatsheet")).toContainText("Search & commands");
  await authedPage.keyboard.press("Escape");
  await expect(authedPage.getByTestId("shortcut-cheatsheet")).toHaveCount(0);
});

test("nav jump Alt+1 and Alt+2 follow the rail order", async ({ authedPage }) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.press("Alt+2");
  await expect(authedPage).toHaveURL(/\/clients(\?.*)?$/);
  await authedPage.keyboard.press("Alt+1");
  await expect(authedPage).toHaveURL(/\/tickets(\?.*)?$/);
});

test("palette chord Cmd/Ctrl+K opens unified search", async ({ authedPage }) => {
  await authedPage.goto("/tickets");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.press("ControlOrMeta+k");
  await expect(authedPage).toHaveURL(/\/search(\?.*)?$/);
});

test("Alt+N on /clients opens the new-client dialog exactly like the button", async ({ authedPage }) => {
  await authedPage.goto("/clients");
  await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  // The create control must exist before the chord can click it — the
  // "no command does what a click cannot" rule in test form.
  await expect(authedPage.getByTestId("new-record")).toBeAttached();
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
  await authedPage.keyboard.press("Alt+n");
  await expect(authedPage.getByText("New client", { exact: true }).nth(1)).toBeVisible();
});

test("all_off profile silences chords and the overlay; ? reference still works", async ({
  authedPage,
  ownerTokens,
}) => {
  const fetcher = authedFetch(ownerTokens.access_token);
  await fetcher("/settings/user", { method: "PUT", body: { key: "shortcuts.profile", value: "all_off" } });

  try {
    await authedPage.goto("/tickets");
    await expect(authedPage.getByTestId("top-bar")).toBeVisible({ timeout: 30_000 });
  // The profile read settles before any chord expectation.
  await expect(authedPage.getByTestId("shortcut-profile-ready")).toBeAttached();
    // The profile read settles before any chord expectation.
    await expect(authedPage.getByTestId("shortcut-profile-ready")).toHaveAttribute("data-profile", "all_off");
    await authedPage.keyboard.down("Alt");
    await expect(authedPage.getByTestId("shortcut-overlay")).toHaveCount(0);
    await authedPage.keyboard.up("Alt");
    // Nav chords are off too.
    await authedPage.keyboard.press("Alt+2");
    await expect(authedPage).not.toHaveURL(/\/clients(?!\/)/);
    // The cheat sheet stays reachable — read-only reference.
    await authedPage.keyboard.press("?");
    await expect(authedPage.getByTestId("shortcut-cheatsheet")).toBeVisible();
    await authedPage.keyboard.press("Escape");
  } finally {
    await fetcher("/settings/user", { method: "PUT", body: { key: "shortcuts.profile", value: "default" } });
  }
});
