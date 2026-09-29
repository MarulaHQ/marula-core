# Tickets list: reference surface

Wave 1 test of the surface pack. One surface, drawn fully, so you can judge the aesthetic and the method before the rest are drawn. Desktop themes: **neutral** and **dark**. Every state is numbered the same in both.

## 1. The moment

**Who is here.** The service desk or triage tech, forty times a day, between tickets. Also the dispatcher looking for the unassigned queue, and the new hire on their first morning.

**What they are trying to finish.** Find the right ticket, and be in it within seconds. Behind that: know what changed since I last looked, know what is about to breach, and clear a batch of easy ones in one pass.

**What gets in the way.** Re-orienting after a handoff. A queue that looks the same whether it is calm or on fire. A table that hides the column they need, or exports a different set than the one on screen.

**How they know they are done.** The queue is quiet. Nothing is bold, nothing is red or blush, and the empty state says so.

**The manifesto test.** This list is where a tech looks like a hero to the client (the breach is caught before the client phones, the printer ticket is taken in one click) and to their leader (the queue is under control, and the export is exactly the set they asked for).

## 2. What this surface exists to teach

This is the **data-surface template**. Clients, billing, assets, stock, purchasing, projects, contracts and audit all copy it. Anything an implementing agent would otherwise improvise is fixed here:

1. One toolbar row above the table: **view switch on the left, filter chips in the middle, Columns, Filter and Export right-aligned**. It never wraps and never pushes the table down.
2. Three densities, chosen from a control in the table footer, and applied to controls and rows together.
3. Views, chips and the filter builder edit one filter state. The state is a shareable link.
4. Columns are per user, persist, and can be reordered and resized.
5. Export says what it will export, and respects the view.
6. Selection replaces the column header with an action bar, in place.
7. Empty and error states share one shape.

## 3. Anatomy (canvas 1600 x 994)

| Region | Size | Notes |
|---|---|---|
| Top bar | 56 | Same as the workbench pack |
| Icon rail | 64 wide | Tickets active (soft gold tile, dark glyph) with the open-count badge, 12, which matches My queue |
| Page header | 64 | Title 26 px semibold. Three attention shortcuts beside it. Primary New ticket at the far right |
| Toolbar | control height + 16 | One row. See section 2 |
| Table | fills | 16 px radius, 1 px border, sticky column header |
| Footer | 52 | Left: shown of total. Right: density control and keyboard hints |
| Timer bar | 60 | Ink in every theme, pinned, unchanged |

Page gutters are 24 px. Popovers are 16 px radius with the floating shadow. The move dialog is 20 px radius.

**Attention shortcuts.** Changed since you looked (gold dot), At risk (blush dot) and Breached (red dot), each a counter that toggles a filter chip. They are the fastest way in, and they only ever count the current view.

## 4. Density

Chosen from the control in the footer, saved per user, applied to the whole table at once.

| Step | Control | Row | Column header | Text | Avatar | Rows fully visible on this canvas |
|---|---|---|---|---|---|---|
| Comfortable (default) | 44 | 48 | 44 | 14 | 26 | 13 |
| Compact | 36 | 40 | 40 | 14 | 24 | 16 |
| Dense | 32 | 32 | 32 | 13 | 20 | 21 |

Rules that hold at every step: text never goes below 12 px (avatar initials stay at 12 px even in a 20 px avatar), the checkbox stays 18 px, and hit areas never shrink below the control height.

## 5. Columns, sorting and resizing

- Default columns, in order: select, Priority (P), ID, Ticket, Client, Status, SLA, Assignee, Last activity. Available but hidden: Contact, Board, Type, Created, Time logged, Source.
- **Ticket** is always shown (lock icon, disabled checkbox). Select is always first.
- Reorder by dragging a header, by dragging a row in the chooser, or from the keyboard by focusing a grip and using the arrow keys. Resize by dragging the divider on a header.
- The chooser is a **popover under the Columns button**, not a side sheet. It costs no table width, and it states that the choice is saved for the user on every device.
- Sorting: click a header. The active header is dark with an arrow. Default sort is SLA soonest first in My queue and last activity newest first elsewhere.

## 6. Filters

- **Views** are named and per user: My queue, Unassigned, All tickets. Each carries its own default filter and sort. The counts are tabular figures.
- **Chips** read as a sentence (Client is Acme Corporation) with the value in semibold and a remove button. If there are too many for the row, the rest collapse into a +N more chip that opens the builder. The dashed Add filter chip always ends the row.
- **Builder** is a popover from the Filter button. Rows are field, operator and value. A switch chooses whether all or any of the conditions must match. The count of matching tickets updates live and the table behind updates with it.
- **Sharing.** The builder shows the link that reproduces the exact rows, with Copy link, and also offers Save as view. Both exist because they answer different needs: a link for the moment, a view for the habit.

