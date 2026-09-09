# Meridian Visual Theme — "Workbench"

> **Status:** Active · **Last updated:** 2026-08-25
> **Status:** Active · **Last updated:** 2026-08-25
> **Purpose:** Meridian's own utilitarian visual identity. Meridian is the open-source core — it must look like a professional tool, first and last.
> **Scope:** binding on all Meridian UI code, docs site, and marketplace surfaces.

## 1. Design stance

**A workbench, not a showroom.** Dense, keyboard-first, chrome-quiet. The UI recedes so tickets, time, and dispatch data dominate. Zero marketing surface: no hero gradients, no decorative illustration, no sparkle. If a visual choice would feel at home in a terminal emulator or a well-made IDE, it belongs in Meridian.

## 2. Identity definition

| Aspect | **Meridian (this theme)** |
|---|---|
| Brand color | **Monochrome + Ember** — greyscale everything, burnt-orange accent `#D64B0A` / `#FF8A4C` used sparingly |
| Temperature | Cool neutral — grey canvas, graphite dark mode |
| Headers | Inter for UI/body; JetBrains Mono for headings (owner override 2026-09-09 via shadcn preset b3J8o4J600 — supersedes the earlier "Inter-only, no serif" rule; still no serif) |
| Corners | 2px throughout — squared tags, not pills |
| Depth | Visible 1px borders, shadows for overlays only |
| Feel | "Workbench" |

**Hard rules:**
1. Every UI color comes from the token set in §3 — no off-palette brand hexes anywhere in the product.
2. Ember appears only on interactive affordances; nothing decorative gets color.
3. **Semantic status colors follow industry-standard PSA conventions** (amber warnings, green success, red danger) so any technician reads status identically no matter what tooling they came from.
4. **The accent is scarce by law.** Monochrome is the default state of every view; Ember appears only on interactive affordances — primary buttons, focus outlines, links, active nav indicator, the primary data series in charts. Target: **≤5% of any viewport** is non-greyscale. If a screen looks colorful, it's wrong.
5. **Both modes are first-class.** Every screen ships pixel-real in light (`#F4F5F7` canvas) and dark (`#101216` canvas) mode from the same token set — dark is never an afterthought, and no surface may ship in only one mode.

---

## 3. Color palette

### Core UI (light mode)

| Role | Hex | Notes |
|---|---|---|
| Canvas | `#F4F5F7` | Cool app grey — primary background |
| Surface | `#FFFFFF` | Cards, tables, panels |
| Text (primary) | `#12161C` | Ink |
| Text (secondary) | `#55606E` | Slate — metadata, captions |
| Border | `#D3D8DF` | Always-visible 1px solid borders on all containers |
| Accent (brand) | `#D64B0A` | **Ember** — burnt orange, interactive elements only (see §3.1) |

### Core UI (dark mode)

| Role | Hex | Notes |
|---|---|---|
| Canvas | `#101216` | Graphite |
| Surface | `#171A1F` | Raised panels |
| Text (primary) | `#E8EBEF` | |
| Text (secondary) | `#9AA4B2` | |
| Border | `#2A2F37` | |
| Accent (brand) | `#FF8A4C` | Ember, lightened for dark contrast |

### Greyscale ramp (the actual identity)

Monochrome carries the design; Ember only marks interaction.

| Step | Hex | Use |
|---|---|---|
| Ink | `#12161C` | Primary text, strongest emphasis |
| Graphite | `#3A424C` | Iconography, chart series |
| Slate | `#55606E` | Secondary text |
| Mist | `#9AA4B2` | Tertiary text, disabled states |
| Fog | `#D3D8DF` | Borders (light mode) |
| Paper | `#FFFFFF` / Canvas `#F4F5F7` | Surfaces / background |

Dark mode mirrors: `#E8EBEF` → `#9AA4B2` → `#2A2F37` → `#171A1F` → `#101216`.

### 3.1 Ember vs Amber — the disambiguation rule

