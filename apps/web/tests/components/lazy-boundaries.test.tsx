// Lazy widget boundaries (p52-frontend-perf-budget).
//
// Purpose: prove that putting Recharts and TipTap behind `next/dynamic` changed
// only WHEN the code arrives. Each lazy widget renders the same markup as the
// module it replaced, and the placeholder it shows first occupies the box the
// real widget will — no layout shift, which is the whole reason a skeleton is
// allowed to exist here and a spinner is not.
//
// The last block holds the mechanical half of the rule: no module outside the
// two leaf files may import the heavy library. A future edit that re-adds a
// static import has to fail here and re-open the decision in review, rather
// than quietly undoing it.
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgingBarChart as DirectAgingBarChart } from "../../components/dashboards/widget-charts";
import { AgingBarChart as LazyAgingBarChart } from "../../components/dashboards/widget-charts-lazy";
import { ReportChart } from "../../components/report-chart-lazy";
import { KbArticleEditor } from "../../lib/editor/kb-article-editor-lazy";
import { KbArticleEditor as DirectKbArticleEditor } from "../../lib/editor/kb-article-editor";

const AGING_ROWS = [
  { bucket: "0_1d", bucket_ordinal: 0, open_count: 4, avg_age_days: 0.5, oldest_age_days: 1 },
  { bucket: "1_3d", bucket_ordinal: 1, open_count: 9, avg_age_days: 2, oldest_age_days: 3 },
  { bucket: "3_7d", bucket_ordinal: 2, open_count: 2, avg_age_days: 5, oldest_age_days: 6 },
];

const ARTICLE = {
  id: "a-1",
  org_id: "o",
  title: "Reset the printer",
  slug: "reset-printer",
  body: "hello",
  status: "published",
  category_id: "cat-a",
  client_visible: true,
  content_jsonb: null,
  created_at: "2026-09-01T10:00:00Z",
  updated_at: "2026-09-02T10:00:00Z",
};

/** The chart geometry a reader actually gets, minus Recharts' per-instance ids. */
function chartShape(container: HTMLElement) {
  const svg = container.querySelector("svg");
  return {
    viewBox: svg?.getAttribute("viewBox") ?? null,
    widths: [...container.querySelectorAll("rect")].map((r) => r.getAttribute("width")),
    bars: container.querySelectorAll("path.recharts-rectangle").length,
  };
}

describe("dashboard chart boundary", () => {
  it("shows a box-reserving skeleton first, then the same chart as the direct import", async () => {
    const lazy = render(<LazyAgingBarChart rows={AGING_ROWS as never} ariaLabel="aging chart" />);

    // Before the chunk lands: a status region at the chart's own 720x200 ratio.
    const skeleton = screen.getByTestId("chart-skeleton");
    expect(skeleton).toHaveAttribute("role", "status");
    expect(skeleton).toHaveAttribute("aria-busy", "true");
    expect(skeleton.getAttribute("style")).toContain("aspect-ratio: 720 / 200");

    await waitFor(() => {
      expect(screen.getByRole("img", { name: "aging chart" })).toBeInTheDocument();
    });
    expect(screen.queryByTestId("chart-skeleton")).not.toBeInTheDocument();
    const lazyShape = chartShape(lazy.container);

    const direct = render(<DirectAgingBarChart rows={AGING_ROWS as never} ariaLabel="aging chart" />);
    const directShape = chartShape(direct.container);

    // Same axes, same bar count, same widths — only the module's arrival moved.
    expect(lazyShape).toEqual(directShape);
    expect(lazyShape.bars).toBe(AGING_ROWS.length);
    expect(lazyShape.viewBox).toBe("0 0 720 200");
  });
});

describe("report preview boundary", () => {
  const barPage = {
    columns: ["day", "count"],
    rows: [
      { day: "2026-01-01", count: 3 },
      { day: "2026-01-02", count: 5 },
    ],
  };
  const measures = [{ agg: "count" as const, column: null }];

  for (const viz of ["bar", "line", "pie"] as const) {
    it(`reserves the ResponsiveContainer height and then draws the ${viz} preview`, async () => {
      // A fresh module registry per case: after the first load the dynamic
      // chunk is cached, so the placeholder only appears on a cold boundary.
      vi.resetModules();
      const { ReportChart: ColdReportChart } = await import("../../components/report-chart-lazy");
      render(<ColdReportChart page={barPage} measures={measures as never} viz={viz} ariaLabel={`${viz} preview`} />);

      const skeleton = screen.getByTestId("chart-skeleton");
      expect(skeleton.getAttribute("style")).toContain("height: 240px");

      await waitFor(() => {
        expect(screen.getByRole("img", { name: `${viz} preview` })).toBeInTheDocument();
      });
      expect(screen.queryByTestId("chart-skeleton")).not.toBeInTheDocument();
    });
  }
});

