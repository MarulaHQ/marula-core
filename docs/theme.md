# Marula Visual Theme

> **Status:** Active · **Applied:** 2026-09-30 (Wave 41 lane 2, `p41-marula-theme`)
> **Identity:** **Marula** — ink and gold on a warm neutral canvas
> **Canonical machine source:** `design/marula/marula.json` (vendored tweakcn export) plus the rendered pack in `brand/marula-mockups/marula-app-mockup-images/DESIGN-NOTES.md`
> **Binding on:** all Marula UI code, the docs site, and marketplace surfaces

## 0. Retired identities

Three identities have been retired. This section is the tombstone: the values
below are CI-blacklisted, and a hex-scan failure naming one of them means
something reached back across a boundary it should not have.

| Retired | When | Its signature values | Replaced by |
|---|---|---|---|
| **Workbench** | 2025-11 | Ember `#D64B0A` / `#FF8A4C`, cool neutral `#F4F5F7` / `#101216` / `#171A1F` / `#12161C`, 2px radius, no serif | Tangerine, then Marula |
| **Tangerine** | Wave 24, retired 2026-09-30 | Primary `#E05D38`, canvas `#E8EBED`, blue-slate dark `#1C2433` / `#2A3040` / `#3D4354`, chart-1 `#86A7C8` | Marula |
| **Ember** | 2025-11, superseded by Tangerine | `#D64B0A` / `#FF8A4C` | Preset primary, now gold |

Two Workbench rules survive the swap and stay binding: the instant-first
envelope and the motion law (§5). One is overridden, the "no serif faces" rule.
A third, "monochrome ≤5%", is satisfied rather than overridden, because Marula
spends colour on affordances and on status, and nowhere else.

## 1. Design stance

**A workbench with a fruit's confidence.** The density and keyboard-first
posture of the first workbench are unchanged. What changed is the material:
an ink-on-warm-neutral canvas, with gold held back for the few places where
something is live. Colour carries meaning. It never decorates.

## 2. Identity definition

| Aspect | **Marula** |
|---|---|
| Brand colour | **Marula gold** `#E8B83A`, with **dusk ink** `#2B2A33` as its counterweight. The two are one system: the primary button is one of them filled with the other. |
| Temperature | **Light: warm neutral** `#F8F7F4` (Neutral) or `#FBF7EC` (cream) · **Dark: warm charcoal** `#2B2A33` |
| Headers | **Figtree** for the interface, **Fraunces** for the wordmark plus page greetings and marketing, and **JetBrains Mono** for code and data with tabular numerals mandatory |
| Corners | **10px** on buttons and inputs, **16–18px** on cards, **12px** scale root, **999px** for pill tags |
| Depth | Almost flat. A surface tone and a 1px border do the work. Shadows are for floating layers |
| Feel | Calm, warm, deliberate |

**Hard rules:**

1. Every hex lives in `packages/ui/src/tokens.css`, the single hex source. The
   shadcn layer (`apps/web/app/globals.css`) bridges to `--m-*` for every
   value the two layers share, and carries preset-only values verbatim as
   `oklch()`. The CI hex-scan enforces it.
2. **The primary flips on mode.** Light is an ink button with gold text, dark is
   a gold button with ink text. Both clear 4.5:1, so the label needs no
   large-text exemption. Marula has no single brand-button colour, and no
   token may be written as if it did.
3. **Gold is never text on a light background.** It measures 1.7:1 on cream.
   Gold-coloured text on light is `--m-gold-text` `#9A5B0F`. Gold fills, and
   gold at display scale. It does neither.
4. **The timer bar is ink in every theme.** It does not follow the mode flip.
   The token file says so in as many words, because a mechanical dark-mode
   pass will otherwise invert it along with everything else.
5. **Semantic status colours follow PSA convention** and survive every theme
   swap (§3.4). A theme restyles surfaces. It does not redefine what a colour
   means.
6. **Both modes are first class.** Dark is a warm charcoal, not a mirrored grey.

---

## 3. Colour palette

### 3.0 Layering contract (single source of truth)

