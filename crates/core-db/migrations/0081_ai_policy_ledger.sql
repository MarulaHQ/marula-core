-- AI availability matrix + usage ledger (ADR 0008; Wave 18 lane 1
-- `p18-ai-policy-ledger`; migration 0081).
--
-- Purpose: the core data model for AI-assisted surfaces:
--   * `ai_surfaces` — the surface registry AS DATA (native generation
--     surfaces, the internal embeddings surface, and plugin-registrable
--     keys seeded deny-by-default). Plugins (Relay first) register org-
--     scoped rows through the same table — no core schema changes.
--   * `ai_model_catalog` — admin-curated (provider, model) pairs with
--     display metadata + an enabled flag; the ONLY models a surface may
--     resolve to.
--   * `ai_surface_grants` — per-surface per-role/user availability rows
--     in the same deny-by-default, audited discipline as the capability
--     registry (no grants ⇒ no AI on that surface).
--   * `ai_model_credentials` — the `ModelCredential` adapter slot: sealed
--     credentials (api_key / oauth_token / subscription_account) for the
--     org AND for personal per-user use. Same AES-256-GCM sealing and
--     never-returned posture as `byok_secrets` (0044): plaintext never
--     enters a log, a trail row, or an event payload.
--   * `ai_usage_ledger` — APPEND-ONLY usage rows (org, user, surface,
--     provider, model, tokens, request id); the "who did this is a
--     query" doctrine applied to AI spend.
--   * `ai_caps` — org-wide + per-user soft (warn + notify) and hard
--     (bound the call with a typed refusal — degrade-never-5xx) token
--     caps per period.
--   * `ai_model_picks` — sticky per-surface model picks (per user + org
--     default), resolved user→org within the admin allowlist with honest
--     fallback when a pick is no longer valid.
--   * `ops.v1_ai_usage_user_progress` / `ops.v1_ai_usage_org_progress` —
--     security_invoker read models for progress against caps (queryable
--     per-user + per-org roll-ups).
--
-- Supersede posture (U6-safe, customization-aware): the flat
-- `ai.completion_provider` / `ai.completion_model` settings defaults are
-- superseded IN CODE (their declarations are re-described as deprecated;
-- legacy values stay readable and keep seeding the org default when the
-- catalog contains the customized pair). No settings rows are touched or
-- dropped here — additive only.
--
-- Non-purpose: no chat runner / tool loop (lane 2), no provider HTTP
-- transports beyond the existing embeddings edge, no UI.

CREATE TABLE ai_surfaces (
  id uuid PRIMARY KEY,
  -- NULL org_id = globally seeded surface (native/internal/plugin keys);
  -- non-NULL = an org-scoped plugin registration. RLS exposes NULL rows
  -- to every org, org rows only to their org.
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  key text NOT NULL,
  label text NOT NULL,
  -- 'native' = shipped generation surfaces; 'internal' = core-internal
  -- (embeddings_search, never a user-facing grant target);
  -- 'plugin' = plugin-registrable key (deny-by-default until granted).
  kind text NOT NULL CHECK (kind IN ('native', 'internal', 'plugin')),
  description text NOT NULL DEFAULT '',
  enabled boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- Org rows are unique per (org, key); global rows (org_id IS NULL) are
-- unique by key alone. One named index covers both via COALESCE.
CREATE UNIQUE INDEX uq_ai_surfaces_key ON ai_surfaces
  (COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::uuid), key);

CREATE TABLE ai_model_catalog (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  provider text NOT NULL,
  model text NOT NULL,
  label text NOT NULL DEFAULT '',
  -- display capability tags (e.g. ["chat","tools"]) — metadata only,
  -- never a security primitive.
  capabilities jsonb NOT NULL DEFAULT '[]',
  -- cost-weight class (1 = cheap … 5 = premium) for admin budgeting.
  cost_weight integer NOT NULL DEFAULT 1 CHECK (cost_weight BETWEEN 1 AND 5),
  enabled boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT uq_ai_model_catalog_pair UNIQUE (org_id, provider, model)
);

CREATE TABLE ai_surface_grants (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  surface_id uuid NOT NULL REFERENCES ai_surfaces(id) ON DELETE CASCADE,
  -- Exactly one grantee anchor: role grant OR user grant (checked).
  grantee_kind text NOT NULL CHECK (grantee_kind IN ('role', 'user')),
  role_id uuid REFERENCES roles(id) ON DELETE CASCADE,
  user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT ai_surface_grants_one_grantee CHECK (
    (grantee_kind = 'role' AND role_id IS NOT NULL AND user_id IS NULL) OR
    (grantee_kind = 'user' AND user_id IS NOT NULL AND role_id IS NULL)
  )
);

