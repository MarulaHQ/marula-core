# Wave 1 design notes: shared kit and Dispatch

Wave 1 of the surface pack. It holds three references: the **Tickets list** (see `DESIGN-NOTES-tickets.md`), the **shared kit** and **Dispatch**. Between them they fix the data-surface template, the chrome, the AI patterns and the densest screen in the app. `DERIVATION-GUIDE.md` maps every other route in the brief to one of them.

Themes drawn: **neutral** and **dark**. State numbering is the same in both. Canvas 1600 by 994; 2x heroes are 3200 by 1988.

## Part A. The shared kit

Drawn once. Every staff surface references these instead of redrawing them.

### A1. Chrome (sheet S1)
- **Top bar, 56 px:** wordmark, search with the Cmd+K hint, Online presence, theme toggle, notification bell with an unread badge, avatar. The search field opens the palette.
- **Icon rail, 64 px.** Both versions are drawn: the **seven-item rail** from the first pack (Tickets, Clients, Devices, Dispatch, Knowledge, Reports, Settings), and the app's **nine-item rail** (Today, Tickets, Clients, Assets, Stock, Purchasing, Knowledge, Insight, Settings). The nine-item rail also has a 188 px expanded form where Dispatch and Boards sit indented under Tickets. The active item is a soft gold tile with a dark glyph (a stone tile with a gold glyph in dark). Only Tickets carries a count badge, and it equals the My queue count.
- **Timer bar, 60 px, ink in every theme:** a gold disc, the gold tabular clock, ticket and work type, the quick note field, the arrow with a count, and Wrap up. Running and paused are both drawn; paused swaps the pause bars for a play triangle and says Paused.

### A2. Context column (sheet S2)
Three variants at 290 px: **workbench** (customer, contact with Call and Email, device with Remote in and Run script, related tickets with priority chips), **client record** (status pills, contract with a blush renewal pill, contacts, sites, owner), and **none**, for list and board surfaces, where filters live in the toolbar row instead. A contract nearing renewal is blush, never red.

### A3. Assistant column (sheet S3)
The agent's output channel, 340 px. It is not a chat window. Three states: **recommendation** (the inverted proposal card with one primary action, View evidence and Not this, plus sources and suggestion chips), **dismissed** (a dashed row with a visible Undo; dismissing is never destructive) and the **stub** (40 px, a spark and a count). The stub is how a surface keeps the affordance visible without spending its one inverted surface, and it is where alert triage opens by default. The card always says what it did not do ("Nothing has been sent").

### A4. Command palette (states K01, K02)
Opened by Cmd/Ctrl+K or by the top bar search field. **Search mode** (default): a field, a Search and Ask switch, results grouped Tickets, Knowledge and Actions with the first result selected, and a footer listing the keys (arrows, Enter, Tab to ask instead, Esc). **Chat mode:** the same box, with a **context chip** stating what the chat can see ("This screen: Tickets, My queue, 12 rows, 3 selected"), the conversation, sources under each answer, and a proposal row for any action, which is dashed and says it waits for approval. The footer offers **Open as a panel**. The Ask switch exists only when general chat has been granted; without the grant the palette is search only and does not hint that anything is missing.

### A5. Chat panel, the fallback (states K03, K04, K05)
When the palette is too small, chat slides up from the timer bar: it **translates in from its anchor**, stays 520 px wide, never takes the viewport, and leaves the timer bar visible so dismissal is obvious. Three states: **open**, **mid-expansion** (the lower edge is still behind the bar) and **collapsed** to a tab sitting on the bar with a one-line preview. On mobile the anchor is the bottom action bar. It is never a floating widget in a corner.

### A6. Notification dock (states K06, K07)
A popover from the bell in the top bar, 420 px, never on a right edge, so it never competes with the assistant column. Items are grouped by day (Today, Yesterday, Earlier this week). Unread items have a gold dot. The one red icon is an SLA breach. A tap opens the ticket, or focuses it if it is already open. **Empty:** a fruit line drawing, "You're all caught up", and Mark all read disabled.

### A7. Component set (sheet S4)
Buttons (primary, secondary, soft, disabled, destructive) at 44, 36 and 32; inputs in default, focused, error and disabled forms; checkbox, toggle, segmented control and filter chips; status and priority pills; toasts; a destructive confirm dialog; the empty and error shapes; the density table; a flat skeleton. **Validation errors are blush, not red**, always with an icon and words. The primary button is ink with gold text in light themes and gold with ink text in dark.

