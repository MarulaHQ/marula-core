#!/usr/bin/env node
// Lighthouse budget sweep for the web app and the marketing site.
//
// Purpose: turn the Lighthouse numbers into a gate. This started as the p34
// verdict walk's smoke sweep — seven marketing routes, no thresholds, any
// non-zero exit counted as failure — and now carries the surface budget the
// frontend performance lane owes: the workspace and portal routes the app
// actually renders, with score floors and Core Web Vitals ceilings that fail
// the run.
//
// Budgets (mobile preset unless --desktop; the harder of the two, so a pass
// here is a pass on desktop):
//
//   scores   performance ≥ 0.85   accessibility ≥ 0.95
//            best-practices ≥ 0.90  seo ≥ 0.90   (seo is budgeted only for
//                                               public and marketing routes —
//                                               an authenticated app page has
//                                               no search surface to score)
//   metrics  LCP ≤ 2500 ms        CLS ≤ 0.1      TBT ≤ 300 ms
//
// The metric ceilings are the web.dev "good" thresholds; the score floors are
// set one step above the "needs improvement" cut so a regression shows up as a
// budget breach rather than a slow drift.
//
// Authenticated routes need a session. Pass one as a Cookie header:
//
//   LIGHTHOUSE_COOKIE="marula_session=…" node scripts/p34-lighthouse-sweep.mjs
//
// Without it the authenticated groups are reported SKIPPED (not scored as their
// login redirect) and the run still exits 0 for the groups it did measure. Add
// --require-auth to make a missing session a failure — that is the form the
// group gate uses, where the stack and the seeded session are both present.
//
// Usage:
//   node scripts/p34-lighthouse-sweep.mjs                  # web app, all groups
//   node scripts/p34-lighthouse-sweep.mjs --target website # marketing site
//   node scripts/p34-lighthouse-sweep.mjs --group workspace --group portal
//   node scripts/p34-lighthouse-sweep.mjs --desktop --out design/qa-…/lighthouse
//
// Environment: WEB_URL (default http://localhost:3000), SITE_URL
// (default http://localhost:3100), LIGHTHOUSE_BIN (default npx),
// LIGHTHOUSE_COOKIE, LIGHTHOUSE_OUT.
//
// Exit codes: 0 every measured route inside budget; 1 a breach; 2 setup problem
// (lighthouse missing, no routes selected, a route redirected away).
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const SCORE_BUDGET = {
  performance: 0.85,
  accessibility: 0.95,
  "best-practices": 0.9,
  seo: 0.9,
};

/** web.dev "good" thresholds, in the units the audit reports. */
const METRIC_BUDGET = {
  "largest-contentful-paint": { ceiling: 2500, unit: "ms" },
  "cumulative-layout-shift": { ceiling: 0.1, unit: "" },
  "total-blocking-time": { ceiling: 300, unit: "ms" },
};

/** Audit ids → the label printed in the summary. */
const METRIC_LABEL = {
  "largest-contentful-paint": "LCP",
  "cumulative-layout-shift": "CLS",
  "total-blocking-time": "TBT",
};

/**
 * Route surfaces. `auth` marks a group that needs LIGHTHOUSE_COOKIE; `seo` marks
 * a group whose pages are meant to be found by a search engine.
 */
const TARGETS = {
  web: {
    baseUrl: process.env.WEB_URL ?? "http://localhost:3000",
    groups: {
      workspace: {
        auth: true,
        seo: false,
        routes: [
          ["dashboards", "/dashboards"],
          ["reports", "/reports"],
          ["kb", "/kb"],
          ["today", "/today"],
          ["tickets", "/tickets"],
        ],
      },
      portal: {
        auth: true,
        seo: false,
        routes: [
          ["portal-tickets", "/portal/tickets"],
          ["portal-kb", "/portal/kb"],
          ["portal-schedule", "/portal/schedule"],
        ],
      },
      public: {
        auth: false,
        seo: true,
        routes: [
          ["login", "/login"],
          ["portal-login", "/portal/login"],
        ],
      },
    },
  },
  website: {
    baseUrl: process.env.SITE_URL ?? "http://localhost:3100",
    groups: {
      marketing: {
        auth: false,
        seo: true,
        routes: [
          ["index", "/"],
          ["ticketing", "/ticketing"],
          ["compare", "/compare"],
          ["billing", "/billing"],
          ["client-portal", "/client-portal"],
          ["integrations", "/integrations"],
          ["developers", "/developers"],
        ],
      },
    },
  },
};

function parseArgs(argv) {
  const options = {
    target: "web",
    groups: null,
    desktop: false,
    requireAuth: false,
    out: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--target") options.target = argv[++i];
    else if (arg === "--group") (options.groups ??= []).push(argv[++i]);
    else if (arg === "--desktop") options.desktop = true;
    else if (arg === "--require-auth") options.requireAuth = true;
    else if (arg === "--out") options.out = argv[++i];
    else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: node scripts/p34-lighthouse-sweep.mjs [--target web|website] [--group <name>]… [--desktop] [--require-auth] [--out <dir>]",
      );
      process.exit(0);
    } else {
      console.error(`lighthouse-budget: unknown argument ${arg}`);
      process.exit(2);
    }
  }
  return options;
}

