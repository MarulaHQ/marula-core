# Meridian Visual Theme — "Tangerine" (shadcn preset)

> **Owner-override banner (2026-09-15, Wave 15 lane `p15-theme-tangerine`, user directive):**
> This identity **supersedes the Workbench identity** (2026-08-25). The Workbench hard rules
> "monochrome ≤5% / Ember interactive-only" (§2 rule 4) and "no serif faces" (§4) are
> **explicitly overridden** by the Tangerine decision, the same way the 2026-09-09 Inter-override
> superseded the earlier "Inter-only" rule. Everything else in the Workbench doctrine —
> instant-first envelope, motion law (≤120ms, transform/opacity, prefers-reduced-motion),
> responsive light+dark parity, semantic PSA status colors, token enforcement — remains binding.
> Canonical machine source: `design/tangerine/tangerine.json` (vendored from
> `https://tweakcn.com/r/themes/tangerine.json`). Do not hand-edit hexes outside `packages/ui/src/tokens.css`.

> **Status:** Active · **Overhaul applied:** 2026-09-15 (Wave 15 lane `p15-theme-tangerine` — full transformation, not token-only)
> **Superseded identity:** "Workbench" (monochrome + Ember `#D64B0A`, 2px radius, no serif, cool neutral `#F4F5F7`/`#101216`)
> **New identity:** **Tangerine** — warm-neutral light (`#E8EBED`), blue-slate dark (`#1C2433`), tangerine primary `#E05D38`, pill tags `999px`, Source Serif 4 headings, generous spacing, soft `rgba(0,0,0,.10)` shadows.
> **Scope:** binding on all Meridian UI code, docs site, and marketplace surfaces.

## 1. Design stance

**A workbench with presence.** Dense, keyboard-first surfaces remain; the Tangerine preset adds warmth (light canvas `#E8EBED`), an indigo-slate dark mode (`#1C2433`), and editorial craft (serif headings, pill badges, rounded controls). Color is purposeful: interactive affordances, status tags, and the chart ramp — never decorative gradients or illustration.

## 2. Identity definition

| Aspect | **Meridian — Tangerine** |
|---|---|
| Brand color | **Tangerine** — preset primary `#E05D38` (`oklch(0.6397 0.172 36.4421)`), identical in light and dark mode. The Ember brand signal (`#D64B0A`) is superseded by the preset primary. |
| Temperature | **Light: warm neutral** (`#E8EBED`) · **Dark: blue-slate** (`#1C2433`) — per the preset, not a mirrored grey |
| Headers | **Source Serif 4** for headings (h1–h6, card headers, dialog titles); **Inter** body; **JetBrains Mono** code/data with tabular numerals |
| Corners | **0.75rem** on buttons, inputs, cards, tags; **pill tags (`999px`)** for status indicators |
| Depth | Soft preset shadows (`rgba(0,0,0,.10)` scale) on overlays; visible 1px borders (`#DCDFE2` light / `#3D4354` dark) |
| Feel | "Workbench with presence" |

**Hard rules:**
1. Every hex lives in `packages/ui/src/tokens.css` — the single hex source. The shadcn layer (`apps/web/app/globals.css`) bridges to `--m-*` variables for shared values and carries preset-only values verbatim as `oklch()`. No off-palette hexes in feature code; CI hex-scan enforces.
2. **Tangerine (`#E05D38`)** marks interactivity: primary buttons, links, focus rings (2px offset outline), active nav indicators.
3. **Semantic status colors follow PSA conventions** and survive the swap: green success `#15803D`/`#4ADE80`, amber warning `#B45309`/`#FBBF24`, red danger `#B91C1C`/`#F87171`. The preset's `destructive` (`#EF4444`) applies to solid destructive buttons (shadcn `--destructive`), not status tags.
4. **Colorful presence, not chaos.** Interactive elements, status pills, and the chart ramp carry color; surfaces stay neutral and text stays readable ink (`#333333` / `#E5E5E5`).
5. **Both modes first-class.** Light and dark ship from the same token set; dark is the preset's own blue-slate family, not a mirrored grey.
6. **Primary-button pairing note (spec tradeoff):** white on `#E05D38` ≈ 3.63:1 — the preset's own pairing; above the 3:1 large/UI floor, below normal-text AA. Accepted as the pinned tradeoff (documented in `tokens.test.ts`).

---

