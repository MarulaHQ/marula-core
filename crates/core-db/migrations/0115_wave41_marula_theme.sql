-- Wave 41 lane 2 (p41-marula-theme) — the Tangerine → Marula preset id.
--
-- The retired identity's preset id was stored per org, in the settings row
-- `appearance.theme` (org scope) and its per-user sibling
-- `appearance.theme_override`. A rename that only changes the enum leaves every
-- existing row pointing at a preset id that no longer exists, and the org
-- silently falls back to the default instead of keeping the theme it chose.
-- This migration is the other half of the rename, on the same
-- supersedes-the-old-value pattern migration 0097 used for the
-- `appearance.theme` → `appearance.mode` split.
--
-- `tangerine` becomes `marula` (the Neutral column, now the pinned default).
-- `slate` and `copper` are dropped outright rather than mapped: they were
-- second-brand neutrals with no authority in the brand spec, and the axis they
-- wanted to move is now `marula` vs `marula-warm`. An org on either falls back
-- to the default, which is the same outcome as mapping it and is honest about
-- what happened.
--
-- Rows are only rewritten when they hold a retired id. `org_default` (the
-- per-user sentinel), `pack:<slug>` references and `high_contrast` are left
-- exactly as they are, so installed theme packs and per-user overrides survive.
--
-- `settings.value` is jsonb, so every literal below is a JSON string
-- ('"tangerine"'::jsonb), not a bare text literal. A bare 'tangerine' would not
-- compare equal to a jsonb string and the migration would silently match zero
-- rows — a rename that renames nothing, which is the worst outcome available.

-- 1. The org-scope theme: rename tangerine -> marula, reset the dropped pair.
UPDATE settings
   SET value = '"marula"'::jsonb
 WHERE key = 'appearance.theme'
   AND scope = 'org'
   AND value = '"tangerine"'::jsonb;

UPDATE settings
   SET value = '"marula"'::jsonb
 WHERE key = 'appearance.theme'
   AND scope = 'org'
   AND value IN ('"slate"'::jsonb, '"copper"'::jsonb);

-- 2. The per-user override: same treatment, minus the sentinel.
UPDATE settings
   SET value = '"marula"'::jsonb
 WHERE key = 'appearance.theme_override'
   AND scope = 'user'
   AND value = '"tangerine"'::jsonb;

UPDATE settings
   SET value = '"marula"'::jsonb
 WHERE key = 'appearance.theme_override'
   AND scope = 'user'
   AND value IN ('"slate"'::jsonb, '"copper"'::jsonb);
