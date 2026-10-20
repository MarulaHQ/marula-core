# Workbar Notes & Tasks Panel — Technical Specification & UI/UX Design

> Status: SPEC (ready for lane implementation) · Authored 2026-09-16
>-line-product owner directive: Nilear-pattern note entry everywhere techs enter task notes.
> Tracking: the private build tracker Wave 17 lane 11 `p17-workbar-notes-tasks` (S1–S8 below are the lane's deliverables).
> Wave/branch renumbered twice 2026-09-16 (Wave 16/17 swap + Wave 15 straggler re-base, then ADR 0009 added
> Wave 16 lane 7 with pair 0079): the lane rides the UX wave with migration pair 0080 / PG port 54380
> (0077–0079 went to Wave 16's backend lanes).
> Governing doctrine: `docs/instant-first.md` (binding), `docs/theme.md` §5 motion law, AGENTS.md override rules, migration-policy §2 posture, `docs/10-agent-platform.md` (approval default).

## 0. Why — the Nilear pattern

Nilear MTX treats working a ticket as a continuous state rather than a sequence of forms
(`internal/design/marula_psa_ticket_workbench_chatgpt_design_doc.md` line 13: "One Working Ticket
Method keeps one ticket actively in progress … time, notes, communication, knowledge and
agent actions all emerge naturally from the same work session"). The dark bottom WorkBar
(`apps/web/components/timer-dock.tsx`) already carries the live timer, quick-note input,
and Wrap up. This spec adds the missing "same work session" surfaces to that bar: a
one-keystroke Notes+Tasks notepad that slides up over the page, entry semantics that keep
client-facing reports uniform, task status actions that feed the notes trail, and a close
gate that makes "every task has a status and a note" a hard precondition — auditable
end to end.

Two binding rules the lane must honor:

1. **One note-entry pattern everywhere.** The composer built for the workbar panel (§2)
   is extracted into a shared component and adopted where techs enter task/note content —
   workbar quick note, workbench-v2 floating composer, and any checklist-item note input.
   Divergent note-entry UIs are not allowed to accrete.
2. **Auditability is not negotiable.** Every action in this spec that changes state
   (note submit, note format, task status change, close-gate auto-close, override
   close) flows through the existing instant-first envelope, on the write path that
   already carries an idempotency key and an audit row (`core_api::instant`, `core_db::audit::insert`
   in the same tx). No parallel writes. Nothing is "just UI".

---

## 1. Pillar 1 — UI animation & interaction design

### 1.1 Placement — the Up Arrow trigger

- **Location:** immediately adjacent (left) of the existing quick-note input
  `data-testid="workbar-note-input"` in the WorkBar action cluster (`timer-dock.tsx`
  right-side group, before Pause/Switch/Wrap up). Icon: chevron / arrow pointing UP,
  12×12 stroke SVG in house style (matching Pause/Switch/Wrap `strokeWidth 1.4–1.6`).
- **Size/shape:** 32×32 (theme control height), `border-radius: 0.75rem`, ghost button
  style `m-workbar-ghost` variant. Disabled (visually dimmed, `aria-disabled`) when no
  current session but never hidden while the bar exists.
- **Affordance mirroring:** the quick-note input keeps focus-seeded typing. Pressing
  `ArrowUp`-labeled button opens the panel **with the composer focused** (focus moves,
  see 1.4). Tooltip "Notes & tasks (⌃N)".
- **Keyboard:** global shortcut `⌘/Ctrl + ⇧ + N` toggles the panel while the WorkBar is
  mounted (ticket-scoped. No session → pressed keys route is inert).
- **a11y:** `aria-expanded` on the trigger; `aria-controls="notes-task-panel"`. The
  trigger is the only always-visible affordance — the panel is a layer, not a pane.

### 1.2 The notepad slide-up panel — "slightly skeuomorphic"

- **Surface:** a panel anchored bottom-center (max-width 720px, desktop ≥1024), full
  width minus 16px gutters on tablet, near-full-screen sheet on <768 (mobile guarantees
  at least the composer + tasks list without horizontal scroll).
- **Rise animation (THE motion):** panel translates up from the WorkBar's top edge using
  `transform: translateY(24px → 0)` + `opacity: 0 → 1`, duration **120ms** (upper bound
  of the motion law. Use 100ms for dismiss, 120ms for open), single decel curve
  `cubic-bezier(0.2, 0.8, 0.2, 1)`. **transform/opacity only** — never width/height/top.
  `prefers-reduced-motion`: opacity-only 1ms, effectively instant.
- **Skeuomorphic cues (subtle, not campy):** the panel reads as a **small paper pad
  sitting on the workbar**, not a generic popover:
  - Paper surface: `--background` token. A faint ruled-line background REPEATED every
    28px row-height unit ONLY behind the note list region (CSS
    `repeating-linear-gradient` on a token-defined divider color at 8% alpha) — the rule
    spacing matches the actual note row rhythm so lines look ruled, not decorative.
  - A "torn top" hint: 2px top inset shadow + 1px `--m-border-dark` edge. Rounded
    top corners 0.75rem, bottom corners squared where it meets the WorkBar (the pad
    visually grows out of the bar).
  - Layered shadow: the preset overlay shadow scale (theme §5 — shadows reserved for
    overlays, this is one): `0 12px 32px rgba(0,0,0,.18)` in dark, `rgba(0,0,0,.10)`
    preset in light.
  - Tiny rotate: `transform: rotate(-0.35deg)` on the PAPER LAYER ONLY (a wrapper div,
    not the scroll container — rotation does not affect hit-testing), giving the pad a
    hand-placed feel. Collapses under reduced-motion too.
  - Header mimics a pad margin: vertical accent rule (3px, `--primary` at 60% alpha)
    running down the left edge of the title row, like a notebook margin.
- **Overlay behavior:** page content behind dims to a 24% scrim (`--foreground` at 6%
  over the normal background. No blur — blur violates perf budgets). Clicking the scrim
  is a DISMISS GUARD: if composer has unsent text, first scrim click shows a 1-time
  inline hint "⌘↵ to send · click again to dismiss" (state cached 5s). Second click
  dismisses. `Escape` always dismisses unless a modal inside (reason prompt, 4.3) is
  open. It closes the reason prompt first.
- **Stacking:** z-layer above page content but below the wrap-up sheet and any native
  `<dialog>`. Only one WorkBar panel instance globally (it is per-session, session-
  keyed by `current.ticket_id`. Switching tickets with the panel open re-targets it and
  re-hydrates).
- **Focus management:** open → focus composer textarea. Close → focus returns to the
  trigger button. Full keyboard trap within panel while open (standard sheet semantics;
  `role="dialog"` `aria-label="Notes and tasks"`).
- **Entrance choreography:** note list items cascade in with **stagger ≤40ms/item**
  (theme §5 cap) on first open only. Subsequent opens render settled. Tasks section
  slides under notes with a 30ms delay. All fully inside the 120ms budget because
  staggered items ANIMATE IN PARALLEL and the budget is per-item, per the theme's cap
  definition.

### 1.3 State & persistence

- Panel open/close is view state, NOT durable server state. But composer text and the
  tasks-section scroll position persist across open/close within the session
  (React state lifted to the WorkBar shell, never unmounted — `display` swap, not
  conditional unmount, to protect draft text and scroll).
- Draft autosave: composer content is debounced-autosaved to the ticket as an
  internal-note DRAFT, never a comment, via the IF2 autosave model: a `note_drafts`
  keyed entity (see §5.2) with idempotency key + entity version, so an interrupted
  session or browser reload restores the draft. Draft is per (user, ticket).
  Autosave indication: quiet 3-dot pulse in composer footer while saving. "Draft
  saved · 12:04" on settle.

---

## 2. Pillar 2 — Notes panel functionality & formatting

### 2.1 Layout — the notes ledger

- **Section order inside the panel (top→bottom):** header row (title "Notes & tasks" +
  ticket number chip + close ✕), TASKS strip (§3), NOTES list (the majority), composer
  pinned at the bottom (directly above the WorkBar edge — visually "on the pad").
- **Every internal note rendered**, newest-last (reading order = composition order. The ledger reads like a pad you filled in). Long lists scroll INSIDE the panel
  (sticky header/composer, middle scroll — never a page-level scrollbar).
- **Line separators between entries:** 1px `--border` full-bleed divider between note
  cards, with 12px vertical padding either side. Individual notes are NOT boxed cards —
  the divider-only separation + ruled-paper backdrop is the ledger look (distinct from
  timeline bubbles in the workbench). Divider color = the same token the ruled lines
  use, reinforcing the "lines on a pad" metaphor.
- **Compact meta line per note (11px, `--muted-foreground`):** time
  (`HH:MM`, tooltip full ISO) · note kind chip if not internal (`Client-visible` /
  `In-house update` pills, theme §3.3 semantic pairs) · attachment paperclip count
  when present · "edited" marker when `updated_at` exists.
- **Client-visible replies made from this panel are visibly out-of-band:** they render
  LEFT-ALIGNED with a 3px `--info`-tone left border, so a tech scrolling the pad can
  see at a glance which lines the customer reads. Panel default intent stays
  `internal` (§2.3), matching "internal notes" purpose of this spec.
- **Empty state:** one sentence + one action (theme §6): "No notes yet — type the first
  one below." (No illustrations.)

### 2.2 Attribution — circular avatars with initials

- **Each note row begins with a 24px circular avatar**, `border-radius: 50%`
  (theme §5 allows `50%` for avatars ONLY), containing the author's INITIALS
  (first+last initials of `display_name`, e.g. "Jordan Reyes" → `JR`. Single-name
  users → first 2 letters). Background: deterministic per-user pick from the theme's
  chart ramp (§3.4) — hash of user id maps to one of the ramp steps. Foreground always
  `--foreground` on light ramp steps or `--background` on dark steps (contrast AA at
  24px is verified with the ramp's documented pairs).
- **The avatar identifies BOTH the author and implicitly the session (the WorkBar
  session being tracked when the note was entered):** each note row whose timestamp
  falls inside the currently-tracked timer session's span gets a 2px
  `--primary` ring on its avatar + a 10px `●` work-dot before the meta line ("this
  session"). Hover tooltip: "Jordan Reyes · this session · 11:42". This is the session
  attribution required by the spec — author initials + session ring, no extra
  machinery.
- **Current user's own notes:** avatar initials + a subtly stronger text weight on
  the name fragment in the meta line. No "me" chip (the ring + familiar layout carries
  it).
- **System-generated note fragments** (auto-appended task text, §3.4) use a distinct
  neutral avatar: circular, `--muted` bg, `⚙` glyph (or generic `M` system mark) —
  a machine never wears initials. This is an auditability surface: humans vs machine
  provenance is scannable.

### 2.3 Input behavior & keyboard model (binding interaction contract)

The composer (shared component — rule §0.1) is a multi-line textarea, NOT an
`<input>`. Behavior:

| Keys | Action |
|---|---|
| `Enter` | **New line inside the draft** (default textarea behavior preserved) |
| `Shift + Enter` | **Soft line break with explicit continuation semantics:** behaves identically to Enter (new line) but suppresses any autosave-debounce flush — implemented as a no-op decorator so autosave keying treats the two identically. The pair exists because muscle memory differs across apps, and honoring both avoids wrong-expectation bugs. (Both never submit — see next row.) |
| `⌘/Ctrl + Enter` | **Submit the note** (fires the envelope mutation). Works from any cursor position. Does not insert a line. |
| `Escape` | Dismiss panel (subject to scrim/unsent guard, §1.2) |
| `⌘/Ctrl + ⇧ + F` | Format/neaten draft (see §2.5) |

- **Submit status:** `aria-live="polite"` region above composer ("Saving…", "Note
  saved", friendly typed error from the envelope — never raw prose).
- **Composer capacity:** max 20,000 chars (envelope-guard parity with comment body
  limit). Counter appears only above 18,000 (quiet by default).
- **Placeholder copy:** "Add note — Enter for a new line, ⌘↵ to save".
- **Paste behavior:** plain-text paste (rich text is stripped client-side. The
  auto-formatter §2.4 would otherwise fight pasted markup). URLs pasted as the entire
  line become markdown links at submit (§2.4 rule L3).

### 2.4 Auto-formatting on submit — bullet standardization (the client-facing guarantee)

Submitting a note posts through `tickets_add_comment` with a rewritten **body**. The
rewrite is a PURE FUNCTION on the draft (`formatNoteBody(draft: string): string`,
client-side, unit-tested. Mirrored server-side later only if mobile/offline replay needs
it — for now the client normalizes and the server stores what it receives, exactly like
pastes today). Rules, applied in order:

- **B1 — Trim:** strip leading/trailing whitespace lines. Collapse 3+ consecutive blank
  lines to 1.
- **B2 — Bullet each line:** every non-empty line becomes a bullet. Leading
  user-typed bullets/dashes (`- `, `* `, `• `, `– `) are normalized to the canonical
  `- ` marker (idempotent). Lines already starting with ordered numerals
  (`1.`, `1)`) are PRESERVED as typed (numbered lists the tech chose stay numbered).
- **B3 — Preserve intentional indentation:** up to 3 leading spaces are kept under
  a bullet (sub-bullets survive: `  - sub item`).
- **B4. Sentence capitalization:** each line's first character is uppercased
  (title-case names NOT attempted, only the first letter. No lowercasing of
  following words, so acronyms/proper nouns are safe).
- **B5 — Terminal punctuation:** no forced addition (adding periods users didn't
  write reads wrong in MSP notes. The formatter may be taught this later, out of
  scope here).
- **Links — L rules:** URLs (`http(s)://…`, `www.`-prefixed) are left verbatim as
  text (the API stores plain markdown. Every render layer renders links). A bare
  line that IS a URL gets a friendly auto-title ONLY when the URL host matches a
  known/internal pattern? — NO: never auto-fetch titles (network on submit path
  violates instant-first side-effects-off-the-write-path). Links stay raw-verbatim,
  rendered clickable (this is rule L1). L2: markdown links `[label](url)` pass
  through untouched. L3: a line consisting solely of a URL with no protocol gets
  `https://` prefix added at submit (safe, deterministic).
- **B6. Idempotence:** `format(format(x)) === format(x)`, properties unit-tested.
- **Display symmetry:** the notes list (and workbench timeline, via shared renderer)
  renders `- ` lines as sheet bullets. What the tech typed as lines reads as a clean
  bulleted summary everywhere — including client-visible exports. This is the
  "standardized, professional appearance for client-facing reports" guarantee: every
  line becomes a bullet, uniformly, in one style.

### 2.5 The "Tidy" button — automatic neaten-up (formatting on demand)

- **Trigger:** a small "Tidy" icon-button (sparkles/wand glyph, tooltip "Tidy note —
  capitalization, grammar, bullets") in the composer tool row NE of the input,
  plus keyboard `⌘/Ctrl + ⇧ + F`.
- **Two-tier mechanism (binding to the AI-doctrine):**
  - **Tier 1 — deterministic (`<not AI>`):** runs `formatNoteBody` (§2.4 rules) on the
    current draft IN PLACE, showing a 120ms text-swap micro-transition. Available
    ALWAYS, offline included, no network. This alone delivers capitalization +
    bulleting + whitespace.
  - **Tier 2. Grammar pass (AI seam):** a "Refine wording" affordance rides the
    existing `CompletionProvider` BYOK seam (`crates/core-domain/src/completion.rs`) —
    new route `POST /api/tickets/{id}/notes/refine` that sends the draft to the
    configured completion provider. **The result is a PROPOSAL, never a direct write**
    (`docs/instant-first.md` §agent-changes / `docs/10-agent-platform.md`): a diff view
    in the composer (added lines green-tinted, removed struck) with **Apply** /
    **Discard**. Apply performs the normal submit-path write with the edited body —
    the AI never touches the ledger directly. Provider `none` → typed "not available
    yet" state on the Tier-2 affordance (reference pattern: Template button behavior in
    the workbench composer), Tier 1 still functional.
- **Auditability of formatting below.** Any Tier-2 apply keeps the ORIGINAL draft in
  the draft autosave history (IF2 record) — audit shows original → formatted delta
  (see §6).

### 2.6 Attachments (images + files)

- **Attachments (images + files):** the composer tool row gets an Attach clip control
  riding the EXISTING endpoints:
  1. `POST /api/v1/tickets/{id}/attachments` (upload-first flow, exactly the
     composer's current pattern in `workbench-v2/composer.tsx` — staged files upload
     immediately, `attachment_ids` handed to the comment create on submit);
  2. comment create consumes `attachment_ids` (field already additive-shipped in
     `AddCommentRequest`).
- **Image treatment:** image attachments get inline thumbnails inside the note row
  (max-height 96px, click → existing attachment viewer/lightbox route if present,
  typed fallback listing name+size). Non-image attachments render the paperclip +
  filename + byte-size row.
- **Paste-upload:** pasting an image from the clipboard into the composer stages the
  upload immediately with a small "Uploading… 45%" chip (envelope-typed failure on
  error, retry affordance).
- **Hyperlinks in notes:** the composer offers a link button (chain icon) wrapping the
  selection as `[label](url)`. There is NO WYSIWYG — the underlying body stays the
  plain markdown the API already stores (consistency + auditability). A bare selected
  URL + link button prompts for a label via the same tiny popover (label optional —
  empty label = raw URL text, rule L1).
- **Uploads while kind = client_visible:** permission/danger styling identical to the
  workbench composer's (attachments on client-visible rows are the customer's
  downloads. A confirm chip "Customer can see this file" if the file is flagged
  internal-only — defers to existing attachment visibility rules. No new model).

---

## 3. Pillar 3 — integrated task management logic

### 3.1 The tasks strip

- **Tasks section at the top of the panel** (§2.1 order — tasks first: the panel is
  "work first, ledger second"), titled "Tasks" with a live count chip
  (`3 open · 1 N/A · 2 done`) and a collapse toggle (chevron. Collapsed state
  remembered per session, not per user).
- **Task source of truth: the ticket's checklist**
  (`GET /api/v1/tickets/{id}/checklist` — `TicketChecklistView { items, progress,
  enforcement_active, blocking }` in `routes/tickets.rs`). The checklist IS the
  ticket-scoped task list in this product (project tasks under
  `/api/projects/{id}/tasks` belong to project engagements and are NOT surfaced
  here — cross-nav only, no duplication). Template-derived items show a small
  `required` pill when the board enforces. Ad-hoc items never carry it.
- **Row anatomy:** status glyph (☐ pending / dotted when checked elsewhere…) ·
  task text (inline `formatNoteBody`-formatted since techs author these too — rule
  §0.1) · assignee avatar (16px mini-initials when assigned) · tri-state action
  cluster on hover/focus (§3.2) · `required` pill as applicable.
- **State model = exactly the server's tri-state:** `checked` (done), `na`
  (not applicable), neither = open. The UI never invents a fourth client-only
  state (in-progress / blocked are out of scope here — inconsistent state languages
  on one ticket break the close gate).
- **Passive state (binding):** tasks live in the strip with NO required action —
  open/unchecked tasks are simply present. The strip never badgers, never pulses,
  never nags. Action only happens on explicit user input (§3.2) or at the close gate
  (§4). This is the "ensure tasks can remain in their current state without
  requiring immediate action" requirement.
- **Mutations ride the existing toggle endpoint** `POST
  /api/v1/tickets/{id}/checklist/{item_id}` (`SetChecklistItemRequest { checked, na,
  + instant envelope }`), one state flip per call, optimistic render + typed
  reconcile per instant-first. The mutual-exclusion server rule (checked+na
  forbidden — Wave 12 lane 6) is enforced client-side too (radio-group semantics in
  the row UI).

### 3.2 The three action buttons per task

Each task row exposes exactly three actions (hover-revealed on desktop. Always-visible icon row on touch. Every action keyboard-reachable — the row is a
tab stop, actions on `⌘1/⌘2/⌘3`-style within-row shortcuts):

| Button | Meaning | Wire call | Consequence |
|---|---|---|---|
| **Mark Completed** (check glyph, success tone on hover) | work verified done | `set_checklist_item { checked: true, na: false }` | + auto-append to notes (§3.3) |
| **Mark Not Applicable** (slashed-circle glyph) | step doesn't apply on this ticket | `set_checklist_item { checked: false, na: true }` | + MANDATORY reason (§3.4) |
| **Mark Incomplete** (re-open glyph / empty box) | reverse a done/N-A state or explicitly decline to complete | `set_checklist_item { checked: false, na: false }` | + MANDATORY reason (§3.4) — mandatory when this action CHANGES a previously-done state |

- **Labels:** buttons use text+icon on ≥1024 viewport, icon-only with `aria-label` +
  tooltip below that. The Incomplete button is disabled (dimmed, not hidden) on rows
  already open (nothing to undo) — but see the reason rule above. Clicking a disabled
  action is a no-op by definition.
- **Optimistic + typed failure:** row glyph flips immediately (instant-first), 409/422
  renders an inline row-level error chip with retry. Errors never toast-disappear.

### 3.3 Automated workflow — Mark Completed appends to notes

On a successful `checked: true` transition, the SYSTEM writes a note to the ticket —
not the user, and not a fake client-side render:

- **Mechanism:** the same UI transaction (after the checklist toggle response lands)
  posts `tickets_add_comment` with `kind: "internal"` and a body composed by a
  shared helper:
  `formatNoteBody("**Completed:** " + item.text)` — i.e. the standardized formatter
  guarantees the ledger line reads like every other line (bullet, capitalized),
  with a bold "Completed:" lead. The comment body is plain markdown
  (`**Completed:** Fix DNS forwarding`) — the bulleting then happens through §2.4 so
  the stored body is `- **Completed:** Fix DNS forwarding`.
- **Provenance — auditable system attribution:**
  - The note is AUTHORED BY THE USER (their `author_id`), because the action was
    theirs. It carries the ⚙-style meta marker (§2.2's system variant list shows both:
    avatar = initiating user's initials. Meta line adds a machine pill
    `auto · task complete`) so humans and the audit trail can distinguish manual
    notes from workflow-generated ones. Implementation: this is mark-up only IF the
    server supports a metadata field. If the current `TicketComment` has no free
    metadata column, the lane ADDS ONE (additive migration, §5: `workflow_meta jsonb
    NULL` on comments) — NEVER a body-text sentinel ("[generated]" strings are
    spoofable and unqueryable). This is a hard requirement. It keeps the
    auditability story queryable (`WHERE workflow_meta ? 'task_complete'`).
  - The append is idempotency-keyed ON THE ITEM'S STATE TRANSITION
    (key = `task-complete:{ticket}:{item_id}:{checked_at}`-shaped) so double-clicks,
    retries and offline replays never double-append. Toggle response already
    returns the mutation envelope (read-your-write) — the note post uses that
    timing + the envelope's seq ordering.
- **Ordering guarantee:** on screen the appended note appears at the pad's bottom
  immediately after the task flips (optimistic), reconciles via the normal
  comment-added WS event. Other viewers of the same ticket see the appended note
  arrive through the event stream with the same auto pill (consistent everywhere).

### 3.4 Conditional workflow — Mark N/A / Mark Incomplete MUST have a reason

- **Mandatory reason prompt:** clicking Mark Not Applicable or Mark Incomplete-open
  opens an inline reason field that expands beneath the task row (NOT a modal —
  the scrim/stacking rules stay intact), focused, with:
  - Label: "Reason (required) — e.g. 'Customer already resolved via vendor'"
  - A submit button "Save & mark N/A" (or "Save & mark incomplete"), disabled
    until non-empty (trimmed).
  - `Escape` collapses the prompt without the action; `⌘↵` submits.
- **Why mandatory, and where it goes:** the reason TEXT flows BOTH ways:
  1. into the checklist item state change (the toggle call, envelope-ordered), AND
  2. as an auto-appended internal note REPLACING the silent append of §3.3 with the
     reason-forwarded variant: `- **N/A:** {reason}` or
     `- **Reopened:** {reason}` (same helper §3.3, same system pill, same
     idempotency key shape `task-na:{…}` / `task-reopen:{…}`).
  Both writes happen in ONE server transaction — the lane extends
  `SetChecklistItemRequest` additively with `note_body: Option<String>` (or the two
  posts are server-side-composed. The lane picks the additive-request variant and
  documents it in the route doc-comment). Client-fakes of "two posts in sequence"
  are NOT acceptable: a crash between them would leave a flipped task with no ledger
  line — the exact auditability hole this spec exists to close. Server-side
  compose-in-tx is the deliverable (§5.1).
- **Reason ≠ comment spam:** the reason lives in the LEDGER (notes) as the human-
  readable record. The checklist item keeps only its state. The ledger is the
  narrative. The strip is the state. Never the reverse.

---

## 4. Pillar 4 — workflow validation & close-guardrails

### 4.1 The close gate — trigger condition

**Trigger:** whenever a ticket transitions to a CLOSED-category status
(`target.category == TicketStatusCategory::Closed` in the status-change path) while
any checklist item on the ticket is in the UNASSIGNED state (neither `checked` nor
`na`). This is the "unassigned status" definition in this product's terms — an open
task with no terminal decision.

- **Where it lives:** the gate is a BACKEND-rule first, UI second — enforced in the
  existing close/status change transaction in `routes/tickets.rs` (the same block
  that already gates on the pre-close checklist. The new rule runs AFTER
  checklist-instantiate so lazily-added template items are counted). A raw API
  caller (curl, integrations) hits the same typed rejection — never only a client
  modal.
- **Typed error contract:** new typed 409 variant `TasksUnassigned { items:
  Vec<TaskGateItem> }` where `TaskGateItem { item_id, text, required: bool }`.
  Additive to the error envelope (`docs/instant-first.md` typed codes doctrine. Add a
  `map_unique_constraint`-style entry per the standing convention note for any new
  constraint — n/a here but the pattern reminder applies to the migration).
- **Existing gates compose:** the pre-close REQUIRED-item checklist gate (already
  shipped) and this gate run together. A close attempt can surface BOTH (response
  carries whichever blocked first. UI renders both surfaces stacked — see 4.2).

### 4.2 The validation prompt (UI)

On catching the typed 409 in ANY closing surface (workbench wrap-up sheet Resolve
path, classic workspace close flow, workbar Wrap up → Resolve, portal admin flows):

- **A centered decision dialog** (theme §6 dialog treatment, not a toast): title
  "Tasks still open on #{number}". Body lists the unassigned items (icon-scanned
  rows, requirement pill where applicable). Copy states plainly:
  "Every task needs a completed / N-A status before this ticket can close."
- **The two resolution paths (spec-required, exact):**
  - **Option A — "Close all tasks automatically."** One click marks every listed
    item `checked` (+ appends the §3.3 auto note per item, batched in ONE
    envelope request with per-item idempotency keys), then RE-ISSUES the original
    close. The batch close-all writes land in one transaction
    (§5.3 batch endpoint). A confirmation sub-line states what will be written:
    "This marks 4 tasks complete and adds a note for each." The bottom of the
    dialog marks this as a REPORTED action: `Closed by: auto-close at close gate`
    lands in each appended note's workflow_meta with `auto: true`.
  - **Option B — "Manually assign statuses and notes to each remaining task."**
    === navigational: closes the dialog, opens the Notes & Tasks panel (§1)
    pre-scrolled to the tasks strip with the first unassigned row's action cluster
    revealed and its row highlighted. The user works down the list (N-A/incomplete
    rows naturally collect their mandatory reasons §3.4). The close re-attempt
    happens manually afterwards — NO silent auto-continue (the user said "manual". The system must not metal around).
  - A third quiet affordance "Not now" (cancel) — closes the dialog, keeps the
    ticket open. Closing never proceeds implicitly.
- **Accessibility:** dialog traps focus. The item list is a real list (aria-list). Option A button is destructive-adjacent (it performs writes) so it is styled
  secondary-confirm, NOT primary — the dialog's emphasis pushes toward manual review
  (Option B is the emphasized first button. "Close all" second). This ordering is
  deliberate: automatic status writing is a convenience, not the product's default
  ethic.

### 4.3 Where the gate does NOT apply

- Waiting/schedule/keep-working paths out of the wrap-up sheet (non-Closed
  categories. The gate is closed-category only).
- Tickets with EMPTY checklists (vacuous pass — same semantics as the existing
  `is_complete()`).
- Merged tickets post-merge (survivor inherits states. The gate evaluates the
  survivor's state at close time — merge itself never flips statuses silently. Cross-ref Wave 11 lane 9 merge marker).

---

## 5. Backend & data changes (additive-only)

> Migration lane rule: one lane-exclusive migration number (next free pair per
> the private build tracker at fire time), additively only, rollback-safe, RLS unchanged,
> U9-customization cross-check if any construct referenced by customization rows
> is touched (none expected).

### 5.1 Server-composed task→note append

- `SetChecklistItemRequest` + additive `note_reason: Option<String>` (strictly
  bound to semantics: when `na: true` or a reopen is detected, requiring a
  non-empty trimmed reason ≤2000 chars, else typed 422).
- In-TX side effect: compose the §3.3/§3.4 body via a SHARED server helper (Rust
  fn, unit-tested, same rules as the client's `formatNoteBody` — implement once in
  `core-domain` as `notes::format_note_body` with the optional client parity shim,
  so the rules live in one place and tests cover `idempotence`), insert the comment,
  set `workflow_meta` (below), keep both writes in the single org_tx. Emits BOTH
  events (`ticket.checklist_updated` + `ticket.comment_added`) in the same tx.
- Batch variant for close-gate Option A: `POST /api/v1/tickets/{id}/checklist/
  complete-all { task_ids: [..], append_note: true }` (envelope batch semantics:
  per-item idempotency keys derived `complete-all:{ticket}:{item}`; 200 returns the
  full updated view + appended-comment list so a client reconciles in one round
  trip). Batch is also usable pre-close (not only at the gate) — the gate reuses it.

### 5.2 Note drafts (IF2 autosave store)

- Additive table `note_drafts (org_id, ticket_id, user_id, body, updated_at)` +
  PK(org,ticket,user). Envelope-aware autosave route
  `PUT /api/v1/tickets/{id}/note-draft` with idempotency key + version semantics
  (IF1/IF2 contract), and delete-on-submit (server deletes the draft row when a
  comment with the same origin key commits — the draft's `idempotency_key` links
  it to the submit. On submit the SAME key claims the write, and the successful
  comment mutation clears the draft).
- Draft retention: 30 days then purged (a draft older than that is stale
  assuming-never-finished). Autosave cadence 800ms debounce, immediate on
  panel-hide and blur.

### 5.3 Comment provenance metadata

- `ticket_comments.workflow_meta jsonb NULL` (additive migration). Contains at most:
  `{ "kind": "task_complete" | "task_na" | "task_reopen" | "task_closeall_"
     {item_id…}, "auto": bool, "item_ids": [uuid…] }`.
  Written only by the server-composed append path. User-written comments never
  carry it. Queryable for audit: "which notes were auto-appended by workflow" is a
  WHERE clause, not a text search.

### 5.4 OpenAPI pin discipline (DX12)

Any endpoint touched above regenerates `packages/sdk/openapi.json` +
`fixtures/openapi/v1/openapi.json` as the COMPACT byte-identical pair +
`pnpm --filter @marula-core/sdk generate`. CI pin check must pass.

---

## 6. Auditability & provenance (the non-negotiables, consolidated)

Every action in this spec produces exactly one authoritative, queryable trail of
records — built from existing machinery, no parallel audit store:

| Action | Trail |
|---|---|
| Note submit | `ticket.comment_added` event + `instant` mutation record (seq, idempotency key, before/after) + audit row in same tx. Attachments linked via `attachment_ids` (their own upload audit rows exist already). |
| Notes formatted (Tier 1 Tidy) | Pure client-side text transform **before any write**: the audit trail sees ONE comment create with the final body. No audit entry needed for a never-committed edit — the draft autosave ledger keeps the pre-tidy draft (§5.2) recoverable within the draft-retention window. |
| Notes refined (Tier 2 grammar) | Refine is a PROPOSAL: no write until Apply. On Apply: normal comment event + `workflow_meta.kind = "refined"` + audit row shows the user-approved delta. Original draft persists in §5.2. AI provenance (provider, model) recorded in the proposal record per `docs/10-agent-platform.md` — never in `workflow_meta` (which is server-workflow only. AI provenance rides the approval pipeline). |
| Task marked any state | `ticket.checklist_updated` event + envelope mutation + audit row (who/when/from→to) — the existing `set_checklist_item` path already does this. Reason text additionally lands as an audited comment (§3.4, same tx). |
| Auto-append (complete/N-A/reopen) | Same comment trail as manual notes + `workflow_meta` marks machine origin + `auto: true` + `author_id` = initiating user |
| Close-gate Option A batch | Batch endpoint audit: one audit row per item state change + one per appended note + the close attempt's own re-audit on success; `workflow_meta.auto = true` on every appended note |
| Draft autosaves | IF2 autosave ledger (§5.2) — not audit-trail material (transient work product), but per-record history exists for undo/forensics within retention |
| Read of trails | `GET /api/audit-events` (existing, filterable) + `GET /api/provenance` (existing) surface all of the above. The lane ADDS provenance-grade filters if needed for `workflow_meta ? 'kind'` queries |
| Undo interplay | Undo of an appended task-note reverts the COMMENT only (a normal audited revert per instant-first undo doctrine). The checklist item's state is untouched by comment-undo — states and narrative are separately reversible, and both reversals are themselves audited |

**Two invariants (test-enforced in the lane's suite):**
1. A checklist status flip and its ledger note commit together — no crash window can
   produce "flipped with no note" (single-tx compose, §5.1) and no retry can produce
   duplicate notes (idempotency key includes the transition identity).
2. Machine-origin notes are machine-marked (`workflow_meta`) at the row level —
   no body-text sentinels, no spoofable prose markers.

---

## 7. Non-goals

- No editing/deleting of committed notes (matches the current API's create-once
  posture. Corrections are follow-up notes).
- No note threading/replies (the ledger is flat by design. Threaded notes break the
  client-facing report format).
- No rich-text/WYSIWYG notes (plain markdown + links + attachments only).
- No project-task (`/api/projects/{id}/tasks`) management here — project tasks surface
  remain their existing views. Only cross-nav, no state duplication.
- No mobile-native WorkBar redesign (panel responsively adapts, §1.2. The mobile
  WorkBar pattern is a separate lane).
- No auto-fetching of link titles or URL previews (side-effects-off-write-path).

## 8. Acceptance checklist (lane exit criteria)

- [ ] Up-arrow trigger adjacent to the workbar note input; `⌘⇧N` opens. A11y states verified
- [ ] Panel rise: translateY+opacity, ≤120ms, reduced-motion collapse, transform-only (CI motion audit / manual verify in both themes)
- [ ] Skeuomorph: ruled lines aligned to row rhythm, paper layer, margin rule, overlay shadow — light AND dark parity screenshots in `internal/design/qa-workbar-notes/`
- [ ] Notes list: divider-separated ledger, initials avatars (ramp-picked), session ring on tracked-session notes, machine avatar on auto notes
- [ ] Keyboard: Enter=new line, Shift+Enter=soft break (no submit), ⌘/Ctrl+Enter=submit. Verified in composer + tasks reason prompt
- [ ] Auto-format: every submitted line is a bullet per B1–B6 (unit tests incl. idempotence). Numbered lists preserved. Links survive verbatim. L3 protocol-fill
- [ ] Tidy: Tier-1 deterministic always available. Tier-2 gated behind `CompletionProvider` BYOK with the proposal→Apply flow and typed "not available yet". No AI path ever writes without approval
- [ ] Attachments: upload-first staged flow with thumbnails. Paste-upload works. Link button produces `[label](url)` markdown
- [ ] Tasks strip renders checklist tri-state. Three action buttons wired to the real toggle endpoint. Reasons mandatory for N/A/reopen. Passive state never nags
- [ ] Completed/N-A/reopen transitions auto-append standardized notes in the SAME tx as the flip; `workflow_meta` written. Duplicate-proof via idempotency keys
- [ ] Close gate: typed 409 `TasksUnassigned` server-enforced against raw API too. Dialog offers Option A (batch auto-close with reported writes) and Option B (panel-guided manual work). "Not now" cancels
- [ ] Shared composer adopted by: workbar panel, workbench-v2 floating composer, checklist reason inputs (§0.1 one-pattern rule verified by grep/review)
- [ ] OpenAPI pins: compact byte-identical pair + SDK regen. CI green incl. e2e suite
- [ ] Migration: additive pair (workflow_meta + note_drafts), tested up/down, RLS unchanged
- [ ] Vision review (glm-5.3-flash in real browser, both themes) is the sign-off gate for panel motion/skeuomorph + notes ledger + tasks strip