- `packages/ui/src/tokens.css` — the **only** file permitted to declare hexes.
  It holds the brand primitive ramp and the semantic `--m-*` layer.
- `apps/web/app/globals.css` — the shadcn/Tailwind variable layer. It reaches
  `--m-*` for every value the two layers share, and carries preset-only values
  as verbatim `oklch()` copied from the vendored export. They cannot drift,
  because shared values are bridged rather than redeclared.
- `packages/ui/src/tokens.ts` — `PALETTE` mirrors the tokens.css hexes 1:1
  (test-enforced). `TOKEN` is the same set addressed by name. Nothing indexes
  `PALETTE` by position, because its order is the hex-scan contract.
- Vendored canonical export: `design/marula/marula.json`.
- Rendered ground truth: the pack in
  `brand/marula-mockups/marula-app-mockup-images/DESIGN-NOTES.md` §7, forty
  tokens across three theme columns.

### 3.1 Brand primitives (mode-independent)

| Role | Hex | Note |
|---|---|---|
| Marula gold | `#E8B83A` | the primary in dark, and the "act here" colour |
| Gold-soft | `#F7E3A8` | the interactive **tint** |
| Gold-text | `#9A5B0F` | deep amber, the only gold that may be text on light |
| Dusk ink | `#2B2A33` | the text colour, the primary in light, and the timer bar |
| Ink-dark | `#222129` | the bar in dark mode. Still ink, never cream |
| Flesh cream | `#F6EFD9` | the dark text colour, and the assistant surface in dark |
| Canopy green | `#5E7F3A` | done, healthy, the Core edition |
| Deep canopy | `#4A6A2A` | small text on a canopy fill, 5.4:1 |
| Unripe lime | `#A9C25D` | success on dark, and the leaf |
| Ripe blush | `#D9822B` | due soon; P2 |
| Bushveld sand | `#CDB38B` | borders, and the illustration strokes |
| Bark grey | `#7C746A` | the neutral mid-tone, and the Cloud edition |

### 3.2 Core UI (light, the `marula` Neutral column)

| Role | Hex |
|---|---|
| Canvas | `#F8F7F4` |
| Surface / card | `#FFFFFF` |
| Icon rail (`--m-nav`) | `#FFFFFF` |
| Context column (`--m-sidebar`) | `#EFEEEA` |
| Timer bar | `#2B2A33` |
| Text / secondary / muted | `#2B2A33` / `#5F584F` / `#6F685D` |
| Border (decorative) | `#E2E0DA` |
| Input border (3:1, its own token) | `#8C877C` |
| Primary / on-primary | `#2B2A33` / `#E8B83A` |
| Accent tint / on-tint | `#F7E3A8` / `#6B4308` |
| Accent text | `#9A5B0F` |
| Focus ring | `#A8641A` |
| Destructive | `#B42318` |

The warm column (`marula-warm`) is the same system on a cream canvas: `#FBF7EC`
background, `#E6DCC2` border, `#6B6357` muted. Every other value is identical.

### 3.3 Core UI (dark)

| Role | Hex |
|---|---|
| Canvas | `#2B2A33` |
| Surface / card | `#34333D` |
| Icon rail / context column | `#222129` |
| Timer bar | `#222129` |
| Text / secondary / muted | `#F6EFD9` / `#BEB5A2` / `#B0A894` |
| Border | `#43424D` |
| Primary / on-primary | `#E8B83A` / `#2B2A33` |
| Accent tint / on-tint | `#3A3944` / `#E8B83A` |
| Focus ring | `#E8B83A` |
| Destructive | `#FB8A80` |

Dark is gold-forward, so the accent text can be gold itself, since gold on
ink measures 7.7:1. Dark hovers **recede** instead of lightening: lightening a
tint toward cream would drop gold from 6.5:1 to 3.9:1.

### 3.4 Semantic status (PSA conventions, fixed across every theme)

