# Design notes for reading the mockup images

For another agent or designer picking this up. It explains what the screens are made of, the rules that produce each state, and the exact colours.

## 1. The product in one paragraph

Marula is an agentic PSA for managed service providers. The ticket workbench on desktop is where a technician works one ticket from open to closed. The mobile app is for field technicians visiting a client site. In both, an assistant drafts the busywork (replies, time entry notes) and always waits for a person to approve. The example ticket is #18432, VPN disconnects every 20 to 30 minutes, for Acme Corporation; the mobile example is #2905, Server room humidity alarm, at Ridgeline Dental.

## 2. Desktop anatomy (canvas 1600 x 1000, minimum width 1280)

- **Top bar (56 px):** wordmark, search with a Cmd+K hint, Online indicator, theme toggle, notifications, avatar.
- **Icon rail (64 px):** Tickets (active, with a count badge), Clients, Devices, Dispatch, Knowledge, Reports, Settings at the bottom.
- **Context column (290 px):** Customer, Contact (with Call and Email), Device (with Remote in and Run script), Related tickets with priority chips, Other devices with online dots.
- **Centre:** breadcrumb and actions (Link, Escalate with an E hint, more), the ticket title, a meta row (ticket number, P1 chip, SLA countdown, assignee) and the status stepper (New, In progress, Waiting, Resolved), section tabs, activity filter chips, the timeline, then the composer.
- **Assistant column (340 px):** the inverted Recommended next step card with a primary action, View evidence and Not this; Sources used; suggestion chips; an Ask box.
- **Timer bar (60 px, ink in every theme):** pause or play, the running clock in gold, ticket and work type, then the quick note field, the arrow button with a count badge, and Wrap up with a W hint.

Keyboard: N focuses the quick note field, W opens Wrap up, E escalates, Cmd or Ctrl plus Enter sends or confirms, Esc closes panels.

## 3. Mobile anatomy (390 x 844 screen)

- Screens: **Today**, **On site** (the job), **Wrap up**. Today has a bottom tab bar (90 px) with Today, Board, a raised 60 px scan button, Time and More, and a pinned timer pill (56 px) above it. On site and Wrap up replace those with a bottom action bar.
- Every tap target is at least 44 px. Primary actions are 52 px tall.
- Text never goes below 12 px.

## 4. The rules behind the states

**Composer (desktop).**
- Reply to client: the composer uses the plain surface colour. The label says Visible to John Smith, the primary button says Send reply, and a Set to Waiting after send checkbox is shown.
- Internal note: the whole composer takes the internal-note tint, which is the same colour internal entries use in the timeline. The label says Only your team can see this, the button says Add note, and the checkbox is hidden. The mode stays selected after sending.
- Sending with the checkbox ticked moves the ticket to Waiting.

**Timeline entry styles.** Client: surface box with a border. Internal: note tint, no border. System: a quiet tint. Time entry: gold tint. Assistant: warm agent tint with a spark icon. Every entry has a small uppercase tag (except Time entry).

**Status stepper.** The current step is gold. Earlier steps show a check mark. Clicking any step sets it.

**Assistant.** Run action: the button becomes Script ran and a System entry is added. Not this: the card collapses to a dashed row with Undo. Questions: answers are added to the timeline. Nothing client-facing is ever sent by the assistant; drafts wait for approval.

**Notes flow.** The quick note field adds a line on Enter. The arrow button shows all lines in a panel, each editable in place. At Wrap up, those lines become the time entry notes.

**Wrap up.** One modal with the time entry (time worked plus time logged earlier, work type, billable, rounding), the notes, a checklist, an outcome choice (Resolve, Waiting on client, Schedule follow-up, Keep working) and two optional actions. The primary button label always names the chosen outcome. Finishing adds the time entry to the timeline, sets the status, resets the timer and announces the next ticket in the queue.

**Mobile.** Start travel becomes I've arrived and restarts the pinned timer as a travel timer; arriving opens the job. Ticking checklist items updates the count and the drafted notes. Wrap up offers three outcomes (Resolved, Follow-up, Waiting), a signature pad and a sync button whose label follows the outcome. While offline, the hint says changes sync automatically and the queued count rises; while online it says Syncs straight away. Tapping the banner on Today toggles offline and online.

## 5. What colour means

| Colour | Meaning in the app |
|---|---|
| **Gold** | Active, current or primary: the current status step, active filter chip, running timer digits, the primary button in dark mode |
| **Green** | Done, healthy, alive, or chosen: checked boxes, Healthy pills, the selected wrap-up outcome, the online banner |
| **Blush / amber** | Due or needs attention soon: Due today pills, P2 |
| **Red** | Only genuine breaches, P1, or destructive actions |
| **Sand / stone tint** | Internal and private: internal notes and the internal-mode composer, and the offline banner |
| **Ink (or cream in dark mode)** | The single inverted surface on a screen: the assistant card. The timer bar is ink in every theme |

