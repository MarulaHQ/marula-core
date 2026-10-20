-- MCP server v1 + IF8 agent platform (the private build tracker Phase 3, lane `p9-mcp-v1`).
--
-- Purpose: three additive tables backing the agent-platform substrate —
-- the capability registry, the versioned agent-skill store, and the IF8
-- proposal pipeline (docs/instant-first.md "Agent-authored changes default
-- to approval" + docs/10-agent-platform.md §4/§5).
--
-- `mcp_capability_grants` — the deny-by-default capability registry. A
-- capability slug ("<namespace>:read" | "<namespace>:write") that has NO
-- row for the org is DENIED: the MCP adapter exposes only tools whose
-- derived capability the org granted. `mode` is the involvement dial per
-- capability: `proposed` (default) means agent-originated record changes
-- in that area surface as proposals awaiting approval; `autonomous` is an
-- explicit, per-capability grant — never a global switch — recorded here
-- so later write-execution lanes consume the dial position from data.
-- Autonomous is legal ONLY on `:write` capabilities (reads have nothing
-- to apply autonomously).
--
-- `agent_skills` — versioned declarative skill manifests (§4): id, slug,
-- integer version, JSON manifest {capabilities, prompts, rules}. Rows are
-- append-only (UNIQUE (org_id, slug, version)); diffing and rollback are
-- reads plus a NEW version — history is never rewritten.
--
-- `agent_proposals` — the IF8 proposal store. One row per proposed change
-- with the full snapshot the approval decision needs: before value, the
-- proposed after value, the agent identity (kind + ref), the skill and
-- its version, the rule-set hash, the session/tool coordinates, the
-- record coordinates (entity/entity_id/op), and the involvement-dial
-- position captured at proposal time. Lifecycle: `proposed` →
-- `approved`/`declined` (human decision) → `applied` (the approved write
-- landed through the IF1 autosave path) — or `withdrawn` by the agent.
-- Declined ⇒ no write at all; the row is the audit record that the
-- proposal existed and died here. `classification` marks client-visible /
-- external-commit proposals, which always thread a human approval
-- regardless of any dial position.
--
-- Non-purpose: no write-execution state machine beyond the proposal
-- lifecycle (v1 ships read tools + proposal CRUD only), no MCP transport
-- state (the adapter is stateless), no rule evaluation (§4 layered rules
-- land with the write-execution lanes).

CREATE TABLE mcp_capability_grants (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- "<namespace>:read" | "<namespace>:write"; namespaces mirror the API
  -- path segments the MCP tool catalog derives from (tickets, kb, ...).
  capability text NOT NULL CHECK (capability ~ '^[a-z][a-z0-9_-]*:(read|write)$'),
  -- The involvement dial for this capability (docs/10-agent-platform §4).
  mode text NOT NULL DEFAULT 'proposed' CHECK (mode IN ('proposed','autonomous')),
  granted_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, capability),
  -- Autonomous is an explicit per-capability grant for record CHANGES;
  -- read capabilities have nothing to act autonomously on.
  CHECK (mode = 'proposed' OR capability LIKE '%:write')
);

CREATE INDEX idx_mcp_capability_grants_org ON mcp_capability_grants (org_id);

ALTER TABLE mcp_capability_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY mcp_capability_grants_org ON mcp_capability_grants
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE agent_skills (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  slug text NOT NULL CHECK (slug ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
  version integer NOT NULL CHECK (version >= 1),
  -- Declarative manifest (§4): {capabilities: [...], prompts: [...], rules: [...]}.
  -- Shape-validated in Rust on every write; the DB pins the JSON type.
  manifest jsonb NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
  created_by uuid NOT NULL REFERENCES users(id),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, slug, version)
);

CREATE INDEX idx_agent_skills_org_slug ON agent_skills (org_id, slug, version DESC);

ALTER TABLE agent_skills ENABLE ROW LEVEL SECURITY;
CREATE POLICY agent_skills_org ON agent_skills
  USING (org_id = current_setting('app.org_id', true)::uuid);

CREATE TABLE agent_proposals (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Record coordinates. v1 restricts the API surface to ticket updates;
  -- the schema stays generic so widening the pipeline is additive.
  entity text NOT NULL,
  entity_id uuid,
  op text NOT NULL CHECK (op IN ('create','update','delete')),
  CHECK (op <> 'update' OR entity_id IS NOT NULL),
  -- IF8 snapshot: the before value the agent saw and the proposed after
  -- value. Objects (or SQL NULL-as-JSON `null` for absent befores);
  -- application re-reads the live entity, so these are the decision
  -- record, not a blind write payload.
  before_value jsonb NOT NULL DEFAULT 'null'::jsonb
    CHECK (jsonb_typeof(before_value) IN ('object','null')),
  proposed_value jsonb NOT NULL
    CHECK (jsonb_typeof(proposed_value) IN ('object','null')),
  -- internal | client_visible | external_commit. Client-visible and
  -- external-commit proposals always thread a human approval, whatever
  -- the dial says (instant-first.md, "Never bypass a human").
  classification text NOT NULL DEFAULT 'internal'
    CHECK (classification IN ('internal','client_visible','external_commit')),
  -- The write-capability this change belongs to (must be granted; the
  -- dial position below is captured from its row at creation time).
  capability text,
  -- Involvement-dial snapshot at proposal time: 'proposed' (default) or
  -- 'autonomous' (explicit per-capability grant, recorded for the
  -- write-execution lanes).
  dial_mode text NOT NULL DEFAULT 'proposed'
    CHECK (dial_mode IN ('proposed','autonomous')),
  -- The entity write-seq the proposal was created against — drift
  -- provenance for the approval UI (application re-reads live state).
  base_version bigint NOT NULL DEFAULT 0,
  -- proposed | approved | declined | withdrawn | applied. Forward-only;
  -- decisions compare-and-set on `proposed` so a proposal can never be
  -- decided twice.
  status text NOT NULL DEFAULT 'proposed'
    CHECK (status IN ('proposed','approved','declined','withdrawn','applied')),
  -- Agent provenance (§5): who proposed, through what skill, under which
  -- rules, in which session, via which tool.
  agent_kind text NOT NULL DEFAULT 'agent' CHECK (agent_kind IN ('agent','api-key','mcp')),
  agent_ref text NOT NULL,
  skill_slug text,
  skill_version integer,
  rule_set_hash text,
  session_id text,
  tool text,
  -- The user the agent acted on behalf of (approver may be this user or
  -- an owner/admin).
  for_user uuid REFERENCES users(id),
  rationale text,
  -- Human decision record; decided_by/decided_at set on approve/decline.
  decided_by uuid REFERENCES users(id),
  decided_at timestamptz,
  decision_note text,
  applied_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_agent_proposals_org_status ON agent_proposals (org_id, status, created_at DESC, id DESC);
CREATE INDEX idx_agent_proposals_org_entity ON agent_proposals (org_id, entity, entity_id);

ALTER TABLE agent_proposals ENABLE ROW LEVEL SECURITY;
CREATE POLICY agent_proposals_org ON agent_proposals
  USING (org_id = current_setting('app.org_id', true)::uuid);
