/**
 * CSS contract tests for components.css.
 * Purpose: every referenced --m-* variable must exist in tokens.css, motion
 * stays ≤120ms on opacity/transform only (theme.md §5).
 * Non-purpose: token palette checks live in tokens.test.ts.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const src = join(process.cwd(), "src");
const componentsCss = readFileSync(join(src, "components.css"), "utf8");
const tokensCss = readFileSync(join(src, "tokens.css"), "utf8");

describe("components.css variable references", () => {
  it("references only variables that tokens.css defines (plus Radix internals)", () => {
    const defined = new Set([...tokensCss.matchAll(/(--[\w-]+):/g)].map((m) => m[1]));
    const allowedExternals = new Set(["--radix-select-trigger-width"]);
    const referenced = [...componentsCss.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]!);
    expect(referenced.length).toBeGreaterThan(20);
    for (const v of referenced) {
      expect(defined.has(v) || allowedExternals.has(v)).toBe(true);
    }
  });
});

describe("motion rules (theme.md §5)", () => {
  it("never declares a transition longer than 120ms", () => {
    for (const m of componentsCss.matchAll(/transition:\s*([^;]+);/g)) {
      const duration = m[1]!.match(/([\d.]+)(ms|s)\b/);
      expect(duration, `transition: ${m[1]}`).not.toBeNull();
      const ms =
        duration![2] === "s" ? parseFloat(duration![1]!) * 1000 : parseFloat(duration![1]!);
      expect(ms, `transition: ${m[1]}`).toBeLessThanOrEqual(120);
    }
  });

  it("guards continuous animation behind prefers-reduced-motion", () => {
    // The spinner's rotate loop is a progress affordance, not a state
    // transition; it must still collapse under reduced-motion.
    expect(componentsCss).toContain("prefers-reduced-motion");
    const guard = componentsCss.slice(componentsCss.indexOf("prefers-reduced-motion"));
    expect(guard).toContain(".m-spinner");
  });

  it("animates opacity/transform only in transitions", () => {
    for (const m of componentsCss.matchAll(/transition:\s*([^;]+);/g)) {
      const props = m[1]!.split(" ").filter((p) => !/^\d|^[\d.]+ms|ease|linear|cubic/.test(p));
      for (const p of props) {
        expect(["opacity", "transform", "none"].some((ok) => p.startsWith(ok))).toBe(true);
      }
    }
  });
});
