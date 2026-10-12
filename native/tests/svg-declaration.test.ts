/**
 * The signature pad's drawing dependency is declared, resolved, and typed.
 *
 * `react-native-svg` was read by the handoff kit's generated icon file but
 * declared by nothing, so it was silently absent from `native/package.json`
 * — the classic way a library a module imports ships as a CVE instead of as
 * a line in the manifest. Wave 43 track 2 lane 12 added the declaration, and
 * this test is the check that keeps it there.
 *
 * What this does NOT do: import the library. `react-native-svg` is a React
 * Native package with a native entry point, so it cannot be loaded by the Node
 * test runner at all — the proof that it works is the native typecheck plus a
 * device run, not a unit test. Asserting a resolve here would be false
 * confidence, so the assertions are about the contract instead.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const NATIVE_PKG = resolve(import.meta.dirname, "..", "package.json");

interface PackageJson {
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
}

function readNativePackage(): PackageJson {
  return JSON.parse(readFileSync(NATIVE_PKG, "utf8")) as PackageJson;
}

describe("react-native-svg declaration", () => {
  it("is a direct dependency of the native app, not a transitive accident", () => {
    const pkg = readNativePackage();
    expect(pkg.dependencies?.["react-native-svg"]).toBeDefined();
  });

  it("resolves through the workspace store, so pnpm can actually link it", () => {
    // The lockfile is where the declaration becomes an installed version.
    const lock = readFileSync(resolve(import.meta.dirname, "..", "..", "pnpm-lock.yaml"), "utf8");
    expect(lock).toContain("react-native-svg@15.15.5");
  });

  it("is present in the lockfile as a dependency of the native package", () => {
    const lock = readFileSync(resolve(import.meta.dirname, "..", "..", "pnpm-lock.yaml"), "utf8");
    // The app's own dependency block, not an unrelated package's.
    expect(lock).toMatch(/native:\s*\n(?:.*\n)*?\s+react-native-svg:/);
  });
});