The primary button is ink with gold text in light themes and gold with ink text in dark mode.

## 6. Typography

Fraunces Medium for the wordmark, the mobile greeting and headings; Figtree for everything else. Body 14 px; ticket title 26 px semibold; small caps labels 12 px, uppercase, wide tracking; pills 13 px semibold; timer digits 24 px tabular. Corner radii: pills and chips fully round, desktop buttons 10, mobile primary buttons 14, composer 14, cards 16 to 18, wrap-up modal 20.

## 7. Exact theme values

Desktop uses Neutral and Dark; mobile uses Light and Dark. Light is the original warm-cream theme, and Neutral is the same theme with cooler, less orange backgrounds.

| Token | Light | Neutral | Dark |
|---|---|---|---|
| `--bg` | #FBF7EC | #F8F7F4 | #2B2A33 |
| `--surf` | #FFFFFF | #FFFFFF | #34333D |
| `--surf2` | #F3EAD2 | #EFEEEA | #3A3944 |
| `--side` | #F3EAD2 | #EFEEEA | #222129 |
| `--nav` | #FFFFFF | #FFFFFF | #222129 |
| `--bar` | #2B2A33 | #2B2A33 | #222129 |
| `--fg` | #2B2A33 | #2B2A33 | #F6EFD9 |
| `--sub` | #5F584F | #5F584F | #BEB5A2 |
| `--bd` | #E6DCC2 | #E2E0DA | #43424D |
| `--inp` | #857A66 | #8C877C | #8A8378 |
| `--muted` | #F3EAD2 | #EFEEEA | #3A3944 |
| `--pr` | #2B2A33 | #2B2A33 | #E8B83A |
| `--prf` | #E8B83A | #E8B83A | #2B2A33 |
| `--acc` | #F7E3A8 | #F7E3A8 | #3A3944 |
| `--accf` | #6B4308 | #6B4308 | #E8B83A |
| `--goldtext` | #9A5B0F | #9A5B0F | #E8B83A |
| `--progb` | #FBEBBF | #FBEBBF | #4A3A1A |
| `--progf` | #6B4308 | #6B4308 | #F2CD6B |
| `--client` | #FFFFFF | #FFFFFF | #34333D |
| `--note` | #F3EAD2 | #EFECE6 | #3A342A |
| `--notef` | #4A443D | #4A443D | #E2CFA9 |
| `--sys` | #FBF7EC | #F8F7F4 | #2F2E37 |
| `--time` | #FBEBBF | #FBEBBF | #4A3A1A |
| `--timef` | #6B4308 | #6B4308 | #F2CD6B |
| `--agent` | #FBF3DC | #F7F3E8 | #33302A |
| `--asb` | #2B2A33 | #2B2A33 | #F6EFD9 |
| `--asf` | #F6EFD9 | #F6EFD9 | #2B2A33 |
| `--asbtn` | #E8B83A | #E8B83A | #2B2A33 |
| `--asbtnf` | #2B2A33 | #2B2A33 | #E8B83A |
| `--asline` | #5F584F | #5F584F | #CDB38B |
| `--green` | #5E7F3A | #5E7F3A | #A9C25D |
| `--gsoft` | #E4EDCB | #E4EDCB | #2E3A25 |
| `--gtext` | #3B5222 | #3B5222 | #D6E5A8 |
| `--red` | #B42318 | #B42318 | #FB8A80 |
| `--redf` | #FFFFFF | #FFFFFF | #2B2A33 |
| `--p2` | #D9822B | #D9822B | #D9822B |
| `--dueb` | #F8DCC0 | #F8DCC0 | #4D2F14 |
| `--duef` | #7A3F0C | #7A3F0C | #F0B27A |
| `--offb` | #F3EAD2 | #EFECE6 | #3A342A |
| `--offf` | #5C4631 | #5C4631 | #E2CFA9 |
| `--leaf` | #5E7F3A | #5E7F3A | #A9C25D |
| `--stem` | #7C746A | #7C746A | #CDB38B |

## 8. What is not built in the mockup

- Only the ticket view is designed. The other rail items, the Details, Time, Files and Related tabs, and the mobile Board, Time and More tabs are inert.
- The reply composer has no placeholder text; when it is emptied after sending it shows blank.
- Call, Email, Remote in, Run script, Rewrite and the mobile call, directions, scan, photo and voice buttons only show a toast describing what they would do.
- All names, tickets and numbers are sample data.
