// Bundle budget gate (p52-frontend-perf-budget) — the ceiling that fails a job.
//
// Purpose: pin the arithmetic the gate decides on, because a budget nobody
// tests is a comment. Three claims matter and each gets a case: the measurement
// is the gzipped first-load set the manifest names (shared chunks counted once),
// the ceiling is baseline plus headroom rounded to a reviewable step, and a
// route can fail either gate independently — over the ceiling, or regressed
// past the baseline while still inside it.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { gzipSync } from "node:zlib";
import { afterEach, describe, expect, it } from "vitest";
import {
  DEFAULT_TOLERANCE_BYTES,
  computeRouteSizes,
  derivedCeiling,
  evaluateBudget,
  formatReport,
} from "../../../../scripts/check-web-bundle-budget.mjs";

const CHUNK_A = "export const shared = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';\n";
const CHUNK_B = "export const page = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbb';\n";

function fakeBuild(): { appDir: string; expected: { shared: number; a: number; b: number } } {
  const appDir = mkdtempSync(path.join(tmpdir(), "bundle-budget-"));
  const nextDir = path.join(appDir, ".next");
  mkdirSync(path.join(nextDir, "static", "chunks"), { recursive: true });
  writeFileSync(path.join(nextDir, "static", "chunks", "shared.js"), CHUNK_A);
  writeFileSync(path.join(nextDir, "static", "chunks", "a.js"), CHUNK_B);
  writeFileSync(path.join(nextDir, "static", "chunks", "b.js"), CHUNK_B);
  writeFileSync(
    path.join(nextDir, "app-build-manifest.json"),
    JSON.stringify({
      pages: {
        "/layout": ["static/chunks/shared.js"],
        "/one/page": ["static/chunks/shared.js", "static/chunks/a.js"],
        "/two/page": ["static/chunks/shared.js", "static/chunks/a.js", "static/chunks/b.js"],
      },
    }),
  );
  const shared = gzipSync(Buffer.from(CHUNK_A), { level: 9 }).length;
  const page = gzipSync(Buffer.from(CHUNK_B), { level: 9 }).length;
  return { appDir, expected: { shared, a: page, b: page } };
}

const created: string[] = [];
afterEach(() => {
  for (const dir of created.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("computeRouteSizes", () => {
  it("reads only routes, gzips their chunks once, and adds shared chunks to each", () => {
    const { appDir, expected } = fakeBuild();
    created.push(appDir);

    const sizes = computeRouteSizes(appDir);

    // A layout entry is not a route.
    expect([...sizes.keys()].sort()).toEqual(["/one", "/two"]);
    expect(sizes.get("/one")?.gzip).toBe(expected.shared + expected.a);
    // The shared chunk is not counted twice even though three routes list it.
    expect(sizes.get("/two")?.gzip).toBe(expected.shared + expected.a + expected.b);
  });
});

describe("derivedCeiling", () => {
  const config = { headroom_pct: 5, min_headroom_bytes: 1024 };

  it("adds the headroom percentage and rounds up to a 4 KiB step", () => {
    // 100_000 + 5% = 105_000, rounded up to the next 4 KiB boundary.
    expect(derivedCeiling(100_000, config)).toBe(106_496);
  });

  it("keeps a floor so a tiny route still has room for a real change", () => {
    expect(derivedCeiling(4_000, config)).toBe(8_192);
  });
});

describe("evaluateBudget", () => {
  const config = {
    tolerance_bytes: DEFAULT_TOLERANCE_BYTES,
    headroom_pct: 5,
    min_headroom_bytes: 1024,
    baseline: { "/dashboards": 200_000, "/kb": 300_000 },
    ceilings: {},
    groups: [
      { name: "workspace-shell", routes: ["/dashboards"], ceiling_bytes: 210_000 },
      { name: "workspace-tools", routes: ["/kb"], ceiling_bytes: 340_000 },
    ],
  };

  function sizes(entries: Record<string, number>) {
    return new Map(Object.entries(entries).map(([route, gzip]) => [route, { raw: gzip * 3, gzip }]));
  }

  it("passes when every route sits at its baseline", () => {
    const result = evaluateBudget(sizes({ "/dashboards": 200_000, "/kb": 300_000 }), config);
    expect(result.ok).toBe(true);
    expect(result.failures).toEqual([]);
  });

  it("absorbs toolchain noise inside the tolerance", () => {
    const result = evaluateBudget(sizes({ "/dashboards": 200_000 + 2_000, "/kb": 300_000 }), config);
    expect(result.ok).toBe(true);
  });

  it("fails a regression that stays under the ceiling", () => {
    // 206_000 is under the 210_000 ceiling but 6_000 over baseline + tolerance.
    const result = evaluateBudget(sizes({ "/dashboards": 206_000, "/kb": 300_000 }), config);
    expect(result.ok).toBe(false);
    expect(result.failures).toHaveLength(1);
    expect(result.failures[0]).toMatchObject({ kind: "regression", route: "/dashboards" });
  });

  it("fails a route past its own ceiling", () => {
    const result = evaluateBudget(sizes({ "/dashboards": 200_000, "/kb": 345_000 }), config);
    expect(result.ok).toBe(false);
    expect(result.failures.map((f) => f.kind)).toContain("ceiling");
    expect(result.failures.find((f) => f.kind === "ceiling")?.route).toBe("/kb");
  });

  it("fails a group whose heaviest route is past the group ceiling", () => {
    const tight = {
      ...config,
      ceilings: { "/kb": 400_000 },
      groups: [{ name: "workspace-tools", routes: ["/kb"], ceiling_bytes: 310_000 }],
    };
    const result = evaluateBudget(sizes({ "/dashboards": 200_000, "/kb": 320_000 }), tight);
    const groupFailure = result.failures.find((f) => f.kind === "group-ceiling");
    expect(groupFailure).toMatchObject({ route: "workspace-tools" });
    expect(groupFailure?.detail).toContain("/kb");
  });

  it("notes a deleted route instead of failing it, so a rename shows as a config diff", () => {
    const result = evaluateBudget(sizes({ "/dashboards": 200_000 }), config);
    expect(result.ok).toBe(true);
    expect(result.warnings.join(" ")).toContain("/kb");
  });

  it("prints a table with the three numbers a reviewer needs", () => {
    const report = formatReport(evaluateBudget(sizes({ "/dashboards": 206_000, "/kb": 300_000 }), config));
    const row = report.split("\n").find((line) => line.startsWith("/dashboards"));
    expect(row).toContain("201.2");
    expect(row).toContain("195.3");
    expect(row).toContain("+5.9");
    expect(row).toContain("REGRESSED");
  });
});
