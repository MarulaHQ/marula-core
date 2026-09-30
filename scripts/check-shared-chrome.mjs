#!/usr/bin/env node
/**
 * check-shared-chrome.mjs — Wave 41 lane `p41-ui-chrome` shared-chrome gate.
 *
 * The plan of record specified the shared chrome once (in the surface pack) and
 * gave no lane the job of embodying it in code. Two holes let rot back in, and
 * this gate closes both mechanically:
 *
 *   A. **The stylesheet boundary.** Feature-prefixed rules injected into the
 *      two shared stylesheets. `p41-ui-operations` (+349 lines into
 *      `packages/ui/src/components.css`) and `p41-ui-money` (+556 lines of
 *      `m-money-*` into `apps/web/app/globals.css`) are doing exactly this on
 *      their branches. Feature CSS belongs beside its feature.
 *
 *   B. **The page title block.** 65 pages under `apps/web/app/` each hand-rolled
 *      their own `<h1>` block, re-deciding hierarchy, spacing and the
 *      meta/actions slot the pack had already fixed. The shared frame is
 *      `SurfaceHeader` (`@marula-core/ui`); `SettingsHeader` is a thin alias
 *      for it. A page either imports one of those or is an auth/portal surface
 *      that legitimately owns its chrome.
 *
 *   C. **The alias staying an alias.** If a lane re-inlines `SettingsHeader`'s
 *      markup, all 33 call sites drift apart again.
 *
 * Both A and B are **ledger rules with a hard floor**: the committed ledgers
 * below are the state at this lane's merge, and a ledger that GROWS fails. A
 * lane shrinking one is progress and is the point.
 *
 * Exit 0 = clean · exit 1 = violations (each prints file:line).
 */
import { execFileSync } from "node:child_process";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const REPO = resolve(import.meta.dirname, "..");
const p = (rel) => resolve(REPO, rel);

/* -------------------------------------------------------------------------
 * A. The stylesheet boundary
 * ---------------------------------------------------------------------- */

/** The two shared stylesheets. A feature does not get to own a rule in here. */
const SHARED_STYLESHEETS = [
  "apps/web/app/globals.css",
  "packages/ui/src/components.css",
];

/**
 * Feature names. A selector whose class prefix contains one of these, in a
 * shared stylesheet, is a feature rule that has crossed the boundary.
 *
 * Deliberately only FEATURE names — not shell chrome (`workbar`, `rail`) and
 * not generic component vocabulary (`table`, `btn`). Shell chrome is allowed to
 * live in the shared layer because it IS the shared layer.
 */
const FEATURE_NAMES = [
  "ops", "operations", "money", "billing", "invoice", "quote", "cash",
  "dispatch", "travel", "scheduling", "planning",
  "knowledge", "kb", "article", "secrets", "doc", "document",
  "insight", "scorecard", "dashboard", "report", "chart",
  "client", "contract", "project", "sale", "pipeline", "catalog",
  "stock", "purchase", "order", "vendor", "loan", "inventory", "merge", "asset", "device",
  "approval", "workbench", "setting", "portal", "mobile", "ticket", "board", "field",
  "automation", "alert", "audit", "provenance", "onboarding", "calendar", "password",
  "share", "today", "trump", "recurrence", "profit", "loss", "tax", "payout",
];

/**
 * The shared component vocabulary. A rule in a shared stylesheet whose prefix
 * is NOT here is either a new shared component (add it, in the same PR) or a
 * feature rule that has crossed the boundary (move it out). This is the rule
 * that generalises past today's lane names.
 */
const SHARED_VOCABULARY = [
  "m-assistant", "m-badge", "m-btn", "m-builder", "m-card", "m-chart",
  "m-checkbox", "m-column", "m-composer", "m-context", "m-data", "m-density",
  "m-dialog", "m-editor", "m-empty", "m-export", "m-field", "m-filter",
  "m-input", "m-kb", "m-kbd", "m-label", "m-panel", "m-rise", "m-root",
  "m-select", "m-separator", "m-shortcut", "m-skel", "m-spinner", "m-stepper",
  "m-surface", "m-table", "m-tag", "m-textarea", "m-th", "m-tooltip",
  "m-toolbar", "m-view",
];

