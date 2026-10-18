/**
 * ESLint 9 flat config for @marula-core/web — Wave 25 lane 1 (p25-shadcn-lint).
 *
 * Purpose: make `docs/theme.md` + Workbench token policy MECHANICAL via
 * @shadcn/lint (an agent-first Tailwind design-system plugin), succeeding the
 * CI hex-scan posture (scripts/review-theme.mjs) for the app surface.
 * Non-purpose: TS/React-hooks correctness — that stays with `pnpm typecheck`
 * and packages/ui's own eslint config; those plugins are registered here only
 * so legacy `eslint-disable` directives still resolve.
 *
 * Oxlint-vs-ESLint decision (recorded in-lane, 2026-09-21): ESLint chosen.
 * Rationale: Oxlint's JS plugin API — the only way it can host this plugin —
 * is alpha (this plugin's oxlint support requires ≥1.80 alpha JS plugins);
 * this repo has no oxlint anywhere and introducing a second linter runtime
 * into the merge-gate is a bigger surface than one ESLint 9 flat config that
 * plugs straight into `turbo run lint`. ESLint 9.39 ≥ the 9.30 peer floor;
 * Node 24 ≥ 20.19 minimum.
 *
 * Posture history (WARN BASELINE preserved at
 * design/qa-p25-shadcn-lint/eslint.config.warn-baseline.mjs):
 *   warn run over apps/web = 4270 findings (no-inline-styles 2310 / 135 files,
 *   no-arbitrary-values 1649, no-restyle 229, no-unknown-classes 51).
 *   Triage classified the backlog (details in landing record):
 *   - ~2077 of 2310 inline styles are token-driven (`var(--m-*)` values) —
 *     the pattern the repo deliberately uses for dynamic status/tone/stagger
 *     values (no raw hexes survive the CI hex-scan). Carved out via the rule's
 *     allowlist, not per-file offs: dynamic values are inline-style-only by
 *     nature, and they are what keeps raw hexes at zero.
 *   - no-restyle findings: 90% fall on FOREIGN component props unknown to the
 *     grammar (`m-custom-*` category classes) — reclassified via the
 *     m-custom-* allow pending tokens.ts registration.
 *   - `"40ms"` stagger inline values: motion-law tokens (≤120ms) — motion
 *     values are legitimately JS-computed (stagger indexes), carved out and
 *     cross-checked by review-theme.mjs.
 *
 * RE-MEASURED 2026-10-12 (Wave 41 lane 16, `p41-leftovers-closeout`) — the
 * counts above had moved, and one of them had moved for a reason nobody had
 * measured. The plugin reads ONE stylesheet (`tailwind.css` in
 * components.json) and follows its `@import` chain to learn which class names
 * this project's Tailwind generates. `app/globals.css` imports `shell.css` and
 * `ledger.css` and nothing else; the other thirteen feature sheets are pulled
 * in by their own components with a plain `import "./x.css"`, which Next.js
 * bundles but the linter never sees. So every class those sheets declared was
 * "unknown": 1233 of the 1274 no-unknown-classes findings were class names that
 * DO have CSS, sitting outside the one chain the plugin reads.
 *   - The fix is `lint-theme.css` (lint-only, never bundled): it imports every
 *     sheet in the app, and components.json now points at it. The count went
 *     1274 → 40, and the residual 40 were real: one missing base class
 *     (`entry-field`, the label/control stack on nine auth surfaces — without
 *     it the inline label sits flush against the input) plus 20 vestigial
 *     names left over from the kit sweep whose CSS had been renamed.
 *   - It only works with `packages/ui/dist` built. Without dist the
 *     `@marula-core/ui/*.css` imports do not resolve, the Tailwind bridge
 *     fails and the rule falls back to the grammar bundled with the plugin —
 *     which does not know `rounded-ctl`. Wave 25's own verification step
 *     ("after a local `pnpm turbo run build --filter @marula-core/ui`") was
 *     load-bearing for a reason nobody had written down.
 *     NO LONGER TRUE, 2026-10-19: the kit's `exports` point at `src` and the
 *     app transpiles it, so `@marula-core/ui/*.css` resolves from source and
 *     the build is no longer load-bearing for this rule. Verified by deleting
 *     `packages/ui/dist` outright: lint stays at 0 errors.
 *   - Re-measured counts, warn posture, over the whole apps/web surface:
 *     no-unknown-classes 0 (was 51 at baseline, 1274 with the blind chain),
 *     require-static-classes 20 / 9 files, no-inline-styles 1520 / 112 files,
 *     no-arbitrary-values 2014 / 155 files.
 * Flip honesty: no-unknown-classes now has a clean sweep behind it and is at
 * error. require-static-classes does NOT flip — its 20 findings are all the
 * same shape, `className={HEAD_CELL_CLASS}` on a `<Th>`/`<Tr>`, one imported
 * identifier from `components/knowledge/ledger.tsx` that the rule does not
 * resolve across modules. Inlining that constant into nine table pages would
 * fragment one shared class string twenty ways to satisfy the linter, which is
 * the wrong trade. no-arbitrary-values and no-inline-styles stay warn with the
 * re-measured backlogs below.
 */
