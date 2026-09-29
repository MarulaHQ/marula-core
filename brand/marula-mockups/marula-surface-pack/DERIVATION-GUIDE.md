# Derivation guide: every route, and which reference it copies

For the implementing agents. **Drawn** means there is a reference image. **Derive** means copy the named reference and change only what the row says. **Skipped** says why. Where a surface holds records, it takes all five data-surface controls from the Tickets list (views, columns, filters, export, density) and the same bulk bar, empty and error shapes.

References: **TL** Tickets list, **WB** ticket workbench (first pack), **KIT** shared kit, **DP** Dispatch, **M** mobile pack (Today, On site, Wrap up).

## Desktop, section 5 of the brief

| Route | Status | Copy | What changes |
|---|---|---|---|
| `/tickets` | Drawn | TL | |
| `/tickets/[id]` | Drawn | WB | |
| `/tickets/classic`, `/tickets/classic/[id]` | Derive | WB | Same three columns and timeline entry types in the classic three-pane arrangement. Nothing new in style |
| `/today` | Derive | TL header, KIT components | Attention cards are the stat pills grown to cards (label, tabular figure, tone). Quick log is a dialog: yesterday, 09:00, 60 minutes, suggested work type never auto-submitted, non-billable needs a reason (blush error if missing) |
| `/boards`, `/boards/[id]` | Derive | TL and the Dispatch queue card | Kanban columns hold the queue card. A card dropped where it makes an overlap follows the Dispatch overlap rule. Mid-drag uses the Dispatch ghost and conflict language. Triage verdict is a row mark (P3) |
| `/boards/[id]/fields` | Derive | KIT components | A list with reorder grips and toggles, in the settings form family |
| `/calendar` | Derive | DP | Week and month with the Dispatch now-line and hatch language. Two overlapping events use the Dispatch stacking rule (neutral, two rows, a reason on request), and only unapproved overlaps are flagged. Ticket deadlines overlay as blush or red-on-breach markers. Personal or team scope is a view switch |
| `/dispatch` | Drawn | DP | |
| `/dispatch/planning` | Drawn | DP | |
| `/dispatch/scheduling` | Skipped | Settings form family | Meeting types, groups and members are a form and a list, no new pattern |
| `/dispatch/travel` | Drawn | DP | |
| `/clients`, `/clients/[id]` | Derive | TL, KIT context column (client variant), WB | List is TL with views Open work, Assets, Billing history. Detail is the workbench's three columns with the client context column. Duplicate mark is a P3 row mark |
| `/projects`, `/projects/[id]` | Derive | TL, DP | Stat pills for active, at risk, complete, overdue (blush for at risk, red only when breached). Phases as a timeline on the Dispatch grid language. Budget bar uses the utilisation bar |
| `/contracts`, `/contracts/[id]` | Derive | TL, KIT components | Status column, renewals as blush pills |
| `/billing`, `/billing/[id]`, `/billing/credit-notes` | Derive | TL, KIT AI ladder panel 6 | Ledger with amount, currency, status, ageing. Audit is a strip above the table and a row mark with a one-click fix. **No inverted surface, no panel** |
| `/approvals` | Derive | TL, KIT AI ladder panel 6 | Decide in context: the entry and its ticket are visible in the row. Bulk pass uses the bulk bar. Non-billable without a reason is a blush mark |
| `/sales/pipeline`, `/sales/quotes`, `/sales/catalog` | Derive | Boards (kanban), TL, KIT | Stage board; quote lifecycle with accept and decline pills; catalogue grid |
| `/assets`, `/assets/[id]`, `/inventory`, `/inventory/workbench` | Derive | TL, WB | Config ledger with a merge queue side by side (the queue is already a human decision surface, so no AI mark inside it) |
| `/stock`, `/stock/[id]`, `/stock/locations`, `/stock/tools`, `/stock/loans` | Derive | TL, DP | Loans: **overdue is red**, due soon is blush, both with words and an icon, never colour alone |
| `/purchasing`, `/purchasing/[id]`, `/purchasing/vendors` | Derive | TL, WB | Receive is a row action on the order |
| `/automations`, `/alerts` | Derive | TL, KIT assistant stub | Rule builder as a form with a suggested condition inline (P3, no panel). Alert feed opens with the **collapsed assistant stub** and a count, never an open column |
| `/kb`, `/kb/secrets`, `/documents`, `/documents/[id]`, `/passwords`, `/imports` | Derive | TL, KIT components | Editor with a publish state; masked secret with a reveal confirmation (the confirm dialog); import wizard as a card stack |
| `/reports`, `/dashboards`, `/scorecards`, `/provenance`, `/audit`, `/search` | Derive | TL, DP Planning cards | Widget grid uses the Planning card; audit and provenance are dense TL tables with no bulk actions. **Search shows no AI affordance at all** (P4) |
| `/settings` and sub-pages | Derive | KIT components | A section rail plus one group at full fidelity in a later wave. Shadow-mode AI governance lives here and only here |
| `/portal/*` | Derive | WB timeline, TL, KIT | Own slim header, `max-w-3xl` column. Never shows internal notes, system events or suggestions. Can run a different theme from the staff app |
| Auth and entry routes | Derive | KIT components | Card stacks. "Skip for now" (resumable, records nothing) is a different state from "never show again" |
| `/s/[token]`, `/share` | Derive | KIT components | No chrome; a read-only view and a dialog with an expiry |
| `/dev` | Skipped | | Internal tooling |

## Mobile, section 7 of the brief

| Route | Status | Copy | What changes |
|---|---|---|---|
| Today, On site, Wrap up | Drawn | M | |
| Board tab list, ticket detail | Next wave | M and WB | The workbench's entry types on a phone |
| Time tab | Next wave | M | Day's entries, running session |
| Scan, Stock, Tools, Asset, Move | Next wave | M | The scan flow is the centre button's destination |
| Notifications, Locked, Login, MFA, More | Next wave | KIT dock and components | |

## Rules that hold on every derived surface

1. **One inverted surface per screen.** The assistant card. The timer bar is chrome.
2. **Red means breach.** A genuine SLA breach, P1, or a destructive action. Overdue, at risk and validation errors are blush.
3. **Gold is where you act.** Never a background for text on light.
4. **No gradients, glass or glow.** Depth is flat colour and a 1 px border. Hatching is an inline SVG pattern.
5. **Figtree runs the interface.** Fraunces is the wordmark, the mobile greeting and marketing only.
6. **Colour is never alone.** Pair it with a word or an icon.
7. **No grant means no trace.** No sparkle, no hint, no "ask AI".
8. **Text never below 12 px.** Targets are 44 px, 32 px at the dense step.
9. **An overlap is a decision, not an error.** Ask once with one-tap reasons, draw it neutral and stacked, and only block what is physically impossible. Flag in blush only what nobody chose.
