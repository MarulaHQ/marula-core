# How to set workflow categories

> **Audience:** admin — reshaping a board's statuses is a settings-owner
> change; staff experience the result as labels only.

## Prerequisites

- Admin access to **Settings** (DELIVERY group: Boards, Workflow,
  Taxonomy, Tags).
- The understanding below, because the whole design hangs on one sentence.

## The one sentence that governs everything

Settings→Workflow opens with it:

> "Behavior binds to a status's semantic category, never its label."

Each status binds to one of four semantic categories. The legend, as the
workflow editor renders it:

- **Open** — entry state
- **In progress** — billable work happens
- **Waiting** — SLA clocks pause
- **Closed** — terminal; billing stops

## 1. Create the board (if needed)

**Settings→Boards**: create boards, groups, members, checklist templates,
with per-board group/default/Administer controls in the table. Note the
copy's division of labor: "statuses, transitions, and custom fields stay
on Workflow" — board administration never touches behavior.

## 2. Edit the workflow per board

**Settings→Workflow** is the per-board editor ("You can edit this
workflow"). Status rows show label, category dropdown, entry flag,
reorder arrows, delete. Two discipline rules:

- Bind each status to the right **semantic category**: label changes are
  free, while category changes alter behavior (SLA pausing, billability).
- Adding a status? Bind it to the same category your teams mean.

## 3. Rename, retire, reshape the taxonomy safely

**Settings→Taxonomy** carries classification: ticket types
(Incident / Service Request / Problem / Change, with Rename/Retire) and
the category/subcategory tree (Add/Edit, Rename / Move / Retire, "Show
retired" toggle). Its own banner says why this is safe:

> "Classification only — SLA, workflow, automations and billing stay bound
> to the four semantic status categories, so renaming or retiring a node
> never changes behavior."

## 4. Tags for org-wide AND-filters, boards for structure

**Settings→Tags** — org-wide labels applied to tickets, with per-tag
color and applied-count, editable and deletable from the tag table. The
copy carries the semantic to respect when filtering: "a ticket must carry
every selected tag" — the ticket list filters with AND semantics.

## What you learned

- Labels are yours to invent. The four categories are the contract.
- Custom fields on boards ride the same doctrine (typed fields per
  board — see also [customize without code](customize.md)).
- Moving a ticket across boards remaps status through semantic categories
  and is audited — nothing downstream retro-bills or re-times because of
  a rename.