CREATE UNIQUE INDEX uq_ai_surface_grants_grantee ON ai_surface_grants
  (org_id, surface_id, grantee_kind,
   COALESCE(role_id, '00000000-0000-0000-0000-000000000000'::uuid),
   COALESCE(user_id, '00000000-0000-0000-0000-000000000000'::uuid));

CREATE INDEX idx_ai_surface_grants_lookup ON ai_surface_grants (org_id, surface_id);
CREATE INDEX idx_ai_surface_grants_user ON ai_surface_grants (org_id, user_id)
  WHERE user_id IS NOT NULL;
CREATE INDEX idx_ai_surface_grants_role ON ai_surface_grants (org_id, role_id)
  WHERE role_id IS NOT NULL;

CREATE TABLE ai_model_credentials (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL = org credential; set = personal credential (serves ONLY that
  -- user's calls, and only while the admin policy flag
  -- `ai.personal_credentials_enabled` is on).
  owner_user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  provider text NOT NULL,
  -- Sealed credential kind (ADR 0008 §2): api-key, oauth-token,
  -- subscription-account (subscription bridges ride official OAuth paths
  -- as plug-in providers against this slot).
  kind text NOT NULL CHECK (kind IN ('api_key', 'oauth_token', 'subscription_account')),
  label text NOT NULL DEFAULT '',
  -- AES-256-GCM ciphertext (nonce || ct) of the secret material — same
  -- sealing as byok_secrets; bytes are never selected back to clients.
  secret_encrypted bytea NOT NULL,
  -- Non-secret metadata (oauth expiry hints, account labels). NEVER
  -- secret bytes.
  credential_meta jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- One active credential per (org, owner anchor, provider, kind): the org
-- anchor is a fixed zero-uuid for org credentials, the user id for
-- personal ones. Named unique INDEX (expression keys can't be table
-- constraints) — violation errors surface this name for typed mapping.
CREATE UNIQUE INDEX uq_ai_model_credentials_owner ON ai_model_credentials
  (org_id,
   COALESCE(owner_user_id, '00000000-0000-0000-0000-000000000000'::uuid),
   provider,
   kind);

CREATE INDEX idx_ai_model_credentials_org ON ai_model_credentials (org_id, status);
CREATE INDEX idx_ai_model_credentials_owner ON ai_model_credentials (org_id, owner_user_id)
  WHERE owner_user_id IS NOT NULL;

-- Append-only usage ledger: the enforcement + progress substrate.
CREATE TABLE ai_usage_ledger (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- Denormalized surface KEY (not FK id) so ledger rows stay readable
  -- even if a plugin surface row is later removed.
  surface text NOT NULL,
  provider text NOT NULL,
  model text NOT NULL,
  prompt_tokens integer NOT NULL DEFAULT 0 CHECK (prompt_tokens >= 0),
  completion_tokens integer NOT NULL DEFAULT 0 CHECK (completion_tokens >= 0),
  request_id text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_ai_usage_ledger_org_time ON ai_usage_ledger (org_id, created_at DESC);
CREATE INDEX idx_ai_usage_ledger_user_time ON ai_usage_ledger (org_id, user_id, created_at DESC);
CREATE INDEX idx_ai_usage_ledger_surface_time ON ai_usage_ledger (org_id, surface, created_at DESC);

-- Append-only discipline, enforced in the database: no UPDATE, no DELETE.
CREATE FUNCTION ai_usage_ledger_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'ai_usage_ledger is append-only: % rejected', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER ai_usage_ledger_append_only_guard
  BEFORE UPDATE OR DELETE ON ai_usage_ledger
  FOR EACH ROW EXECUTE FUNCTION ai_usage_ledger_append_only();

-- Caps: one row per scope (org-wide user_id NULL / per-user) per period.
CREATE TABLE ai_caps (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL = org-wide cap; set = per-user cap (user cap wins for that user).
  user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  period text NOT NULL DEFAULT 'month' CHECK (period IN ('month')),
  soft_token_limit bigint CHECK (soft_token_limit IS NULL OR soft_token_limit > 0),
  hard_token_limit bigint CHECK (hard_token_limit IS NULL OR hard_token_limit > 0),
  -- At least one bound must be set; a cap row with neither is noise.
  CONSTRAINT ai_caps_has_a_limit CHECK (soft_token_limit IS NOT NULL OR hard_token_limit IS NOT NULL),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX uq_ai_caps_scope ON ai_caps
  (org_id,
   COALESCE(user_id, '00000000-0000-0000-0000-000000000000'::uuid),
   period);

-- Sticky per-surface model picks (ADR 0008 §1: user pick → org default,
-- always within the admin allowlist). user_id NULL = the org default
-- pick for a surface.
CREATE TABLE ai_model_picks (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  user_id uuid REFERENCES users(id) ON DELETE CASCADE,
  surface_id uuid NOT NULL REFERENCES ai_surfaces(id) ON DELETE CASCADE,
  provider text NOT NULL,
  model text NOT NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX uq_ai_model_picks_surface ON ai_model_picks
  (org_id,
   COALESCE(user_id, '00000000-0000-0000-0000-000000000000'::uuid),
   surface_id);

-- ---------------------------------------------------------------------------
-- RLS: every org-scoped table carries the standard tenancy policy
-- (app.org_id), matching 0044. ai_surfaces also exposes global rows.
-- ---------------------------------------------------------------------------

ALTER TABLE ai_surfaces ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_surfaces_org ON ai_surfaces
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_model_catalog ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_model_catalog_org ON ai_model_catalog
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_surface_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_surface_grants_org ON ai_surface_grants
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_model_credentials ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_model_credentials_org ON ai_model_credentials
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_usage_ledger ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_usage_ledger_org ON ai_usage_ledger
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_caps ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_caps_org ON ai_caps
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE ai_model_picks ENABLE ROW LEVEL SECURITY;
CREATE POLICY ai_model_picks_org ON ai_model_picks
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- Surface registry seed data (registry AS DATA — migration-seeded, not
-- enum-coded). Plugin keys are seeded present but deny-by-default: no
-- grant rows exist, so nothing may call them until an admin grants.
-- ---------------------------------------------------------------------------

INSERT INTO ai_surfaces (id, org_id, key, label, kind, description) VALUES
  ('0a100000-0000-0000-0000-000000000001', NULL, 'general_chat',
   'General agent chat', 'native',
   'The general agent chat surface (ADR 0008 §3 runner).'),
  ('0a100000-0000-0000-0000-000000000002', NULL, 'ticket_note_assist',
   'Ticket note assist', 'native',
   'In-editor drafting assistance inside ticket note composers.'),
  ('0a100000-0000-0000-0000-000000000003', NULL, 'doc_drafting',
   'Document drafting', 'native',
   'Drafting assistance inside the documents surface.'),
  ('0a100000-0000-0000-0000-000000000004', NULL, 'email_drafting',
   'Email drafting', 'native',
   'Drafting assistance inside client-email composers.'),
  ('0a100000-0000-0000-0000-000000000005', NULL, 'embeddings_search',
   'Embeddings search', 'internal',
   'Internal retrieval surface backing vector search — not a user grant target.'),
  ('0a100000-0000-0000-0000-000000000006', NULL, 'data_analysis',
   'Data analysis', 'plugin',
   'Relay-premium plugin surface (REL-I1) — registered key, no core implementation.'),
  ('0a100000-0000-0000-0000-000000000007', NULL, 'automation_writing',
   'Automation writing', 'plugin',
   'Relay-premium plugin surface (REL-I1) — registered key, no core implementation.'),
  ('0a100000-0000-0000-0000-000000000008', NULL, 'dashboard_generation',
   'Dashboard generation', 'plugin',
   'Relay-premium plugin surface (REL-I1) — registered key, no core implementation.'),
  ('0a100000-0000-0000-0000-000000000009', NULL, 'transcription',
   'Transcription AI', 'plugin',
   'Relay-premium plugin surface (REL-I1) — registered key, no core implementation.');

-- ---------------------------------------------------------------------------
-- Progress read models (ops.v1_* contract, security_invoker = true so the
-- underlying RLS applies exactly as on the base tables).
-- ---------------------------------------------------------------------------

CREATE VIEW ops.v1_ai_usage_user_progress WITH (security_invoker = true) AS
WITH period AS (
  SELECT date_trunc('month', now()) AS period_start
),
usage AS (
  SELECT l.org_id, l.user_id, SUM(l.prompt_tokens + l.completion_tokens) AS used_tokens
  FROM ai_usage_ledger l, period p
  WHERE l.created_at >= p.period_start
  GROUP BY l.org_id, l.user_id
),
caps AS (
  SELECT org_id, user_id, period, soft_token_limit, hard_token_limit
  FROM ai_caps
  WHERE period = 'month' AND user_id IS NOT NULL
)
SELECT
  u.org_id,
  u.user_id,
  p.period_start AS period,
  COALESCE(u.used_tokens, 0) AS used_tokens,
  c.soft_token_limit AS user_soft_limit,
  c.hard_token_limit AS user_hard_limit,
  oc.soft_token_limit AS org_soft_limit,
  oc.hard_token_limit AS org_hard_limit,
  -- effective limits: per-user cap when present, else org-wide
  COALESCE(c.hard_token_limit, oc.hard_token_limit) AS effective_hard_limit,
  COALESCE(c.soft_token_limit, oc.soft_token_limit) AS effective_soft_limit,
  CASE WHEN COALESCE(c.hard_token_limit, oc.hard_token_limit) IS NOT NULL
       THEN ROUND(COALESCE(u.used_tokens, 0)::numeric
                  / COALESCE(c.hard_token_limit, oc.hard_token_limit) * 100, 2)
       ELSE NULL END AS pct_hard,
  CASE WHEN COALESCE(c.soft_token_limit, oc.soft_token_limit) IS NOT NULL
       THEN ROUND(COALESCE(u.used_tokens, 0)::numeric
                  / COALESCE(c.soft_token_limit, oc.soft_token_limit) * 100, 2)
       ELSE NULL END AS pct_soft
FROM usage u
CROSS JOIN period p
LEFT JOIN caps c ON c.org_id = u.org_id AND c.user_id = u.user_id
LEFT JOIN ai_caps oc ON oc.org_id = u.org_id AND oc.user_id IS NULL AND oc.period = 'month';

CREATE VIEW ops.v1_ai_usage_org_progress WITH (security_invoker = true) AS
WITH period AS (
  SELECT date_trunc('month', now()) AS period_start
),
usage AS (
  SELECT l.org_id, SUM(l.prompt_tokens + l.completion_tokens) AS used_tokens,
         COUNT(DISTINCT l.user_id) AS active_users
  FROM ai_usage_ledger l, period p
  WHERE l.created_at >= p.period_start
  GROUP BY l.org_id
),
by_surface AS (
  SELECT l.org_id, l.surface, SUM(l.prompt_tokens + l.completion_tokens) AS surface_tokens
  FROM ai_usage_ledger l, period p
  WHERE l.created_at >= p.period_start
  GROUP BY l.org_id, l.surface
)
SELECT
  u.org_id,
  p.period_start AS period,
  COALESCE(u.used_tokens, 0) AS used_tokens,
  COALESCE(u.active_users, 0) AS active_users,
  oc.soft_token_limit AS org_soft_limit,
  oc.hard_token_limit AS org_hard_limit,
  CASE WHEN oc.hard_token_limit IS NOT NULL
       THEN ROUND(COALESCE(u.used_tokens, 0)::numeric / oc.hard_token_limit * 100, 2)
       ELSE NULL END AS pct_hard,
  CASE WHEN oc.soft_token_limit IS NOT NULL
       THEN ROUND(COALESCE(u.used_tokens, 0)::numeric / oc.soft_token_limit * 100, 2)
       ELSE NULL END AS pct_soft,
  COALESCE(
    (SELECT jsonb_object_agg(s.surface, s.surface_tokens)
     FROM by_surface s WHERE s.org_id = u.org_id),
    '{}'::jsonb
  ) AS tokens_by_surface
FROM usage u
CROSS JOIN period p
LEFT JOIN ai_caps oc ON oc.org_id = u.org_id AND oc.user_id IS NULL AND oc.period = 'month';

-- ---------------------------------------------------------------------------
-- Soft-cap notification kind (ADR 0008: soft caps warn + notify, riding
-- the existing notification surface). Extends the two kind CHECKs —
-- same evolution pattern as 0012/0038. Drop/re-add on a text CHECK is
-- metadata-only here (no data rewrite); the appended value keeps every
-- existing kind.
-- ---------------------------------------------------------------------------

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery','ai_cap_warn'));

ALTER TABLE notification_org_defaults DROP CONSTRAINT notification_org_defaults_kind_check;
ALTER TABLE notification_org_defaults ADD CONSTRAINT notification_org_defaults_kind_check
  CHECK (kind IN ('mention','assignment','sla_risk','sla_breach','escalation','missing_time','ticket_update','report_delivery','ai_cap_warn'));

GRANT SELECT ON ops.v1_ai_usage_user_progress TO meridian_app;
GRANT SELECT ON ops.v1_ai_usage_org_progress TO meridian_app;