## 7. Selection and bulk actions

Selecting any row replaces the column header with an action bar, in place, so the layout never jumps. The bar is a gold tint and is deliberately **not** an inverted surface. Actions: Assign, Set status, Set priority, Move to board, Merge (needs two or more), Export N, Clear selection. Selected rows get a gold tint and a 3 px gold bar on their left edge. Shift-click selects a range, and X toggles the row under the keyboard cursor.

**Move to board** opens a dialog that shows the consequence before it happens. Statuses belong to boards, so every distinct status among the selected tickets gets a row: matched (green tint, "Same meaning"), suggested (gold tint), or **needs a choice** (blush). The primary button is disabled and says "Choose to continue" until nothing is left unresolved. The preview lists each ticket before and after.

**Take it.** In the Unassigned view each row offers a one-click Take it. It is the shortest path from unassigned to owned.

## 8. Export

Export always says what it will do: rows, filters, sort, and column count. It exports **exactly** the current view, with visible columns in the user's order. The button carries the row count. Above 5,000 rows the popover adds a blush note and switches to two actions: Download now, and Email me when it's ready (primary). Below that there is one action, Download CSV. In the live page, the small case downloads a real file.

## 9. Colour on this surface

| Colour | Means here |
|---|---|
| Gold | Active or yours: the unread dot, the selected row and its bar, the current view, the pressed control, the primary button in dark mode |
| Blush | Attention soon: At risk pills (42m left), the large-export note, P2, the Needs a choice tag, and the load-error mark |
| Red | Only a genuine SLA breach (the Breached pill) and P1 |
| Green | Done or chosen: Same meaning in the move dialog |
| Sand and stone tints | Waiting on client, and the quiet neutral pills |

Colour is never alone. A breached SLA says **Breached 1h 05m** with an alert icon. Paused says **Paused** with a pause icon. Unassigned is a dashed avatar and the word, or the Take it button. Unread is a gold dot **and** a bold title.

There is no inverted surface on this screen except the timer bar. Nothing on the list is an AI surface: triage lives in the workbench column (drawn in the first pack), and appears as a row mark on Boards. The list carries no AI affordance at all.

## 10. States drawn

| # | State | Why it exists |
|---|---|---|
| 01 | Default, My queue | The reference for everything |
| 02 | Filtered | Chips, badge and counts |
| 03 | Filter builder | Compound conditions and the shareable link |
| 04 | Column chooser | The most-requested control |
| 05 | Bulk selected | Selection replaces the header |
| 06 | Move to board | Conflict preview |
| 07 | Export, large | Row count and the email path |
| 08 | Compact density | The middle step |
| 09 | Dense density | Where density earns its keep |
| 10 | Empty, no results | Empty caused by filters |
| 11 | Empty, quiet queue | Empty caused by success. The manifesto state |
| 12 | Load error | The one error the list can reach |

The brief asks for six to ten states. This is twelve, because the empty state has two different causes and the error state is required.

## 11. How the other data surfaces derive from this

Copy the frame, the toolbar order, the density table, the column and filter behaviour, the bulk bar, the export popover and the empty and error shapes. Change only these:

| Surface | Views | Default columns | Attention shortcuts | Extra |
|---|---|---|---|---|
| Clients | Open work, Assets, Billing history | Client, Contact, Open tickets, Contract, Renews, Owner | Renewing in 60 days, No contract | Row mark for a possible duplicate (P3) |
| Billing | Ledger, Ageing, Credit notes | Invoice, Client, Amount, Currency, Status, Ageing | Overdue, Awaiting send | Flagged strip above the table (P3) |
| Approvals | Today, This week, Mine | Entry, Tech, Ticket, Hours, Billable, Reason | Unapproved, Non-billable | Inline verdict per row |
| Assets | Devices, Config items, Merge queue | Asset, Client, Type, Warranty, Last seen | Warranty ending, Offline | Merge queue side by side |
| Stock and loans | Ledger, Tools, Loans | Item, Location, On hand, Reorder | Due soon (blush), Overdue (red, with text) | |
| Purchasing | Orders, Vendors | PO, Vendor, Status, Value, Expected | Awaiting receive | Receive action on the row |
| Projects | Active, At risk | Project, Client, Phase, Budget, Due | At risk (blush), Overdue (red only when breached) | |
| Audit and provenance | Recent, By actor | When, Actor, Action, Entity | None | No bulk actions |

