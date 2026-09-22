# How to manage dashboards & report schedules

> **Audience:** admin — report and dashboard management is a settings
> adjacent reporting task; staff pin widgets for themselves (the widget
> grid is shared, the pins are yours).

## Prerequisites

- Access to **Reports** (/reports) and **Dashboards**
  (/reports/dashboards). Org-wide dashboards are shared, while a
  schedule is an admin-managed row.
- The tools mental model (compile-on-demand, versioned views) — this
  how-to assumes it. The
  [reporting explanation](../explanation/customizations-are-data.md)
  companion covers the deeper why.

## 1. The widget is the only engine — accept it

Dashboards are the **widget grid**, and the grid is the only engine:
preset tabs (My day / Team load / SLA health) ship prepopulated widget
cards, and **Add widget** composes more.
("the widget grid is the only engine — widgets, not report builder",
per the audit's own note) and pairs with the free-form builder one tab
away.

## 2. What a widget bar can express

Dashboard widgets read the same versioned read model the report builder
compiles against (`v1_*` views — "views, never raw tables"). A widget is
a report with a chart attached to the grid, so widget authoring is report
authoring with the constraints the builder imposes: group-bys, measures,
filters, a window. If a widget needs a construction the builder cannot
express, the widget cannot either — write the report, keep the dashboard
honest.

## 3. Build and save reports in the builder

**Reports→Builder**: Data (a "versioned operational read model… views
only — never raw tables"), group-bys, measures, filters, chart,
window. Compile-on-demand means **nothing is saved until you save the
report** — the spec compiles server-side against read-model views at run
time. Saved reports persist, and per-view depth lives in the
[read model catalog](read-model.md).

## 4. Schedules: hand standing reports to standing readers

**Reports→Schedules** manages scheduled sends (report delivery of a
standing report on cadence — the seeded demo org carries a "Weekly
ticket digest" row). Delivery rides notification surfaces and respects
quiet hours the same way other email does — SLA breaches and escalations
break through quiet hours, scheduled digests do not.

## 5. Where metrics come from when you build these

- **Dashboards** preset tabs back onto the perf read model
  (`my_time`, `my_throughput`, and friends from the seeded capture).
- **Scorecards** (/reports/scorecards) are separate and coaching-shaped:
  private by default ("you see your own card; owners and admins see the
  team"), per-user SLA%/breached/first-response stats over a Last-30-days
  window, opt-in gamification off by default, CSAT explicitly reserved
  ("surveys land in a later release").
- **Audit** and **provenance** are admin query surfaces, not widgets —
  see [provenance vs audit](../explanation/provenance-vs-audit.md).

## What you learned

- Widgets are the dashboard engine. Reports are the builder's product.
  Schedules are standing sends. Three surfaces, one read model.
- Compile-on-demand keeps unsaved drafts weightless. Save to persist.
