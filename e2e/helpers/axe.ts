// axe-core enforcement helpers (p29-accessibility-deep, deliverable D).
//
// The gate is HONEST but not hostage to the whole backlog:
//   - A surface with a committed baseline fails on any critical violation
//     NOT present in that baseline (NEW regressions).
//   - Known criticals live in e2e/axe-baseline.json (machine-readable,
//     per-item owner) and ride the fix backlog instead of blocking.
//   - `AXE_UPDATE_BASELINE=1 pnpm e2e` rewrites the baseline from the run —
//     ONLY after a deliberate review of the diff.
//   - `AXE_STRICT=1` additionally fails on baselined criticals that are
//     still present (used by Wave 35 re-verification once the backlog
//     drains).
//
// Scan output is also written to design/qa-p29-accessibility-deep/
// axe-results.json every run so the coverage matrix and the next wave's
// re-verification read the same evidence.

import { AxeBuilder } from "@axe-core/playwright";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import * as path from "node:path";
import type { Page } from "@playwright/test";

const E2E_DIR = __dirname;
const BASELINE_PATH = path.join(E2E_DIR, "..", "axe-baseline.json");
const RESULTS_DIR = path.join(E2E_DIR, "..", "..", "design", "qa-p29-accessibility-deep");

export type AxeImpact = "minor" | "moderate" | "serious" | "critical" | null;

export interface AxeNode {
  target: string[];
  html: string;
}

type ScanViolation = Omit<AxeViolation, "nodes"> & { nodes: AxeNode[] };

export interface AxeViolation {
  id: string;
  impact: AxeImpact;
  help: string;
  helpUrl?: string;
  nodes: AxeNode[];
}

export interface BaselineEntry {
  /** Rule id, e.g. "color-contrast". */
  id: string;
  impact: AxeImpact;
  /** Owner lane/agent responsible for the fix (backlog accountability). */
  owner: string;
  /** Where the violation lives, human-scannable. */
  where?: string;
}

export interface AxeBaseline {
  _meta: { updated: string; note: string };
  surfaces: Record<string, BaselineEntry[]>;
}

export function loadBaseline(): AxeBaseline {
  try {
    return JSON.parse(readFileSync(BASELINE_PATH, "utf8")) as AxeBaseline;
  } catch {
    return { _meta: { updated: "", note: "" }, surfaces: {} };
  }
}

export function writeBaseline(baseline: AxeBaseline): void {
  writeFileSync(BASELINE_PATH, `${JSON.stringify(baseline, null, 2)}\n`);
}

/** Every shipped web surface the audit sweeps (Wave 34 scope included;
 * the in-flight plugin lane's surfaces ride Wave 35). */
export const SURFACES: { path: string; label: string }[] = [
  { path: "/today", label: "today" },
  { path: "/tickets", label: "tickets" },
  { path: "/boards", label: "boards" },
  { path: "/calendar", label: "calendar" },
  { path: "/dashboards", label: "dashboards" },
  { path: "/clients", label: "clients" },
  { path: "/projects", label: "projects" },
  { path: "/contracts", label: "contracts" },
  { path: "/reports", label: "reports" },
  { path: "/scorecards", label: "scorecards" },
  { path: "/inventory", label: "inventory" },
  { path: "/purchasing", label: "purchasing" },
  { path: "/stock", label: "stock" },
  { path: "/sales", label: "sales" },
  { path: "/assets", label: "assets" },
  { path: "/kb", label: "kb" },
  { path: "/alerts", label: "alerts" },
  { path: "/approvals", label: "approvals" },
  { path: "/automations", label: "automations" },
  { path: "/audit", label: "audit" },
  { path: "/billing", label: "billing" },
  { path: "/search", label: "search" },
  { path: "/settings", label: "settings" },
  { path: "/settings/ai", label: "settings-ai" },
  { path: "/settings/appearance", label: "settings-appearance" },
  { path: "/settings/shortcuts", label: "settings-shortcuts" },
  { path: "/settings/surveys", label: "settings-surveys" },
  { path: "/settings/reply-snippets", label: "settings-reply-snippets" },
  { path: "/onboarding", label: "onboarding" },
];