### A8. The AI ladder (sheet S5)
One decision surface (billing description audit, on a small ledger) in every state a granted surface can reach, so agents copy states rather than invent them:
1. **No grant:** no trace of AI. Not hidden, absent. The empty state never says "ask AI".
2. **Live, nothing to propose:** the 40 px stub with the spark and no count.
3. **Live, a proposal:** the inverted card, the only inversion on screen.
4. **Dismissed, undoable:** a dashed row with Undo.
5. **Approval failed:** says nothing was changed, and offers Retry or Edit by hand.
6. **Inline verdict (P3):** a row mark and a one-click fix, plus a strip above the table ("12 of 200 flagged"). No panel, no inverted surface.
7. **Nothing flagged:** a green strip that says so.

Rules that hold everywhere: a grant either exists or it does not, and the UI never re-asks. Shadow mode is invisible on the surface and visible only in Settings. An agent's write is proposed, awaiting approval, and a client-visible commitment never skips a human. There are four patterns and no fifth: ambient card, bulk plan, inline verdict, invisible.

## Part B. Dispatch

### B1. The moment
**Who is here.** The dispatcher, at 07:40, with a day to place; the tech who needs to know where they are going; the account manager checking whether a client's visit is safe.

**What they are trying to finish.** Answer "who is free and where are they going?", place every ticket, and prove none of it is a double booking.

**What gets in the way.** A collision found by the client at 9am. A technician at 110% buried in a table. A plan from software that cannot be overridden.

**How they know they are done.** The queue is empty or deliberately deferred, no lane is over 100% without a reason, and the banner is gone.

**The manifesto test.** To the client, the tech who arrives when promised. To the leader, a day that was planned rather than survived. The AI is a shortcut for a good dispatcher, never a replacement.

### B2. Anatomy (canvas 1600 by 994)

| Region | Size | Notes |
|---|---|---|
| Page header | 64 | Dispatch title, sub-navigation (Board, Planning, Travel), date navigator, New job |
| Toolbar | control + 16 | Views on the left (Day, Week, Unassigned with a count), filter chips, Filter, and Plan with AI only when granted |
| Collision banner | 44 | Only when there are collisions. Blush, with a Review first action |
| Queue | 288 wide | Unassigned tickets with priority filter, select all, and a one-line instruction |
| Lane header | 176 wide | Avatar, name, presence line, utilisation bar and percentage |
| Ruler | 36 | Hours from 07:00 to 18:00, and an ink (gold in dark) chip at the current time |
| Footer | 52 | Legend, density control, keyboard hints |

The board runs 07:00 to 18:00 with **15 minute snapping**. Working hours (08:00 to 17:00) are white; the shoulders are the page tint. Everything before the now-line is quieter.

### B3. Lanes, capacity and travel
- **Utilisation** is the time a technician is busy (job minutes plus travel minutes) over a working day of 480 minutes. **Overlapped time counts once by default**, so two remote jobs side by side do not make someone look overloaded. A setting counts it in full instead. Above 100% the bar's overflow is blush, the percentage gets an alert icon, and the header says **Over by 53m**. It is never colour alone.
- **Travel** is a hatched strip before an on-site job: 12 minutes plus 10 for each zone apart, nothing for remote work and nothing between two jobs at the same client.
- **Presence** in the header line: Remote with the ticket, On site with the client, Free with the next job time, or Off today.
- **Absences** are hatched blocks that behave like jobs for conflict purposes and cannot be dragged.

### B4. Blocks

| Block | Look |
|---|---|
| Scheduled | Stone surface, 1 px border, a 4 px priority bar on the left (red only for P1, blush P2), ticket number, client, title, mode, duration |
| Live (running now) | Gold tint |
| Done | Page background, quieter |
| Proposed | Warm agent tint, dashed border, a numbered badge matching the plan panel. Never inverted |
| Selected | Two pixel ring in the primary colour, with a small toolbar (Split, Unassign, Open) |
| Stacked | Two jobs double-booked on purpose split the lane into two rows, each one line, with a neutral "2 at once" badge and a ×2 chip in the lane header. No hatch, no blush, no alert |
| Colliding | An overlap nobody chose (an overrun, a sudden absence): blush outline, alert icon and a hatched overlap where the two meet |

Hatching is an inline SVG pattern, not a gradient.

### B5. Drag, conflict and the keyboard
Every mouse action has a keyboard twin.

| Action | Mouse | Keyboard |
|---|---|---|
| Place from the queue | Drag a card onto a lane | Enter on a card, arrows to move, Enter to drop |
| Move a block | Drag it | Enter on a block picks it up, arrows move it, Enter drops |
| Resize | Drag its right edge | Plus and minus while picked up, or on a block under the cursor |
| Split | Toolbar Split | S at the cursor |
| Unassign | Toolbar Unassign | U |
| Cancel | Release outside the board | Esc |

The three states the brief asks for are all shown: **the ghost with a gold drop zone** (a dashed gold ghost, the lane outlined in gold, and an ink chip with the time range and duration), **the conflict before the drop** (a blush hatched ghost and lane, and a chip that names what it overlaps), and **the rejection** (the lane flashes blush, the ticket stays in the queue, and a toast says why).