describe("article editor boundary", () => {
  it("shows the editor skeleton first, then the whole editor the direct import renders", async () => {
    render(<KbArticleEditor article={ARTICLE as never} onSaved={() => {}} onRejection={() => {}} />);

    const skeleton = screen.getByRole("status", { name: "Loading article editor" });
    expect(skeleton).toHaveAttribute("aria-busy", "true");
    // The toolbar strip and the canvas box, at the heights the editor occupies.
    expect(skeleton.querySelector(".m-editor-toolbar")).not.toBeNull();
    expect(skeleton.getAttribute("class")).toContain("grid gap-3 p-4");

    await waitFor(() => {
      expect(screen.getByRole("toolbar", { name: "Article formatting" })).toBeInTheDocument();
    });
    expect(screen.queryByRole("status", { name: "Loading article editor" })).not.toBeInTheDocument();
    const lazyTools = screen.getAllByRole("button").map((b) => b.getAttribute("aria-label"));
    expect(screen.getByRole("textbox")).toBeInTheDocument();

    const direct = render(
      <DirectKbArticleEditor article={ARTICLE as never} onSaved={() => {}} onRejection={() => {}} />,
    );
    const directTools = [...direct.container.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"));
    // Both editors offer the same commands in the same order.
    expect(lazyTools).toEqual(directTools);
    expect(directTools).toContain("Bold");
    expect(directTools).toContain("Task list");
  });
});

// --- the rule that keeps the boundary from being undone ------------------------

const WEB_ROOT = path.resolve(__dirname, "../..");
const SCANNED_DIRS = ["app", "components", "lib"];
const SOURCE = /\.(ts|tsx)$/;

/** Files allowed to touch the heavy library directly. */
const ALLOWED_RECHARTS = new Set(["components/dashboards/widget-charts.tsx", "components/report-preview.tsx"]);
const ALLOWED_TIPTAP = new Set(["lib/editor/kb-article-editor.tsx"]);

function walk(dir: string, found: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry.startsWith(".")) continue;
    const absolute = path.join(dir, entry);
    if (statSync(absolute).isDirectory()) walk(absolute, found);
    else if (SOURCE.test(entry)) found.push(absolute);
  }
  return found;
}

function relative(file: string): string {
  return path.relative(WEB_ROOT, file).split(path.sep).join("/");
}

describe("heavy libraries stay behind their boundary", () => {
  const files = SCANNED_DIRS.flatMap((dir) => walk(path.join(WEB_ROOT, dir)));

  it("imports Recharts only where the lazy boundary can reach it", () => {
    const offenders = files
      .filter((file) => /from\s+"recharts"/.test(readFileSync(file, "utf8")))
      .map(relative)
      .filter((file) => !ALLOWED_RECHARTS.has(file));
    expect(offenders).toEqual([]);
  });

  it("imports TipTap only in the editor leaf (and its extension modules)", () => {
    const offenders = files
      .filter((file) => /from\s+"@tiptap\//.test(readFileSync(file, "utf8")))
      .map(relative)
      .filter((file) => !ALLOWED_TIPTAP.has(file) && !file.startsWith("lib/editor/extensions/"));
    expect(offenders).toEqual([]);
  });

  it("routes each consumer through the lazy module", () => {
    const consumers = {
      "components/dashboards/report-widget.tsx": "widget-charts-lazy",
      "components/report-builder.tsx": "report-chart-lazy",
      "app/kb/page.tsx": "kb-article-editor-lazy",
    };
    for (const [file, boundary] of Object.entries(consumers)) {
      const source = readFileSync(path.join(WEB_ROOT, file), "utf8");
      expect(source, `${file} should import ${boundary}`).toContain(boundary);
    }
  });
});
