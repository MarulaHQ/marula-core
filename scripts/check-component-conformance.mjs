#!/usr/bin/env node
/**
 * check-component-conformance.mjs — Wave 48 lane 2 (`component-conformance-sweep`).
 *
 * p47-conformance's SURFACE-DELTAS ledger named four missing mechanisms, and
 * "component conformance" was one of them: whether each kit component's anatomy
 * matches its drawn counterpart (chips, pills, menus, filter bar, column
 * chooser, density switch). Neither existing tool can test it — check-shared-chrome
 * guards the stylesheet boundary and check-token-conformance guards colours,
 * and both take the kit's own class names as ground truth rather than the pack.
 *
 * This check reads BOTH sides every run:
 *
 *   1. THE INVENTORY — every component `packages/ui` ships is enumerated from
 *      the two public barrels (`.` and `./kit`), not from a hand-written list.
 *      The barrel grows, the inventory grows with it, and every new component
 *      must be mapped in the same PR.
 *
 *   2. THE MAP — each shipped component is mapped to its counterpart in the
 *      core surface pack (`brand/packs/core-surface-pack`, the same resolution
 *      rules scripts/check-surface-pack.mjs uses). A component left unmapped,
 *      or mapped to an anchor the pack no longer contains (a panel renamed or
 *      a drawing dropped), is drift and fails.
 *
 *   3. THE REVERSE RULE — the pack's component panels are enumerated from the
 *      pack source itself. A drawn panel with no claimed implementation fails,
 *      exactly the shape the row asks for ("a pack counterpart with no
 *      implementation").
 *
 *   4. ANATOMY — the areas p47 named get real assertions on both sides: the
 *      pack's density table is parsed and compared against the schema module's
 *      DENSITY_STEP_SPEC (loaded from source, so checker and tokens cannot
 *      drift — the posture check-theme-packs.mjs takes), and the chip / pill /
 *      builder / chooser / footer / bulk parts are asserted against kit-side
 *      needles.
 *
 *   5. THE RESIDUAL LEDGER — drawn counterparts nobody has implemented yet.
 *      The ledger may only SHRINK: a lane closes a row by deleting it in the
 *      same PR, and the row's `verify` is checked live, so deleting it without
 *      the work standing keeps the run red. Adding a row is a review-visible
 *      act, the same rule the token ledger carries.
 *
 * Exit codes: 0 clean · 1 violations · 2 the pack could not be resolved
 * (a clean checkout of core has no pack; so nothing is asserted and nothing is
 * claimed — run this where the pack exists, or set MARULA_PACK_ROOT). The
 * check is not wired into CI for the same reason check-surface-pack.mjs is
 * not; lanes run it at commit and gate time.
 *
 * Usage:
 *   node scripts/check-component-conformance.mjs [--self-test]
 */
