-- Secret category trees + generalized secret records + org-level vault with
-- category-level RBAC (Wave 16 lane 3, p16-vault-trees-org-rbac, ADR 0006 +
-- ADR 0004 template doctrine). Lane-exclusive migration pair 0078 (PG port
-- 54378 is the branch-local test port).
--
-- Purpose: the native vault generalizes beside `passwords` — the shipped
-- PasswordEntry surface stays untouched; this migration ADDS:
--
-- (1) `secret_folders` — client- AND org-scoped secret category trees.
--     Exactly one of org_id/client_id non-null (CHECK, per the wave brief
--     sketch): org-scope rows carry org_id, client-scope rows carry
--     client_id only. Unique (parent, slug) per scope via the
--     deterministic NULL-sentinel index: every NULL of a coordinate
--     contends through the same `uuid_nil()` bucket, so sibling slugs stay
--     unique and distinct NULL parents never collide. Seeded defaults
--     (passwords / api-keys / certificates / recovery-codes / documents)
--     are immutable seed rows (`is_default_seed`) planted per org; admin
--     customization FORKS the seed (a new row graph via `forked_from`),
--     never edit-in-place (ADR 0004 template doctrine); user-added folders
--     are ordinary rows. `restricted` is the deny-by-default category
--     flag: a restricted folder's contents are invisible to a role unless
--     that role holds a `secret_category_grants` row — the financial:read
--     gate shape ("hidden without the grant").
--
-- (2) `secret_records` — SecretRecord generalization beside PasswordEntry
--     (client/org nullable scope like folders; folder_id; kind
--     password/api-key/certificate/recovery-code/document; metadata
--     columns). Document-kind payload bytes never materialize here:
--     files ride object storage as ciphertext with pointer+hash rows
--     (`object_key` pointer, `file_sha256` hash, `file_bytes`), never
--     blobs in a text column (the wave brief's ciphertext-to-object-
--     storage rule; the ciphertext itself is a sealed secret_fields row
--     whose bytes the API layer writes to object storage before the
--     pointer row commits).
--
-- (3) `secret_fields` — every material field of a record, sealed
--     AES-256-GCM (core-api crypto::seal). New ciphertexts bind an AAD of
--     `(org_id, record_id, field_key)` so ciphertext bits cannot be
--     replayed across coordinates; rows forwarded by the deterministic
--     importer below retain the shipped passwords seal (no AAD) and open
--     through the legacy path. Plaintext is reachable ONLY through the
--     audited reveal endpoint (mirrors /passwords/{id}/reveal) —
--     ciphertext never appears in any metadata/list DTO.
--
-- (4) `secret_category_grants` — the role × folder ACL. One row grants
--     `can_read`/`can_edit` on ONE category; grants are DATA and flow
--     through the central policy module (routes check the matrix
--     permission `secrets:read` = view/delete and `secrets:edit` =
--     create/edit/reveal FIRST, then the category gate). Preset roles are
--     updated in this migration, AS DATA per ADR 0004, to hold the
--     secrets matrix alongside the Passwords matrix.
--
-- (5) Deterministic forward importer — existing `passwords` rows are
--     mapped into `secret_records` + `secret_fields` (one-time, inside
--     this migration's transaction, deterministic by coordinate; NEVER a
--     destructive cutover): the `passwords` table and its UI/responses
--     remain. Org-level entries land in the org's seeded `passwords`
--     folder; client entries land in a client-scoped fork of that
--     folder, created on first occurrence. The sealed secret/notes
--     ciphertexts are copied verbatim (legacy seal).
--
-- Upgrade-gate tags: additive-only (U9 gate — new tables + new CHECK arms
-- on document_links; no existing table loses a column or policy). The U6
-- preflight flags nothing (no views touched). Customization coverage: the
-- 5 seeded folders per org and every preset role row are
-- customizations-as-data — the org-manifest (U5) section 0079 carries
-- them verbatim.
--
-- Audit: every vault mutation (folders, records, fields, grants, reveal)
-- emits its domain event through the route layer into the EXISTING outbox
-- + append-only audit_events — no parallel audit store.
--
-- The unique constraint names below are consumed by the typed
-- reason-code registry (core-api error.rs map_unique_constraint):
--   `secret_folders_scope_key`      -> `folder_slug_taken`
--   `secret_records_scope_key`      -> `record_title_taken`
--   `secret_fields_record_key`      -> `field_key_taken`
--   `secret_grants_role_folder_key` -> `secret_grant_taken`

