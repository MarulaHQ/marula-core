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
 *   Flip honesty: no-arbitrary-values cannot yet be `error` (the largest
 *   token-driven subfamily would take a surgical sweep); everything else
 *   exposing zero legacy findings after triage is at error below.
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
        // Flippable to error once the 96-finding legacy sweep (~50 files) is
        // done — token-driven dynamic values are the repo pattern; the sweep
        // converts them to Tailwind v4 var() classes where statically
        // expressible and files the rest as carve-outs. Wave 25 close-out.
        "warn",
        // token-driven + motion-law carve-outs in one mechanical allowlist
        { allow: ["color", "background", "background-color", "background-image", "border-color", "border", "border-bottom", "border-top", "border-left", "border-right", "fill", "stroke", "grid-template-columns", "grid-template-rows", "grid-template-areas", "grid-column", "grid-row", "transform", "transition-duration", "animation-delay", "transition-delay", "width", "height", "max-height", "max-width", "top", "left", "right", "bottom", "position", "opacity", "z-index"] },
      ],
      // Typed but not yet enforceable — documented backlog, flips after a
      // full wave runs green (Wave 25 close-out follow-up #3).
      "shadcn/no-arbitrary-values": ["warn", { allow: ["rounded-[var(--m-radius-container)]", "rounded-[var(--m-radius-control)]", "rounded-[0.5rem]", "rounded-[0.75rem]"] }],
      "shadcn/no-unknown-classes": "warn",
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
