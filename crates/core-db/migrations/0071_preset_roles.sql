-- Wave 13 lane 6 `p13-preset-roles` — RB1 preset roles as data + RB2 key.
--
-- ADR 0004: the shipped presets (owner, admin, dispatcher, senior_tech,
-- technician, bookkeeper, auditor, read_only) exist as DATA on the same
-- permission matrix as custom roles — seed rows in `roles` that every org
-- carries (is_system, canonical), forked by orgs into custom copies and
-- restorable, never edited in place. This migration backfills any preset
-- a pre-lane org is missing (idempotent), adds the fork-origin tag, pins
-- the directory role-precedence as one SQL function, and migrates the
-- financial-visibility wire key to `financial:read` (RB2).
--
-- The permission arrays below mirror core_domain::policy::BuiltinRole::
-- permissions() (the canonical definitions); roles_flow asserts the
-- backfilled rows are byte-identical to a fresh org's seeds, so the two
-- copies cannot silently drift.

ALTER TABLE roles ADD COLUMN IF NOT EXISTS forked_from text;

-- Directory "highest-precedence builtin role" resolution lives here (one
-- place, shared by the directory listing and set_builtin_role). New
-- presets slot between dispatcher and technician by seniority; anything
-- unknown sorts last so a stray spelling can never shadow owner.
CREATE OR REPLACE FUNCTION preset_builtin_precedence(builtin text)
RETURNS integer
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $$
  SELECT CASE builtin
    WHEN 'owner' THEN 0
    WHEN 'admin' THEN 1
    WHEN 'dispatcher' THEN 2
    WHEN 'senior_tech' THEN 3
    WHEN 'technician' THEN 4
    WHEN 'bookkeeper' THEN 5
    WHEN 'auditor' THEN 6
    WHEN 'read_only' THEN 7
    ELSE 8
  END
$$;

-- RB2 (ADR 0004): the financial-visibility permission is its own wire key
-- `financial:read`. Role rows store specials as bare strings, so the
-- existing spelling migrates in place (builtin roles resolve grants from
-- code regardless, but the rows are the data orgs see and fork).
UPDATE roles
SET permissions = (
  SELECT coalesce(jsonb_agg(
    CASE
      WHEN jsonb_typeof(elem) = 'string' AND elem #>> '{}' = 'view_financials'
        THEN to_jsonb('financial:read'::text)
      ELSE elem
    END
    ORDER BY ord
  ), '[]'::jsonb)
  FROM jsonb_array_elements(permissions) WITH ORDINALITY AS t(elem, ord)
)
WHERE permissions @> '"view_financials"'::jsonb;

-- Backfill every shipped preset row an org is missing. Arrays are built
-- in SQL (the migration carries its own copy of the canonical matrix by
-- necessity — the established 0048 taxonomy pattern).
DO $$
DECLARE
  org record;
  missing record;
  v_modules text[] := ARRAY[
    'tickets','clients','time','scheduling','projects','commercial',
    'invoicing','kb','documents','passwords','reporting','admin','plugins'
  ];
  v_actions text[] := ARRAY['view','create','edit','delete','export'];
  v_full jsonb;
  v_view_all jsonb;
  v_owner_specials jsonb;
  v_senior jsonb;
  v_bookkeeper jsonb;