## 3. Color palette

### 3.0 Layering contract (single source of truth)

- `packages/ui/src/tokens.css` — the **only** file that may declare hexes. Holds the primitive ramp + semantic `--m-*` layer.
- `apps/web/app/globals.css` — shadcn/Tailwind variable layer. References `--m-*` for every value both layers share; preset-only values (accent tints, sidebar, muted scale) appear as verbatim `oklch()` copied from the vendored preset. The layers cannot drift because shared values are bridged, not redeclared.
- `packages/ui/src/tokens.ts` — `PALETTE` mirrors tokens.css hexes 1:1 (test-enforced).
- Vendored canonical preset: `design/tangerine/tangerine.json`.

### 3.1 Core UI (light mode)

| Role | Hex | Source |
|---|---|---|
| Canvas | `#E8EBED` | preset background |
| Surface / card | `#FFFFFF` | preset card |
| Text (primary) | `#333333` | preset foreground |
| Text (secondary) | `#4B5563` | preset secondary-foreground |
| Text (muted) | `#6B7280` | preset muted-foreground |
| Border | `#DCDFE2` | preset border |
| Primary / accent (interactive) | `#E05D38` | preset primary |
| Accent tint (hover surfaces, shadcn `--accent`) | `oklch(0.9119 0.0222 243.8174)` / fg `oklch(0.3791 0.1378 265.5222)` | preset accent (oklch-only, shadcn layer) |
| Destructive (solid buttons) | `#EF4444` | preset destructive |
| Sidebar | `oklch(0.903 0.0046 258.3257)` | preset sidebar (oklch-only) |

### 3.2 Core UI (dark mode)

| Role | Hex | Source |
|---|---|---|
| Canvas | `#1C2433` | preset dark background (blue-slate) |
| Surface / card | `#2A3040` | preset dark card |
| Overlay / popover | `#262B38` | preset dark popover |
| Text (primary) | `#E5E5E5` | preset dark foreground |
| Text (secondary/muted) | `#A3A3A3` | preset dark muted-foreground |
| Border / input | `#3D4354` | preset dark border |
| Primary | `#E05D38` | preset keeps primary identical in dark |
| Accent tint (shadcn `--accent`) | `oklch(0.338 0.0589 267.5867)` / fg `oklch(0.8823 0.0571 254.1284)` | preset dark accent (oklch-only) |
| Destructive (solid buttons) | `#EF4444` | preset destructive, identical both modes |
| Sidebar | `oklch(0.31 0.0283 267.7408)` | preset dark sidebar (oklch-only) |

### 3.3 Semantic status (PSA conventions — survive the swap)

Status tags are pills (`999px`) and keep industry-standard colors for AA on tinted backgrounds:

| Meaning | Light | Dark |
|---|---|---|
| Success / Resolved | `#15803D` on `#DCFCE7` | `#4ADE80` on `#14532D` |
| Warning / SLA risk | `#B45309` on `#FEF3C7` | `#FBBF24` on `#78350F` |
| Danger / Breach | `#B91C1C` on `#FEE2E2` | `#F87171` on `#7F1D1D` |
| Neutral / Waiting | `#4B5563` on `#E5E7EB` | `#A3A3A3` on `#2A303E` |

### 3.4 Chart ramp (preset)

`--m-chart-1..5`: `#86A7C8`, `#EEA591`, `#5A7CA6`, `#466494`, `#334C82` — dark mode swaps chart-2 to `#E6A08F` (preset dark variant). Chart primaries ride `chartTokensFor(theme)` (`apps/web/lib/chart-tokens.ts`), which materializes the ramp for SVG.

### 3.5 Ember (previous brand) — retired

The Workbench Ember hexes (`#D64B0A` / `#FF8A4C`) are retired and CI-blacklisted. Interactive color is the preset primary `#E05D38` in both modes.

---

## 4. Typography

