-- Template registry (Wave 13 lane 5 `p13-template-registry`; the private build tracker
-- KB3 + KB4, ADR 0004 "template registry" — planning A13).
--
-- One `Template` entity, typed: ticket / note / email / kb / quote_invoice
-- / project / workflow. No per-type template tables (ADR 0004: all
-- template kinds ride this one row shape). Body text carries typed
-- variables (`{{client.name}}`, `{{ticket.number}}`) drawn from a CLOSED
-- per-kind vocabulary — resolution happens at render time from real entity
-- context (core-domain::templates), never raw string interpolation of user
-- data: an unknown or out-of-vocabulary placeholder is a typed 400, never
-- a silent empty string.
--
-- Shipped defaults are immutable seed rows (KB4, defaults-first): every
-- org carries the canonical set with `builtin` tags + `is_system` (mirrors
-- lane 6's preset-roles pattern — forked_from records the fork origin and
-- orgs fork, never edit in place; restore resets a forked preset row to
-- the canonical copy compiled into core-domain::templates). Org-created
-- templates are first-class rows on the same shape. The seed bodies mirror
-- core_domain::templates::DefaultTemplate byte-for-byte; a core-db test
-- pins the two copies so they cannot silently drift (0071's pattern).
--
-- Quote-invoice templates ride Wave 12 lane 2's quote-catalog shapes (the
-- quote row + roll-up totals are the render context); this registry adds
-- the reusable content layer, not a parallel quote schema.

CREATE TABLE templates (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Closed vocabulary (core_domain::entities::templates::TemplateKind).
  kind text NOT NULL CHECK (kind IN
    ('ticket','note','email','kb','quote_invoice','project','workflow')),
  slug text NOT NULL,
  name text NOT NULL,
  description text NOT NULL DEFAULT '',
  -- Email/tabular subject line where the kind supports one; nullable so
  -- note/KB/project scaffolds stay body-only.
  subject text,
  body text NOT NULL,
  -- Declared variables for editor UX: [{"key","label"}]. Validated at
  -- write time against the kind's closed vocabulary in core-domain.
  variables jsonb NOT NULL DEFAULT '[]',
  -- Shipped preset tag (e.g. 'ticket_first_response'), NULL for
  -- org-created rows.
  builtin text,
  -- System rows are the shipped defaults: immutable from the API
  -- (orgs fork), restorable to the canonical copy.
  is_system boolean NOT NULL DEFAULT false,
  -- For a forked copy: the origin's builtin tag (or the origin slug for
  -- forks of org-created templates, later).
  forked_from text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, kind, slug)
);

CREATE INDEX idx_templates_org_kind ON templates (org_id, kind);

ALTER TABLE templates ENABLE ROW LEVEL SECURITY;
CREATE POLICY templates_org ON templates
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- Backfill the shipped defaults for every org (idempotent — defends
-- pre-lane orgs exactly like 0071's preset backfill). Bodies/subjects
-- mirror core_domain::templates::DefaultTemplate; the byte-drift test in
-- core-db keeps the two copies honest.
DO $$
DECLARE
  org record;
  missing record;
BEGIN
  FOR org IN SELECT id FROM orgs LOOP
    FOR missing IN
      SELECT * FROM (VALUES
        ('ticket', 'standard_service_ticket', 'Standard service ticket',
         'Standard intake skeleton for a managed-service request.',
         NULL::text,
         E'## Request\n{{client.name}} reported: {{ticket.summary}}\n\n## Triage\nPriority: {{ticket.priority}}\nStatus: {{ticket.status}}\n\n## Steps\n- [ ] Confirm scope with {{contact.name}}\n- [ ] Investigate\n- [ ] Document resolution\n'),
        ('note', 'ticket_resolution_note', 'Ticket resolution note',
         'Closing-note skeleton: what was wrong, what fixed it.',
         NULL,
         E'Root cause: \nResolution: \nVerified with {{contact.name}} on {{ticket.number}}.\n'),
        ('email', 'ticket_first_response', 'Ticket first response',
         'Warm acknowledgement email for a newly triaged ticket.',
         'Re: {{ticket.summary}} ({{ticket.number}})',
         E'Hi {{contact.name}},\n\nThanks for reaching out — {{ticket.summary}} is logged as {{ticket.number}} and is currently {{ticket.priority}} priority.\n\nWe will keep you posted here. Reply to this email to add detail.\n\n{{user.name}}\n'),
        ('kb', 'article_starter', 'KB article starter',
         'Problem/cause/fix skeleton for a staff KB article.',
         NULL,
         E'## Problem\n\n## Environment\n\n## Cause\n\n## Fix\n\nMaintained by {{user.name}}.\n'),
        ('quote_invoice', 'standard_quote_cover', 'Standard quote cover note',
         'Cover-email body for a sent quote from the quote catalog.',
         'Quote {{quote.number}} from {{org.name}}',
         E'Hi {{contact.name}},\n\nPlease find our quote {{quote.number}}: {{quote.total}} {{quote.currency}}, valid as sent.\n\nHappy to walk through any line.\n\n{{user.name}}\n'),
        ('project', 'client_onboarding_scaffold', 'Client onboarding scaffold',
         'Phase/plan skeleton for onboarding a new client.',
         NULL,
         E'## Onboarding: {{client.name}}\n\n1. Kickoff with {{contact.name}}\n2. Asset inventory\n3. Access and credentials handover\n4. Monitoring baseline\n5. First review\n'),
        ('workflow', 'escalation_playbook', 'Escalation playbook',
         'Escalation runbook body for out-of-SLA tickets.',
         NULL,
         E'## Escalate {{ticket.number}}\n\nTrigger: SLA at risk for {{client.name}}.\n\n1. Notify {{assignee.name}}\n2. Engage {{contact.name}}\n3. Escalate internally if unresolved in 2h\n')
      ) AS t(k, s, n, d, subj, b)
      WHERE NOT EXISTS (
        SELECT 1 FROM templates tpl
        WHERE tpl.org_id = org.id AND tpl.kind = missing.k AND tpl.slug = missing.s
      )
    LOOP
      INSERT INTO templates (id, org_id, kind, slug, name, description, subject, body, variables, builtin, is_system)
      VALUES (gen_random_uuid(), org.id, missing.k, missing.s, missing.n, missing.d, missing.subj, missing.b, '[]'::jsonb, missing.s, true);
    END LOOP;
  END LOOP;
END $$;