import { plugin as shadcn } from "@shadcn/lint";
import tsParser from "@typescript-eslint/parser";
import typescriptEslint from "@typescript-eslint/eslint-plugin";
import reactHooks from "eslint-plugin-react-hooks";
import { defineConfig, globalIgnores } from "eslint/config";

const THEME_NOTE =
  "Marula design doctrine lives in docs/theme.md (the Marula ink-and-gold token layer, Figtree/Fraunces/JetBrains Mono typography, the primary that flips ink-to-gold on mode, the non-flipping ink timer bar, the motion law at or under 120ms) and the vendored reference pack at design/mockups/marula-app-mockup-images/DESIGN-NOTES.md. Fix by using theme tokens and components. Never retype a hex and never restyle a primitive. Gold is never text on a light background: use --m-gold-text.";

export default defineConfig([
  globalIgnores(["node_modules/**", ".next/**", "public/**"]),
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    plugins: {
      shadcn,
      // Registered (not enabled) so pre-existing `eslint-disable` directives
      // resolve instead of erroring.
      "@typescript-eslint": typescriptEslint,
      "react-hooks": reactHooks,
    },
    settings: {
      shadcn: {
        // Primitives ship from @marula-core/ui (packages/ui/src/components).
        // apps/web/components.json points at @/components/ui only nominally —
        // no such directory exists; recognition must ride the workspace pkg.
        componentImports: ["^@marula-core/ui(/|$)"],
        ignoreImports: ["^@marula-core/ui/tokens$", "^@marula-core/ui/components\\.css$"],
        mergeFunctions: ["cn"],
        note: THEME_NOTE,
      },
    },
    rules: {
      // Mechanical wins — replace/augment the CI hex-scan posture. Clean
      // backlog after triage ⇒ held at error.
      "shadcn/no-raw-colors": "error",
      "shadcn/no-restyle": [
        "error",
        {
          // Permissive-correct base per the record (layout+typography only);
          // naming carve-outs are repo-owned classes (m-* motion/state
          // helpers, row-hover) whose styling is declared in globals.css —
          // allowed BY NAME so the rule can still see schema drift elsewhere.
          allow: ["layout", "typography", "m-*", "row-hover"],
          contracts: [
            { pattern: "^Card$", allow: ["layout", "typography", "spacing", "shape", "motion", "effects", "m-*", "row-hover"] },
            { pattern: "^CardContent$", allow: ["layout", "typography", "spacing", "m-*", "row-hover"] },
            { pattern: "^Input$", allow: ["layout", "typography", "spacing", "m-*", "row-hover"] },
            { pattern: "^Tr$", allow: ["layout", "typography", "spacing", "shape", "m-*", "row-hover"] },
            { pattern: "^Td$", allow: ["layout", "typography", "spacing", "shape", "m-*", "row-hover"] },
            { pattern: "^Tag$", allow: ["layout", "typography", "spacing", "m-*", "row-hover"] },
            { pattern: "^Button$", allow: ["layout", "typography", "spacing", "m-*", "row-hover"] },
          ],
          message: {
            color: "Color goes through theme tokens/sources — {{component}} already carries one; restyling it invites drift from docs/theme.md.",
            spacing: "Do not set spacing on {{component}} — use a size ({{sizes|none}) or spacing on the parent.",
            shape: "Let the component own radius; the 0.75rem control radius is doctrine.",
            default: "Use a {{component}} variant ({{variants|none defined}}) or spacing/typography on parent — see docs/theme.md §6.",
          },
        },
      ],
      "shadcn/no-inline-styles": [
        // Re-measured 2026-10-12: 1520 findings / 112 files, and NOT the
        // token-driven backlog the 96 baseline implied. The top properties are
        // display 209, fontSize 168, gap 127, padding 115, fontWeight 98,
        // alignItems 79, margin 74, borderRadius 73 — static layout and
        // typography written inline, which the allowlist below (colour,
        // token-driven values, motion) deliberately does not cover. Flipping
        // requires the sweep to classes, not an allowlist widening; adding
        // those props to `allow` would silence the rule instead of enforcing
        // it. Recorded in design/qa-p41-leftovers-closeout/LANDING.md.
        "warn",
        // token-driven + motion-law carve-outs in one mechanical allowlist
        { allow: ["color", "background", "background-color", "background-image", "border-color", "border", "border-bottom", "border-top", "border-left", "border-right", "fill", "stroke", "grid-template-columns", "grid-template-rows", "grid-template-areas", "grid-column", "grid-row", "transform", "transition-duration", "animation-delay", "transition-delay", "width", "height", "max-height", "max-width", "top", "left", "right", "bottom", "position", "opacity", "z-index"] },
      ],
      // Re-measured 2026-10-12: 2014 findings / 155 files, of which ~1741 are
      // off-scale font sizes (text-[13px] 594, text-[12px] 523, text-[11px]
      // 300, text-[10px] 138, and sixteen other sizes down to text-[9px]). The
      // Wave 25 prediction held: this is blocked on a type-scale decision plus
      // a whole-scale migration, not on a lint fix. Stays warn.
      "shadcn/no-arbitrary-values": ["warn", { allow: ["rounded-[var(--m-radius-container)]", "rounded-[var(--m-radius-control)]", "rounded-[0.5rem]", "rounded-[0.75rem]"] }],
      // PROMOTED 2026-10-12 (Wave 41 lane 16). The 1274 baseline findings were
      // a tooling blind spot, not debt: the app's thirteen feature sheets sit
      // outside the plugin's `@import` chain (see the header note). With
      // lint-theme.css importing every sheet the count fell to 40 real
      // findings, `entry-field` got the CSS it was missing, and 20 vestigial
      // names from the kit sweep were dropped. Clean sweep at zero ⇒ error.
      "shadcn/no-unknown-classes": "error",
      // STAYS WARN (2026-10-12): 20 findings / 9 files, every one the same
      // shape — `className={HEAD_CELL_CLASS}` on a table header cell, an
      // imported identifier from components/knowledge/ledger.tsx that the rule
      // does not resolve across modules. Inlining it into nine pages would
      // fragment one shared class string; the code is already static in spirit.
      "shadcn/require-static-classes": "warn",
    },
  },
  // ---- Carve-outs: styles are OWNED by these surfaces ----
  {
    // Tiptap editor internals: ProseMirror nodes/marks render stylesheets and
    // attribute-controlled classes we do not own as Tailwind surfaces.
    plugins: { shadcn },
    files: ["lib/editor/**"],
    rules: {
      "shadcn/no-restyle": "off",
      "shadcn/no-inline-styles": "off",
      "shadcn/no-raw-colors": "off",
      "shadcn/no-unknown-classes": "off",
      "shadcn/require-static-classes": "off",
      "shadcn/no-arbitrary-values": "off",
    },
  },
  {
    // recharts chart tooltip styling reuses computed theme-token style
    // objects — recharts' Tooltip/CartesianGrid want style props, not classes.
    plugins: { shadcn },
    files: ["components/report-preview.tsx", "components/dashboards/widget-charts.tsx", "components/dashboards/report-widget.tsx"],
    rules: {
      "shadcn/no-inline-styles": "off",
      "shadcn/no-unknown-classes": "off",
      "shadcn/require-static-classes": "off",
    },
  },
  // tests intentionally exercise views; no-restyle contracts still apply, so
  // only the warn-only rules are silenced there.
  {
    plugins: { shadcn },
    files: ["tests/**"],
    rules: {
      "shadcn/no-unknown-classes": "off",
      "shadcn/require-static-classes": "off",
    },
  },

  {
    // Wave 41 lane 20 — the kit is the one door for primitives and icons
    // (design/marula-handoff-kit/ui-package/eslint.snippet.mjs). The app
    // imports components from @marula-core/ui or @marula-core/ui/kit, never
    // Phosphor directly and never the design folder itself.
    files: ["**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          paths: [
            {
              name: "@phosphor-icons/react",
              message: "Use the kit's Icon door: import { Icon } from '@marula-core/ui/kit'.",
            },
          ],
          patterns: [
            {
              group: ["**/design/marula-handoff-kit/**", "@marula-core/ui/src/kit/**"],
              message: "Import the vendored kit from '@marula-core/ui/kit', not the design folder or a deep path.",
            },
          ],
        },
      ],
    },
  },
]);