| Role | Family | Weight | Notes |
|---|---|---|---|
| **UI / body / headings (default)** | **Inter**, system-ui fallback | 400 / 500 / 600 / 700 | 13px default. **Everything defaults to Inter** — including h1–h6 and `font-semibold` labels. next/font serves the webfont (`--font-sans`). |
| Display headings (opt-in) | **Source Serif 4** | 600–700 | Only via the `font-heading` utility on genuinely large display titles. The app's h1–h6 are mostly small section labels (`text-sm font-semibold`) and stay Inter. Tailwind's `font-serif` maps to the same webfont. **OWNER DIKTAT 2026-09-18: mockup-v2 wins — `--font-serif` is defined but never rendered there, so Inter-only display type is the canonical bar. De-serif surface card display titles (commit titles/labels) back to Inter (`font-semibold`); Source Serif now appears ONLY where explicitly themed above (the AuthBrand wordmark, docs headers). Lanes must not treat serif display titles as the default.** |
| Code / data / logs | **JetBrains Mono** | 400 | next/font (`--font-mono`); tabular numerals mandatory |

The Workbench "no serif" rule is **superseded** (see override banner), but the practical
contract is **Inter-first**: serif is an opt-in accent for display typography, never applied
blanket to h1–h6 (owner clarification 2026-09-15 after QA — `font-semibold` labels must render
sans). No decorative serif in body text.

## 5. Shape, density, motion

- **Radius:** `0.75rem` on controls and containers; `999px` for pill tags; `50%` avatars/spinners only.
- **Borders over shadows:** visible 1px borders on containers; the preset shadow scale (`rgba(0,0,0,.10)`) is reserved for overlays.
- **Density:** 32px control height, 28px row height, 20px card padding — generous, not sparse.
- **Motion:** transitions ≤120ms, transform/opacity only; `prefers-reduced-motion` collapses animations to 0.01ms. **Subtle, tasteful animation is encouraged — do not ship statically** (owner directive 2026-09-15): entrance/transition motion (fade-up reveals, staggered list entrances, panel slide-ins, hover micro-feedback, tab/filter morphs) brings pages and transitions to life. Constraints are performance, not austerity: transform/opacity only (never layout properties — no width/height/top/left animation), short and snappy (≤120ms, minimal easing, no spring physics), stagger ≤40ms/item, and everything collapses under `prefers-reduced-motion`.
- **Keyboard-first:** 2px offset focus outline in `#E05D38`; inline shortcut hints; every primary flow mouse-independent.

---

## 6. Component styling

- **Buttons:** 0.75rem radius, weight 500–700. Primary = solid `#E05D38`, white text (spec tradeoff §2.6). Secondary = surface + 1px border. Destructive = `#EF4444` solid/outline.
- **Status indicators:** pill tags (`999px`, weight 700, 11px) using §3.3 pairs — never squared.
- **Tables:** sticky headers, 28px rows, hover via surface-hover mix.
- **Cards:** `#FFFFFF` / `#2A3040`, 0.75rem radius, 1px border, 20px padding.
- **Charts:** preset ramp §3.4; gridlines at border color; no gradients.
- **Forms:** labels above fields; validation in danger color with icon — never color alone.
- **Empty states:** one sentence + one action. No illustrations.

## 7. Logo / mark direction

- Wordmark: "Meridian" in Source Serif 4 SemiBold.
- Mark: meridian line crossing a globe wireframe; the meridian stroke carries `#E05D38` — one stroke of tangerine in an otherwise neutral mark.
- No gradients; survives 16px favicon and single-color print.

## 8. Voice

Dry, precise, documentation-grade — unchanged by the visual overhaul.

## 8.5 Theme-token schema v1 + preset range (Wave 24 `p24-theming-system`)

**The Tangerine token set behind this file is generalized into a VERSIONED theme-token schema** (`THEME_TOKEN_SCHEMA_VERSION = 1`): a theme is a map of LIGHT/DARK values over the CLOSED semantic-role vocabulary (the §3 roles — background/surface/overlay/border/text tiers, accent/on-accent, destructive, the four status pairs, chart-1..5). Motion (§5), typography (§4), radius, and the shadow scale are NOT part of the vocabulary on purpose — no theme can restyle them, so docs/theme.md override rules win by construction.