Rules for deriving: the Ticket-like primary column is always required and always first after ID. Red stays reserved for breach. A surface with only one sensible arrangement still keeps the view switch, with a single view, so the toolbar looks the same everywhere.

## 12. Answers proposed for the brief's open questions

- **Column chooser placement (Q4):** a popover under the button. A side sheet takes a third of the table on the surface where you most need to see the columns.
- **Async export (Q5):** drawn as real, with a threshold at 5,000 rows. If the product stays synchronous, the email action becomes a row-count warning and the rest is unchanged.
- **Filter sharing (Q6):** both. A link for a moment, a saved view for a habit. The builder offers them side by side.
- **Views on Tickets (Q3):** three, per user.

## 13. Decisions to confirm

- **Folder convention.** The brief's tree puts both themes under one `states/`. This pack puts the theme above it (`web/tickets/neutral/states`, `web/tickets/dark/states`), the same as the mobile convention and the first pack. The manifest carries the theme field either way.
- **Rail active item.** The brief says a gold-filled square with the ink glyph. The first pack, which is the aesthetic bar, uses a soft gold tile with a dark glyph in light themes and a stone tile with a gold glyph in dark. This surface follows the pack.
- **Themes.** Neutral and dark only, as decided. Cream is not drawn on the web.
- **Canvas.** 1600 by 994 as the brief specifies (the first pack was 1600 by 1000). Hero images are 3200 by 1988.
- **Seven-item rail only.** The nine-item rail variant is part of the shared kit.

## 14. Exact values used on this surface

Full sets are in `tokens.json`.

| Token | Neutral | Dark |
|---|---|---|
| **Surfaces** | | |
| `--bg` | #F8F7F4 | #2B2A33 |
| `--surf` | #FFFFFF | #34333D |
| `--side` | #EFEEEA | #222129 |
| `--muted` | #EFEEEA | #3A3944 |
| `--bar` | #2B2A33 | #222129 |
| **Text and lines** | | |
| `--fg` | #2B2A33 | #F6EFD9 |
| `--sub` | #5F584F | #BEB5A2 |
| `--bd` | #E2E0DA | #43424D |
| `--inp` | #8C877C | #8A8378 |
| **Gold: active, yours, selected** | | |
| `--pr` | #2B2A33 | #E8B83A |
| `--prf` | #E8B83A | #2B2A33 |
| `--acc` | #F7E3A8 | #3A3944 |
| `--accf` | #6B4308 | #E8B83A |
| **Status and priority** | | |
| `--progb` | #FBEBBF | #4A3A1A |
| `--progf` | #6B4308 | #F2CD6B |
| `--st-new-b` | #ECE7DF | #3F3D45 |
| `--st-new-f` | #4A443D | #D8CFBA |
| `--st-wait-b` | #F3EAD2 | #3A342A |
| `--st-wait-f` | #5C4631 | #E2CFA9 |
| `--st-sched-b` | #FFFFFF | #2B2A33 |
| `--st-sched-f` | #4A443D | #D8CFBA |
| `--st-sched-bd` | #CDB38B | #5F584F |
| `--st-res-b` | #D5E3C5 | #26361A |
| `--st-res-f` | #2F4A1E | #BFD99A |
| `--st-br-b` | #FBE1DE | #4A1A18 |
| `--st-br-f` | #8F1C13 | #F9A097 |
| `--dueb` | #F8DCC0 | #4D2F14 |
| `--duef` | #7A3F0C | #F0B27A |
| `--red` | #B42318 | #FB8A80 |
| `--redf` | #FFFFFF | #2B2A33 |
| `--p2` | #D9822B | #D9822B |
| **Done and healthy** | | |
| `--green` | #5E7F3A | #A9C25D |
| `--gsoft` | #E4EDCB | #2E3A25 |
| `--gtext` | #3B5222 | #D6E5A8 |
| **Floating layers** | | |
| `--shadow` | 0 16px 40px rgba(43,42,51,.18) | 0 16px 40px rgba(0,0,0,.4) |
| `--scrim` | rgba(34,33,41,.55) | rgba(0,0,0,.6) |
| `--asb` | #2B2A33 | #F6EFD9 |
| `--asf` | #F6EFD9 | #2B2A33 |
| **Logo** | | |
| `--leaf` | #5E7F3A | #A9C25D |
| `--stem` | #7C746A | #CDB38B |

## 15. Not built

- The other rail destinations and the workbench are not wired. Opening a ticket shows a toast.
- Set status, Set priority and Assign in the bulk bar change the data. Merge only shows a toast.
- In a published copy of the page, browsers block file downloads, so Export downloads only work when the HTML file is opened directly.
- All names, tickets and figures are sample data. The 40,213 row total is illustrative.
