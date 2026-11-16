-- The pod documents' role keys, moved onto the keys a session actually
-- reports (migration 0227).
--
-- Purpose: `project_pod_settings.role_key` was written from three names the
-- product uses nowhere else — `project_manager`, `project_coordinator`,
-- `project_tech`. `Me.roles` carries a builtin role's preset tag
-- (`dispatcher`, `senior_tech`, `technician`, …) or a custom role's name, so a
-- document saved under one of those three could never match a holder: the pod
-- it granted stayed invisible, and the ceiling read as if the role had been
-- given nothing at all.
--
-- The mapping is the meaning each legacy name carried:
--
--   project_manager     -> dispatcher
--   project_coordinator -> senior_tech
--   project_tech        -> technician
--
-- An org may already hold a document under the target key, from an admin who
-- used the real preset. Both rows describe one role, so their pods arrays
-- merge instead of one row winning: the union keeps first-occurrence order
-- (the legacy row's pods first, then the ones only the target had), and the
-- legacy row is deleted afterwards. Nothing an admin granted is lost.
--
-- Compat: forward-only data change, no schema change, every statement scoped
-- by WHERE. Rows under any other key — including custom role names — are not
-- touched.

-- 1. Merge into an existing document under the target key.
WITH legacy_keys(old_key, new_key) AS (
  VALUES
    ('project_manager', 'dispatcher'),
    ('project_coordinator', 'senior_tech'),
    ('project_tech', 'technician')
)
UPDATE project_pod_settings AS legacy
SET pods = (
      SELECT ARRAY(
        SELECT pod
          FROM unnest(legacy.pods || target.pods)
               WITH ORDINALITY AS merged(pod, ord)
         GROUP BY pod
         ORDER BY MIN(ord)
      )
    ),
    updated_at = now()
FROM legacy_keys, project_pod_settings AS target
WHERE legacy.role_key = legacy_keys.old_key
  AND target.org_id = legacy.org_id
  AND target.surface = legacy.surface
  AND target.role_key = legacy_keys.new_key;

-- 2. Drop the merged legacy rows; their pods now live on the target row.
WITH legacy_keys(old_key, new_key) AS (
  VALUES
    ('project_manager', 'dispatcher'),
    ('project_coordinator', 'senior_tech'),
    ('project_tech', 'technician')
)
DELETE FROM project_pod_settings AS legacy
USING legacy_keys, project_pod_settings AS target
WHERE legacy.role_key = legacy_keys.old_key
  AND target.org_id = legacy.org_id
  AND target.surface = legacy.surface
  AND target.role_key = legacy_keys.new_key;

-- 3. Rename what is left: these orgs had no target row, so the key moves and
--    the unique (org_id, surface, role_key) constraint stays satisfied.
WITH legacy_keys(old_key, new_key) AS (
  VALUES
    ('project_manager', 'dispatcher'),
    ('project_coordinator', 'senior_tech'),
    ('project_tech', 'technician')
)
UPDATE project_pod_settings AS remaining
SET role_key = legacy_keys.new_key,
    updated_at = now()
FROM legacy_keys
WHERE remaining.role_key = legacy_keys.old_key;
