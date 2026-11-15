---
description: "A project is delivery work with dates: phases, tasks, a plan you reorder, and a timeline you move bars on. This guide walks that loop end to end."
---

# How to work a project — the plan and the timeline

> **Audience:** both. Coordinators spend their day here. Technicians read the
> same page to see where their work sits and why a date moved.

A ticket answers "what happened today". A project answers "what are we
delivering, in what order, and when does it finish". Marula keeps the two
separate: a project holds phases and tasks, the plan is a table you edit in
place, and the timeline draws that plan on a calendar you can move work
around in. A move on the chart is previewed before it is written, and the change
can be undone by name afterwards.

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

The header above the tabs is the promise: when the project started and when
go-live is due, plus the next milestone. When someone asks "are we going to
make it", that is the row they mean.

## 2. State the plan: phases, then tasks

A plan is a list of phases, and each phase holds tasks. Use **Phase** to add a
container and **Task** (or press `A`) to add work. A new row appears in place,
under its phase. `Enter` commits it, and `Escape` walks away without writing.

Each task row carries the facts a coordinator needs in one line:

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

Work with no dates is not hidden and not guessed at. It collects in a
**Not dated yet** group at the bottom of the table, because undated work is a
real state and a plan that hides it lies about its own readiness.

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
(`PUT /api/projects/{id}/calendar` and `POST /api/projects/{id}/zones`). Both
surfaces read them. A zone is drawn as a band across the chart, labelled by kind: **Change
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

The whole order is sent as one request, and the table you see is the server's
until the server confirms, so a refusal leaves the plan exactly as it was and
tells you why.

## 5. Read the timeline

The **Timeline** tab draws the same plan as bars on a calendar. The toolbar is
where the reading happens:

- **Chart** or **View as table** — the table is the non-visual equivalent and
  carries the same columns, so colour and position are never the only way to
  read the plan.
- **Day / Week / Month** — the scale of the grid.
- **Fit** frames the plan as it stands. **Today** frames the next four weeks.
- **Baseline** overlays the agreed plan behind the current one.
- **Critical path** names the chain that decides the finish date and draws it
  in ink with heavier elbows.
- **Summary only** collapses every phase to one row.
- **Not dated** opens the tray of work that has no dates and is therefore not
  on the chart.

## 6. Move work on the chart — the preview comes first

Drag a bar to move it, or drag a card from the **Not dated** tray onto a day to
place it. Nothing is written yet: a preview bar appears saying **Preview,
nothing saved**, how many tasks move, and what happens downstream.

That sentence is the point of the whole surface. Moving a bar ripples: a later
task may be pushed, the finish may shift, and the change may land on work the
same person is already doing in another project. The preview says which, then:

- **Apply** writes it, in one transaction, with one audit entry per change.
- **Cancel** (or `Escape`) discards it.
- After applying, an **Undo** bar names what you did and reverses it through the
  same engine.

Two refusals you will meet, both stated rather than silent:

- **A predecessor that has not finished.** A task cannot start before the work
  it depends on ends. The bar marks the earliest legal day and says which
  blocker is in the way. `S` still opens the form if the task's real dates are
  different from its place in the graph.
- **A booking in another project.** If the move lands on the same person's
  committed time elsewhere, the preview names the other project and when that
  work ends. It is a warning with the fact attached, not a wall.

## 7. Scale a phase or a selection

**Scale** changes how long a set of work runs, without retyping dates. Choose the scope: a phase, or rows you
ticked. Set a factor such as `1.25` and read the projection, which gives hours
before and after plus the new finish. Apply moves the dates. The panel says
plainly that this scale cannot be undone from the panel, so scale with the change
in front of you.

## 8. Capture a baseline, then read the change

A baseline is what the plan said when it was agreed. Press **Baseline**, add a
note if the agreement needs one, and capture. From then on the button carries
the capture date, the timeline can overlay it, and a task that slipped shows
the gap between promise and reality instead of quietly restating today as if it
were always the plan.

Capture a second baseline when a change is approved. Two baselines and one
current plan is how a coordinator answers "we moved it, here is when and by how
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

## 10. What this build does not do yet

Worth knowing before you look for it:

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
- [How to run dispatch](staff-dispatch.md) — where a technician's day and a
  project's tasks meet.
- [How to use keyboard shortcuts](staff-keyboard-shortcuts.md) — the keys this
  page assumes.