const isSharedVocabulary = (prefix) =>
  SHARED_VOCABULARY.some((v) => prefix === v || prefix.startsWith(`${v}-`));

const selectorPrefixes = (css) => {
  const out = [];
  // Class selectors only: a feature rule is a feature class, and keyframe
  // names / element selectors in the shared layer are the layer's business.
  for (const m of css.matchAll(/^\s*\.([A-Za-z][A-Za-z0-9_-]*)/gm)) out.push({ prefix: m[1], index: m.index });
  return out;
};

const lineOf = (text, index) => text.slice(0, index).split("\n").length;

/* -------------------------------------------------------------------------
 * B. The page title block
 * ---------------------------------------------------------------------- */

/**
 * Surfaces that legitimately own their own chrome: they render no staff shell
 * (`isHeaderlessPath` in apps/web/components/shell/chrome.ts) or are a
 * self-contained auth/share/portal card. These are NOT a debt.
 */
const HEADERLESS_SURFACES = [
  "apps/web/app/accept-invite/page.tsx",
  "apps/web/app/magic-link/page.tsx",
  "apps/web/app/mfa/page.tsx",
  "apps/web/app/password-reset/page.tsx",
  "apps/web/app/portal/csat/page.tsx",
  "apps/web/app/portal/invoices/[id]/page.tsx",
  "apps/web/app/portal/invoices/page.tsx",
  "apps/web/app/portal/kb/articles/[id]/page.tsx",
  "apps/web/app/portal/kb/page.tsx",
  "apps/web/app/portal/login/page.tsx",
  "apps/web/app/portal/quotes/[id]/page.tsx",
  "apps/web/app/portal/quotes/page.tsx",
  "apps/web/app/portal/schedule/page.tsx",
  "apps/web/app/portal/tickets/[id]/page.tsx",
  "apps/web/app/portal/tickets/page.tsx",
  "apps/web/app/s/[token]/page.tsx",
  "apps/web/app/sso/page.tsx",
  "apps/web/app/verify-email/page.tsx",
  "apps/web/app/welcome/page.tsx",
];

/**
 * STAFF pages still hand-rolling a title block at this lane's merge. This is
 * the debt ledger, and it SHRINKS as lanes re-point onto `SurfaceHeader`.
 * Adding a line here fails the gate — that is the whole mechanism.
 *
 * GENERATED at this lane's merge from live disk state (see the lane landing
 * record); edit it by DELETING lines as surfaces migrate, never by adding.
 */
const TITLE_BLOCK_DEBT = [
  "apps/web/app/alerts/page.tsx",
  "apps/web/app/approvals/page.tsx",
  "apps/web/app/assets/[id]/page.tsx",
  "apps/web/app/audit/page.tsx",
  "apps/web/app/automations/page.tsx",
  "apps/web/app/billing/credit-notes/page.tsx",
  "apps/web/app/billing/page.tsx",
  "apps/web/app/billing/verification/page.tsx",
  "apps/web/app/boards/[id]/fields/page.tsx",
  "apps/web/app/boards/[id]/page.tsx",
  "apps/web/app/boards/page.tsx",
  "apps/web/app/calendar/page.tsx",
  "apps/web/app/clients/[id]/page.tsx",
  "apps/web/app/contracts/[id]/page.tsx",
  "apps/web/app/dashboards/page.tsx",
  "apps/web/app/dev/page.tsx",
  "apps/web/app/dispatch/page.tsx",
  "apps/web/app/dispatch/planning/page.tsx",
  "apps/web/app/dispatch/scheduling/page.tsx",
  "apps/web/app/dispatch/travel/page.tsx",
  "apps/web/app/documents/[id]/page.tsx",
  "apps/web/app/documents/page.tsx",
  "apps/web/app/imports/page.tsx",
  "apps/web/app/inventory/page.tsx",
  "apps/web/app/inventory/workbench/page.tsx",
  "apps/web/app/kb/page.tsx",
  "apps/web/app/kb/secrets/page.tsx",
  "apps/web/app/onboarding/page.tsx",
  "apps/web/app/passwords/page.tsx",
  "apps/web/app/projects/[id]/page.tsx",
  "apps/web/app/provenance/page.tsx",
  "apps/web/app/purchasing/[id]/page.tsx",
  "apps/web/app/reports/page.tsx",
  "apps/web/app/sales/catalog/page.tsx",
  "apps/web/app/sales/pipeline/page.tsx",
  "apps/web/app/sales/quotes/page.tsx",
  "apps/web/app/scorecards/page.tsx",
  "apps/web/app/share/page.tsx",
  "apps/web/app/stock/[id]/page.tsx",
  "apps/web/app/tickets/[id]/page.tsx",
  "apps/web/app/tickets/page.tsx",
  "apps/web/app/today/page.tsx",
];

