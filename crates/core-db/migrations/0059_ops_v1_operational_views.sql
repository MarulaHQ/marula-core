-- Read-model operational views v1, tranche 2 (the private build tracker U3; policy §2; ADR 0001).
--
-- Purpose: extend the versioned `ops` read model (0026 reports slice, 0041
-- undo slice) with the stable operational surfaces external reporting tools
-- and direct-SQL readers need: clients, sites, contacts, board statuses,
-- invoices, invoice lines — plus the ticket type/category/subcategory
-- taxonomy joins on ops.v1_tickets (0048 landed after 0026 authored that
-- view). Every report/external tool reads THESE views, never physical
-- tables: stored reports stay upgrade-stable (policy §5) and this is the
-- direct-SQL contract ADR 0001 promises external BI.
--
-- BINDING POLICY (upgrade-compatibility-policy §2/§5):
--   * The view list + column lists below ARE the v1 API contract for direct
--     SQL. Column ADDITIONS are additive (safe, ship in ordinary migrations
--     as appended trailing columns); renames/removals/type changes require a
--     NEW versioned namespace (`ops.v2_*`) shipped alongside — never an
--     in-place break. The U6 upgrade preflight flags view changes.
--   * THIS FILE appends six trailing columns to ops.v1_tickets via
--     CREATE OR REPLACE VIEW (legal: existing columns keep name/type/order;
--     additions land at the END of the column list) and adds six new
--     views. Nothing existing is renamed, retyped, or dropped.
--
-- SECURITY MODEL — RLS through views (identical to 0026/0041):
--   Every view is created `WITH (security_invoker = true)` (PG15+). The
--   view executes with the CALLER's privileges, so every underlying table's
--   RLS policy applies exactly as if the caller had queried the base
--   table: a `meridian_app` session bound via set_config('app.org_id', …)
--   sees its own org's rows through these views and nothing else. Views
--   also select org_id explicitly so callers/tests can assert tenancy.
--   (Default definer-rights views would bypass RLS evaluation for the view
--   owner — silently leaking cross-org rows; that is why EVERY view here
--   opts in. The core-db read-model smoke test proves owner vs app-role
--   scoping on every view in the schema.)
--
-- Deprecation story: this contract version is `v1`. A future breaking
-- change ships `ops.v2_*` alongside, announces the deprecation with a
-- `removal_target` (policy §3), and never edits v1 in place.

-- ---------------------------------------------------------------------------
-- ops.v1_tickets — EXTENDED with the 0048 classification trees. The original
-- 23 columns (0026) keep name/type/order; six taxonomy columns append at the
-- end. `category` remains the SEMANTIC workflow category from board_statuses
-- (the behavior-binding primitive); the taxonomy columns are
-- classification-only (§5) and never affect behavior.
-- Appended columns (v1, 2026-09):
--   ticket_type_id uuid          nullable; tickets.type_id → ticket_types
--   ticket_type_name text        denormalized type name
--   ticket_category_id uuid      resolved ROOT category: the referenced
--                                node's parent when the node is a
--                                subcategory, else the node itself; NULL
--                                when unclassified
--   ticket_category_name text
--   ticket_subcategory_id uuid   set ONLY when the referenced node is a
--                                subcategory (parent_id NOT NULL)
--   ticket_subcategory_name text
CREATE OR REPLACE VIEW ops.v1_tickets WITH (security_invoker = true) AS
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
       COALESCE(sl.breached, false) AS breached,
       tt.id AS ticket_type_id,
       tt.name AS ticket_type_name,
       COALESCE(tcp.id, tc.id) AS ticket_category_id,
       COALESCE(tcp.name, tc.name) AS ticket_category_name,
       CASE WHEN tc.parent_id IS NOT NULL THEN tc.id END AS ticket_subcategory_id,
       CASE WHEN tc.parent_id IS NOT NULL THEN tc.name END AS ticket_subcategory_name
FROM tickets t
JOIN board_statuses s ON s.id = t.status_id
JOIN boards b ON b.id = t.board_id
LEFT JOIN clients c ON c.id = t.client_id
LEFT JOIN users u ON u.id = t.assignee_id
LEFT JOIN ticket_sla_state sl ON sl.ticket_id = t.id
LEFT JOIN ticket_types tt ON tt.id = t.type_id
LEFT JOIN ticket_categories tc ON tc.id = t.category_id
LEFT JOIN ticket_categories tcp ON tcp.id = tc.parent_id;

-- ---------------------------------------------------------------------------
-- ops.v1_clients — one row per client company.
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), name text,
--   notes text, created_at timestamptz, updated_at timestamptz.
-- (clients.restricted_to is per-user RBAC plumbing, not reporting data;
--  exposing it later is an additive change.)
CREATE VIEW ops.v1_clients WITH (security_invoker = true) AS
SELECT cl.id,
       cl.org_id,
       cl.name,
       cl.notes,
       cl.created_at,
       cl.updated_at
FROM clients cl;