| Meaning | Light | Dark |
|---|---|---|
| Success / Resolved | `#3B5222` on `#E4EDCB` | `#D6E5A8` on `#2E3A25` |
| Warning / SLA risk | `#6B4308` on `#FBEBBF` | `#F2CD6B` on `#4A3A1A` |
| Due today / P2 | `#7A3F0C` on `#F8DCC0` | `#F0B27A` on `#4D2F14` |
| Danger / Breach | `#B42318` on `#FDE8E5` | `#FB8A80` on `#3B1A18` |
| Neutral / Waiting | `#4A443D` on `#EFECE6` | `#E2CFA9` on `#3A342A` |

Due-today has its own pair rather than reusing warning. The two read as the
same urgency at a glance and would collapse into one row in a review.

### 3.5 Chart ramp

`--m-chart-1..5` is gold, canopy, blush, bark, lime. Light leads with the ring
gold `#A8641A`, because gold as a 2px stroke on cream is invisible. Dark leads
with gold `#E8B83A`. One leading hue, two grounds, so a chart reads the same either
way. Chart colours materialise through `chartTokensFor(theme)` in
`apps/web/lib/chart-tokens.ts`, which resolves SVG presentation attributes that
cannot read CSS custom properties.

### 3.6 The assistant inversion

One surface per screen inverts: the assistant card is ink with cream text in
light, cream with ink text in dark. Its internals are five tokens, not three:
`--m-asb` surface, `--m-asf` foreground, `--m-asbtn` the action,
`--m-asbtnf` that action's text, `--m-asline` the divider inside the card.
That last one measures 2.02:1 and is decorative. WCAG 1.4.11 permits that,
because the rule identifies no control and no state.

### 3.7 Timeline entry tints

Five classes, each a tint paired with a text colour: client-visible, internal
note, system, time and assistant. The tint carries the class and a small
uppercase tag names it, and neither is load-bearing alone. The tokens are
`--m-client/-note/-sys/-time/-agent`.

---

## 4. Typography

| Role | Family | Weight | Notes |
|---|---|---|---|
| **Interface, body, headings** | **Figtree**, system-ui fallback | 300–900 | 14px default. The 26px ticket title is Figtree. `--font-sans`. |
| **Display (opt-in, narrow)** | **Fraunces**, Georgia fallback | 100–900 | The wordmark, page greetings and marketing only. Never dense UI, never a ticket title, never a table cell. `--font-heading`. |
| **Code, data, logs** | **JetBrains Mono** | 100–800 | Tabular numerals mandatory. `--font-mono`. |

All three are `next/font/local` against woff2 vendored in `LEGAL/fonts/`, with
the OFL licence for each sitting beside it. Not `next/font/google`, which
fetches at build time: a build behind a proxy, or in CI with no egress, or
during a Google outage, then fails for a reason unrelated to this code.

The native app boots Figtree and JetBrains Mono and deliberately does not boot
Fraunces. 121KB of display face for three surfaces is weight a field app should
not carry.

---

## 5. Shape, density, motion

- **Radius:** 10px controls, 16–18px cards, 12px scale root, 999px pill tags.
  The scale root is unchanged from the previous identity. What changed is that
  controls no longer sit *on* it.
- **Borders over shadows:** a 1px border and a surface tone do the separating.
  The shadow scale is for overlays and other floating layers.
- **Density:** three user-selectable steps. `comfortable` is the default
  (44px controls, 48px rows, 24px card padding), `compact` is 36/40/16, and
  `dense` is 32/32/12. Dense reproduces the values this codebase shipped
  before Marula, so the fleet, stock and billing tables built at 28px rows do
  not regress for anyone who reaches for it. **44px remains the tap-target
  floor on every step**, and only pointer targets sit at 32px under `dense`.
- **Motion:** transitions at or under 120ms, on transform and opacity, with
  `prefers-reduced-motion` collapsing them. Subtle motion is encouraged:
  entrance reveals, staggered list entrances, panel slide-ins, hover
  micro-feedback, tab and filter morphs. The constraints are performance, not
  austerity. Never animate a layout property.
- **Keyboard-first:** a 2px offset focus outline in the mode's focus-ring token.
  Inline shortcut hints. Every primary flow works without a mouse.

---

## 6. Component styling

