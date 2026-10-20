-- Read-model reporting views v1 (the private build tracker R0 = U3; ADR 0001).
--
-- Purpose: the FIRST consumer-facing slice of the versioned read-model
-- layer: five views in the `ops` schema that expose operational data for
-- the reporting engine AND for external BI tools (Metabase/Superset/
-- Grafana/PowerBI) connecting directly to Postgres.
--
-- BINDING POLICY (upgrade-compatibility-policy §2/§5, docs/adr/0001):
--   * Reports NEVER read physical tables. Everything downstream of this
--     migration queries ONLY `ops.v1_*` — so renaming a physical column
--     never breaks a saved report; upgrading a view definition is the
--     single controlled seam, covered by the U4 deprecation clock and
--     U6 preflight.
--   * The view list + columns below ARE the v1 API contract. Column
--     additions are additive (safe); renames/removals require a NEW
--     versioned namespace (`ops.v2_*`), never an in-place break.
--
-- SECURITY MODEL — RLS through views:
--   Every view is created `WITH (security_invoker = true)` (PG15+,
--   verified on our pgvector/pgvector:pg17 image). Under security-invoker
--   semantics the view executes with the CALLER's privileges, so the
--   row-level-security policies of every underlying table apply exactly
--   as if the caller had queried the base table: a `meridian_app`
--   session bound via set_config('app.org_id', …) sees its own org's
--   rows through these views and nothing else. Views also select
--   org_id explicitly so callers/tests can assert tenancy.
--   (Without security_invoker, default definer-rights views would bypass
--   RLS evaluation for the view owner — silently leaking cross-org rows;
--   that is why EVERY view here opts in.)
--
-- NONE-MODE OPERATOR RECIPE (external BI escape hatch, ADR 0001):
--   Zero Meridian code required. Create a read-only login role, grant it
--   the ops schema, and bind it consciously to one org's scope:
--
--     CREATE ROLE meridian_reports_ro LOGIN PASSWORD '<secret>'
--       NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
--     GRANT USAGE ON SCHEMA ops TO meridian_reports_ro;
--     GRANT SELECT ON ALL TABLES IN SCHEMA ops TO meridian_reports_ro;
--     ALTER DEFAULT PRIVILEGES IN SCHEMA ops
--       GRANT SELECT ON TABLES TO meridian_reports_ro;
--     -- RLS-through-views: without an app.org_id binding the role sees
--     -- ZERO rows (default deny). Bind deliberately, one org per role:
--     ALTER ROLE meridian_reports_ro SET app.org_id = '<org uuid>';
--
--   In-app RBAC does NOT extend into external tools (ADR 0001): granting
--   this role is the conscious act. Review per-scope before sharing creds.
--
-- Non-purpose: no report-spec storage (R3 owns tables), no pre-aggregation
-- marts (acknowledged scale limit in ADR 0001), no grants beyond the
-- application role needed to serve /api/reports/*.

CREATE SCHEMA IF NOT EXISTS ops;

GRANT USAGE ON SCHEMA ops TO meridian_app;

-- ---------------------------------------------------------------------------
-- ops.v1_tickets — one row per ticket, workflow truth joined in.
-- Columns (v1):
--   id uuid               ticket id (PK of this view)
--   org_id uuid           owning org (RLS seam)
--   number text           human ticket number (org-unique)
--   board_id uuid         owning board
--   board_name text       denormalized board name
--   status_id uuid        current status
--   status_name text      current status name (board_statuses)
--   category text         semantic category FROM BOARD STATUSES
--                         ('open'|'in_progress'|'waiting'|'closed') — the
--                         workflow layer is authoritative; tickets.category
--                         mirrors it by transactional update
--   priority text         'p1'..'p4'
--   title text            ticket title
--   client_id uuid        nullable client
--   client_name text      nullable denormalized client name
--   site_id uuid          nullable site
--   assignee_id uuid      nullable staff assignee
--   assignee_name text    nullable denormalized assignee display name
--   requester_id uuid     nullable staff requester
--   due_at timestamptz    nullable business deadline
--   resolved_at timestamptz  set when moved into closed category
--   created_at timestamptz
--   updated_at timestamptz
--   first_response_due timestamptz   current SLA clock target (nullable when
--                                    no SLA state row exists yet)
--   resolution_due timestamptz
--   breached boolean      SLA breach flag from the state machine (false when
--                         no SLA state)
CREATE VIEW ops.v1_tickets WITH (security_invoker = true) AS
SELECT t.id,
       t.org_id,
       t.number,
       t.board_id,
       b.name AS board_name,
       t.status_id,
       s.name AS status_name,
       s.category,
       t.priority,
       t.title,
       t.client_id,
       c.name AS client_name,
       t.site_id,
       t.assignee_id,
       u.display_name AS assignee_name,
       t.requester_id,
       t.due_at,
       t.resolved_at,
       t.created_at,
       t.updated_at,
       sl.first_response_due,
       sl.resolution_due,
       COALESCE(sl.breached, false) AS breached
FROM tickets t
JOIN board_statuses s ON s.id = t.status_id
JOIN boards b ON b.id = t.board_id
LEFT JOIN clients c ON c.id = t.client_id
LEFT JOIN users u ON u.id = t.assignee_id
LEFT JOIN ticket_sla_state sl ON sl.ticket_id = t.id;

-- ---------------------------------------------------------------------------
-- ops.v1_time_entries — one row per logged work entry.
-- Columns (v1):
--   id uuid                entry id (PK of this view)
--   org_id uuid            owning org (RLS seam)
--   ticket_id uuid         charged ticket
--   ticket_number text     denormalized ticket number
--   client_id uuid         nullable client inherited from the ticket
--   client_name text       nullable denormalized client name
--   user_id uuid           authoring staff member
--   user_display_name text staff display name (join convenience for
--                          grouped-by-user reporting)
--   started_at timestamptz work interval start
--   ended_at timestamptz   work interval end
--   minutes integer        billed minutes (>=0)
--   billable boolean       non-billable flag drives billable_minutes math
--                          in reports
--   work_type text         nullable classification
--   day_utc date           calendar day of started_at in UTC — the v1
--                          time-by-day grouping key (local-day grouping is
--                          a deliberate later addition; see README)
--   timer_session_id uuid  nullable origin session
--   created_at timestamptz
CREATE VIEW ops.v1_time_entries WITH (security_invoker = true) AS
SELECT e.id,
       e.org_id,
       e.ticket_id,
       t.number AS ticket_number,
       t.client_id,
       c.name AS client_name,
       e.user_id,
       u.display_name AS user_display_name,
       e.started_at,
       e.ended_at,
       e.minutes,
       e.billable,
       e.work_type,
       CAST(e.started_at AT TIME ZONE 'UTC' AS date) AS day_utc,
       e.timer_session_id,
       e.created_at
FROM time_entries e
JOIN tickets t ON t.id = e.ticket_id
LEFT JOIN clients c ON c.id = t.client_id
LEFT JOIN users u ON u.id = e.user_id;

-- ---------------------------------------------------------------------------
-- ops.v1_sla_outcomes — per-ticket SLA attainment + breach context.
-- One row per ticket with SLA state (joins carry no duplicates: PKs).
-- Columns (v1):
--   ticket_id uuid          FK to ops.v1_tickets.id
--   org_id uuid             owning org (RLS seam)
--   ticket_number text      denormalized ticket number
--   priority text           ticket priority at outcome time
--   assignee_id uuid        nullable staff assignee (self-scoping dim)
--   ticket_created_at timestamptz   window filter anchor
--   policy_id uuid          governing SLA policy
--   policy_name text        denormalized policy name
--   calendar_name text      nullable attached business calendar (0015);
--                           NULL keeps the embedded-calendar regime
--   calendar_timezone text  nullable calendar tz for local-hour context
--   first_response_due timestamptz  nullable target
--   first_responded_at timestamptz  nullable actual
--   response_met boolean    THREE-valued: NULL until both due+actual exist,
--                           else actual <= due
--   resolution_due timestamptz      nullable target
--   resolved_at timestamptz         ticket resolution instant
--   resolution_met boolean  THREE-valued like response_met
--   breached boolean        breach flag from the escalation engine
--   paused_business_secs bigint    cumulative paused budget consumption
CREATE VIEW ops.v1_sla_outcomes WITH (security_invoker = true) AS
SELECT st.ticket_id,
       st.org_id,
       t.number AS ticket_number,
       t.priority,
       t.assignee_id,
       t.created_at AS ticket_created_at,
       p.id AS policy_id,
       p.name AS policy_name,
       cal.name AS calendar_name,
       cal.timezone AS calendar_timezone,
       st.first_response_due,
       st.first_responded_at,
       CASE
         WHEN st.first_response_due IS NOT NULL AND st.first_responded_at IS NOT NULL
           THEN st.first_responded_at <= st.first_response_due
       END AS response_met,
       st.resolution_due,
       t.resolved_at,
       CASE
         WHEN st.resolution_due IS NOT NULL AND t.resolved_at IS NOT NULL
           THEN t.resolved_at <= st.resolution_due
       END AS resolution_met,
       st.breached,
       st.paused_business_secs
FROM ticket_sla_state st
JOIN sla_policies p ON p.id = st.policy_id
LEFT JOIN sla_calendars cal ON cal.id = p.calendar_id
JOIN tickets t ON t.id = st.ticket_id;

-- ---------------------------------------------------------------------------
-- ops.v1_gap_days — missing-time audit ledger (0017) per (user, local day).
-- Columns (v1):
--   id uuid                 audit ledger id (PK of this view)
--   org_id uuid             owning org (RLS seam)
--   user_id uuid            audited staff member
--   user_display_name text  denormalized name
--   user_timezone text      the tz the day was computed in
--   day date                LOCAL calendar day (per-user tz semantics)
--   activity boolean        had ticket activity that day
--   logged_minutes integer  minutes actually logged
--   expected_minutes integer  org threshold at audit time
--   has_gap boolean         computed gap verdict
--   nudge_sent_at timestamptz  nullable EOD nudge delivery
--   updated_at timestamptz  last ledger touch
CREATE VIEW ops.v1_gap_days WITH (security_invoker = true) AS
SELECT g.id,
       g.org_id,
       g.user_id,
       u.display_name AS user_display_name,
       u.timezone AS user_timezone,
       g.day,
       g.activity,
       g.logged_minutes,
       g.expected_minutes,
       g.has_gap,
       g.nudge_sent_at,
       g.updated_at
FROM time_gap_audits g
JOIN users u ON u.id = g.user_id;

-- ---------------------------------------------------------------------------
-- ops.v1_first_response — one row per ticket: created → earliest STAFF
-- comment. Portal replies are structurally excluded: portal identities are
-- not staff rows (0021), so their comments always carry author_id NULL;
-- any staff-authored note (internal/client_visible/in_house_update) counts
-- as having responded — "we reacted" is the measured event.
-- Columns (v1):
--   ticket_id uuid                     ticket id (PK of this view)
--   org_id uuid                        owning org (RLS seam)
--   ticket_number text                 denormalized ticket number
--   client_id uuid                     nullable client
--   assignee_id uuid                   nullable staff assignee
--                                      (self-scoping dimension)
--   created_at timestamptz             ticket creation instant
--   first_staff_comment_at timestamptz earliest staff-authored comment;
--                                      NULL while unanswered
--   first_response_secs bigint         latency seconds (NULL while
--                                      unanswered); negative values are
--                                      clock-skew artifacts, surfaced as-is
--   first_response_due timestamptz     current SLA response target (nullable)
CREATE VIEW ops.v1_first_response WITH (security_invoker = true) AS
SELECT t.id AS ticket_id,
       t.org_id,
       t.number AS ticket_number,
       t.client_id,
       t.assignee_id,
       t.created_at,
       fc.first_staff_comment_at,
       CASE WHEN fc.first_staff_comment_at IS NOT NULL
            THEN EXTRACT(epoch FROM (fc.first_staff_comment_at - t.created_at))::bigint
       END AS first_response_secs,
       sl.first_response_due
FROM tickets t
LEFT JOIN LATERAL (
  SELECT MIN(c.created_at) AS first_staff_comment_at
  FROM ticket_comments c
  WHERE c.ticket_id = t.id
    AND c.author_id IS NOT NULL
) fc ON TRUE
LEFT JOIN ticket_sla_state sl ON sl.ticket_id = t.id;

GRANT SELECT ON ALL TABLES IN SCHEMA ops TO meridian_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA ops GRANT SELECT ON TABLES TO meridian_app;
