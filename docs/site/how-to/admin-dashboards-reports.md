---
description: "Marula gives service leads and owners three ways to track how the shop is running: live Dashboards (/reports/dashboards), a custom Report Builder…"
---

# How to manage dashboards & report schedules

> **Audience:** admin — report and dashboard management is a settings
> adjacent reporting task; staff pin widgets for themselves (the widget
> grid is shared, the pins are yours).

Marula gives service leads and owners three ways to track how the shop is running: live **Dashboards** (`/reports/dashboards`), a custom **Report Builder** (`/reports`), and private coaching **Scorecards** (`/reports/scorecards`). Every chart and table reads from the same versioned reporting views so numbers never disagree between screens.

## Prerequisites

- Access to **Reports** (`/reports`) and **Dashboards** (`/reports/dashboards`).
- A quick look at [Why customizations are data](../explanation/customizations-are-data.md) if you want to know why reports query stable `v1_*` views rather than internal database tables.

## 1. Use and customize live dashboards

Open **Reports → Dashboards** (`/reports/dashboards`). Dashboards are built on a shared widget grid with preset tabs ready out of the box:

- **My day:** Personal hours logged and tickets moved today.
- **Team load:** Active queue distribution and technician capacity.
- **SLA health:** First-response and resolution compliance across your boards.

Click **Add widget** to place additional chart or counter cards onto the grid.

## 2. How widgets and reports share the same engine

Every dashboard widget is backed by a report query over Marula's versioned operational read model (`ops.v1_*` views). When you configure a widget or a custom report, you pick from the same building blocks:

- **Data source** (one of the `v1_*` views such as tickets, time entries, SLA outcomes, or invoices)
- **Group-by fields** and **measures**
- **Filters** and **date window**
- **Chart type**

## 3. Build and save custom reports

Open **Reports → Builder**. Choose your data view, grouping, measure, filter clauses, and chart style.

The builder compiles and previews your query on demand without cluttering your saved list. When the numbers look right, click **Save** so your team can run the report anytime or attach it to an email schedule. For the exact columns available in each view, see [How to report against the read model](read-model.md).

## 4. Schedule recurring email digests

Open **Reports → Schedules** to send saved reports automatically on a recurring cadence (for example, the preloaded *"Weekly ticket digest"*). Scheduled report emails use your workspace email transport and respect each recipient's notification quiet hours.

## 5. Review technician scorecards and CSAT

Open **Reports → Scorecards** (`/reports/scorecards`) for individual performance and coaching metrics over the last 30 days:

- **Privacy by default:** Each technician sees only their own scorecard, while owners and admins can view the full team.
- **Metrics included:** SLA compliance percentage, breached count, average first-response time, and customer satisfaction (**CSAT**) averages and low-score counts once [CSAT surveys](admin-csat-surveys.md) are active.
- **Opt-in gamification:** Leaderboard comparisons stay **Off** by default unless an admin turns them on.

For field-by-field edit history or security event reviews, use the dedicated [Provenance and Audit screens](../explanation/provenance-vs-audit.md).

## What you learned

- **Dashboards** display live widget grids, **Builder** creates custom saved reports, and **Schedules** emails them out on a timer.
- Every widget and report queries the stable `v1_*` read-model views so upgrades never break your saved reports.