- **Buttons:** 10px radius. Primary is the flipping pair (§2 rule 2).
  Secondary is a surface with a 1px border. Destructive is `#B42318`. The
  assistant variant is the one action inside the inverted card.
- **Status indicators:** pill tags (999px, bold, 11px) over the §3.4 pairs.
  Priority is a separate family: square-ish, bold, fixed width, so that a
  priority never reads as a status.
- **Tables:** sticky headers, a 48px row at the default density, hover via the
  surface-hover mix. Column order and column width are user state, and the
  header owns them.
- **Cards:** `rounded-xl`, 1px border, 24px padding.
- **Charts:** the §3.5 ramp. Gridlines at border colour. No gradients.
- **Forms:** labels above fields. Validation renders **on the field**, with
  `border-destructive` and inline text, never a page-level banner, never
  colour alone.
- **Disabled:** a muted fill with muted text, not reduced opacity. Opacity
  drags the label under the readability floor.
- **Empty states:** one sentence, one action, and a small line illustration in
  fruit, leaf or canopy, drawn in sand and bark tones.

## 7. Logo / mark direction

- Wordmark: "Marula" in Fraunces SemiBold.
- Mark: a marula fruit crossing a globe wireframe, in ink and gold. One gold
  stroke in an otherwise neutral mark.
- No gradients. Survives a 16px favicon and single-colour print.


## 8. Voice

Dry, precise, documentation-grade — unchanged by the visual overhaul.

## 8.5 Theme-token schema v1 + preset range (Wave 24 `p24-theming-system`, repointed to Marula in Wave 41)

**The Marula token set behind this file is generalized into a VERSIONED theme-token schema** (`THEME_TOKEN_SCHEMA_VERSION = 1`): a theme is a map of LIGHT/DARK values over the CLOSED semantic-role vocabulary (the §3 roles — background/surface/overlay/border/text tiers, accent/on-accent, destructive, the four status pairs, chart-1..5). Motion (§5), typography (§4), radius, and the shadow scale are NOT part of the vocabulary on purpose — no theme can restyle them, so docs/theme.md override rules win by construction.

- **What a theme may move, and what it may not (wave41 D8, brand spec §4.5).** A theme
  restyles surfaces. It may shift the **neutral, chrome and accent-tint axes**
  (`SHIFTABLE_AXIS`). It may not restyle the **primary** or its ink-to-gold mode
  flip, `destructive`, the status pairs, or the assistant inversion. The fixed
  roles are simply absent from `ROLE_TO_TOKEN`, so a preset naming one is
  dropped rather than repainted, and `assertPresetIsLegal` rejects it outright
  at the pack boundary. `scripts/check-theme-packs.mjs` asserts the same rule as
  a diff: every role a preset does not declare must be byte-equal to `marula`'s.
- **Built-in presets**, all of which the org picks and any user can override:
  - `marula` — the Neutral light column, the pinned **default-v1** and the default for web and desktop
  - `marula-warm` — the cream light column, and the default on mobile
  - `high_contrast` — the accessibility preset, buying its contrast on the shiftable axis
  `slate` and `copper` are **dropped** in wave41: both were second-brand neutrals
  with no authority in the brand spec, and the axis they wanted is now `marula`
  versus `marula-warm`. `tangerine` is renamed to `marula` by migration 0115.
  Machine source: `packages/ui/src/theme-tokens.ts` (frontend) + `crates/core-domain/src/theme_pack.rs` (backend boundary). Application is a `data-preset` attribute on `<html>` (attribute-scoped token overrides at the same specificity — the unset attribute IS the tokens.css baseline, so nothing changes unless the org or user picks a preset).