- **Built-in presets (shipped, selectable org-wide with per-user override):** `tangerine` (the pinned **default-v1**, byte-identical to tokens.css — zero visual change to Applied core surfaces), `slate` (neutral tool-grade grey), `copper` (brand-warm), `high_contrast` (accessibility). Machine source: `packages/ui/src/theme-tokens.ts` (frontend) + `crates/core-domain/src/theme_pack.rs` (backend boundary); application is a `data-preset` attribute on `<html>` (attribute-scoped token overrides at the same specificity — the unset attribute IS the Tangerine baseline, so nothing changes unless the org or user picks a preset).
- **Settings keys (typed registry):** `appearance.theme` (ORG scope — preset id or `pack:<slug>`), `appearance.theme_override` (USER scope — `org_default` sentinel wins user→org, same precedence as the sticky AI model picks), `appearance.mode` (USER scope — system/light/dark, U6-safe supersedes of the flat old user `appearance.theme` key via migration 0097).
- **Theme packs (leg B) ride ADR 0009 V1** — marketplace registry / pinned GitHub / local zip staged imports, `artifact_kind: "theme"`, **zero capabilities** (the ingestion gate refuses capability declarations: topics restyle and nothing else). The manifest carries ONLY token overrides; **typed rejection of off-schema keys** (a `font_family` key is a 422 with the key named, never a silent ignore).
- **Import by export (tweakcn / shadcn):** the pack manifest may be a RAW tweakcn export (the vendored `design/tangerine/tangerine.json` is the exemplar shape — `{ name, cssVars: { light, dark } }` over the shadcn variable vocabulary, frequently `oklch()`) or a shadcn registry-item theme (same `cssVars` shape). The ingestion seam coerces: identity from the export's `name` (version 1.0.0 + current schema version stamped), oklch → sRGB hex, shadcn variable names mapped onto schema roles (background/card/popover/border/foreground/secondary-foreground/muted-foreground/primary/primary-foreground/destructive/chart-N). Unmapped export variables (sidebar ramps, hover tints, radius, fonts) are DROPPED — the schema stays restyle-only; derived tones recompute client-side. The coerced manifest must pass the same validate() gate (contrast floors included), so an export whose converted basics fail AA is refused outright: upload your export file as the zip's `manifest.json` and it imports with no hand translation.
- **Status pairs note:** shadcn/tweakcn exports carry no success/warning tags; the PSA status colors (§3.3) survive every pack — exactly per rule 3 above.

## 9. Enforcement

- Tokens live in `packages/ui/src/tokens.css` (+ `tokens.ts` PALETTE mirror); CI hex-scan (`.github/workflows/ci.yml`) fails on raw hex outside token files and blacklists the retired Workbench hexes.
- **`@shadcn/lint` (Wave 25 lane 1) mechanizes the token layer for `apps/web`:** `pnpm turbo run lint` runs ESLint 9 + `@shadcn/lint` over 277 files with `settings.shadcn.componentImports` on `@meridian/ui`. At `error`: `no-raw-colors` (raw palette colors — succeeds the hex-scan posture) and `no-restyle` (permissive-correct `layout+typography` base with per-component contracts; component styling stays owned by `packages/ui/src/**`, Tiptap editor internals, and the recharts tooltip style props via explicit carve-outs). At `warn` pending post-Wave-25 sweeps: `no-inline-styles` (96 token-driven findings), `no-arbitrary-values` (1476, mostly the `text-[13px]` off-scale font size), `no-unknown-classes` (51 `m-wb-*` classes needing `@utility` declarations), `require-static-classes`. Every finding's error text appends this file + mockup v2 as the fix reference.
- `packages/ui/tests/tokens.test.ts` proves hex-source uniqueness, PALETTE sync, dark-mode completeness, and WCAG floors.
- `scripts/review-theme.mjs` asserts the generated layers against the vendored `design/tangerine/tangerine.json` (value-level comparison, dark-mode anchors, chart ramp). Wave 24 note: the per-theme hex-scan cross-checks the `no-raw-colors` settings (`apps/web/eslint.config.mjs`).
- **`scripts/check-theme-packs.mjs` (Wave 24 leg C) — the PER-THEME gate:** runs the retired-hex blacklist AND the AA/3:1 floors over EVERY shipped built-in preset (both modes), plus any installed theme-pack manifests passed as CLI args / `THEME_PACK_MANIFESTS` (CI loops per installed theme); it enforces the closed role vocabulary, so an off-schema token in any theme is mechanically rejected, not applied.
- Reference mockups: `design/mockups/meridian-ticket-workbench-html-mockup-v2.html` (+ `…-v2-light.png` / `…-v2-dark.png`) — **the canonical aesthetic reference** (includes the dark-mode toggle and both modes); the earlier v1 mockup is kept for history only. QA evidence under `design/qa-tangerine/`. Wave 25 lint evidence under `design/qa-p25-shadcn-lint/`.

<!-- END -->
