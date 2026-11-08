---
description: "Read the seven-day dispatch horizon: demand by priority against roster hours, capacity per technician, skill gaps and the project load already committed."
---

# How to plan the week ahead

> **Audience:** staff — the horizon answers "can next week carry this work, and
> where is it short", before anything is placed.

The **Planning** tab (`/dispatch/planning`) looks past today's board. Nothing on
this page moves a ticket: it forecasts, and it reports. Placement stays on the
[dispatch board](staff-dispatch.md).

## Prerequisites

- A signed-in session with **Dispatch** access.
- Technicians on your service boards, with working hours set. Capacity comes from
  the roster, so a technician with no hours shows no capacity.
- Open tickets with priorities. Demand comes from booked and waiting work.

## 1. Read the three project panels first

These sit above the chart because they change how you read it. All three are
read-only.

| Panel | What it tells you |
|---|---|
| **Committed project load** | Share of a normal full-time load already promised to projects, in percent-days. One technician at 100% for two days is 200 percent-days. Appointments are not counted here. |
| **Phase allocations** | Who is stated as working on which project phase, in addition to their booked appointments. |
| **Work next** | The order a dispatcher set for a technician. It changes what is offered first; it never changes a ticket's priority. |

## 2. Read the seven-day capacity chart

Each column is a day. Demand stacks from the bottom in priority order: P1, then
P2, then P3 and P4. The dashed line is the roster's hours for that day.

- A day under the line prints how much time is **spare**.
- A day over the line prints **Over by** the difference, in the day's colour.
  A day with no roster hours prints **No cover**.
- The label under each column is total demand against total capacity, for example
  `54h of 60h`.

The chart adds the team together, so an over-capacity day does not say who is
overloaded. The table under it does.

## 3. Find whose day is full

The depth table lists one row per technician and one column per day, with each
cell showing utilization as a percentage. A cell over 100% is marked. The
tooltip on a cell gives free minutes against available minutes.

Start with the technicians at or over 100% on the day you are worried about, then
go back to the board and move work.

## 4. Check the skill catalogue

The catalogue compares what each skill can supply with what the week is asking
for:

| Column | Meaning |
|---|---|
| **Skill** | The skill as recorded on staff records. |
| **Techs** | How many people hold it. |
| **Capacity** | Hours those holders can offer this week. |
| **Demand** | Hours of work that need the skill. |
| **Gap** | The difference. A positive gap is room; a negative one is a shortage. |

A skill shortage is a hiring and training signal, not something the board can fix
by moving a ticket to someone who does not hold it.

## 5. Open the connector panel when integrations disagree

**Connector status** opens a read-only panel with the technicians and the
schedule the dispatch connector reports. When it is not configured, the panel
says so instead of showing empty tables. Use it to check whether an external
scheduling system and Marula agree about who is working when.

## What this page does not do

- It does not place, reassign or reprioritize work. Use the [board](staff-dispatch.md).
- It does not change capacity. Roster hours and working time live with the
  technician record; see [Manage users and invites](admin-users-invites.md).
- Its date arrows are not wired to a week picker yet, so the header shows the
  week the page loaded. Press **Refresh** or **Today** to load the figures again
  after work moves.

## Related

- [Dispatch from the board](staff-dispatch.md) — placing the work this page forecasts.
- [Turn on travel time and mileage reconciliation](admin-routing-travel.md) — the routing and day-start settings behind travel estimates.
- [Manage dashboards and report schedules](admin-dashboards-reports.md) — the reports to send upward once the week is planned.