CREATE TABLE secret_folders (
  id uuid PRIMARY KEY,
  -- Exactly-one-of scope per the wave brief sketch.
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  parent_id uuid REFERENCES secret_folders(id) ON DELETE CASCADE,
  slug text NOT NULL,
  label text NOT NULL,
  -- Closed seeded-configuration vocabulary.
  kind text NOT NULL CHECK (kind IN
    ('passwords', 'api-keys', 'certificates', 'recovery-codes', 'documents')),
  -- Immutable shipped default (ADR 0004 template doctrine).
  is_default_seed boolean NOT NULL DEFAULT false,
  -- Deny-by-default secret category (financial:read gate shape).
  restricted boolean NOT NULL DEFAULT false,
  forked_from uuid REFERENCES secret_folders(id) ON DELETE SET NULL,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (num_nonnulls(org_id, client_id) = 1)
);

-- Exactly the brief's unique (parent, slug), scope-qualified; NULL
-- coordinates contend through the deterministic uuid_nil() bucket.
CREATE UNIQUE INDEX secret_folders_scope_key ON secret_folders (
  COALESCE(org_id, uuid '00000000-0000-0000-0000-000000000000'),
  COALESCE(client_id, uuid '00000000-0000-0000-0000-000000000000'),
  COALESCE(parent_id, uuid '00000000-0000-0000-0000-000000000000'),
  slug
);
CREATE INDEX idx_secret_folders_scope
  ON secret_folders (org_id, client_id, parent_id, created_at);

CREATE TYPE secret_record_kind AS ENUM
  ('password', 'api-key', 'certificate', 'recovery-code', 'document');

CREATE TABLE secret_records (
  id uuid PRIMARY KEY,
  -- Scope pair mirrors the owning folder (equality enforced in the
  -- write path).
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  -- Folder delete must be an explicit vault surgery (records move or are
  -- deleted first, each emitting its event separately), so the FK blocks
  -- cascade here (default NO ACTION).
  folder_id uuid NOT NULL REFERENCES secret_folders(id),
  kind secret_record_kind NOT NULL,
  title text NOT NULL,
  expires_at timestamptz,
  -- Ciphertext file pointer + hash (object storage), documents only.
  object_key text,
  file_sha256 text,
  file_bytes bigint,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (num_nonnulls(org_id, client_id) = 1)
);

-- One title per (scope, folder) — a vault record is referenced by its
-- title inside its category. The index name is consumed by the typed
-- reason-code registry: `record_title_taken`.
CREATE UNIQUE INDEX secret_records_scope_key ON secret_records (
  COALESCE(org_id, uuid '00000000-0000-0000-0000-000000000000'),
  COALESCE(client_id, uuid '00000000-0000-0000-0000-000000000000'),
  folder_id,
  title
);
CREATE INDEX idx_secret_records_scope
  ON secret_records (org_id, client_id, folder_id, updated_at DESC);
CREATE INDEX idx_secret_records_folder ON secret_records (folder_id, updated_at DESC);