-- ---------------------------------------------------------------------------
-- ops.v1_sites — one row per client site with the client name denormalized.
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), client_id uuid,
--   client_name text, name text, address text, city text, region text,
--   postal_code text, country text, latitude double precision,
--   longitude double precision, timezone text, created_at timestamptz.
CREATE VIEW ops.v1_sites WITH (security_invoker = true) AS
SELECT si.id,
       si.org_id,
       si.client_id,
       cl.name AS client_name,
       si.name,
       si.address,
       si.city,
       si.region,
       si.postal_code,
       si.country,
       si.latitude,
       si.longitude,
       si.timezone,
       si.created_at
FROM sites si
JOIN clients cl ON cl.id = si.client_id;

-- ---------------------------------------------------------------------------
-- ops.v1_contacts — one row per client contact with client/site names
-- denormalized.
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), client_id uuid,
--   client_name text, site_id uuid (nullable), site_name text (nullable),
--   name text, email text, phone text, title text, is_primary boolean,
--   created_at timestamptz.
CREATE VIEW ops.v1_contacts WITH (security_invoker = true) AS
SELECT co.id,
       co.org_id,
       co.client_id,
       cl.name AS client_name,
       co.site_id,
       si.name AS site_name,
       co.name,
       co.email,
       co.phone,
       co.title,
       co.is_primary,
       co.created_at
FROM contacts co
JOIN clients cl ON cl.id = co.client_id
LEFT JOIN sites si ON si.id = co.site_id;

-- ---------------------------------------------------------------------------
-- ops.v1_board_statuses — one row per board status: the org's workflow
-- customization surface (names/colors are custom; `category` is the stable
-- semantic primitive behavior binds to, §5).
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), board_id uuid,
--   board_name text, name text, category text
--   ('open'|'in_progress'|'waiting'|'closed'), sort_order integer,
--   is_entry boolean, color text (nullable).
CREATE VIEW ops.v1_board_statuses WITH (security_invoker = true) AS
SELECT bs.id,
       bs.org_id,
       bs.board_id,
       b.name AS board_name,
       bs.name,
       bs.category,
       bs.sort_order,
       bs.is_entry,
       bs.color
FROM board_statuses bs
JOIN boards b ON b.id = bs.board_id;

-- ---------------------------------------------------------------------------
-- ops.v1_invoices — one row per invoice document. Money is integer minor
-- units + ISO 4217 currency (no conversion, 0036 semantics).
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), run_id uuid (nullable
--   origin run), run_number text (nullable), client_id uuid, client_name
--   text, contract_id uuid, number text (org-unique), status text
--   ('draft'|'issued'|'paid'|'void'), currency text, period_from date
--   (nullable), period_to date (nullable), issued_at date, due_on date,
--   total_minor bigint, total_paid_minor bigint, balance_minor bigint
--   (computed: total_minor − total_paid_minor), dunning_state text
--   ('none'|'reminder_sent'|'overdue'|'final_notice'|'collections'),
--   dunning_updated_at timestamptz (nullable), created_by uuid (nullable),
--   created_at timestamptz, updated_at timestamptz.
CREATE VIEW ops.v1_invoices WITH (security_invoker = true) AS
SELECT i.id,
       i.org_id,
       i.run_id,
       r.number AS run_number,
       i.client_id,
       cl.name AS client_name,
       i.contract_id,
       i.number,
       i.status,
       i.currency,
       i.period_from,
       i.period_to,
       i.issued_at,
       i.due_on,
       i.total_minor,
       i.total_paid_minor,
       i.total_minor - i.total_paid_minor AS balance_minor,
       i.dunning_state,
       i.dunning_updated_at,
       i.created_by,
       i.created_at,
       i.updated_at
FROM invoices i
JOIN clients cl ON cl.id = i.client_id
LEFT JOIN invoice_runs r ON r.id = i.run_id;

-- ---------------------------------------------------------------------------
-- ops.v1_invoice_lines — one row per persisted invoice line (kind mirrors
-- core_billing BillingLineKind, 0036). Client/currency denormalized from the
-- invoice for grouped reporting without a second join.
-- Columns (v1): id uuid (PK), org_id uuid (RLS seam), invoice_id uuid,
--   invoice_number text, client_id uuid, contract_id uuid, kind text
--   ('recurring_base'|'retainer_base'|'retainer_overage'|'time_materials'|
--   'fixed_fee'), description text, period_start date, period_end date,
--   quantity_minutes bigint, unit_amount_minor bigint, amount_minor bigint,
--   currency text (inherited from the invoice), position integer.
CREATE VIEW ops.v1_invoice_lines WITH (security_invoker = true) AS
SELECT l.id,
       l.org_id,
       l.invoice_id,
       i.number AS invoice_number,
       i.client_id,
       l.contract_id,
       l.kind,
       l.description,
       l.period_start,
       l.period_end,
       l.quantity_minutes,
       l.unit_amount_minor,
       l.amount_minor,
       i.currency,
       l.position
FROM invoice_lines l
JOIN invoices i ON i.id = l.invoice_id;

-- Explicit grants (belt-and-braces next to the 0026 default privileges:
-- they depend on the role that runs the migration, so new views must not
-- rely on cross-migration role continuity).
GRANT SELECT ON ALL TABLES IN SCHEMA ops TO meridian_app;
