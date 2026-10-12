-- The workflow node registry's persistence (TASKLIST Wave 43 lane 13,
-- `p43-node-registry`; head was 0126).
--
-- TWO changes, and the split between them is the lane's load-bearing
-- decision:
--
-- (A) `plugin_surface_entries` gains the five fields that turn a surface
--     entry into a workflow NODE (the A2 six-field contract). Today an
--     entry carries an input schema alone, so a workflow could call a
--     plugin but could not use what it returned. The five:
--       output_schema     — what the entry returns, so a downstream
--                           value.reference or condition can name a field
--       idempotent        — flow.retry MUST refuse to wrap any node
--                           declared false; defaults FALSE, because an
--                           undeclared plugin action's retry-safety is
--                           unknown and a plugin that wants retries must
--                           say so
--       emits             — the event types the entry can produce, the
--                           mechanism the static loop refusal reads. A
--                           plugin that emits `ticket.status_changed` and
--                           then waits for it would otherwise deadlock a
--                           workflow. Defaults empty, the safe side.
--       audience          — human / ai / both; defaults HUMAN so a silent
--                           plugin never joins the LLM tool catalogue
--       agent_description — model-facing prose beside the human label
--
--     `params_schema` keeps riding inside `definition` where it has
--     always lived, so the manifest shape does not move. No FK on emits:
--     a declared event type validates against lane 2's registry
--     (`event_types`, 0124) at GRANT time, never here — the posture
--     0126(C) already states.
--
-- (B) `workflow_node_catalogue` pins a RELEASE of the Rust catalogue.
--     This is the honest way to keep a "registry table for declarative
--     nodes" without forking a second registry, which ADR 0016 rule (c)
--     forbids: the node declarations live in core-domain and nowhere else,
--     so the table records WHICH release a document was validated against
--     plus a digest over every declaration, and carries no node rows. A
--     workflow saved under one release and read under another is a fact
--     the executor and the run ledger must be able to see, and two builds
--     claiming the same version must not be able to differ silently.
--
--     Global, like `event_types` (0124): it defines what the catalogue IS,
--     not who may read it, so it carries no org_id and no RLS policy.
--     `assert_rls_coverage` only inspects tables carrying org_id.

-- ============================================================================
-- (A) the six-field plugin node contract
-- ============================================================================

ALTER TABLE plugin_surface_entries
    ADD COLUMN output_schema jsonb,
    ADD COLUMN idempotent boolean NOT NULL DEFAULT false,
    ADD COLUMN emits text[] NOT NULL DEFAULT '{}',
    ADD COLUMN audience text NOT NULL DEFAULT 'human',
    ADD COLUMN agent_description text;

-- The audience vocabulary is closed and checked here, not only in Rust:
-- a row the CHECK refused can never round-trip into the registry, which
-- is the same reason 0126 puts the surface-kind CHECK beside the parse
-- arm.
ALTER TABLE plugin_surface_entries
    ADD CONSTRAINT plugin_surface_entries_audience
    CHECK (audience IN ('human', 'ai', 'both'));

-- The emit seam is bounded: a manifest declares at most a handful of
-- event types, and an unbounded array in this column is a way to smuggle
-- a long list nobody reviewed at consent time.
ALTER TABLE plugin_surface_entries
    ADD CONSTRAINT plugin_surface_entries_emits_bounded
    CHECK (cardinality(emits) <= 32);

COMMENT ON COLUMN plugin_surface_entries.output_schema IS
    'JSON Schema of what the entry returns. NULL means nothing it returns is referenceable downstream.';
COMMENT ON COLUMN plugin_surface_entries.idempotent IS
    'False when repeating the action is unsafe. flow.retry and the operator retry both refuse a false.';
COMMENT ON COLUMN plugin_surface_entries.emits IS
    'Event types this entry can produce, validated against event_types at grant time. Read by the static loop refusal.';
COMMENT ON COLUMN plugin_surface_entries.audience IS
    'human | ai | both — palette visibility against the LLM tool catalogue.';
COMMENT ON COLUMN plugin_surface_entries.agent_description IS
    'Model-facing prose for this entry, beside the human label. NULL falls back to the label.';

-- ============================================================================
-- (B) the catalogue release pin
-- ============================================================================

CREATE TABLE workflow_node_catalogue (
    -- The release the lane's registry declares. Semver, because a node
    -- renamed under the same version is a lie the digest would catch but
    -- a saved workflow would not.
    version text PRIMARY KEY,
    -- sha256 over every declaration (id, label, agent description,
    -- family, tier, idempotent, audience, executor path, both schemas and
    -- the emit list), in declaration order. The integrity anchor: a build
    -- claiming this version with different declarations is detectable.
    digest text NOT NULL,
    node_count integer NOT NULL CHECK (node_count > 0),
    -- The lane that published the release, for the audit trail.
    published_by text,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_workflow_node_catalogue_recorded
    ON workflow_node_catalogue (recorded_at DESC);

-- The pin is immutable history: a re-record of the same version is a
-- no-op rather than a rewrite, which is the same rule the run ledger
-- follows (a record that must not be edited).