- **Settings keys (typed registry):** `appearance.theme` (ORG scope: preset id or `pack:<slug>`), `appearance.theme_override` (USER scope: the `org_default` sentinel wins user→org, same precedence as the sticky AI model picks), and `appearance.mode` (USER scope: system/light/dark, a U6-safe supersedes of the flat old user `appearance.theme` key via migration 0097).
- **Theme packs (leg B) ride ADR 0009 V1** — marketplace registry / pinned GitHub / local zip staged imports, `artifact_kind: "theme"`, **zero capabilities** (the ingestion gate refuses capability declarations: topics restyle and nothing else). The manifest carries ONLY token overrides; **typed rejection of off-schema keys** (a `font_family` key is a 422 with the key named, never a silent ignore).
- **Import by export (tweakcn / shadcn):** the pack manifest may be a RAW tweakcn export (the vendored `design/marula/marula.json` is the exemplar shape — `{ name, cssVars: { light, dark } }` over the shadcn variable vocabulary, frequently `oklch()`) or a shadcn registry-item theme (same `cssVars` shape). The ingestion seam coerces: identity from the export's `name` (version 1.0.0 + current schema version stamped), oklch → sRGB hex, shadcn variable names mapped onto schema roles (background/card/popover/border/foreground/secondary-foreground/muted-foreground/primary/primary-foreground/destructive/chart-N). Unmapped export variables (sidebar ramps, hover tints, radius, fonts) are DROPPED — the schema stays restyle-only. Derived tones recompute client-side. The coerced manifest must pass the same validate() gate (contrast floors included), so an export whose converted basics fail AA is refused outright: upload your export file as the zip's `manifest.json` and it imports with no hand translation.
- **Status pairs note:** shadcn/tweakcn exports carry no success/warning tags. The PSA status colours (§3.4) survive every pack, exactly per rule 5 above and the D8 fixed axis.

## 9. Enforcement

- Tokens live in `packages/ui/src/tokens.css` (+ the `tokens.ts` PALETTE mirror). CI hex-scan (`.github/workflows/ci.yml`) fails on raw hex outside the token files. The blacklist covers three dead families: Workbench, Tangerine, and Ember. It is append-only, because the point of a retired colour is that it can never come back.
- **`@shadcn/lint` (Wave 25 lane 1) mechanizes the token layer for `apps/web`:** `pnpm turbo run lint` runs ESLint 9 + `@shadcn/lint` over 277 files with `settings.shadcn.componentImports` on `@marula-core/ui`. At `error`: `no-raw-colors` (raw palette colors — succeeds the hex-scan posture) and `no-restyle` (permissive-correct `layout+typography` base with per-component contracts. Component styling stays owned by `packages/ui/src/**`, Tiptap editor internals, and the recharts tooltip style props via explicit carve-outs). At `warn` pending post-Wave-25 sweeps: `no-inline-styles` (96 token-driven findings), `no-arbitrary-values` (1476, mostly the `text-[13px]` off-scale font size), `no-unknown-classes` (51 `m-wb-*` classes needing `@utility` declarations), `require-static-classes`. Every finding's error text appends this file + mockup v2 as the fix reference.
- `packages/ui/tests/tokens.test.ts` proves hex-source uniqueness, PALETTE sync, dark-mode completeness, and WCAG floors.
- `scripts/review-theme.mjs` asserts the generated layers against the vendored `design/marula/marula.json` (value-level comparison, the primary's mode flip, the non-flipping bar, the chart ramp, the vendored font files). Its blacklist scan strips comments first, so a hex named in a tombstone reads as documentation rather than a defect. Wave 24 note: the per-theme hex-scan cross-checks the `no-raw-colors` settings (`apps/web/eslint.config.mjs`).
- **`scripts/check-theme-packs.mjs` (Wave 24 leg C) — the per-preset gate:** runs the retired-hex blacklist, the AA/3:1 floors and the D8 fixed-axis diff over EVERY shipped built-in preset, in both modes. It also scores any installed pack manifests passed as CLI args or via `THEME_PACK_MANIFESTS` (CI loops per installed theme). It enforces the closed role vocabulary, so an off-schema token in any theme is mechanically rejected rather than applied.
- Reference mockups: `design/mockups/meridian-ticket-workbench-html-mockup-v2.html` (+ its `-light.png` and `-dark.png`) is the **aesthetic** reference for layout, hierarchy and density, and it still describes the retired identity, so it is checked for geometry only. The brand pack in `brand/marula-mockups/` is the authority for colour and type. Wave 41 lane 4 draws the new mockups against these tokens. QA evidence under `design/qa-p41-marula-theme/`.

<!-- END -->
