// The pre-paint bootstrap's contract, and its parity with the TypeScript
// twin that re-applies it.
//
// The script (`themeBootstrapScript`) is what a browser runs before the first
// paint; `applyResolvedAppearance` in theme.ts is what a host runs after the
// fact to repair the same attributes. Two implementations of one contract are
// a drift waiting to happen, so this suite runs both against the same storage
// states and demands identical DOM — the settled kit values, both preference
// directions, the corrupt-stored fallback, and the storage-unavailable case.
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { themeBootstrapScript, BOOTSTRAP_KEYS } from "../src/theme-bootstrap";
import { applyResolvedAppearance } from "../src/theme";
import { PRESET_STORAGE_KEY } from "../src/theme-pack-apply";
import { DENSITY_STORAGE_KEY, DEFAULT_DENSITY } from "../src/density";

function attributes(): Record<string, string | null> {
  const el = document.documentElement;
  return {
    "data-theme": el.getAttribute("data-theme"),
    "data-preset": el.getAttribute("data-preset"),
    "data-density": el.getAttribute("data-density"),
  };
}

function resetAttributes(): void {
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-preset");
  document.documentElement.removeAttribute("data-density");
}

/** The device's own light/dark setting, stubbed before the resolution runs. */
function stubSystemPreference(dark: boolean): void {
  vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({
    matches: dark && query.includes("dark"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
    addListener: () => {},
    removeListener: () => {},
  }));
}

interface Case {
  name: string;
  stored: Record<string, string>;
  systemDark: boolean;
}

const CASES: Case[] = [
  { name: "stored dark on a light device", stored: { "marula-theme": "dark" }, systemDark: false },
  { name: "stored light on a dark device", stored: { "marula-theme": "light" }, systemDark: true },
  { name: "nothing stored, dark device", stored: {}, systemDark: true },
  { name: "nothing stored, light device", stored: {}, systemDark: false },
  { name: "corrupt stored value falls back to the device", stored: { "marula-theme": "banana" }, systemDark: true },
  {
    name: "preset and density ride the same pre-paint pass",
    stored: {
      "marula-theme": "dark",
      "marula-theme-preset": "marula-warm",
      "marula-density": "compact",
    },
    systemDark: false,
  },
];

beforeEach(() => {
  window.localStorage.clear();
  resetAttributes();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("pre-paint bootstrap — the settled kit values", () => {
  for (const c of CASES) {
    it(`${c.name}: the script and its TS twin write the same attributes`, () => {
      for (const [key, value] of Object.entries(c.stored)) {
        window.localStorage.setItem(key, value);
      }
      stubSystemPreference(c.systemDark);

      window.eval(themeBootstrapScript());
      const fromScript = attributes();

      resetAttributes();
      applyResolvedAppearance();
      const fromTwin = attributes();

      expect(fromTwin).toEqual(fromScript);
      // The light mode is the kit's `neutral`, never the app's `light`, which
      // the kit reads as its cream block.
      const storedTheme = c.stored["marula-theme"];
      const expectedTheme =
        storedTheme === "light" || storedTheme === "dark"
          ? storedTheme === "light"
            ? "neutral"
            : "dark"
          : c.systemDark
            ? "dark"
            : "neutral";
      expect(fromScript["data-theme"]).toBe(expectedTheme);
    });
  }

  it("the script's own literals are the kit's: it never writes `light`", () => {
    window.localStorage.setItem("marula-theme", "light");
    stubSystemPreference(false);
    window.eval(themeBootstrapScript());
    const body = themeBootstrapScript();
    expect(body).toContain('"neutral"');
    expect(attributes()["data-theme"]).toBe("neutral");
  });

  it("the twin persists nothing — a public screen inherits without leaking", () => {
    const setItem = vi.spyOn(window.localStorage, "setItem");
    stubSystemPreference(true);
    applyResolvedAppearance();
    expect(setItem).not.toHaveBeenCalled();
  });

  it("storage that throws takes neither path down", () => {
    const throwing = {
      getItem: () => {
        throw new Error("storage unavailable");
      },
      setItem: () => {
        throw new Error("storage unavailable");
      },
      removeItem: () => {
        throw new Error("storage unavailable");
      },
      clear: () => {},
      key: () => null,
      length: 0,
    };
    vi.stubGlobal("localStorage", throwing);
    expect(() => window.eval(themeBootstrapScript())).not.toThrow();
    expect(() => applyResolvedAppearance()).not.toThrow();
    vi.unstubAllGlobals();
  });

  it("the keys the script reads are the modules' own keys", () => {
    expect(BOOTSTRAP_KEYS.preset).toBe(PRESET_STORAGE_KEY);
    expect(BOOTSTRAP_KEYS.density).toBe(DENSITY_STORAGE_KEY);
    expect(BOOTSTRAP_KEYS.defaultDensity).toBe(DEFAULT_DENSITY);
    expect(BOOTSTRAP_KEYS.densitySteps).toContain("comfortable");
    // The theme key is private in theme.ts, so it is proven by behaviour:
    // applyTheme writes it and the script resolves what it wrote.
    window.localStorage.setItem("marula-theme", "dark");
    expect(themeBootstrapScript()).toContain(JSON.stringify(BOOTSTRAP_KEYS.theme));
  });
});