### B5b. Double-booking is a decision, not a mistake
Sometimes a technician is double-booked and that is fine: a remote job waiting on a client while another runs, or a dispatcher who knows a visit will be short. The interface never treats an overlap as an error. It asks once, remembers why, and gets out of the way. Only physical impossibilities are blocked.

| Situation | What happens |
|---|---|
| Free slot | The gold ghost, as before |
| **Overlap** on an occupied slot | The ghost drops into the lower half of the lane as a stacked preview. The lane outline stays gold and a **neutral** chip reads "Overlaps #18421 (13:30 to 14:15). Double-book?" |
| **Release** over an overlap | A card asks one question and offers **one-tap reasons**: Remote in parallel, Waiting on the client, I'll handle it (keys 1 to 3, Esc cancels). Both jobs stay on the board, stacked, and nothing is sent to the client |
| Overlap between **different sites** | The chip and ghost turn blush, with the road distance ("Different sites, 32 min apart"). It is still allowed. Remote in parallel is not offered for two site visits |
| **Blocked** | Someone who is off, a job outside board hours, or a breach of the policy or of the limit. The ghost is a blush hatch, the chip says why, and a release is rejected with a toast. Nothing is red |

**Once approved.** The pair is drawn stacked with a neutral "2 at once" badge, the lane header carries a quiet ×2 chip, and the reason shows in the block's toolbar and its accessible name. Approved overlaps are **never counted in the banner**.

**Collisions are what nobody chose.** An overrun or a sudden absence produces an overlap the dispatcher did not approve. Those are outlined blush, counted in the banner, and offer **Review first** and **Keep it**. Keep it turns a job-on-job collision into an approved overlap with one click. It is never offered for someone who is off.

**Policy** (Dispatch settings, set by an admin for the team): *Never*, *Remote work only*, or *Always ask me* (the default); a limit on jobs at once (default 3); and whether overlapped time counts once (default) or in full. A team that wants strict scheduling picks Never and gets the old behaviour.

**The keyboard has the same steps.** Enter drops, the card opens, 1 to 3 chooses a reason, Esc cancels.

**Planning with AI.** The planner places work in free slots. Only if no free slot fits and the policy allows it may it propose a remote job alongside another remote job, and it marks that row as an approved overlap that can be undone. In the plan editor an overlap **appears the moment it is created** and the row asks for a decision with the same three reasons. Confirm stays disabled until every overlap is decided and nothing is blocked ("Decide on 1 overlap to confirm"). A decided overlap is a neutral "on purpose" pill, not a warning.

### B6. Views
- **Day** is the board.
- **Week** is a matrix of technicians by day: a percentage and a job count, blush and an alert icon with the words "Over by" above 100%, a grey Off cell for leave, and today's column underlined in gold. A cell opens that day.
- **Unassigned** is a table with the data-surface template: selection replaces the header with an action bar (Assign to, Place on the board, Plan with AI if granted). The bulk actions work with no AI, in the same place with the same affordances, so the AI is a shortcut and not a separate product.

### B7. Planning with AI (pattern P2)
Present only when `decision_dispatch` is granted; the viewer bar has a switch to show both worlds.
1. **Scope first.** A popover under Plan with AI offers the current selection, everything in the queue, everything matching the current filter, or a range (the day, the week, the next two weeks). A preview counts the tickets and the hours before anything is proposed, and says they stay in the queue.
2. **A plan to review.** The queue gives way to a 352 px right panel, which is **not inverted**, so the grid keeps its width. Proposed blocks sit on the grid itself, numbered to match panel rows. Each row shows the ticket, a technician select, earlier and later nudges, a remove button, and **the reason** ("Hardware skill, 22 min travel").
3. **Edit visually.** Proposed blocks can be dragged, resized (by the edge), split and moved to another lane, with the same keyboard twins as a manual block.
4. **Overlaps and conflicts appear instantly.** An overlap outlines the block blush and turns its row blush with three reason buttons ("1 to decide"). A hard conflict, such as booking someone who is off, says "Conflict" and cannot be approved. Confirm names what is left ("Decide on 1 overlap to confirm", "Fix 1 conflict to confirm").
5. **Confirm is all or nothing.** If it fails (someone was marked off a moment ago), the panel says **Nothing was booked** and why, with Show the conflict and Retry.
6. **The human boundary.** The footer states that confirmations for clients are prepared for the dispatcher to send, never sent for them.

