-- Meridian core: orgs, users, roles, sessions, API keys, audit.
-- RLS doctrine: every user-data table carries org_id + an isolation policy.
-- The application connects as `meridian_app` (non-owner) and sets
-- `app.org_id` per transaction; unset org_id sees zero rows (default-deny).

CREATE EXTENSION IF NOT EXISTS pgcrypto;

DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'meridian_app') THEN
    CREATE ROLE meridian_app LOGIN PASSWORD 'meridian_app';
  END IF;
END
$$;

CREATE TABLE orgs (
  id uuid PRIMARY KEY,
  name text NOT NULL,
  slug text NOT NULL UNIQUE,
  ticket_prefix text NOT NULL DEFAULT 'MER',
  ticket_sequence bigint NOT NULL DEFAULT 1000,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  email text NOT NULL,
  display_name text NOT NULL,
  password_hash text NOT NULL,
  timezone text NOT NULL DEFAULT 'UTC',
  is_active boolean NOT NULL DEFAULT true,
  mfa_totp_secret text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, email)
);
CREATE INDEX idx_users_email ON users (lower(email));

CREATE TABLE roles (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  builtin text,
  permissions jsonb NOT NULL DEFAULT '[]',
  is_system boolean NOT NULL DEFAULT false,
  UNIQUE (org_id, name)
);

CREATE TABLE user_roles (
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, role_id)
);

CREATE TABLE auth_sessions (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  refresh_token_hash text NOT NULL,
  user_agent text,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz
);
CREATE INDEX idx_auth_sessions_user ON auth_sessions (user_id);

CREATE TABLE api_keys (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  key_hash text NOT NULL UNIQUE,
  prefix text NOT NULL,
  scopes jsonb NOT NULL DEFAULT '[]',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  last_used_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE audit_events (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  actor_kind text NOT NULL CHECK (actor_kind IN ('human','agent','plugin','connector','system')),
  actor_id text,
  action text NOT NULL,
  entity_type text,
  entity_id text,
  detail jsonb NOT NULL DEFAULT '{}',
  ip text,
  occurred_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_org_time ON audit_events (org_id, occurred_at DESC);

-- ---------------------------------------------------------------------------
-- Row level security
-- ---------------------------------------------------------------------------
ALTER TABLE orgs ENABLE ROW LEVEL SECURITY;
CREATE POLICY org_self ON orgs
  USING (id = current_setting('app.org_id', true)::uuid);

ALTER TABLE users ENABLE ROW LEVEL SECURITY;
CREATE POLICY users_org ON users
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE roles ENABLE ROW LEVEL SECURITY;
CREATE POLICY roles_org ON roles
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE user_roles ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_roles_org ON user_roles
  USING (EXISTS (SELECT 1 FROM users u WHERE u.id = user_roles.user_id
                 AND u.org_id = current_setting('app.org_id', true)::uuid));

ALTER TABLE auth_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY auth_sessions_org ON auth_sessions
  USING (EXISTS (SELECT 1 FROM users u WHERE u.id = auth_sessions.user_id
                 AND u.org_id = current_setting('app.org_id', true)::uuid));

ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
CREATE POLICY api_keys_org ON api_keys
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY audit_events_org ON audit_events
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- App-role privileges (RLS still constrains rows)
-- ---------------------------------------------------------------------------
GRANT USAGE ON SCHEMA public TO meridian_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO meridian_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO meridian_app;
