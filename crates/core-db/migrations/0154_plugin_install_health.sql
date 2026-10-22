-- Plugin install health: the failure counter behind quarantine (Wave 53 lane 3,
-- `p53-sandbox-isolation`; ADR 0023 Decisions 2 and 4).
--
-- Why this table exists: a plugin invocation runs in a supervised worker
-- process, and a worker that dies mid-call leaves the parent with
-- `outcome = "worker_failed"` and a restart. One death is a flake and the pool
-- absorbs it. A run of them from the same install is that install's artifact
-- taking its worker down every time, and the pool must stop feeding it a fresh
-- worker on every request. The decision — quarantine after three consecutive
-- worker failures — is the pool's (`runtime::QUARANTINE_AFTER_FAILURES`); this
-- table is the state it needs, and it is in the DATABASE rather than in the
-- process for one reason: an in-memory counter is reset by a `SIGTERM`, which
-- makes quarantine an attacker's free option.
--
-- What it deliberately does NOT carry: a status, a grant, an enable flag, a
-- revoked reason, a lifecycle stage. Quarantine is not disablement — clearing it
-- touches no grant and no manifest — so it cannot be a column on
-- `plugin_installs` without either overloading `plugin_install_status` or
-- writing a status that means two things. W53-4 owns revoke; if revoke needs
-- columns they belong on `plugin_installs` in a later migration, not here.
--
-- The counter and the quarantine share one row on purpose. They are two views
-- of one fact (this install's recent worker deaths) read by one query on the
-- invocation path, and a second table would double that read for no operator
-- benefit.
--
-- RLS doctrine: org-scoped user data, org-bound reads (assert_rls_coverage).

CREATE TABLE plugin_install_health (
  -- One row per install. The install is the unit quarantine applies to, and
  -- `ON DELETE CASCADE` means an uninstall takes its quarantine with it.
  install_id uuid PRIMARY KEY REFERENCES plugin_installs(id) ON DELETE CASCADE,
  -- Carried for the RLS policy. The writer derives it from `plugin_installs`
  -- in the same statement rather than trusting a caller-supplied value.
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- Consecutive worker failures. Any successful invocation resets it to zero,
  -- so a plugin that fails once an hour is never quarantined.
  consecutive_failures integer NOT NULL DEFAULT 0,
  quarantined_at timestamptz,
  -- A machine-readable reason, not prose: the static refusal string the audit
  -- row already carries (`plugin_worker_failed`).
  quarantine_reason text,
  cleared_at timestamptz,
  cleared_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT plugin_install_health_failures_nonnegative
    CHECK (consecutive_failures >= 0),
  -- Quarantine carries its reason; nothing else may.
  CONSTRAINT plugin_install_health_quarantine_reason
    CHECK (quarantined_at IS NULL OR quarantine_reason IS NOT NULL)
);

CREATE INDEX plugin_install_health_org_idx
  ON plugin_install_health (org_id);
-- The operator surface: "what is quarantined right now". Partial, because the
-- common case is an empty result and most rows are not quarantined.
CREATE INDEX plugin_install_health_quarantined_idx
  ON plugin_install_health (org_id, quarantined_at)
  WHERE quarantined_at IS NOT NULL;

ALTER TABLE plugin_install_health ENABLE ROW LEVEL SECURITY;
CREATE POLICY plugin_install_health_org ON plugin_install_health
  USING (org_id = current_setting('app.org_id', true)::uuid);

COMMENT ON TABLE plugin_install_health IS
  'Consecutive plugin worker failures per install and the quarantine they '
  'trigger. Written by the plugin pool on a failure path only; cleared by an '
  'operator, never by the clock (an expiry would make quarantine free to reset '
  'by waiting).';