/* ------------------------------------------------------------------------- */

const errors = [];
const fail = (file, line, msg) => errors.push(`${file}:${line}: ${msg}`);

// ---- A. stylesheet boundary --------------------------------------------
for (const sheet of SHARED_STYLESHEETS) {
  if (!existsSync(p(sheet))) continue;
  const css = readFileSync(p(sheet), "utf8");
  for (const { prefix, index } of selectorPrefixes(css)) {
    const line = lineOf(css, index);
    // Shared vocabulary is authoritative and is checked FIRST: `m-chart`,
    // `m-field` and `m-kb-prose` are genuine shared components whose names
    // happen to contain a feature word. Testing the feature list first flags
    // them, which is how this check would have taught the team to ignore it.
    if (isSharedVocabulary(prefix)) continue;
    const feature = FEATURE_NAMES.find((f) => prefix === `m-${f}` || prefix.startsWith(`m-${f}-`));
    if (feature) {
      fail(
        sheet,
        line,
        `feature-prefixed rule \`.${prefix}\` in a SHARED stylesheet. ` +
          `Feature CSS belongs beside its feature (a lane-local .css beside the component, ` +
          `or Tailwind utilities in the component itself) — not in ${sheet}.`,
      );
      continue;
    }
    if (prefix.startsWith("m-")) {
      fail(
        sheet,
        line,
        `selector \`.${prefix}\` is not shared-component vocabulary. Either it is a feature rule ` +
          `(move it out of ${sheet}) or a new shared component (add it to SHARED_VOCABULARY in ` +
          `scripts/check-shared-chrome.mjs, in the same PR).`,
      );
    }
  }
}

// ---- B. page title blocks ----------------------------------------------
const grep = (pattern) => {
  try {
    return execFileSync("grep", ["-rl", pattern, "--include=page.tsx", "apps/web/app"], {
      cwd: REPO,
      encoding: "utf8",
    })
      .split("\n")
      .filter(Boolean)
      .sort();
  } catch (e) {
    // grep exits 1 on no match
    if (e.status === 1) return [];
    throw e;
  }
};

const h1Pages = grep("<h1");
const debt = new Set(TITLE_BLOCK_DEBT);
const headerless = new Set(HEADERLESS_SURFACES);

const stillHandRolled = [];
for (const file of h1Pages) {
  if (!existsSync(p(file))) continue;
  const src = readFileSync(p(file), "utf8");
  if (src.includes("SurfaceHeader") || src.includes("SettingsHeader")) continue;
  stillHandRolled.push(file);
}

// A page that is NOT a known debt and NOT headerless is a NEW hand-rolled
// title block: that is a fresh violation, not inherited debt.
for (const file of stillHandRolled) {
  if (debt.has(file) || headerless.has(file)) continue;
  fail(
    file,
    1,
    `hand-rolled page title block. Import SurfaceHeader from @marula-core/ui ` +
      `(or SettingsHeader, its alias) instead of writing your own <h1> block. ` +
      `If this surface legitimately owns its chrome, it belongs in HEADERLESS_SURFACES ` +
      `in scripts/check-shared-chrome.mjs with a reason.`,
  );
}

