# ui

> **Purpose:** The Marula design system: the token layer from docs/theme.md (ink, gold and warm neutrals), shadcn/Radix-based components, a three-step density ladder, and the theme-token preset schema.
> **Non-purpose:** No off-palette brand values. CI fails on any hex outside the Marula token set, and on any hex from a retired identity. No feature-specific components.

## Consuming

```ts
import { Button, Tag, useTheme } from "@marula-core/ui";
```

Import both stylesheets once at the app root, then initialize the theme:

```ts
import "@marula-core/ui/tokens.css";
import "@marula-core/ui/components.css";
import { applyTheme, resolveInitialTheme } from "@marula-core/ui";

applyTheme(resolveInitialTheme()); // sets data-theme on <html>
```

## Layout of truth

- `src/tokens.css` — the single source of hex values (light `:root` + `[data-theme="dark"]`). Derived tones use `color-mix()` against palette primitives only.
- `src/tokens.ts` — typed mirror (`PALETTE`, status maps, WCAG helpers). A test asserts it stays in sync with the CSS.
- `src/components.css` — the `.m-*` component classes; every declaration references a token variable.
- `tests/` — mechanical enforcement: palette allowlist, dark-mode completeness, WCAG contrast floors, no raw hex outside `tokens.css`, motion ≤120ms on opacity/transform.

## Known spec tension

There is none left, and that is the point. The previous identity shipped white
on a single accent at roughly 3.6:1 and documented the gap as an accepted
tradeoff. Marula has no such gap: the primary inverts on mode, so the label
clears 4.5:1 in both directions, and every status pair clears full AA as text.
`tests/tokens.test.ts` asserts each of those as a regression guard, so the
tradeoff cannot be reintroduced quietly.

One value is deliberately below a WCAG floor: `--m-asline`, the divider inside
the assistant card, measures 2.02:1. It is the rendered pack's own value and
the rule is decorative, which 1.4.11 permits because it identifies no control
and no state. The test pins it at 1.5:1 and below 3:1 so nobody "fixes" it into
a state it was never carrying.
