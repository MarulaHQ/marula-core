---
description: "A project is delivery work with dates: phases, tasks, a plan you reorder, and a timeline you move bars on. This guide walks that loop end to end."
---

# How to work a project — the plan and the timeline

> **Audience:** both. Coordinators spend their day here. Technicians read the
> same page to see where their work sits and why a date moved.

A ticket answers "what happened today". A project answers a harder question:
what are we delivering, in what order, and when does it finish. Marula keeps the
two apart. The plan is a table you edit in place. The timeline draws that plan on
a calendar you can move work around in, and a move on the chart is previewed
before it is written. Any change you apply can be undone by name afterwards.

## Prerequisites

- **See a project:** the `projects:read` permission.
- **Change dates, order or the plan:** `projects:edit`.
- **Read the money tab:** `financial:read`. Hours are visible to anyone who
  can read the project. Currency needs the money permission.
- The demo org ships with one example, **Acme Network Refresh**: three phases,
  nine tasks, two calendar zones and a captured baseline. Sign in at
  `owner@demo.local` / `demo-password-123` to follow along.

## 1. Open a project and learn its four tabs

**Projects** lists every project with its status, client and dates. Open a row
to reach the record, which carries four tabs:

- **Plan** — the table: phases, tasks, who owns them, dates, effort, blockers.
- **Timeline** — the same plan drawn on a calendar.
- **Tasks** — the delivery summary: phase progress and every task with its
  status.
- **Time and budget** — the money side. Separate guide:
  [How to bill project work](project-billing.md).

The header above the tabs is the row people mean when they ask "are we going
to make it": when the project started, when go-live is due, and the next
milestone. Beside the tabs, **Change orders** opens
the scope and billing record covered in section 13.

## 2. State the plan: phases, then tasks

A plan is a list of phases, and each phase holds tasks. Use **Phase** to add a
container and **Task** (or press `A`) to add work. A new row appears in place,
under its phase. `Enter` commits it, and `Escape` walks away without writing.

One row, one task, and the facts a coordinator needs on it:

| Column | What it says |
|---|---|
| Task | The name, the work type mark, and a **Critical** badge when the task is on the critical path |
| Status | New, in progress, blocked, done — colour is never the only cue |
| Who | The person assigned, or the role the work still needs |
| Start | The first working day, or **No date** |
| Days | Working days the work spans, from its calendar |
| Effort | Estimated hours |
| Depends on | The tasks that must finish first |

The row menu holds the rest: open the details, set a start date, add a task to
that phase, move the row up or down, and scale the phase.

Undated work is a real state, so it collects in a **Not dated yet** group at
the bottom of the table instead of being guessed at or left out. A plan that
hides its undated work looks finished when it is not.

Filters across the top narrow the table to what needs attention:
**Unassigned**, **Late or at risk**, **Blocked**, and **Critical path**. A
filter never changes the plan, only what you are looking at.

## 3. Give work dates

A task needs two facts or neither: when it starts and how long it runs. A
start with no length is not a bar on a timeline, and a length with no start has
nowhere to sit, so the API refuses half a pair rather than inventing the other
half.

Three ways to set them:

- **From the plan:** the row menu's **Set start date…**.
- **From the timeline:** press `S` on a focused bar, or use **Set dates** on a
  card in the **Not dated** tray.
- **By dragging:** drag the bar, or drag a tray card onto a day.

Lengths are working time, not calendar time. A project inherits the
organisation's working week (Monday to Friday, eight hours a day by default)
and may override it for its own dates — a six-day client site, a project that
never plans Mondays. The demo project includes Saturdays, so its **Days**
column is larger than a Monday-to-Friday span of the same range.

The working week and the calendar zones are set through the API today
(`PUT /api/projects/{id}/calendar` and `POST /api/projects/{id}/zones`). The
plan and the timeline both read them. A zone is drawn as a band across the
chart, labelled by kind: **Change
window** or **Closed**. The project's own label is in the tooltip.

## 4. Put the work in the order it runs

Order is part of the plan, not a display preference. Hover a phase row or a
task row and a grip appears at the start of its name:

- **Drag the row** onto another row. The upper half of the target puts it
  above, the lower half below, and the row under the pointer shows a gold edge
  where it would land.
- **Drag a task onto a phase row** and the task moves into that phase. One
  request both re-parents it and renumbers it, so a task can never end up
  ordered inside a phase it does not belong to.
- **Keyboard:** focus the grip and press the up or down arrow, or use
  **Move up** / **Move down** in the row menu.

The whole order is sent as one request, and the table does not move until the
server confirms it, so a refusal leaves the plan exactly as it was and
tells you why.

## 5. Read the timeline

The **Timeline** tab draws the same plan as bars on a calendar. The toolbar
changes what you read and how much of it you see:

- **Chart** or **View as table** — the table is the non-visual equivalent and
  carries the same columns, so colour and position are never the only way to
  read the plan.
- **Day / Week / Month** — the scale of the grid.
- **Fit** frames the plan as it stands. **Today** frames the next four weeks.
- **Baseline** overlays the agreed plan behind the current one.
- **Critical path** names the chain that decides the finish date and draws it
  with a heavier line.
- **Summary only** collapses every phase to one row.
- **Not dated** opens the tray of work that has no dates and is therefore not
  on the chart.

## 6. Move work on the chart — the preview comes first

Drag a bar to move it, or drag a card from the **Not dated** tray onto a day to
place it. Nothing is written yet: a preview bar appears saying **Preview,
nothing saved**, how many tasks move, and what happens downstream.

A move ripples. A later task may be pushed, the finish may shift, or the change
may land on work the same person is already doing in another project. The preview
says which:

- **Apply** writes it, in one transaction, with one audit entry per change.
- **Cancel** (or `Escape`) discards it.
- After applying, an **Undo** bar names what you did and reverses it through the
  same engine.

Two refusals you will meet:

- **A predecessor that has not finished.** A task cannot start before the work
  it depends on ends. The bar marks the earliest legal day and says which
  blocker is in the way. `S` still opens the form if the task's real dates are
  different from its place in the graph.
- **A booking in another project.** If the move lands on the same person's
  committed time elsewhere, the preview names the other project and when that
  work ends. It warns you and hands you the fact; it does not stop you.

## 7. Scale a phase or a selection

**Scale** changes how long a set of work runs, without retyping dates. Choose
the scope: a phase, or rows you ticked. Set a factor such as `1.25` and read the
projection, which gives hours before and after plus the new finish. Apply moves
the dates. Scale has no undo in the panel, and the panel says so, so do it
with the change in front of you.

## 8. Capture a baseline, then read the change

A baseline is what the plan said when it was agreed. Press **Baseline**, add a
note if the agreement needs one, and capture. From then on the button carries
the capture date, the timeline can overlay it, and a task that slipped shows
the gap between promise and reality instead of quietly restating today as if it
were always the plan.

Capture a second baseline when a change is approved. With two baselines beside
the current plan, a coordinator can answer "we moved it, here is when and by how
much" without reconstructing history from memory.

## 9. Read the risk

The project reads its own plan and returns findings rather than a score. Nine
rules run: overdue tasks, tasks that should have started, imminent work with
nobody assigned, burn ahead of progress, no recent update, overdue milestones,
long-blocked work, slippage against the baseline, and a plan that still has
undated work.

The **Late or at risk** filter in the plan shows the tasks those rules touch,
and `GET /api/projects/{id}/risk` returns the findings with their severity and
the sentence that explains each one. Nothing here changes the plan, because risk
is a read.

## 10. See every project on one axis

**Projects → Portfolio** draws the quarter as one chart: every project on the
same time axis, grouped by client, with a summary line per group. **Quarter**
and **Four weeks** change the zoom. **At risk only** narrows the chart to the
projects whose own risk findings fired, and each one names its reasons in words
rather than only turning a colour.

Work with no dates does not sit on the chart. It collects in the **Not
scheduled** tray below, because a bar drawn from a guessed date reads as a
promise the plan never made.

The share button copies a link that reproduces the exact view. The server keeps
the snapshot it sent, so the same link still shows the same range and grouping
later; it does not follow the live plan underneath. Anyone who opens it still
needs to be signed in with `projects:view` and access to the organisation.

## 11. Start from a template

**Projects → Templates** is the gallery of shapes your team already runs well.
Each card shows the shape at a glance: phases as a proportional strip, the
task count, the typical duration and the effort range. **Shipped** means Marula
provided it; **Yours** means someone in the workspace wrote it. Archived
templates stay hidden until **Show archived** is ticked.

- **Use** starts a project from the template.
- **Open** reads the template's phases, roles and tasks.
- **Preview** shows what the template will create without writing anything.

Without `projects:create`, the gallery is a reading room: preview only.
Using a template, forking one and writing a new one all need that permission.
A billable template says what it needs before it can
start, so the billing method is a decision you make with the shape in front of
you, not a surprise on the first invoice.

## 12. Work one task on its own page

Open a task from the plan, from **My work**, or from a pod deep link. The task
page is the plan row taken out of the Gantt so you can pick it up without the
whole project around it. It carries:

- the status, the due date, the effort left and the original estimate
- what the task waits on and what waits on it
- **Task pods**: status and blocker, the time and its timer, plus materials,
  links and notes

**Open the plan** returns to the project. The right-hand column is where you
record the work you are doing. The plan's status and completion still
come from the project record.

**My project work** (also on Today) gathers your assigned tasks across every
project. Waiting is its own state, and the reason is spelled out: waiting on
the client, another task, a vendor, access or parts. A blocked task does not
look like one you have not started.

## 13. Change the scope without rewriting history

The **Change orders** tab on a project records what changed, who agreed, what
it moved and what it billed. The steps are visible on the record:

**Draft → Awaiting approval → Approved → Applied → Billed**

Two rules are drawn into the layout, and they are the reason the tab exists:

- **Approval never applies.** Approving the document does not move the plan.
- **Apply never bills.** Applying the change moves the dates and effort; a
  separate billing release carries the money.

A change that needs the client's agreement sits at **Waiting for the client**
until they acknowledge it in the portal. The apply action stays unavailable
until then, so the plan cannot run ahead of the answer.

When the money is ready, **Release the billing line** stages a release. The
next billing run that reaches its window picks it up; a pending release can be
cancelled before then. If a release fails, the plan change stays applied and
the row says **Applied, billing failed**, with the reason. The change is not
silently reverted, because the work already moved.

Amounts need `financial:read`. Without it the tab still shows the document and
its progress, but the money fields say that financial access is needed.

## 14. What this build does not do yet

These are not in the build yet, so the surfaces will look unfinished:

- **No board view on a project.** The Board segment is disabled. A project board
  needs the organisation's workflow-to-column map, which this build does not
  carry yet.
- **A project task is not a ticket.** There is no link between the two in this
  build, so a task's conversation does not live on a ticket. Time is logged on a
  ticket or on a project phase, never on both.
- **Calendar zones and the working week are API-set.** The surfaces draw them.
  The editors are not there yet.

## Where to go next

- [How to bill project work](project-billing.md) — contracts, phase budgets and
  milestone releases.
- [How to run dispatch](staff-dispatch.md) — where a technician's day meets a
  project's tasks.
- [How to use keyboard shortcuts](staff-keyboard-shortcuts.md) — the keys this
  page assumes.
