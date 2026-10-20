-- Per-org custom domain + generic DNS verification (the private build tracker Wave 47 lane A8,
-- `p47-org-custom-domains`).
--
-- NUMBERING: this is 0135, the next free slot at lane start (main's head was
-- 0134_rag_vector_namespace.sql). The board claim is a reservation at claim
-- time, not a guarantee at merge time: any lane claiming a migration number
-- must re-check it against main's head immediately before merge, and the
-- loser renumbers (see the 0131 header for the precedent).
--
-- (A) THREE nullable columns on `orgs`, and nothing else:
--   * `custom_domain` — the tenant's hostname, stored NORMALIZED (lowercased,
--     trailing dot stripped, validated by `adapters_core::dns::normalize_domain`
--     before any write). NULL means the org has no custom domain.
--   * `custom_domain_verified_at` — set only when Core has resolved the
--     `_marula-verify.<domain>` TXT record through the system resolver and
--     matched the issued token. NULL means unverified (or, after a domain
--     change, verification was reset). The TXT token is the sole ownership
--     authority; the A/CNAME check only has to look PLAUSIBLE, because an
--     edge-proxied record (Cloudflare orange cloud, any CDN) resolves to the
--     edge's IPs rather than the origin's.
--   * `custom_domain_verification_token` — the random challenge Core issues
--     when a domain is set, echoed back in a TXT record at
--     `_marula-verify.<domain>`. Regenerated on every domain change; cleared
--     with the domain.
--
-- (H) Core stores the verified domain and exposes the verification workflow
-- ONLY. It does NOT terminate TLS or route traffic — the operator's own edge
-- (Caddy/Traefik, per the V2 TLS posture) reads `custom_domain` back through
-- the API and provisions certs/routes itself.
--
-- (D) No DNS-provider table and no provider enum: the DNS-record-management
-- seam is a plugin contract (`adapters_core::dns::DnsRecordProvider`), and no
-- Core endpoint ever requires a registered provider. The manual paste path
-- (`NoneDnsRecordProvider`) is always available, so no schema records a
-- provider choice.
--
-- RLS: the three columns live on `orgs`, whose `org_self` policy (0001) already
-- scopes every SELECT/UPDATE to `app.org_id` (omitted WITH CHECK falls back to
-- USING, so it constrains the new row too). No new policy is needed — the
-- ADMIN gate on the mutating endpoints is the API layer's job (house
-- convention), applied on top of the row-level scope.

ALTER TABLE orgs ADD COLUMN custom_domain text;
COMMENT ON COLUMN orgs.custom_domain IS
  'Tenant custom domain, normalized (lowercased). NULL = none. Unique across orgs (case-insensitive, partial on NOT NULL).';

ALTER TABLE orgs ADD COLUMN custom_domain_verified_at timestamptz;
COMMENT ON COLUMN orgs.custom_domain_verified_at IS
  'When Core last confirmed the _marula-verify.<domain> TXT token through the system resolver. NULL = unverified (or reset by a domain change).';

ALTER TABLE orgs ADD COLUMN custom_domain_verification_token text;
COMMENT ON COLUMN orgs.custom_domain_verification_token IS
  'Random TXT challenge issued with the current domain; compared against the resolved _marula-verify.<domain> record. Regenerated on domain change.';

-- Case-insensitive uniqueness, partial so any number of orgs may carry NO
-- domain (a plain UNIQUE would already allow multiple NULLs, but `lower()`
-- also defends against a caller that normalizes imperfectly — the write path
-- lowercases, this enforces it regardless). A collision raises SQLSTATE 23505
-- on index `orgs_custom_domain_key`, mapped to a typed API conflict.
CREATE UNIQUE INDEX orgs_custom_domain_key
  ON orgs (lower(custom_domain))
  WHERE custom_domain IS NOT NULL;
