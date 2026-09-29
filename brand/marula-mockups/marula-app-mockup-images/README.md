# Marula app mockups: image pack

These images were rendered from the interactive Marula mockup (`source/marula-app-mockups.html`), so every one is a real state of the design, not a redraw. The pack is meant to let a person or another agent understand both the layouts and the small interaction details.

## Themes

| Platform | Themes in this pack |
|---|---|
| Desktop and web | **Neutral light** (the chosen light theme) and **Dark** |
| Mobile | **Light** (warm cream) and **Dark** |

The same states are captured in every theme, so you can compare them directly. `overview/compare-desktop-neutral-vs-dark.png` and `overview/compare-mobile-light-vs-dark.png` put the two themes side by side.

## Folder map

| Path | Contents |
|---|---|
| `desktop/neutral/states` and `desktop/dark/states` | 22 states each, 1600x994 px (the app canvas at 1x, which suits vision models) |
| `desktop/*/hero-2x` | Three key screens at 2x (3200x1988 px) for people and presentations |
| `mobile/light/states` and `mobile/dark/states` | 13 states each, 964x1872 px (phone frame on the theme background, 2x) |
| `mobile/*/hero-3x` | Three key screens at 3x (1446x2808 px) |
| `overview` | Contact sheets of every state per set, plus the two theme comparisons |
| `source` | The interactive mockup, with fonts embedded so it renders identically offline |
| `DESIGN-NOTES.md` | Anatomy, the rules behind each state, colour meaning, and exact theme values |
| `manifest.json` | Machine-readable index of every image with a title and what to notice |

File names are `NN-state-name.png`. The number is the same in every theme.

## How the images were made

- Fraunces (wordmark, headings and the mobile greeting) and Figtree (interface) are the real brand fonts.
- Time is frozen: the date is Tuesday 29 September 2026 (11:41 on desktop, 13:47 on mobile) and timers do not tick, so digits stay at 00:25:05 (desktop) and 00:47:12 (mobile) unless the state changes them.
- Toasts are captured where they appear in the mockup. On mobile they are placed inside the phone frame.

## Desktop states (same in neutral light and dark)

| # | Image | What it shows |
|---|---|---|
| 01 | `01-workbench-reply-default.png` | **Default ticket workbench, replying to the client.** Composer is in Reply to client mode with the assistant's drafted reply. The assistant's Recommended next step card is the one inverted surface on screen (ink in light, cream in dark). The timer bar is ink in every theme. Stepper shows In progress in gold. P1 chip is red. Label reads Visible to John Smith. |
| 02 | `02-timeline-scrolled-to-top.png` | **Every timeline entry type, in order.** Client entry: bordered surface box with a gold Client tag. Internal note: note-tinted box, no border, Internal tag. System: plain tinted box, System tag. Time entry: gold-tinted box with Billable at right. Assistant suggestion: warm agent tint, spark icon in deep amber, Suggestion tag. |
| 03 | `03-composer-internal-note.png` | **Internal note mode: the composer changes colour.** Switching to Internal note re-tints the whole composer to the same note colour internal entries use in the timeline, so what you type previews where it will land. The label changes to Only your team can see this, the primary button becomes Add note, and the Set to Waiting after send checkbox disappears. |
| 04 | `04-internal-note-added.png` | **Internal note added.** After Add note: a new Internal entry appears at the bottom of the timeline in the tint the composer had. The composer stays in internal mode and is emptied. Toast: Internal note added. |
| 05 | `05-reply-sent-status-waiting.png` | **Client reply sent, status moves to Waiting.** Send reply with Set to Waiting after send ticked: the reply lands as a Client entry, the stepper moves to Waiting (earlier steps gain check marks), and the toast reads Reply sent. Status set to Waiting. |
| 06 | `06-status-set-resolved.png` | **Status stepper set to Resolved.** Clicking Resolved in the header stepper: New, In progress and Waiting show check marks, Resolved is the current step in gold. Toast: Status set to Resolved. |
| 07 | `07-filter-client.png` | **Activity filter: Client.** Active chip is gold-tinted. The timeline shows only client-visible messages. |
| 08 | `08-filter-internal.png` | **Activity filter: Internal.** Only internal notes are listed, in the note tint. |
| 09 | `09-filter-agent.png` | **Activity filter: Agent.** Only assistant entries are listed, in the warm agent tint. |
| 10 | `10-assistant-script-ran.png` | **Assistant action run.** Run profile reset script performed: the button becomes Script ran (disabled) with a check icon, a System entry is appended (Script ran on JOHN-LAPTOP: Always-On VPN profile reset, 0 errors), and a toast confirms. |
| 11 | `11-assistant-suggestion-dismissed.png` | **Assistant suggestion dismissed.** Not this replaces the card with a dashed Suggestion dismissed row and an Undo link. |
| 12 | `12-assistant-question-answered.png` | **Asking the assistant a question.** The Likely cause? chip fills the ask box, submitting appends the assistant's answer to the timeline, with the question in bold. |
| 13 | `13-timer-paused.png` | **Timer paused.** The pause icon becomes a play triangle and a toast confirms. Timer digits are frozen in this export. |
| 14 | `14-quick-note-typed.png` | **Quick note field in use.** The single-line note field on the timer bar, left of the arrow button and Wrap up, with a line typed in. No large UI is needed to add a note. |
| 15 | `15-note-lines-open.png` | **Note lines panel open.** The arrow button is pressed (gold border, dark fill) and shows a count badge (4). A panel above the bar lists every note line with a timestamp; each line is editable in place. The footer explains the lines become the time entry notes at wrap-up. |
| 16 | `16-note-line-added.png` | **Note line added.** Pressing Enter in the quick note field adds a line: badge becomes 5, toast reads Added to your notes, and the panel shows the new 11:41 line. |
| 17 | `17-keyboard-focus-on-send.png` | **Keyboard focus ring.** Two-pixel ring in the primary colour with a two-pixel offset around Send reply, shown on keyboard focus only. |
| 18 | `18-wrap-up-resolve.png` | **Wrap up modal, Resolve selected.** Time entry 0:25 (plus 0:45 logged earlier) with Remote support, Billable and Rounds to 0:30 tags. Notes are built from the note lines. Checklist has green ticks. The chosen outcome card has a two-pixel green border and green tint (green means chosen or done). Send John the resolution summary is ticked. Primary button reads Resolve and open next. |
| 19 | `19-wrap-up-waiting-on-client.png` | **Wrap up modal, Waiting on client.** Selected outcome moves to Waiting on client and the primary button becomes Set to Waiting and open next. |
| 20 | `20-wrap-up-schedule-follow-up.png` | **Wrap up modal, Schedule follow-up.** Selected outcome is Schedule follow-up and the primary button becomes Schedule and open next. |
| 21 | `21-wrap-up-keep-working.png` | **Wrap up modal, Keep working.** Selected outcome is Keep working and the primary button becomes Save and keep working. |
| 22 | `22-wrap-up-completed-resolved.png` | **After wrap up: resolved.** Modal closed. Stepper shows Resolved, the timeline gains a Time entry (0:25 Remote support, Billable), the timer bar resets to 00:00:00, and the toast reads Resolved #18432. Opening #18440 next. |