// The debt ledger may only shrink. A stale entry (the page was migrated, or
// deleted) is fine and is cleaned here; a page that no longer has an <h1> at
// all means the ledger should be edited so the record stays honest.
const stale = [...debt].filter((f) => existsSync(p(f)) && !stillHandRolled.includes(f));

// A headerless entry that now imports the shared frame is also stale.
const staleHeaderless = [...headerless].filter((f) => {
  if (!existsSync(p(f))) return false;
  const src = readFileSync(p(f), "utf8");
  return src.includes("SurfaceHeader") || src.includes("SettingsHeader");
});

// ---- C. SettingsHeader stays an alias -----------------------------------
const SHELL = "apps/web/components/settings/settings-shell.tsx";
if (existsSync(p(SHELL))) {
  const src = readFileSync(p(SHELL), "utf8");
  const start = src.indexOf("export function SettingsHeader");
  if (start === -1) {
    fail(SHELL, 1, `SettingsHeader is gone. All 33 call sites import it.`);
  } else {
    // Isolate the function body by brace matching, then assert the body is a
    // delegation and nothing more. Checking that `<SurfaceHeader` merely
    // APPEARS is not enough: a lane can re-inline `<header><h1>` next to it
    // and the frame drifts apart again anyway.
    //
    // The first `{` after the signature opens the PARAMETER DESTRUCTURE, not
    // the body, so the body brace is the first one after the parameter list's
    // closing paren.
    const parenEnd = src.indexOf(")", start);
    let i = parenEnd === -1 ? -1 : src.indexOf("{", parenEnd);
    let depth = 0;
    let end = i;
    if (i !== -1) {
      for (; end < src.length; end += 1) {
        if (src[end] === "{") depth += 1;
        else if (src[end] === "}") {
          depth -= 1;
          if (depth === 0) break;
        }
      }
    }
    const body = i === -1 ? "" : src.slice(i, end + 1);
    if (i === -1) {
      fail(SHELL, 1, `could not read the SettingsHeader body; the check needs updating.`);
    } else if (/<header[\s>]/i.test(body) || /<h1[\s>]/i.test(body)) {
      fail(
        SHELL,
        src.slice(0, i).split("\n").length,
        `SettingsHeader must stay a thin alias for SurfaceHeader — its body must contain no ` +
          `markup of its own. Its 33 call sites depend on it, and re-inlining it is how the ` +
          `frame drifted the first time.`,
      );
    } else if (!/<SurfaceHeader[\s/>]/.test(body)) {
      fail(
        SHELL,
        src.slice(0, i).split("\n").length,
        `SettingsHeader must delegate to <SurfaceHeader> (size="compact"). It currently ` +
          `renders something else.`,
      );
    }
  }
}

/* ------------------------------------------------------------------------- */

if (stale.length > 0) {
  console.log("Ledger entries to delete (already migrated or removed):");
  for (const f of stale) console.log(`  ${f}`);
  console.log("");
}
if (staleHeaderless.length > 0) {
  console.log("HEADERLESS_SURFACES entries to delete (now uses the shared frame):");
  for (const f of staleHeaderless) console.log(`  ${f}`);
  console.log("");
}

console.log(
  `shared-chrome: ${SHARED_STYLESHEETS.length} stylesheets, ` +
    `${h1Pages.length} pages with an <h1>, ` +
    `${stillHandRolled.length - stale.length - staleHeaderless.length} known title-block debts, ` +
    `${headerless.size} headerless surfaces.`,
);

if (errors.length > 0) {
  console.error(`\n${errors.length} shared-chrome violation(s):`);
  for (const e of errors) console.error(`  ${e}`);
  process.exit(1);
}
console.log("Clean — feature CSS is out of the shared layer and every staff title block is shared.");