/** Run one Lighthouse pass and return the parsed report. */
function lighthouse(url, { outFile, desktop, cookie }) {
  const bin = process.env.LIGHTHOUSE_BIN ?? "npx";
  const args = [
    ...(bin === "npx" ? ["--yes"] : []),
    // `npx lighthouse` resolves the CLI; LIGHTHOUSE_BIN wins when a checkout
    // pins one, which is what a CI image does.
    ...(bin === "npx" ? ["lighthouse"] : []),
    url,
    "--output=json",
    `--output-path=${outFile}`,
    "--quiet",
    "--chrome-flags=--headless --no-sandbox --disable-dev-shm-usage",
  ];
  if (desktop) args.push("--preset=desktop");
  if (cookie !== undefined) args.push(`--extra-headers=${JSON.stringify({ Cookie: cookie })}`);
  const result = spawnSync(bin, args, { stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`lighthouse exited ${result.status} for ${url}`);
  }
  return JSON.parse(readFileSync(outFile, "utf8"));
}

/** Compare one report against the budget; returns the breach list. */
function breaches(report, { seo }) {
  const failures = [];
  const scores = {};
  for (const [category, floor] of Object.entries(SCORE_BUDGET)) {
    if (category === "seo" && !seo) continue;
    const score = report.categories?.[category]?.score;
    if (score === null || score === undefined) continue;
    scores[category] = Math.round(score * 100);
    if (score < floor) failures.push(`${category} ${Math.round(score * 100)} < ${Math.round(floor * 100)}`);
  }
  const metrics = {};
  for (const [auditId, budget] of Object.entries(METRIC_BUDGET)) {
    const value = report.audits?.[auditId]?.numericValue;
    if (value === undefined || value === null) continue;
    const rounded = Math.round(value * 100) / 100;
    metrics[METRIC_LABEL[auditId]] = budget.unit === "ms" ? Math.round(rounded) : rounded;
    if (rounded > budget.ceiling) {
      failures.push(`${METRIC_LABEL[auditId]} ${rounded}${budget.unit} > ${budget.ceiling}${budget.unit}`);
    }
  }
  return { failures, scores, metrics };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const target = TARGETS[options.target];
  if (target === undefined) {
    console.error(`lighthouse-budget: unknown target ${options.target} (expected ${Object.keys(TARGETS).join(" or ")})`);
    process.exit(2);
  }
  const selected = Object.entries(target.groups).filter(
    ([name]) => options.groups === null || options.groups.includes(name),
  );
  if (selected.length === 0) {
    console.error(`lighthouse-budget: no group matched ${options.groups?.join(", ")}`);
    process.exit(2);
  }

  const outDir = path.resolve(
    options.out ?? process.env.LIGHTHOUSE_OUT ?? "design/qa-p52-frontend-perf-budget/lighthouse",
  );
  mkdirSync(outDir, { recursive: true });

  const cookie = process.env.LIGHTHOUSE_COOKIE;
  const mode = options.desktop ? "desktop" : "mobile";
  const summary = { target: options.target, mode, baseUrl: target.baseUrl, routes: [] };
  const failures = [];
  let authSkipped = 0;

  for (const [groupName, group] of selected) {
    if (group.auth && cookie === undefined) {
      const message = `${groupName}: SKIPPED — authenticated group, no LIGHTHOUSE_COOKIE`;
      console.log(message);
      summary.routes.push({ group: groupName, skipped: "no-session" });
      authSkipped++;
      continue;
    }
    for (const [name, route] of group.routes) {
      const url = `${target.baseUrl}${route}`;
      const outFile = path.join(outDir, `${name}-${mode}.json`);
      let report;
      try {
        report = lighthouse(url, { outFile, desktop: options.desktop, cookie: group.auth ? cookie : undefined });
      } catch (error) {
        failures.push(`${name}: ${error.message}`);
        summary.routes.push({ group: groupName, name, route, error: error.message });
        continue;
      }
      const finalUrl = report.finalDisplayedUrl ?? report.finalUrl ?? url;
      if (group.auth && !finalUrl.replace(/\/$/, "").endsWith(route.replace(/\/$/, ""))) {
        // A session that expired scores the login page under a workspace name,
        // which is worse than no number at all.
        const message = `${name}: redirected to ${finalUrl} — the session cookie is not valid for this run`;
        failures.push(message);
        summary.routes.push({ group: groupName, name, route, redirectedTo: finalUrl });
        continue;
      }
      const verdict = breaches(report, { seo: group.seo });
      const measured = {
        group: groupName,
        name,
        route,
        scores: verdict.scores,
        metrics: verdict.metrics,
        failures: verdict.failures,
        report: path.relative(process.cwd(), outFile),
      };
      summary.routes.push(measured);
      if (verdict.failures.length > 0) failures.push(`${name}: ${verdict.failures.join("; ")}`);
      const scoreLine = Object.entries(verdict.scores)
        .map(([k, v]) => `${k.slice(0, 4)} ${v}`)
        .join("  ");
      const metricLine = Object.entries(verdict.metrics)
        .map(([k, v]) => `${k} ${v}`)
        .join("  ");
      console.log(`${verdict.failures.length === 0 ? "ok  " : "FAIL"} ${name.padEnd(18)} ${scoreLine}  | ${metricLine}`);
    }
  }

  writeFileSync(path.join(outDir, "summary.json"), `${JSON.stringify(summary, null, 2)}\n`);

  if (authSkipped > 0 && options.requireAuth) {
    failures.push(`${authSkipped} authenticated group(s) skipped: --require-auth was set but LIGHTHOUSE_COOKIE is unset`);
  }
  if (failures.length > 0) {
    console.error(`\nlighthouse-budget: ${failures.length} failure(s)`);
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log(`\nlighthouse-budget: every measured route inside budget (${mode}). Summary: ${path.relative(process.cwd(), path.join(outDir, "summary.json"))}`);
}

main();