### B8. Planning and Travel
- **Planning** (read-only): seven stacked bars against a dashed capacity line, P1 red, P2 blush, the rest neutral; a blush label for a day that is over ("Over by 6h") or has no cover; a skill catalogue with Short and Spare per skill. **Loading** is flat skeleton blocks and a sentence.
- **Travel:** a Drafts table with Draft and Confirmed pills, and a **row mark for a possible duplicate** with a Merge link (pattern P3). A drawer form with a validation error in blush. A **14-day reconciliation** comparing planned with logged travel, with an "Off by" mark and a Review link.
- **Scheduling** (meeting types, groups, members) is **skipped**; it is a settings-style form, which the Settings reference covers.

### B9. Density

| Step | Lane | Block | Queue card | Lanes fully visible (12 on the board) |
|---|---|---|---|---|
| Comfortable | 88 | 68 | 92 | 7 |
| Compact | 64 | 46 | 66 | 10 |
| Dense | 44 | 30 | 42 | 12 |

A lane with a stacked overlap grows a little to keep both rows readable (6 px at compact, 14 px at dense). At compact, blocks lose their third line and queue cards their meta line. At dense, blocks are one line, lane headers keep only a name and a percentage (the alert icon stays), and text never drops below 12 px.

### B10. Colour on this surface

| Colour | Means here |
|---|---|
| Gold | Now: the now-line, running jobs, the drop zone, the selected queue card, the current view |
| Blush | Attention: collisions, a hard block, a cross-site overlap, over-capacity, due-soon, validation errors, an over-length day in Planning |
| Red | A genuine SLA breach, and P1 only |
| Green | Chosen or done: the selected AI scope, the "No conflicts" pill, Confirmed |
| Hatch | Something that is not a job: travel, an absence |
| Neutral | A double booking someone chose: the stacked rows, the "2 at once" badge, the ×2 chip and the reason pill |

## Part C. Decisions to confirm

1. **"Plan with AI" trigger.** §6.6 says "enabled only on a selection" while §6.3 lists queue, filter and range as scopes. Wave 1 follows §6.3: the button is always available when granted, and the scope defaults to the selection when there is one.
2. **Double-booking is allowed once the dispatcher says why.** Only someone being off, board hours, the policy and the limit block a drop. Collisions nobody chose stay in the banner. Wave 1 first drew every overlap as an error; this replaces that.
3. **Chat panel entry.** It opens from "Open as a panel" in the palette, so the timer bar gains no new button.
4. **The Ask tab** exists only when general chat is granted.
5. **Roster.** Twelve technicians, so density is a real choice. Lane order puts the likely plan targets in the first visible lanes.
6. **Folder layout.** Theme sits above `states/` (`web/dispatch/neutral/states`), as in the first pack and the mobile convention.
7. **Rail.** Both compositions are drawn (Q1 answered), and the seven-item rail is what the surfaces use.

## Part D. Exact values

Full sets are in `tokens.json`.

| Token | Neutral | Dark |
|---|---|---|
| **Surfaces** | | |
| `--bg` | #F8F7F4 | #2B2A33 |
| `--surf` | #FFFFFF | #34333D |
| `--surf2` | #EFEEEA | #3A3944 |
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
| `--goldtext` | #9A5B0F | #E8B83A |
| **Timeline and agent tints** | | |
| `--note` | #EFECE6 | #3A342A |
| `--agent` | #F7F3E8 | #33302A |
| `--time` | #FBEBBF | #4A3A1A |
| `--sys` | #F8F7F4 | #2F2E37 |
| **Assistant (the inverted surface)** | | |
| `--asb` | #2B2A33 | #F6EFD9 |
| `--asf` | #F6EFD9 | #2B2A33 |
| `--asbtn` | #E8B83A | #2B2A33 |
| `--asbtnf` | #2B2A33 | #E8B83A |
| `--asline` | #5F584F | #CDB38B |
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
| **Done, healthy, chosen** | | |
| `--green` | #5E7F3A | #A9C25D |
| `--gsoft` | #E4EDCB | #2E3A25 |
| `--gtext` | #3B5222 | #D6E5A8 |
| **Floating layers** | | |
| `--shadow` | 0 16px 40px rgba(43,42,51,.18) | 0 16px 40px rgba(0,0,0,.4) |
| `--scrim` | rgba(34,33,41,.55) | rgba(0,0,0,.6) |
| **Logo** | | |
| `--leaf` | #5E7F3A | #A9C25D |
| `--stem` | #7C746A | #CDB38B |

## Part E. Not built
- Wave 1 is desktop only. The overlap policy card is a snippet; the full Settings reference is a later wave. Mobile Scan, the mobile ticket list and detail, Time and the mobile edges are next.
- Scheduling is skipped, as above.
- The date navigator, New job, Columns and Export on the Unassigned view show toasts. The column chooser and export mechanics are the Tickets list's.
- Drag works with a mouse and a keyboard. Touch dragging is not implemented in this mockup.
- All names, tickets and times are sample data, frozen at Tuesday 29 September 2026, 11:41.
