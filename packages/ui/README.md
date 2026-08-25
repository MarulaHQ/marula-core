# ui

> **Purpose:** Workbench design system: tokens from docs/theme.md (greyscale + Ember accent), shadcn/Radix-based components, 2px radii, compact density.
> **Non-purpose:** No off-palette brand values — CI fails on any hex outside the Workbench token set. No feature-specific components.

## Consuming

```ts
import { Button, Tag, useTheme } from "@meridian/ui";
```

Import both stylesheets once at the app root, then initialize the theme:

```ts
import "@meridian/ui/tokens.css";
import "@meridian/ui/components.css";
import { applyTheme, resolveInitialTheme } from "@meridian/ui";

applyTheme(resolveInitialTheme()); // sets data-theme on <html>
```

## Layout of truth

- `src/tokens.css` — the single source of hex values (light `:root` + `[data-theme="dark"]`). Derived tones use `color-mix()` against palette primitives only.
- `src/tokens.ts` — typed mirror (`PALETTE`, status maps, WCAG helpers). A test asserts it stays in sync with the CSS.
- `src/components.css` — the `.m-*` component classes; every declaration references a token variable.
- `tests/` — mechanical enforcement: palette allowlist, dark-mode completeness, WCAG contrast floors, no raw hex outside `tokens.css`, motion ≤120ms on opacity/transform.

## Known spec tension

theme.md §6 pins white text on solid Ember (#D64B0A) for primary buttons in
light mode (~4.3:1: passes large-text/UI floors, below normal-text 4.5). Dark
mode uses ink-on-lightened-Ember and passes full AA. Dark danger tags
(#F87171/#7F1D1D ≈ 3.6:1) are spec-pinned values held to the UI floor. Both
are asserted as regression guards in `tests/tokens.test.ts`.
