# Marula surface pack, wave 1

Three references, drawn in neutral light and dark at 1600 by 994.

| Folder | What is in it |
|---|---|
| `web/tickets` | The data-surface template: 12 states plus 2x heroes per theme |
| `web/dispatch` | The densest surface: 29 states plus 2x heroes per theme, including how double-booking is handled |
| `shared` | The kit: palette (2 states), chat panel (3), notification dock (2), and five sheets (chrome, context columns, assistant column, components, the AI ladder) |
| `overview` | Contact sheets of every state, and neutral-versus-dark comparisons |
| `source` | The three interactive pages, with fonts embedded so they render offline |

Files are `web/<surface>/<theme>/states/NN-name.png`. `manifest.json` indexes every image with a `what_to_notice` line written as an assertion about the picture.

Read in this order:
1. `DESIGN-NOTES-tickets.md`, then `DESIGN-NOTES.md` (the kit and Dispatch).
2. `DERIVATION-GUIDE.md` for how every other route copies one of these.
3. `tokens.json` for the exact colours per theme.

## Start with these six images
- `web/tickets/neutral/states/01-default.png`
- `web/dispatch/neutral/states/01-board-rest.png`
- `web/dispatch/neutral/states/13-plan-review.png`
- `shared/neutral/K02-palette-chat.png`
- `shared/neutral/S5-ai-ladder.png`
- `overview/wave1-neutral-vs-dark.png`

## Frozen clock
Tuesday 29 September 2026, 11:41. Timers do not tick, so digits stay put.

## Not in this wave
Mobile (Scan, ticket detail, Time, the edges), the Settings reference and the portal in a second theme.