## Mobile states (same in light and dark)

| # | Image | What it shows |
|---|---|---|
| 01 | `01-today-offline.png` | **Today, offline (default).** Offline banner in the sand tint. Next visit card with a blush Due today pill, a primary Start travel button, and square call and directions buttons. Quick tiles for Scan asset, Add time and Photo. Later list. Pinned running timer (ink bar, gold digits, pause). Tab bar with a raised centre scan button. |
| 02 | `02-today-scrolled-to-bottom.png` | **Today, scrolled to the bottom.** Reveals the Later list that sits under the pinned timer. |
| 03 | `03-today-back-online.png` | **Today, back online.** Tapping the banner toggles it to a green Back online. All changes synced. |
| 04 | `04-today-travelling.png` | **Travel started.** Start travel becomes I've arrived, the pinned timer is relabelled Travel to Ridgeline Dental and restarts, and a toast confirms. |
| 05 | `05-today-timer-paused.png` | **Timer paused.** The pause icon on the pinned timer becomes a play triangle. |
| 06 | `06-on-site-checklist-3-of-5.png` | **On site job screen.** Header with an On site pill. Scanned asset row with a green Healthy pill. Checklist with large tap rows: completed steps have green checked boxes and struck-through text. The inverted assistant card shows notes drafted from the checklist. Photo, Voice and Parts buttons. Bottom bar shows the on-site timer and Wrap up. |
| 07 | `07-on-site-checklist-complete.png` | **Checklist complete.** 5 of 5 done: the count and the assistant's drafted notes update. |
| 08 | `08-wrap-up-resolved.png` | **Wrap up, Resolved selected.** On-site and travel time cards, the drafted notes, an Outcome segmented control (selected segment has the green tint), a dashed signature pad, and a Resolve and sync button with an offline hint. |
| 09 | `09-wrap-up-follow-up.png` | **Wrap up, Follow-up.** Button becomes Schedule follow-up and sync. |
| 10 | `10-wrap-up-waiting.png` | **Wrap up, Waiting.** Button becomes Set to Waiting and sync. |
| 11 | `11-wrap-up-signed.png` | **Client sign-off drawn.** A signature drawn in the pad; the stroke colour follows the theme's text colour. |
| 12 | `12-wrap-up-online-hint.png` | **Wrap up while online.** The hint below the button changes to Syncs straight away. |
| 13 | `13-after-resolve-back-on-today.png` | **After resolving, offline.** Returns to Today with a toast (Resolved #2905. Will sync when you reconnect.) and the banner now reads 4 changes will sync when you reconnect. |

## Start here

If you only open four files: `desktop/neutral/states/01-workbench-reply-default.png`, `desktop/neutral/states/03-composer-internal-note.png` (the colour change between replying to the client and writing an internal note), `desktop/neutral/states/18-wrap-up-resolve.png`, and `mobile/light/states/01-today-offline.png`. Then read `DESIGN-NOTES.md`.
