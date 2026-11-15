-- Describe-it surfaces for quotes and contracts.
--
-- The design pack puts a "Describe it" door beside New quote / New contract,
-- and states the rule plainly: no AI grant means the door is not there. That
-- needs a real grant target, so these two native surfaces are seeded like the
-- other shipped generation surfaces. Deny-by-default: no grant rows exist, so
-- no org may call them until an admin grants the surface in Settings → AI.
--
-- Native (not plugin): the endpoints ship in core and the capability is core's
-- own, not a marketplace registration.

INSERT INTO ai_surfaces (id, org_id, key, label, kind, description) VALUES
  ('0a100000-0000-0000-0000-00000000000a', NULL, 'quote_authoring',
   'Quote authoring', 'native',
   'Turn notes or a sentence into a quote draft for review in the ordinary builder.'),
  ('0a100000-0000-0000-0000-00000000000b', NULL, 'contract_authoring',
   'Contract authoring', 'native',
   'Turn notes or a sentence into a contract draft for review in the ordinary builder.')
ON CONFLICT (COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::uuid), key)
DO UPDATE SET
  label = EXCLUDED.label,
  description = EXCLUDED.description,
  kind = EXCLUDED.kind;