import { existsSync, readFileSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { execFileSync } from "node:child_process";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const p = (rel) => resolve(REPO, rel);

/* -------------------------------------------------------------------------
 * Pack resolution — same rules as check-surface-pack.mjs
 * ---------------------------------------------------------------------- */

function defaultPackRoot() {
  if (process.env.MARULA_PACK_ROOT) return process.env.MARULA_PACK_ROOT;
  for (const c of ["../brand/packs/core-surface-pack", "brand/marula-mockups/marula-surface-pack"]) {
    if (existsSync(resolve(REPO, c, "manifest.json"))) return c;
  }
  // Walk up: a worktree lane may sit several levels from the org home.
  let dir = REPO;
  for (let i = 0; i < 6; i += 1) {
    const up = resolve(dir, "..");
    for (const rel of ["brand/packs/core-surface-pack", "brand/marula-mockups/marula-surface-pack"]) {
      if (existsSync(resolve(up, rel, "manifest.json"))) return resolve(up, rel);
    }
    dir = up;
  }
  return null;
}

const packRoot = defaultPackRoot();

/* -------------------------------------------------------------------------
 * 1. The inventory — enumerated from the barrels
 * ---------------------------------------------------------------------- */

const KIT_BARREL = "packages/ui/src/index.ts";
const KIT_UI_BARREL = "packages/ui/src/kit/index.ts";

/**
 * Parse a barrel's `export * from "./…"` lines into component modules.
 * Component modules live under components/, kit/ui/, kit/icons/ or editor/;
 * everything else (lib re-exports, token APIs) is not a component module.
 */
function enumerateBarrel(barrelPath) {
  const abs = p(barrelPath);
  if (!existsSync(abs)) return [];
  const src = readFileSync(abs, "utf8");
  const out = [];
  for (const m of src.matchAll(/export \* from "(\.[^"]+)"/g)) {
    const target = m[1];
  // Component modules sit under components/, kit/ui/, kit/icons/ or editor/
  // relative to their own barrel's directory.
  if (!/^(?:\.\/(?:components|editor|icons|ui))\//.test(target)) continue;
    out.push({
      id: target.split("/").pop(),
      module: join(dirname(p(barrelPath)), ...target.split("/").slice(1)),
    });
  }
  return out;
}

const shipped = [
  { barrel: "ui", path: KIT_BARREL, components: enumerateBarrel(KIT_BARREL) },
  { barrel: "kit", path: KIT_UI_BARREL, components: enumerateBarrel(KIT_UI_BARREL) },
];

const moduleTextCache = new Map();
function moduleText(module) {
  if (!moduleTextCache.has(module)) {
    const abs = module.endsWith(".tsx") ? module : `${module}.tsx`;
    moduleTextCache.set(module, existsSync(abs) ? readFileSync(abs, "utf8") : null);
  }
  return moduleTextCache.get(module);
}

/* -------------------------------------------------------------------------
 * 2. The map — component → pack counterpart
 *
 * Anchor needles are literal strings in the pack's own source HTML (the
 * drawings the pack shipped), kept distinctive so a rename or a dropped panel
 * breaks them. `kit` needles are the repo-side assertion each component's
 * implementation still carries the drawn parts. `standalone` marks plumbing
 * with no drawn counterpart; its `note` is the reason, and a standalone
 * without a reason fails.
 * ---------------------------------------------------------------------- */

const SK = "source/shared-kit.html";
const TL = "source/tickets-list.html";
const TODAY = "source/today.html";
const KV = "source/knowledge-vault.html";
const CL = "source/clients.html";

const PACK_COUNTERPARTS = {
  /* --- components/ — the `@marula-core/ui` barrel --- */
  "ui/badge": { anchors: [[SK, "Status and priority"]], note: "the tag/pill family" },
  "ui/assistant-card": { anchors: [[SK, "Assistant column: three states"]], note: "the pack's one inverted surface: proposal, evidence, Not this" },
  "ui/bulk-action-bar": {
    anchors: [[TL, 'class="tr bulk"']],
    kit: [["packages/ui/src/components.css", "m-bulk-bar"]],
    note: "the drawn bulk bar replaces the column header in place (§7)",
  },
  "ui/button": { anchors: [[SK, "Buttons"]] },
  "ui/card": { anchors: [[TODAY, 'class="cardx"']], note: "the attention cards on Today and the form families" },
  "ui/checkbox": { anchors: [[SK, "Selection"], [SK, 'type="checkbox"']] },
  "ui/column-chooser": {
    anchors: [[TL, 'aria-label="Choose columns"'], [TL, "Always shown"], [TL, 'aria-label="Reorder ']],
    kit: [["packages/ui/src/components.css", "m-column-chooser"]],
    note: "popover under the Columns button, drag grips, locked Ticket column (§5)",
  },
  "ui/composer-frame": {
    standalone:
      "plumbing for the composer's note/reply mode; the pack draws the composer on the surface sheets, not as a component panel",
  },
  "ui/data-toolbar": { anchors: [[TL, 'class="tb"']], note: "the toolbar row the data-surface template fixes (§5)" },
  "ui/density-switch": { anchors: [[SK, "Density"]], note: "the three steps control; numbers asserted in group A" },
  "ui/dialog": { anchors: [[SK, "Confirm dialog"], [SK, 'class="dlg"']] },
  "ui/empty-state": { anchors: [[SK, "Empty and error shapes"], [SK, 'class="eshape"']] },
  "ui/export-control": {
    anchors: [[TL, 'aria-label="Export to CSV"'], [TL, "Exactly what you see, nothing else."]],
    kit: [["packages/ui/src/components/export-control.tsx", '"row"'], ["packages/ui/src/components/export-control.tsx", '"filter"'], ["packages/ui/src/components/export-control.tsx", "sorted by"], ["packages/ui/src/components/export-control.tsx", '"column"']],
    note: "the export statement — rows, filters, sort, columns, stated before the commit (§8)",
  },
  "ui/field": { anchors: [[SK, "Inputs"]] },
  "ui/filter-bar": {
    anchors: [[TL, 'class="chips"'], [SK, 'class="chip add"'], [TL, 'class="chip more"']],
    kit: [["packages/ui/src/components.css", "m-filter-chips"], ["packages/ui/src/components.css", "m-filter-builder"]],
    note: "chips read as a sentence and collapse into +N more; the dashed Add filter ends the row (§6)",
  },
  "ui/input": { anchors: [[SK, "Inputs"], [SK, 'class="field"']], note: "the drawn field anatomy: default, focused, error, disabled" },
  "ui/kbd": { anchors: [[SK, 'class="kbd"']], note: "⌘K hint in the top bar; J/K hint chips in the footer" },
  "ui/select": {
    anchors: [[TL, 'aria-label="Field"'], [TL, 'aria-label="Operator"'], [TL, 'aria-label="Value"']],
    note: "the builder's field/operator/value rows are the drawn select shapes",
  },
  "ui/separator": { standalone: "layout utility; the pack separates with spacing, never with rules" },
  "ui/spinner": {
    standalone:
      "indeterminate loading ring; the pack's loading shapes are the skeleton family (see the skeleton-loader residual), and a spinner is the chrome-level fallback",
  },
  "ui/status-stepper": {
    anchors: [[KV, 'class="stp2"']],
    note: "the setup wizard's step list shows the pack's step anatomy: check for done, gold for current",
  },
  "ui/surface-columns": {
    anchors: [[TL, 'aria-label="Choose columns"']],
    note: "column identity, ordering and locking; the chooser (§5) is its drawn shape",
  },
  "ui/surface-footer": {
    anchors: [[TL, 'class="ft2"']],
    kit: [["packages/ui/src/components.css", "m-surface-footer"], ["packages/ui/src/components/surface-footer.tsx", "m-surface-footer-num"]],
    note: "the 52px strip — N of M, load-more, density slot, J/K hints; total optional",
  },
  "ui/surface-header": {
    anchors: [[TL, 'class="top"']],
    note: "the page title block fixed once by the pack — title, meta, actions",
  },
  "ui/table": { anchors: [[TL, 'class="tr th"'], [TL, 'class="tc"']], note: "header row and body cells as drawn" },
  "ui/time": { anchors: [[SK, "Timer bar, 60 px, ink in every theme"]] },
  "ui/toaster": { anchors: [[SK, "Toasts"], [SK, 'class="tst"']] },
  "ui/tooltip": { standalone: "hover affordance; static mockups do not draw hover" },
  "ui/view-switch": { anchors: [[TL, 'aria-label="Views"']], note: "My queue / Unassigned segmented control (§6)" },
  /* --- components/editor/ --- */
  "ui/block-menu": { standalone: "KB editor furniture; the pack draws editor page states, not the menu's anatomy" },
  "ui/prose": { standalone: "KB editor furniture; the pack draws editor page states, not the prose internals" },
  "ui/toolbar": { standalone: "KB editor toolbar; the editor is drawn on knowledge page states, not as a component panel" },
  /* --- kit/ui — the `@marula-core/ui/kit` barrel --- */
  "kit/Icon": { standalone: "the icon component; every pack drawing consumes glyphs inline" },
  "kit/app-rail": { anchors: [[SK, "Chrome: rail, top bar and timer bar"], [SK, 'class="railx"'], [SK, "7 items"], [SK, "9 items, expanded"]] },
  "kit/assistant-card": { anchors: [[SK, "Assistant column: three states"]] },
  "kit/avatar": {
    anchors: [[SK, 'class="avatar"'], [TL, 'class="av"'], [TL, 'class="av un"']],
    note: "initials avatar; the dashed unassigned state is drawn on the tickets sheet",
  },
  "kit/brand": { anchors: [[SK, "Top bar, 56 px"]], note: "the wordmark sits in the drawn top bar; the marks themselves carry brand-asset checks" },
  "kit/button": { anchors: [[SK, "Buttons"]] },
  "kit/callout": {
    standalone:
      "info/error note; the pack's closest drawn shape is the export popover's big-note, an app composition rather than a kit panel",
  },
  "kit/card": { anchors: [[TODAY, 'class="cardx"']] },
  "kit/checkbox": { anchors: [[SK, "Selection"], [SK, 'type="checkbox"']] },
  "kit/chip": {
    anchors: [[SK, 'class="chip"'], [SK, 'aria-label="Remove"'], [SK, 'class="chip add"'], [TL, 'class="chip more"']],
    kit: [
      ["packages/ui/src/kit/ui/chip.tsx", 'data-slot="chip"'],
      ["packages/ui/src/kit/ui/chip.tsx", 'data-slot="chip-add"'],
      ["packages/ui/src/kit/ui/chip.tsx", 'data-slot="chip-more"'],
    ],
    note: "chip anatomy asserted both sides: label + value + remove button, the dashed add chip, the +N more collapse",
  },
  "kit/data-row": { anchors: [[TL, 'class="tr th"'], [TL, 'class="tc"']], note: "ledger row anatomy: cells in a grid, selected and current states" },
  "kit/empty-state": { anchors: [[SK, "Empty and error shapes"]] },
  "kit/field": { anchors: [[SK, "Inputs"]] },
  "kit/kbd": { anchors: [[SK, 'class="kbd"']] },
  "kit/modal": { anchors: [[SK, "Confirm dialog"]] },
  "kit/pill": {
    anchors: [[SK, "Status and priority"]],
    kit: [
      ["packages/ui/src/kit/ui/pill.tsx", 'data-slot="pill"'],
      ["packages/ui/src/kit/ui/pill.tsx", "breached"],
      ["packages/ui/src/kit/ui/pill.tsx", "attention"],
    ],
    note: "status pills carrying the pack's colour-never-alone rule: red is breach or P1 only",
  },
  "kit/priority-chip": { anchors: [[SK, "Status and priority"]], kit: [["packages/ui/src/kit/ui/priority-chip.tsx", "P4"]] },
  "kit/progress": { standalone: "awaiting a drawn counterpart — no pack panel draws a determinate bar" },
  "kit/segmented": { anchors: [[SK, 'class="seg"']] },
  "kit/stat-card": { anchors: [[TL, 'class="stat"']], note: "the hero's figure tiles with their gold/blush dots" },
  "kit/strip": { standalone: "full-width attention strip; drawn only as surface compositions, not as a kit panel" },
  "kit/switch": { anchors: [[SK, "Selection"], [SK, 'class="tg"']] },
  "kit/tabs": { anchors: [[CL, 'class="tabs"']], note: "the record-detail tab row (clients template)" },
  "kit/toast": { anchors: [[SK, 'class="tst"']] },
  "kit/top-bar": { anchors: [[SK, "Top bar, 56 px"]] },
  "kit/marula-icons": { standalone: "the glyph sprite is an asset that the kit's own icon tests cover; the pack consumes glyphs inline" },
};

/* -------------------------------------------------------------------------
 * 3. Reverse coverage — the pack's drawn panels must be claimed
 *
 * Panels enumerated from the source's <h3> headings. Headings that are feature
 * areas rather than component families are declared here.
 * ---------------------------------------------------------------------- */

const NON_COMPONENT_PANELS = new Set(["Rules", "Rules that hold", "Workbench", "Client record", "None"]);

/* -------------------------------------------------------------------------
 * 5. The residual ledger — drawn counterparts with no implementation
 *
 * SHRINK ONLY. Close a row by deleting it in the PR that lands the work; each
 * row's `verify` is the mechanical assertion the deleting PR must leave green,
 * so the deletion is earned. Adding a row is a review-visible act.
 * ---------------------------------------------------------------------- */

function repoHas(rel, needles) {
  const abs = p(rel);
  if (!existsSync(abs)) return false;
  const src = readFileSync(abs, "utf8");
  return needles.every((n) => src.includes(n));
}

/** True when any file under rel (a tree of .d.ts and friends) mentions the header name. */
function grepTreeHas(needle, relUnderRepo) {
  const abs = p(relUnderRepo);
  if (!existsSync(abs)) return false;
  try {
    const out = execFileSync("grep", ["-rl", "-e", needle, "--no-messages", abs], { encoding: "utf8" });
    return out.trim().length > 0;
  } catch {
    return false;
  }
}

/** Some shipped component implements the drawn family (needle searchable in its module). */
function someShippedModuleServes(needle) {
  for (const { components } of shipped) {
    for (const c of components) {
      const text = moduleText(c.module);
      if (text !== null && text.includes(needle)) return true;
    }
  }
  return false;
}

const RESIDUAL_LEDGER = [
  {
    id: "skeleton-loader",
    panel: "Skeleton",
    drawn: "the pack draws the loading shape as flat skeleton blocks, no shimmer (S4)",
    missing:
      "no component in packages/ui ships the skeleton family — components.css carries the m-skel vocabulary entry but no module emits it, so surfaces hand-roll their loading splashes",
    ownedBy: "a follow-up row (place the owner's name here when it is adopted), not Wave 48 lane 3",
    verify: () => someShippedModuleServes("m-skel") || someShippedModuleServes("skeleton"),
  },
  {
    id: "x-total-count-header",
    drawn: "the footer's N of M — shown of total, never an invented denominator",
    missing:
      "SurfaceFooter takes `total`, but the list endpoints send no x-total-count, so adopted surfaces print `N {noun}` for want of the denominator — the contract addition re-pins via scripts/regen-openapi-pins.sh",
    ownedBy: "Wave 48 lane 3 (p48-surface-adoption)",
    verify: () =>
      repoHas("packages/sdk/src/client.ts", ["x-total-count"]) || grepTreeHas("x-total-count", "packages/sdk/src/generated"),
  },
];

/* -------------------------------------------------------------------------
 * Self-test — synthetic fixtures, no pack, no stack, no disk beyond the repo
 * ---------------------------------------------------------------------- */

function selfTest() {
  const failures = [];
  const assert = (label, ok) => {
    if (!ok) failures.push(label);
  };

  assert(
    "barrel enumeration matches a real line",
    /export \* from "(\.[^"]+)"/.exec('export * from "./components/button";')?.[1] === "./components/button",
  );
  const barrels = enumerateBarrel(KIT_BARREL);
  assert("barrel enumeration skips lib re-exports", barrels.every((c) => c.module.includes("/src/components/") || c.module.includes("/src/editor/")));
  assert("the kit barrel enumerates the handoff kit's modules", enumerateBarrel(KIT_UI_BARREL).some((c) => c.id === "chip"));

  const fakePack = `<h3>Toasts</h3><h3>Buttons</h3><span class="chip"><span class="v">Acme</span><button aria-label="Remove">x</button></span><span class="chip add">Add filter</span><span class="chip more">+1 more</span>[['Comfortable',48,44]]`;
  const density = parsePackDensity(fakePack);
  assert("the density parse reads the pack's label/row/control triples", density[0]?.label === "Comfortable" && density[0]?.row === 48 && density[0]?.control === 44);
  assert("the density parse stops at malformed rows", parsePackDensity("[['Comfortable',X,44]]").length === 0);

  const panels = ["Toasts", "Buttons", "Rules"];
  assert("panel enumeration reads only h3 text", enumeratePanels(fakePack).join(",") === panels.filter((x) => x !== "Rules").join(","));

  const textOf = { "source/fake.html": fakePack };
  assert("an anchor is satisfied only when every needle is present", anchorRead([["source/fake.html", '"Remove"']], (rel) => textOf[rel] ?? null));
  assert("a missing needle fails the anchor", !anchorRead([["source/fake.html", '"Broom handles"']], (rel) => textOf[rel] ?? null));
  assert("an anchor to a missing file fails", !anchorRead([["source/absent.html", '"Remove"']], (rel) => textOf[rel] ?? null));

  if (failures.length > 0) {
    console.error(`component-conformance detector self-test: ${failures.length} failed:`);
    for (const f of failures) console.error(`  FAIL  ${f}`);
    return 1;
  }
  console.log("component-conformance detector self-test: 9 checks pass (no pack, no stack)");
  return 0;
}