CREATE TABLE secret_fields (
  id uuid PRIMARY KEY,
  record_id uuid NOT NULL REFERENCES secret_records(id) ON DELETE CASCADE,
  -- Scope columns mirror the owning record (self-contained RLS gate,
  -- equality enforced in the write path).
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  client_id uuid REFERENCES clients(id) ON DELETE CASCADE,
  field_key text NOT NULL,
  value_ciphertext bytea NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- The index name is consumed by the typed reason-code registry:
-- `field_key_taken`.
CREATE UNIQUE INDEX secret_fields_record_key
  ON secret_fields (record_id, field_key);
CREATE INDEX idx_secret_fields_scope ON secret_fields (org_id, client_id, record_id);

CREATE TABLE secret_category_grants (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  folder_id uuid NOT NULL REFERENCES secret_folders(id) ON DELETE CASCADE,
  can_read boolean NOT NULL DEFAULT false,
  can_edit boolean NOT NULL DEFAULT false,
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (can_read OR can_edit)
);

-- One grant row per (role, folder) — an upsert surface. The index name is
-- consumed by the typed reason-code registry: `secret_grant_taken`.
CREATE UNIQUE INDEX secret_grants_role_folder_key
  ON secret_category_grants (role_id, folder_id);
CREATE INDEX idx_secret_grants_folder
  ON secret_category_grants (org_id, folder_id);

-- ---------------------------------------------------------------------------
-- RLS backstop on every new table (org_id + current_setting guard). The
-- exactly-one-of scope means client-scope rows carry org_id NULL; their
-- isolation spine is the owning client's org.
-- ---------------------------------------------------------------------------
ALTER TABLE secret_folders ENABLE ROW LEVEL SECURITY;
CREATE POLICY secret_folders_org ON secret_folders
  USING (
    org_id = current_setting('app.org_id', true)::uuid
    OR (org_id IS NULL AND client_id IN (
      SELECT c.id FROM clients c
      WHERE c.org_id = current_setting('app.org_id', true)::uuid))
  );

ALTER TABLE secret_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY secret_records_org ON secret_records
  USING (
    org_id = current_setting('app.org_id', true)::uuid
    OR (org_id IS NULL AND client_id IN (
      SELECT c.id FROM clients c
      WHERE c.org_id = current_setting('app.org_id', true)::uuid))
  );

ALTER TABLE secret_fields ENABLE ROW LEVEL SECURITY;
CREATE POLICY secret_fields_org ON secret_fields
  USING (
    org_id = current_setting('app.org_id', true)::uuid
    OR (org_id IS NULL AND client_id IN (
      SELECT c.id FROM clients c
      WHERE c.org_id = current_setting('app.org_id', true)::uuid))
  );

ALTER TABLE secret_category_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY secret_category_grants_org ON secret_category_grants
  USING (org_id = current_setting('app.org_id', true)::uuid);

-- ---------------------------------------------------------------------------
-- Record links: secret records join the cross-entity RecordLink seam
-- (asset related-items pattern from Wave 13 lane 5). The shipped
-- document_links CHECK constraints widen additively (0054 pattern) so
-- `secret` is a first-class from/to kind with typed labels.
-- ---------------------------------------------------------------------------
ALTER TABLE document_links DROP CONSTRAINT document_links_from_kind_check;
ALTER TABLE document_links DROP CONSTRAINT document_links_to_kind_check;
ALTER TABLE document_links
  ADD CONSTRAINT document_links_from_kind_check
  CHECK (from_kind IN ('document', 'password', 'asset', 'secret'));
ALTER TABLE document_links
  ADD CONSTRAINT document_links_to_kind_check
  CHECK (to_kind IN ('document', 'password', 'client', 'ticket', 'kb_article', 'asset', 'secret'));

-- ---------------------------------------------------------------------------
-- Seeds: per-org default category tree (immutable rows, ADR 0004).
-- Idempotent per-org loop mirrors 0070/0071 doctrine.
-- ---------------------------------------------------------------------------
DO $seed$
DECLARE
  org record;
  k text;
BEGIN
  FOR org IN SELECT id FROM orgs LOOP
    FOREACH k IN ARRAY ARRAY['passwords', 'api-keys', 'certificates', 'recovery-codes', 'documents'] LOOP
      INSERT INTO secret_folders (id, org_id, slug, label, kind, is_default_seed)
      VALUES (gen_random_uuid(), org.id, k,
              CASE k
                WHEN 'passwords' THEN 'Passwords'
                WHEN 'api-keys' THEN 'API Keys'
                WHEN 'certificates' THEN 'Certificates'
                WHEN 'recovery-codes' THEN 'Recovery Codes'
                ELSE 'Documents'
              END,
              k, true);
    END LOOP;
  END LOOP;
END
$seed$;

-- ---------------------------------------------------------------------------
-- Deterministic forward importer: passwords -> secret_records (one-time,
-- inside this migration's transaction). Org-level entries -> the org's
-- seeded `passwords` folder; client entries -> a per-client forked
-- `passwords` folder created on first occurrence. The shipped
-- ciphertexts are copied verbatim (legacy seal).
-- ---------------------------------------------------------------------------
DO $importer$
DECLARE
  org record;
  p record;
  folder_id uuid;
  record_id uuid;
BEGIN
  FOR org IN SELECT id FROM orgs LOOP
    SELECT f.id INTO folder_id FROM secret_folders f
      WHERE f.org_id = org.id AND f.is_default_seed AND f.slug = 'passwords';

    FOR p IN SELECT * FROM passwords WHERE org_id = org.id AND client_id IS NULL LOOP
      record_id := gen_random_uuid();
      INSERT INTO secret_records (id, org_id, client_id, folder_id, kind, title,
                                  expires_at, created_by, updated_by, created_at, updated_at)
      VALUES (record_id, org.id, NULL, folder_id, 'password', p.title,
              p.expires_at, p.created_by, p.updated_by, p.created_at, p.updated_at);
      INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
      VALUES (gen_random_uuid(), record_id, org.id, NULL, 'secret', p.secret_ciphertext);
      IF p.notes_ciphertext IS NOT NULL THEN
        INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
        VALUES (gen_random_uuid(), record_id, org.id, NULL, 'notes', p.notes_ciphertext);
      END IF;
    END LOOP;
  END LOOP;

  FOR p IN SELECT * FROM passwords WHERE client_id IS NOT NULL
           ORDER BY client_id, created_at, title LOOP
    SELECT f.id INTO folder_id FROM secret_folders f
      WHERE f.client_id = p.client_id AND f.slug = 'passwords'
      LIMIT 1;
    IF NOT FOUND THEN
      SELECT f.id INTO folder_id FROM secret_folders f
        WHERE f.org_id = p.org_id AND f.is_default_seed AND f.slug = 'passwords'
        LIMIT 1;
      INSERT INTO secret_folders (id, client_id, slug, label, kind, is_default_seed,
                                  restricted, forked_from)
      VALUES (gen_random_uuid(), p.client_id, 'passwords', 'Passwords', 'passwords',
              false, false, folder_id)
      RETURNING id INTO folder_id;
    END IF;

    record_id := gen_random_uuid();
    INSERT INTO secret_records (id, org_id, client_id, folder_id, kind, title,
                                expires_at, created_by, updated_by, created_at, updated_at)
    VALUES (record_id, NULL, p.client_id, folder_id, 'password', p.title,
            p.expires_at, p.created_by, p.updated_by, p.created_at, p.updated_at);
    INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
    VALUES (gen_random_uuid(), record_id, NULL, p.client_id, 'secret', p.secret_ciphertext);
    IF p.notes_ciphertext IS NOT NULL THEN
      INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
      VALUES (gen_random_uuid(), record_id, NULL, p.client_id, 'notes', p.notes_ciphertext);
    END IF;
  END LOOP;
END
$importer$;

-- ---------------------------------------------------------------------------
-- Preset roles updated AS DATA (ADR 0004): append the secrets matrix
-- grants to every shipped preset role row carrying the Passwords matrix,
-- with the identical action shape (Owner/Admin full matrix; Technician /
-- Senior Tech view+create+edit — working material, exactly like their
-- Passwords row; Dispatcher / Bookkeeper / Auditor / ReadOnly view only).
-- Custom roles are NOT touched — admins grant secrets:* explicitly
-- (additive widening through the stored jsonb arrays).
-- ---------------------------------------------------------------------------
DO $presets$
DECLARE
  org record;
  grant_json jsonb;
  action text;
  role_builtin text;
BEGIN
  FOR org IN SELECT id FROM orgs LOOP
    FOREACH role_builtin IN ARRAY
      ARRAY['owner', 'admin', 'dispatcher', 'senior_tech', 'technician',
            'bookkeeper', 'auditor', 'read_only']
    LOOP
      FOREACH action IN ARRAY
        CASE role_builtin
          WHEN 'owner' THEN ARRAY['view', 'create', 'edit', 'delete', 'export']
          WHEN 'admin' THEN ARRAY['view', 'create', 'edit', 'delete', 'export']
          WHEN 'technician' THEN ARRAY['view', 'create', 'edit']
          WHEN 'senior_tech' THEN ARRAY['view', 'create', 'edit']
          ELSE ARRAY['view']
        END
      LOOP
        grant_json := jsonb_build_object('matrix', jsonb_build_object(
          'module', 'secrets', 'action', action));
        UPDATE roles r
          SET permissions = r.permissions || grant_json
          WHERE r.org_id = org.id
            AND r.builtin IS NOT DISTINCT FROM role_builtin::text
            AND NOT (r.permissions @> grant_json);
      END LOOP;
    END LOOP;
  END LOOP;
END
$presets$;