/** Scan one surface. Waits for the network to settle so async states render. */
export async function scanSurface(page: Page, route: string): Promise<AxeViolation[]> {
  await page.goto(route, { waitUntil: "networkidle" });
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const mapped: ScanViolation[] = (results.violations ?? []).map((v) => ({
    id: v.id,
    impact: (v.impact ?? null) as AxeImpact,
    help: v.help,
    helpUrl: v.helpUrl,
    nodes: (v.nodes ?? []).slice(0, 10).map((n) => ({
      target: (n.target as unknown as string[]) ?? [],
      html: n.html.slice(0, 200),
    })),
  }));
  return mapped;
}

function keyOf(v: AxeViolation): string {
  return `${v.id}::${v.nodes.map((n) => n.target.join(" ")).join("|")}`;
}

/** Diff a scan against the baseline: returns violations absent from it. */
export function newViolations(surface: string, violations: AxeViolation[], baseline: AxeBaseline): AxeViolation[] {
  const known = new Set((baseline.surfaces[surface] ?? []).map((e) => e.id));
  return violations.filter((v) => !known.has(v.id));
}

export interface AuditOutcome {
  surface: string;
  route: string;
  violations: AxeViolation[];
  newCritical: AxeViolation[];
}

/** Run the full sweep; persist evidence; return per-surface outcomes. */
export async function runAudit(
  pages: { surface: string; route: string; page: Page }[],
): Promise<AuditOutcome[]> {
  const baseline = loadBaseline();
  const outcomes: AuditOutcome[] = [];
  for (const { surface, route, page } of pages) {
    const violations = await scanSurface(page, route);
    const newCritical = newViolations(surface, violations, baseline).filter(
      (v) => v.impact === "critical",
    );
    outcomes.push({ surface, route, violations, newCritical });
  }

  // Evidence for the coverage matrix + Wave 35 re-verification.
  mkdirSync(RESULTS_DIR, { recursive: true });
  writeFileSync(
    path.join(RESULTS_DIR, "axe-results.json"),
    `${JSON.stringify({ generated: new Date().toISOString(), outcomes }, null, 2)}\n`,
  );

  // Baseline update mode: snapshot everything found, owners default to this lane.
  if (process.env.AXE_UPDATE_BASELINE === "1") {
    const updated = baseline;
    updated._meta = {
      updated: new Date().toISOString(),
      note: "Snapshot of axe violations per surface. NEW criticals not listed here FAIL the E2E gate; fix or re-snapshot deliberately (AXE_UPDATE_BASELINE=1).",
    };
    updated.surfaces = {};
    for (const o of outcomes) {
      updated.surfaces[o.surface] = o.violations.map((v) => ({
        id: v.id,
        impact: v.impact,
        owner: "p29-accessibility-deep",
      }));
    }
    writeBaseline(updated);
  }

  // Failure reporting (AXE_STRICT also fails on baselined criticals).
  const failures: string[] = [];
  for (const o of outcomes) {
    for (const v of o.newCritical) {
      failures.push(`${o.route}: NEW critical ${v.id} — ${v.help}`);
    }
    if (process.env.AXE_STRICT === "1") {
      const known = new Set((loadBaseline().surfaces[o.surface] ?? []).map((e) => e.id));
      for (const v of o.violations) {
        if (v.impact === "critical" && known.has(v.id)) {
          failures.push(`${o.route}: BASELINED critical still present: ${v.id} — ${v.help}`);
        }
      }
    }
  }
  if (failures.length > 0) {
    throw new Error(`axe audit failures (${failures.length}):\n${failures.join("\n")}`);
  }
  return outcomes;
}