/** Every needle must appear in its file's text; a missing file fails. */
function anchorRead(anchors, textOf) {
  return anchors.every(([rel, needle]) => {
    const text = textOf(rel);
    return text !== null && text.includes(needle);
  });
}

function parsePackDensity(text) {
  const out = [];
  for (const m of text.matchAll(/\[['`](\w+)['`],(\d+),(\d+)\]/g)) {
    out.push({ label: m[1], row: Number(m[2]), control: Number(m[3]) });
  }
  return out;
}

function enumeratePanels(html) {
  return [...html.matchAll(/<h3>([^<$]{2,80})<\/h3>/g)].map((m) => m[1].trim());
}

if (process.argv.includes("--self-test")) {
  process.exit(selfTest());
}

/* -------------------------------------------------------------------------
 * Main
 * ---------------------------------------------------------------------- */

const passes = [];
const errors = [];
const check = (ok, label, extra = "") => {
  if (ok) passes.push(label);
  else errors.push(`${label}${extra ? ` — ${extra}` : ""}`);
};

if (!packRoot) {
  console.error(
    "exit 2 — the core surface pack was not resolved (looked at MARULA_PACK_ROOT, ../brand/packs/core-surface-pack and the local marula-mockups copies, walking up six levels).\n" +
      "Component conformance is asserted against the pack's source drawings, so this run asserts nothing. Run it where the org home is checked out, or point MARULA_PACK_ROOT at the pack.",
  );
  process.exit(2);
}

const packIntact = existsSync(join(packRoot, "manifest.json")) && existsSync(join(packRoot, SK));
check(packIntact, `pack resolved with manifest.json and ${SK}`, `root=${packRoot}`);

/* --- 1. Enumeration ------------------------------------------------------- */

const inventory = [];
for (const { barrel, path: barrelPath, components } of shipped) {
  check(components.length > 0, `${barrel} barrel enumerates at least one component module`, barrelPath);
  for (const c of components) {
    inventory.push(`${barrel}/${c.id}`);
    if (moduleText(c.module) === null) {
      errors.push(`${barrel} barrel re-exports "./…/${c.id}" but no file exists at ${c.module}(or .tsx)`);
    }
  }
}

/* --- 2. The map covers every shipped component ---------------------------- */

const unmapped = inventory.filter((key) => PACK_COUNTERPARTS[key] === undefined);
check(
  unmapped.length === 0,
  "every shipped component has a pack-counterpart map entry",
  unmapped.length ? `missing: ${unmapped.join(", ")}` : "",
);

const stale = Object.keys(PACK_COUNTERPARTS).filter((key) => !inventory.includes(key));
if (stale.length > 0) console.log(`map entries to delete (component no longer shipped): ${stale.join(", ")}`);

/* --- 3. Pack-side anchors still exist ------------------------------------- */

const packTextCache = new Map();
const packText = (rel) => {
  const key = `${packRoot}/${rel}`;
  if (!packTextCache.has(key)) {
    const abs = join(packRoot, rel);
    packTextCache.set(key, existsSync(abs) ? readFileSync(abs, "utf8") : null);
  }
  return packTextCache.get(key);
};
const kitText = (rel) => {
  const abs = p(rel);
  return existsSync(abs) ? readFileSync(abs, "utf8") : null;
};

let anchorChecks = 0;
for (const [key, entry] of Object.entries(PACK_COUNTERPARTS)) {
  if (!inventory.includes(key)) continue; // stale entries are handled above
  if (entry.standalone !== undefined) {
    if (!entry.standalone) errors.push(`${key}: standalone entry carries no reason (write the reason in standalone)`);
    continue;
  }
  for (const [rel, needle] of entry.anchors ?? []) {
    anchorChecks += 1;
    const text = packText(rel);
    check(text !== null && text.includes(needle), `${key}: pack anchor survives`, `${rel} no longer contains ${JSON.stringify(needle)}`);
  }
  for (const [rel, needle] of entry.kit ?? []) {
    anchorChecks += 1;
    const text = kitText(rel);
    check(text !== null && text.includes(needle), `${key}: kit side still asserts the drawn part`, `${rel} no longer contains ${JSON.stringify(needle)}`);
  }
  if ((entry.anchors ?? []).length === 0) errors.push(`${key}: mapped with no anchors and no standalone reason — it cannot be verified`);
}

/* --- 4. Reverse coverage --------------------------------------------------- */

const skText = packText(SK);
check(skText !== null, `the shared kit sheet is readable`, SK);
const sheetPanels = (skText ? enumeratePanels(skText) : []).filter((panel) => !NON_COMPONENT_PANELS.has(panel));

// A panel is claimed by a drawn entry whose anchor needle names it (panel name
// inside the needle, or the needle inside the panel).
const drawnByKey = Object.entries(PACK_COUNTERPARTS).filter(([key, e]) => e.standalone === undefined && inventory.includes(key));
check(drawnByKey.length > 0, "at least one drawn counterpart is mapped", "");

// A panel is claimed by a drawn entry whose anchor needle names it, or by a
// residual-ledger row that pins it as owned debt (the "pin with a reason" path
// the row allows).
const ledgerPanels = new Map(RESIDUAL_LEDGER.filter((r) => r.panel).map((r) => [r.panel, r.id]));

for (const panel of sheetPanels) {
  const claimedBy = drawnByKey.filter(([, e]) => (e.anchors ?? []).some(([, needle]) => needle.includes(panel) || panel.includes(needle))).map(([key]) => key);
  if (claimedBy.length > 0) continue;
  if (ledgerPanels.has(panel)) {
    console.log(`  pinned: pack panel "${panel}" has no implementation — residual "${ledgerPanels.get(panel)}" owns it`);
    continue;
  }
  check(false, `pack panel "${panel}" is claimed by a mapped implementation`, "a drawn counterpart with no implementation — map it or pin it with a reason");
}

/* --- Anatomy --------------------------------------------------------------- */

const DENSITY_STEP_KEYS = { Comfortable: "comfortable", Compact: "compact", Dense: "dense" };
const packDensity = parsePackDensity(skText ?? "");
check(packDensity.length === 3, "the pack's density table has exactly three steps", `parsed ${packDensity.length}`);
if (packDensity.length === 3) {
  const schemaUrl = pathToFileURL(join(REPO, "packages/ui/src/density.ts")).href;
  const schema = await import(schemaUrl).catch((e) => {
    console.error(`FAIL: cannot load packages/ui/src/density.ts: ${e.message}`);
    process.exit(1);
  });
  check(schema.DENSITY_STEP_SPEC !== undefined, "DENSITY_STEP_SPEC is exported from the schema module", "");
  if (schema.DENSITY_STEP_SPEC) {
    for (const { label, row, control } of packDensity) {
      const key = DENSITY_STEP_KEYS[label];
      if (!key) {
        errors.push(`pack density step "${label}" answers to no DENSITY_STEP_SPEC key — one side renamed the ladder`);
        continue;
      }
      const spec = schema.DENSITY_STEP_SPEC[key];
      check(spec && spec.row === row && spec.control === control, `density ${key} matches the pack drawing`, `pack ${row}/${control}, spec ${spec?.row}/${spec?.control}`);
    }
  }
} else {
  errors.push("could not parse the pack's density table — the S4 drawing changed; refresh the parse in the same PR");
}

/* --- The residual ledger ---------------------------------------------------- */

for (const row of RESIDUAL_LEDGER) {
  check(row.verify(), `residual "${row.id}" is still owed`, row.missing);
}

/* ------------------------------------------------------------------------- */

console.log(
  `component-conformance: ${inventory.length} shipped components · ${anchorChecks} anchors · ${sheetPanels.length} drawn panels · ${RESIDUAL_LEDGER.length} residual rows · pack ${packRoot ? "resolved" : "MISSING"}`,
);

if (errors.length > 0) {
  console.error(`\n${errors.length} component-conformance violation(s):`);
  for (const e of errors) console.error(`  ${e}`);
  process.exit(1);
}
console.log("Clean — every shipped component maps to the pack, every drawn family is claimed, the anatomy holds, and the residual ledger is honest.");