Both live in the orange family, so they are separated by **role**, strictly:

- **Ember (`#D64B0A`)** = *interactivity*. Buttons, links, focus rings, active indicators. Never a status, never a background fill behind text.
- **Amber (`#B45309` / `#FBBF24`)** = *warning state*. SLA risk tags, alert banners. Never an action, never plain text.

If an element can be clicked, it may be Ember. If it means "careful," it is Amber. An element must never be both.

### Semantic status

| Meaning | Light | Dark |
|---|---|---|
| Success / Resolved | `#15803D` on `#DCFCE7` | `#4ADE80` on `#14532D` |
| Warning / SLA risk | `#B45309` on `#FEF3C7` | `#FBBF24` on `#78350F` |
| Danger / Breach | `#B91C1C` on `#FEE2E2` | `#F87171` on `#7F1D1D` |
| Neutral / Waiting | `#55606E` on `#E5E7EB` | `#9AA4B2` on `#262B33` |

## 4. Typography

| Role | Family | Weight | Notes |
|---|---|---|---|
| Everything (headers, UI, body) | **Inter** (system-ui fallback) | 400 / 500 / 600 | No serif faces, ever. Headers distinguish themselves by size + weight 600 only |
| Code / data / logs | JetBrains Mono | 400 | A tool convention, not a brand signal |
| Numerals in time/billing/invoice contexts | Inter | 500 | **Tabular numerals** (`font-variant-numeric: tabular-nums`) mandatory — columns of times must align |

## 5. Shape, density, motion

- **Radius: 2px** on buttons/inputs/tags; 3px maximum on cards/modals. Nothing fully-rounded.
- **Borders over shadows:** every container gets a visible 1px border; shadows reserved for overlays (modals, popovers, drag ghosts).
- **Density:** compact by default — 32px control height, 24px table rows, tight list padding. A dispatcher's board is a *dense information surface*, not a marketing page.
- **Motion:** transitions ≤120ms, opacity/transform only; no springs, no bounce, no entrance animations.
- **Keyboard-first:** visible focus = 2px offset outline in accent color; shortcut hints rendered inline (`⌘K`, `G then T`); every primary flow operable without a mouse.

---

## 6. Component styling

- **Buttons:** 2px radius, weight 500. Primary = solid Ember, white text. Secondary = surface background + 1px border + ink text. Destructive = danger red outline style (filled only on confirmation steps).
- **Status indicators:** squared **tags** (2px radius), never fully-rounded pills — `Resolved`, `SLA risk`, `Waiting` use the semantic palette above.
- **Tables are first-class citizens:** sticky headers, column resize, saved views, inline edit, keyboard row navigation. Lists that could be tables should be tables.
- **Forms:** labels above fields, visible borders, validation messages in danger color below the field with an icon — never color alone.
- **Charts/data viz:** greyscale series in graphite/slate/mist with Ember reserved for the primary series alone; gridlines at border color; no gradients.
- **Empty states:** one sentence + one action button. No illustrations.

## 7. Logo / mark direction

- Wordmark: "Meridian" set in Inter SemiBold, sentence case.
- Mark: a single vertical meridian line crossing a minimal circle/globe wireframe — a longitude reference, rendered monochrome (ink on light, warm white on dark). Ember appears *only* as the meridian line itself — one thin stroke of orange in an otherwise greyscale mark.
- Monochrome-first; no gradients; must survive 16px favicon rendering and single-color print.

## 8. Voice

Dry, precise, documentation-grade. The product speaks like its own release notes: "Bulk-resolve 40 tickets." / "Timer paused — reason recorded." No exclamation marks, no marketing adjectives — terse and instrumental.

## 9. Enforcement

- Design tokens live in `packages/ui` (`tokens.css` / TS constants) generated from this spec — no hardcoded hex values in feature code; CI lint fails on raw hex outside token files.
- A CI assertion scans built CSS for any hex outside the Workbench token set and fails if one appears.

<!-- END -->