BEGIN
  -- owner/admin: full matrix on every module + the full special set.
  SELECT jsonb_agg(
           jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', a))
           ORDER BY ARRAY_POSITION(v_modules, m), ARRAY_POSITION(v_actions, a)
         )
    INTO v_full
    FROM unnest(v_modules) m, unnest(v_actions) a;

  -- read-only/auditor floor: view-only on every module.
  SELECT jsonb_agg(
           jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', 'view'))
           ORDER BY ARRAY_POSITION(v_modules, m)
         )
    INTO v_view_all
    FROM unnest(v_modules) m;

  v_owner_specials := ('["manage_users","manage_roles","manage_boards","approve_time",'
    || '"financial:read","manage_billing","manage_plugins","manage_api_keys",'
    || '"view_audit_log","portal_admin"]')::jsonb;

  -- senior_tech: technician's working set escalated — full tickets + KB,
  -- vault use (view/create/edit), client/time/scheduling/reporting reads,
  -- time writes, approvals. NO financial:read.
  v_senior := (SELECT jsonb_agg(jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', a))
                   ORDER BY ord)
                 FROM (VALUES
                   ('tickets','view',1),('tickets','create',2),('tickets','edit',3),
                   ('tickets','delete',4),('tickets','export',5),
                   ('kb','view',6),('kb','create',7),('kb','edit',8),
                   ('kb','delete',9),('kb','export',10),
                   ('passwords','view',11),('passwords','create',12),('passwords','edit',13),
                   ('clients','view',14),('time','view',15),('time','create',16),
                   ('time','edit',17),('scheduling','view',18),('scheduling','edit',19),
                   ('reporting','view',20)
                 ) AS t(m, a, ord))
    || '["approve_time"]'::jsonb;


  -- bookkeeper: full invoicing control, money-adjacent reads, report
  -- views + exports, and the two financial specials.
  v_bookkeeper := (SELECT jsonb_agg(jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', a))
                      ORDER BY ord)
                    FROM (VALUES
                      ('invoicing','view',1),('invoicing','create',2),('invoicing','edit',3),
                      ('invoicing','delete',4),('invoicing','export',5),
                      ('clients','view',6),('commercial','view',7),('time','view',8),
                      ('reporting','view',9),('reporting','export',10)
                    ) AS t(m, a, ord))
    || '["financial:read","manage_billing"]'::jsonb;

  FOR org IN SELECT id FROM orgs LOOP
    -- The five historical presets (defends pre-0047-era seeds too).
    FOR missing IN
      SELECT * FROM (VALUES
        ('owner', 'Owner', v_full || v_owner_specials),
        ('admin', 'Admin', v_full || v_owner_specials),
        ('read_only', 'Read Only', v_view_all)
      ) AS t(b, name, perms)
      WHERE NOT EXISTS (
        SELECT 1 FROM roles r WHERE r.org_id = org.id AND r.builtin = t.b
      )
    LOOP
      INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
      VALUES (gen_random_uuid(), org.id, missing.name, missing.b, missing.perms, true);
    END LOOP;

    -- Dispatcher: full tickets/scheduling/clients + view/edit on the
    -- secondary set + vault view + board admin.
    IF NOT EXISTS (SELECT 1 FROM roles WHERE org_id = org.id AND builtin = 'dispatcher') THEN
      INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
      VALUES (gen_random_uuid(), org.id, 'Dispatcher', 'dispatcher',
        (SELECT jsonb_agg(jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', a))
                 ORDER BY ord)
           FROM (VALUES
             ('tickets','view',1),('tickets','create',2),('tickets','edit',3),
             ('tickets','delete',4),('tickets','export',5),
             ('scheduling','view',6),('scheduling','create',7),('scheduling','edit',8),
             ('scheduling','delete',9),('scheduling','export',10),
             ('clients','view',11),('clients','create',12),('clients','edit',13),
             ('clients','delete',14),('clients','export',15),
             ('time','view',16),('time','edit',17),
             ('projects','view',18),('projects','edit',19),
             ('commercial','view',20),('commercial','edit',21),
             ('kb','view',22),('kb','edit',23),
             ('reporting','view',24),('reporting','edit',25),
             ('passwords','view',26)
           ) AS t(m, a, ord))
          || '["manage_boards"]'::jsonb,
        true);
    END IF;

    -- Technician: full tickets + KB, vault use, client/time/scheduling/
    -- reporting reads, time + scheduling writes.
    IF NOT EXISTS (SELECT 1 FROM roles WHERE org_id = org.id AND builtin = 'technician') THEN
      INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
      VALUES (gen_random_uuid(), org.id, 'Technician', 'technician',
        (SELECT jsonb_agg(jsonb_build_object('matrix', jsonb_build_object('module', m, 'action', a))
                 ORDER BY ord)
           FROM (VALUES
             ('tickets','view',1),('tickets','create',2),('tickets','edit',3),
             ('tickets','delete',4),('tickets','export',5),
             ('kb','view',6),('kb','create',7),('kb','edit',8),
             ('kb','delete',9),('kb','export',10),
             ('passwords','view',11),('passwords','create',12),('passwords','edit',13),
             ('clients','view',14),('time','view',15),('time','create',16),
             ('time','edit',17),('scheduling','view',18),('scheduling','edit',19),
             ('reporting','view',20)
           ) AS t(m, a, ord))
        , true);
    END IF;

    -- The three Wave-13 presets.
    FOR missing IN
      SELECT * FROM (VALUES
        ('senior_tech', 'Senior Tech', v_senior),
        ('bookkeeper', 'Bookkeeper', v_bookkeeper),
        ('auditor', 'Auditor', v_view_all || '["view_audit_log"]'::jsonb)
      ) AS t(b, name, perms)
      WHERE NOT EXISTS (
        SELECT 1 FROM roles r WHERE r.org_id = org.id AND r.builtin = t.b
      )
    LOOP
      INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
      VALUES (gen_random_uuid(), org.id, missing.name, missing.b, missing.perms, true);
    END LOOP;
  END LOOP;
END $$;
