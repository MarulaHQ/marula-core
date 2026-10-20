-- KB rich content (Wave 13 lane 4 `p13-kb-rich`; the private build tracker KB1 + KB2,
-- ADR 0004). Three adds, all org-scoped and RLS-gated:
--
--   * kb_embed_providers — the oEmbed allowlist registry. An embed URL is
--     only sanctioned when it matches a REGISTERED provider row; raw
--     arbitrary iframes are never accepted (ADR 0004: registered
--     components decide what may render). Rows with `org_id IS NULL` are
--     the platform-shipped standard set (YouTube, Loom, Vimeo — is_system,
--     immutable from the API); org rows are the admin-managed extension
--     (registry-extensible). Each provider pins host_patterns (text[],
--     matched against the URL host exactly) + a url_regex over the
--     resource URL (Rust regex syntax, validated at write time) + the
--     oEmbed endpoint template carrying {url}. The embed HTML is always
--     fetched server-side from the endpoint over HTTPS at render time —
--     user-supplied HTML never reaches the page.
--
--   * kb_embeds — per-article embed instances resolved against the
--     registry at write time (provider_id snapshot; SET NULL when a
--     provider row is later removed so the embed degrades to "blocked",
--     never to "unvalidated"). The embed inherits the article's
--     client-facing gate exactly as body text does (0031 flags; no
--     separate visibility flag on embeds).
--
--   * kb_external_documents — the linked-external-document type: a
--     SharePoint/OneDrive/Drive (or other) URL stored as a first-class
--     resource with title, permission note, and last-verified link-check
--     columns (ADR 0004: linked, not trapped). A nullable article_id
--     attaches it to one article or leaves it standalone in the KB tree;
--     client-facing exposure rides the linked article's client-visible
--     gate — a standalone row is always staff-only. url_kind is a closed
--     vocabulary for the classifier in core-kb; the link-check is recorded
--     as data (last_verified_at + last_verified_status), never a side
--     effect of reads.

CREATE TABLE kb_embed_providers (
  id uuid PRIMARY KEY,
  -- NULL = platform-shipped system row (is_system true); org rows are the
  -- admin-managed extensions.
  org_id uuid REFERENCES orgs(id) ON DELETE CASCADE,
  name text NOT NULL,
  slug text NOT NULL,
  host_patterns text[] NOT NULL,
  url_regex text NOT NULL,
  oembed_endpoint text NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  is_system boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE NULLS NOT DISTINCT (org_id, slug)
);

CREATE INDEX idx_kb_embed_providers_org ON kb_embed_providers (org_id);

-- System seeds (ADR 0004: YouTube/Loom/Vimeo-class). Fixed ids so the
-- seed is idempotent under the migration checksum and stable for tests.
INSERT INTO kb_embed_providers (id, org_id, name, slug, host_patterns, url_regex, oembed_endpoint, enabled, is_system) VALUES
  ('69000000-0000-0000-0000-000000000001', NULL, 'YouTube', 'youtube',
   ARRAY['youtube.com','www.youtube.com','m.youtube.com','youtu.be','www.youtu.be'],
   '^(https://)?(www\.)?(youtube\.com/(watch\?v=[A-Za-z0-9_-]+|shorts/[A-Za-z0-9_-]+|embed/[A-Za-z0-9_-]+)|youtu\.be/[A-Za-z0-9_-]+)([?&].*)?$',
   'https://www.youtube.com/oembed?url={url}&format=json', true, true),
  ('69000000-0000-0000-0000-000000000002', NULL, 'Loom', 'loom',
   ARRAY['loom.com','www.loom.com'],
   '^(https://)?(www\.)?loom\.com/share/[A-Za-z0-9]+([?&].*)?$',
   'https://www.loom.com/v1/oembed?url={url}&format=json',
   true, true),
  ('69000000-0000-0000-0000-000000000003', NULL, 'Vimeo', 'vimeo',
   ARRAY['vimeo.com','www.vimeo.com','player.vimeo.com'],
   '^(https://)?(www\.)?vimeo\.com/[0-9]+([?&].*)?$',
   'https://vimeo.com/api/oembed.json?url={url}',
   true, true);

CREATE TABLE kb_embeds (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  article_id uuid NOT NULL REFERENCES kb_articles(id) ON DELETE CASCADE,
  url text NOT NULL,
  -- Registry row resolved at write time; SET NULL on provider deletion
  -- (org rows are deletable) so the embed's sanction source can vanish —
  -- resolution re-runs at render, a NULL provider renders nothing.
  provider_id uuid REFERENCES kb_embed_providers(id) ON DELETE SET NULL,
  provider_slug text,
  -- Title from the provider's oEmbed response (may be empty at write
  -- time; the resolver refreshes it server-side).
  title text NOT NULL DEFAULT '',
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (org_id, article_id, url)
);

CREATE INDEX idx_kb_embeds_org_article ON kb_embeds (org_id, article_id);

CREATE TABLE kb_external_documents (
  id uuid PRIMARY KEY,
  org_id uuid NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
  -- NULL = standalone KB resource (staff-only); when attached the row
  -- inherits the article's client-facing gate for portal reads.
  article_id uuid REFERENCES kb_articles(id) ON DELETE SET NULL,
  title text NOT NULL,
  url text NOT NULL,
  url_kind text NOT NULL CHECK (url_kind IN ('sharepoint','onedrive','google_drive','other')),
  -- Who may open the link and what they need (e.g. "M365 group ACME-IT").
  -- Documentation of access, never enforcement — the target system owns
  -- the permission.
  permission_note text NOT NULL DEFAULT '',
  last_verified_at timestamptz,
  last_verified_status text CHECK (last_verified_status IN ('ok','unreachable')),
  created_by uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_kb_external_documents_org ON kb_external_documents (org_id, article_id);
CREATE INDEX idx_kb_external_documents_org_kind ON kb_external_documents (org_id, url_kind);

ALTER TABLE kb_embed_providers ENABLE ROW LEVEL SECURITY;
CREATE POLICY kb_embed_providers_org ON kb_embed_providers
  USING (org_id IS NULL OR org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE kb_embeds ENABLE ROW LEVEL SECURITY;
CREATE POLICY kb_embeds_org ON kb_embeds
  USING (org_id = current_setting('app.org_id', true)::uuid);

ALTER TABLE kb_external_documents ENABLE ROW LEVEL SECURITY;
CREATE POLICY kb_external_documents_org ON kb_external_documents
  USING (org_id = current_setting('app.org_id', true)::uuid);