# ADR 0034: Project delivery

**Date:** 2026-11-15
**Status:** Proposed
**Ruling:** Projects are scheduled work inside Marula, not a separate planning
tool: a task carries its own time, dependencies compute dates, and the day a
technician sees holds planned work and service work together.

## Context

Marula shipped projects as a first-class record early: projects, phases,
tasks, dependencies, saved views, template instantiation and phase burn all
exist in the product today. What the module did not have was time. A task
carried a status and a phase carried a window, so the product could say that a
project was late but not when the work sat, what moved when something slipped,
or which technician was free on Thursday.

That was deliberate. The first cut was a record of intent, and a date the
product invents is worse than no date at all: it reads as a commitment, it
changes silently when a calendar changes, and it cannot be compared against
anything. The redesign adds time as a stored, explainable fact instead of a
derived claim.

The change also settles questions that a scheduler forces and a record does
not: which days count, how a person's availability differs from a project's,
what happens to work that is placed but not yet offered, whether a checkpoint
before a cutover can block anything, and how planned work reaches the day
without changing service work.

## Decision

### 1. Task time is stored

A task records when it starts and how long it takes, plus the effort it is
expected to need and how much of that effort remains. A task without a start
stays without one. The product draws undated work as undated and never guesses
a date to fill a column.

### 2. One schedule function

The ripple preview and the ripple apply call the same function. Dependencies
compute dates; a date is never computed in a second place with a second rule.
Preview is therefore a read of the same computation the apply performs, which
is what makes the preview trustworthy.

### 3. Calendar resolution is ordered

A project's own working pattern wins where it is set. The organisation's
calendar is the base. Personal weekly hours scale how much a person can carry
and never move a date. Absences and calendar zones are subtracted after the
calendar resolves.

### 4. Assignment moves through named states

An assignment is `tentative`, `offered`, `accepted`, `declined`, `expired` or
`withdrawn`. A tentative placement belongs to the coordinator until they
publish or remove it; it does not lapse on its own. The offer window starts
when the offer is published, so the two states cannot be confused.

### 5. One board, one policy

Dispatch and project work share a board. A workflow map decides whether the
board shows service work, project work, or both, and whether work may cross
between them. The board is not duplicated per work kind.

### 6. A ticket and a task are one record in three modes

A ticket may be linked to a task they belong with, promoted into a project
task, or absorbed by it. Promotion and demotion preserve the record: the
conversation, the attachments and the audit trail follow the work rather than
being rebuilt.

### 7. Templates are versioned documents

A template is a versioned document with a published version and, separately,
the versions an organisation has forked. A shipped version is immutable. Tuning
happens at instantiation: a plan may be scaled, steps may be skipped, dates may
shift, and roles or people may be remapped. All of it lands on a copy, never on
the template.

### 8. Baselines and change are separate acts

A baseline is captured at kickoff and again when a change is approved. Applying
a change and billing for it are two steps with a decision between them: the
plan moves when the work is agreed, and money moves when the change is
approved.

### 9. Project workflow is organisation data

The statuses a project task moves through are mapped to the board by
organisation data, independent of the service workflow. Two workflows, one
board, no shared hard-coded status list.

### 10. Approval gates are optional

A checkpoint before a cutover is a record with a mode, an approver and a
decision of `go`, `go with conditions` or `no go`. An informational checkpoint
behaves like a milestone and blocks nothing. An enforced checkpoint blocks the
tasks that depend on it. A template may carry one, a project manager may add,
remove or change one when a project's scope changes, and a project that wants
none carries none.

### 11. AI is suggestion-grade

An AI surface proposes; a person disposes. Every surface starts in shadow with
deny-by-default grants. Deterministic rules own severity, so a claim's weight
does not depend on a model's confidence. Every suggestion carries the evidence
it was drawn from, and a surface's threshold is set per surface rather than
globally.

### 12. Money visibility is layered

Hours are broadly visible: a coordinator needs them to place work. Currency
sits behind the financial permission. Contractor cost is off by default and is
widened by the organisation deliberately, with the change recorded.

### 13. Billing attaches above the task

Money is billed against a phase, a milestone or a change order. A single task
is never billed. Billing reads the plan; the plan does not read billing.

## Already true, and written down here for the first time

These behaviours shipped before this record and are load-bearing. A reader
currently has to infer them from the code.

- Dependency validation refuses a self-dependency, a cross-project dependency
  and a cycle, and names the refusal in the error reason. Completion of a task
  with unfinished dependencies is refused with `task_dependencies_unsatisfied`.
- A phase is drawn on the timeline only when both its dates exist. A phase with
  one date is undated work, not a half-drawn bar.
- `planned_days` counts weekend days, because it describes a planning span and
  is not a claim about working days.
- The template spec is additive-only and validated before a single row is
  written; expansion then happens inside one transaction, so a template that
  fails halfway writes nothing.
- Phase burn carries a percent figure and an alert band: warn at eighty percent,
  over at a hundred, and no band at all for a phase with no budget.
- A saved project view is a versioned document with a closed filter vocabulary,
  not a free-form query.
- Project billing methods and the project and phase status vocabulary are part
  of the domain entity, and the permissions `projects:read`, `projects:write`
  and `financial:read` already exist.

## Alternatives considered

**Keep planning beside the PSA, synced.** This is the shape of the dedicated
planning tools. It was rejected because two sources of truth for the same work
mean the day diverges from the plan within a week, and the person doing the
work updates the tool that pays them.

**Make every project task a ticket.** Tempting for reuse, and rejected: tickets
carry service-level agreements, board semantics and a queue position that
planned work does not want, and rolling a project up over thousands of tickets
is expensive for a number that a phase already carries.

**Derive dates, never store them.** Rejected. A derived plan cannot hold a
baseline, cannot explain why a date moved, and changes silently when a calendar
or an estimate changes. Storing the date and explaining the change is the
honest version.

**Bill per task.** Rejected. It turns the money tab into an invoice editor and
duplicates the contract engine, and clients buy outcomes, not task hours.

**Require an approval gate before cutover.** Rejected. A small shop that does
not want a gate should not have to delete one from every template. The gate
exists, is enforceable, and is never mandatory.

## Consequences

- A task now has a time and a plan holds a baseline, so a slipped date can be
  shown against the promise rather than asserted.
- Two consumers read the same calendar function, so a change of rule reaches
  the plan, the team schedule and the reports at once.
- The day is a union of service and project work with one preference model,
  which means ticket behaviour can be preserved exactly while project work
  appears beside it.
- The module gains several tables and endpoints it did not have. Each is
  org-scoped with row-level security and writes an audit row, like every other
  table in the product.
- An organisation that uses none of this sees the product it had: the approval
  gate, the dispatch policy and the change-order flow are all optional by
  construction.
- The cost is that a project manager must date work for the plan to say
  anything. That is the point: the plan states what a person decided, not what
  a formula guessed.
