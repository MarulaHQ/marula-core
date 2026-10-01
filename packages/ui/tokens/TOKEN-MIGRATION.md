# Token migration: the kit's tokens into `@marula-core/ui`

Wave 41 lane 20 (`p41-ui-kit-sweep`). This records how the landed `--m-*`
layer reconciles with the handoff kit's token system, so the switch is one
reviewed step instead of a guess.

## The new source of truth

`tokens/tokens.json` is the only hand-edited token file. `node tokens/generate.mjs`
writes `tokens.css`, `tokens.ts` and `CONTRAST.md` from it. `theme.css` is a
static Tailwind 4 mapping and ships unchanged.

The generator is proven: its three outputs are byte-identical to the kit's
reference files in `design/marula-handoff-kit/tokens/`, and `node tokens/generate.mjs
--check` fails if someone edits an output by hand. `CONTRAST.md` holds at 60 of 60 AA.

## Theme names: the D2 mapping

The landed app uses two names, `marula` and `marula-warm` (wave41 plan D2). The
kit uses `neutral` and `light`. They are the same two themes.

| Landed name | Kit name | Where it is default |
|---|---|---|
| `marula` (Neutral column) | `neutral` | web and desktop |
| `marula-warm` (cream) | `light` | mobile, and the portal as `sand` |

The kit also accepts `sand` and `cream` as aliases of `light`, so the portal
and the marketing site need no second definition. The migration keeps the kit
names on `data-theme` and maps the landed names onto them, rather than
carrying both spellings in the token file.

## The alias map

Every `--m-*` variable that has a kit equivalent becomes an alias, never a
second copy of the value:

| Landed | Kit token |
|---|---|
| `--m-canvas` | `--background` |
| `--m-surface` | `--card` |
| `--m-overlay-surface` | `--popover` |
| `--m-nav` | `--mc-nav` |
| `--m-sidebar` | `--mc-side` |
| `--m-surface-2` | `--mc-surface-2` |
| `--m-bar` | `--mc-bar` |
| `--m-text` | `--foreground` |
| `--m-text-secondary` | `--muted-foreground` |
| `--m-border` | `--border` |
| `--m-input`, `--m-border-strong` | `--input` |
| `--m-primary` | `--primary` |
| `--m-on-primary` | `--primary-foreground` |
| `--m-accent` (the gold tint) | `--mc-act` |
| `--m-on-accent` | `--mc-act-fg` |
| `--m-accent-text` | `--mc-gold-text` |
| `--m-destructive`, `--m-status-danger-fg` | `--destructive` |
| `--m-status-success-fg` | `--mc-green-text` |
| `--m-status-success-bg` | `--mc-green-soft` |
| `--m-status-warning-fg` | `--mc-progress-fg` |
| `--m-status-warning-bg` | `--mc-progress` |
| `--m-status-due-fg` | `--mc-attn-fg` |
| `--m-status-due-bg` | `--mc-attn` |
| `--m-status-neutral-fg` | `--mc-note-fg` |
| `--m-status-neutral-bg` | `--mc-note` |
| `--m-status-offline-fg` | `--mc-off-fg` |
| `--m-status-offline-bg` | `--mc-off` |
| `--m-client` | `--mc-entry-client` |
| `--m-sys` | `--mc-entry-system` |
| `--m-time` / `--m-time-fg` | `--mc-entry-time` / `--mc-entry-time-fg` |
| `--m-agent` | `--mc-entry-agent` |
| `--m-note` / `--m-note-fg` | `--mc-note` / `--mc-note-fg` |
| `--m-asb` / `--m-asf` | `--mc-assist` / `--mc-assist-fg` |
| `--m-asbtn` / `--m-asbtnf` | `--mc-assist-btn` / `--mc-assist-btn-fg` |
| `--m-asline` | `--mc-assist-line` |
| `--m-scrim` | `--mc-scrim` |
| `--m-shadow` | `--mc-shadow` |
| `--m-gold` / `--m-cream` / `--m-canopy` / `--m-lime` / `--m-blush` / `--m-sand` / `--m-bark` / `--m-ink` | the matching `--mc-*` brand token |
| `--m-leaf` / `--m-stem` / `--m-p2` | `--mc-leaf` / `--mc-stem` / `--mc-p2` |
| `--m-font-ui` / `--m-font-heading` / `--m-font-mono` | `--font-sans` / `--font-display` / `--font-mono` |
| `--m-density-control` / `--m-density-row` | `--mc-ctl` / `--mc-row` |
| `--m-radius-scale` / `--m-radius-container` | `--radius` / `--radius-card` |

Tokens with no kit equivalent stay in `--m-*` and are not aliased: the data
meters (`--m-meter-*`), the edition cues (`--m-edition-*`), the chart ramp
(`--m-chart-*`), the disabled pair, the two extra shadow strengths, the
`--m-text-muted` third text level, `--m-card-padding`,
`--m-density-control-sm` and the composed `--m-accent-soft/-muted/-hover`.

## Conflicts the switch has to settle

Three differences are decisions, not aliases. Each one is why the switch runs
under the kit gate rather than blind.

1. **`--primary` means different things.** The kit's `--primary` is the ink
   button; its `--accent` is the neutral hover surface; its `--mc-act` is the
   gold tint. The landed `globals.css` bridges shadcn `--primary` to
   `--m-accent`, the gold tint. Adopting the kit moves that meaning, so every
   `bg-primary` call site has to be re-read as either `bg-primary` (ink) or
   `bg-mc-act` (tint).
2. **Control radius.** The landed `--m-radius-control` is 10px, the kit's
   `--radius-ctl` is 12px and its `--radius-field` is 10px. The landed value
   matches the kit's field radius, so the alias above maps scale and container
   only and leaves the control radius to the sweep.
3. **Shadow weight.** The landed shadow is `0 8px 24px`; the kit's
   `--mc-shadow` is `0 16px 40px`. The kit's heavier shadow is the reference
   and wins on dialogs and the toast.

## Order of work

1. Generate and check the tokens (done: byte-identical, 60 of 60 AA).
2. Replace the `--m-*` definitions in `packages/ui/src/tokens.css` with the
   alias layer above, keeping the un-aliased tokens as literals.
3. Re-point `apps/web/app/globals.css` so the shadcn bridge reads the kit
   variables, and settle the `--primary` meaning per call site.
4. Move the theme names onto `data-theme` values `neutral` / `dark` / `light`
   (with `sand` and `cream` aliased to `light`) in the no-flash bootstrap in
   `apps/web/app/layout.tsx`.
5. Re-run `packages/ui` vitest, then the kit proof and axe, then a screen's
   visual diff. Only after that does a second screen follow.
