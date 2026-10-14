// The pre-paint paint order, proven at the jsdom level: the staff app's first
// screen (the login route) must never paint the wrong mode, and the resolved
// attribute must survive hydration.
//
// This is the light-weight repro for the Wave 47 lane A3 defect
// (`p47-prepaint-theme-flash`). The E2E legs — `e2e/flows/01-auth.spec.ts` and
// `e2e/flows/theme-matrix.spec.ts` — assert the same settled values in a real
// browser; this suite pins the mechanism without one, so a regression fails
// in milliseconds instead of a 30-minute gate run.
//
// The three legs, in the order a browser performs them:
//   1. the inline bootstrap script runs before first paint and writes the
//      KIT's resolved attribute (`neutral` for the app's light, `dark` for
//      dark) — the first paint is the resolved mode, never the baseline;
//   2. hydration keeps that attribute (React never removes an attribute it
//      did not render, on a successful hydration);
//   3. when hydration recovery REPLACES <html> (the documented #418 wipe that
//      left the app on the baseline until something wrote again), the
//      ThemeBootstrap layout effect restores the resolved attribute before
//      the browser paints the post-hydration DOM — the flash has no frame to
//      appear in.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { renderToString } from "react-dom/server";
import { act } from "react";
import { hydrateRoot, type Root } from "react-dom/client";
import { themeBootstrapScript } from "@marula-core/ui/theme-bootstrap";

vi.mock("next/font/local", () => ({
  default: () => ({ variable: "figtree-mock", className: "figtree-mock" }),
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/login",
}));

// Raw hydrateRoot (no @testing-library wrapper) needs the act environment
// declared, or React's act() refuses to flush the commit.
(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { AuthProvider } from "../../lib/api";
import { ThemeBootstrap } from "../../components/theme-bootstrap";
import LoginPage from "../../app/login/page";
import { MockApi, ME_FIXTURE, seedAuthTokens } from "../helpers/mock-api";

/** The layout's <body> contents, the part React actually hydrates against. */
function bodyTree(): React.ReactElement {
  return (
    <>
      <ThemeBootstrap />
      <AuthProvider>
        <div className="app-root">
          <LoginPage />
        </div>
      </AuthProvider>
    </>
  );
}

/** Stub the system preference the way a device would, before the script runs. */
function stubSystemPreference(dark: boolean): void {
  vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({
    matches: dark && query.includes("dark"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }));
}

/** Run the SAME inline script the root layout ships, as the browser would. */
function runPrePaintBootstrap(): void {
  window.eval(themeBootstrapScript());
}

function setupApi(): void {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.install();
}

beforeEach(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-preset");
  document.documentElement.removeAttribute("data-density");
  seedAuthTokens();
  setupApi();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("login route — the pre-paint paint order", () => {
  it("paints the resolved mode before any render: a stored Dark rides the script, not the baseline", () => {
    window.localStorage.setItem("marula-theme", "dark");
    stubSystemPreference(false);

    runPrePaintBootstrap();

    // The KIT's own literals: the app's light is `neutral` (the kit's default
    // block), dark is `dark`. The settled E2E assertion is exactly this regex.
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("paints `neutral` — never the app's `light`, which the kit reads as cream", () => {
    window.localStorage.setItem("marula-theme", "light");
    stubSystemPreference(true);

    runPrePaintBootstrap();

    expect(document.documentElement.getAttribute("data-theme")).toBe("neutral");
  });

  it("resolves the system preference when nothing is stored, in both directions", () => {
    stubSystemPreference(true);
    runPrePaintBootstrap();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    document.documentElement.removeAttribute("data-theme");
    vi.restoreAllMocks();
    stubSystemPreference(false);
    runPrePaintBootstrap();
    expect(document.documentElement.getAttribute("data-theme")).toBe("neutral");
  });

  it("hydration keeps the resolved attribute — a clean pass adds no console error", async () => {
    window.localStorage.setItem("marula-theme", "dark");
    stubSystemPreference(false);
    runPrePaintBootstrap();

    const container = document.createElement("div");
    document.body.appendChild(container);
    const tree = bodyTree();
    container.innerHTML = renderToString(tree);

    const errors: unknown[][] = [];
    const spy = vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
      errors.push(args);
    });
    const root: Root = hydrateRoot(container, tree);
    await act(async () => {});
    spy.mockRestore();

    const hydrationErrors = errors.filter((args) =>
      args.some((a) => typeof a === "string" && a.toLowerCase().includes("hydrat")),
    );
    expect(hydrationErrors).toEqual([]);
    // The attribute the script wrote pre-paint is still there afterwards.
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    await act(async () => {
      root.unmount();
    });
    container.remove();
  });

  it("recovers from the hydration wipe: the attribute is restored before the browser paints", async () => {
    window.localStorage.setItem("marula-theme", "dark");
    stubSystemPreference(false);
    runPrePaintBootstrap();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    const container = document.createElement("div");
    document.body.appendChild(container);
    const tree = bodyTree();
    container.innerHTML = renderToString(tree);

    // What React's hydration recovery does when it replaces <html>: the
    // script-set attribute is gone from the DOM, and nothing has re-applied
    // it yet. Without the recovery leg this is the frame the user sees — the
    // light canvas under a dark preference.
    document.documentElement.removeAttribute("data-theme");

    const root: Root = hydrateRoot(container, tree);
    await act(async () => {});

    // ThemeBootstrap's layout effect flushed before paint (`act` covers the
    // full commit including layout effects), so no wrong-mode frame exists.
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    // The recovery never persists a preference on a public screen.
    expect(window.localStorage.getItem("marula-theme")).toBe("dark");

    await act(async () => {
      root.unmount();
    });
    container.remove();
  });
});
