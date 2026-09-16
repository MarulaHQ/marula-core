# ADR 0009 — Plugin & skill distribution: sources, staged installs, and manifest-rendered settings

**Status:** Accepted (user directive + design session) · 2026-09-16
**Scope:** `crates/plugin-host` install lifecycle, `plugins/` + `packages/plugin-kit`,
agent skills (`AgentSkill`), marketplace/registry surface, settings IA (Plugins + Agent
Skills sections)

## Context

The Wave 14 plugin runtime v2 landed the SEAM: declarative manifests only
(no eval-style loading), catalog-entry pinning, deny-by-default admin
enable/disable/revoke/uninstall, audit events, egress-allowlist posture, the
`PluginInstallation` store (migration 0076). What does not exist yet is
*distribution*: no way to browse a public marketplace, import a zip, pull from a
GitHub repo, update an install, or configure one — and no story at all for agent
skills sourced from community marketplaces/repositories. Both are now first-scope
(drop into **Wave 16** so the UX pass and AI waves build on a complete management
surface).

## Decision

1. **Two artifacts, two governance profiles, one management surface.** Plugins
   (WASM executables + HTTP connectors) are high-blast-radius: capability consent,
   digest verification, egress allowlist, per-install settings. Agent skills
   (declarative prompts/rules manifests) are data: versioned, rollback-able,
   low-blast-radius — but content is treated as UNTRUSTED exactly like retrieved
   RAG content (`docs/10-agent-platform.md` §4): never executed as instructions,
   always quoted/scoped into the rules layers. Settings IA gains **Plugins** and
   **Agent Skills** sections; they share the install/provenance machinery, never
   the lifecycle semantics.
2. **Sources are adapters behind one distribution seam.** v1 sources:
   - **Public marketplace registry** (the BUILD-PROMPT v1 format: static JSON
     manifest + git tag, code-signed WASM artifacts; committed catalog fixtures are
     the fixture shape);
   - **GitHub repo pull** — tag/release ONLY (never HEAD): install records pin
     (owner/repo/tag/commit/digest) the same way the marketplace entry does; repos
     without pinned tags are refused;
   - **Local zip import** — sandboxed validation before anything registers:
     manifest schema check, artifact digests, size/entry caps, capability list
     diffed into the same consent screen.
   - **Skills from community marketplaces/repo APIs (free and open sources only)**
     and manual skill uploads.
   Every source records the SAME provenance chain: `{source_kind, source_url,
   fetched_ref, artifact_digest, uploader}` — append-only, shown on the install
   record, feeding the audit trail.
3. **User uploads go through a staged pipeline.** A member import (skill or
   plugin) lands **PENDING/staged**: validated, never executed, invisible except to
   its uploader and admins; an owner/admin reviews the consent diff and enables it
   (the deny-by-default lifecycle already shipped — staging is the row *before*
   install, not a bypass of it).
4. **Settings render from manifests.** A plugin declares its settings as
   JSON-Schema in the manifest; the Plugins section renders per-install config FROM
   that schema (same typed-registry doctrine as KB embeds / widget props). No
   hand-authored per-plugin settings pages; a plugin with no declared schema gets
   consent/enable/disable only.
5. **Lifecycle completeness (binding):** install (from any source) → consent diff
   (capability scopes (+ settings schema shown)) → enable/disable → **update** (new
   version = re-consent diff; old version pinned until consented) → disable/enable
   → **uninstall is terminal** (system-of-record data the plugin wrote lives in
   primary data owned by the org, never deleted with the plugin; link rows survive
   like the vault/document semantics).
6. **WASM and HTTP connectors ship in one lifecycle track** (distinct tabs —
   connectors declare webhook endpoints + scopes, WASM declares capabilities +
   resource profile) so no source (zip/registry/GitHub) creates a second governance
   regime. Skills reuse only the skills half: import/edit/enable/version/rollback —
   no executable lifecycle.

## Consequences

- **Wave 16 gains lane 7 `p16-plugin-skill-mgmt` (migration 0079 / port 54379)**
  and lane 8 `p16-plugin-skills-closeout` (UI; no migration) — renumbering
  knock-ons: Wave 17 workbar pair → **0080/54380**; Wave 18 AI pairs
  (`p18-ai-policy-ledger` / `p18-agent-chat-runner`) → **0081 / 0082** (ADR 0008
  consequence note updated to match).
- Lane 7 builds on the Wave 14 seam — it EXTENDS the lifecycle (sources, update
  path, settings schema, staged state), it does not fork it; consent screens stay
  one component shared by all sources.
- Skills imported from public marketplaces land staged even when the source is
  "public registry" reviewed by admins; the per-user help is browsing, not bypass.
- Supply-chain invariants: deny-by-default, admin-consent, digest-pinned,
  egress-allowlisted, audit-logged — unchanged from the runtime v2 seam; the
  keyboard-shortcut version of this decision is "nothing executes that did not
  arrive digest-verified with provenance AND pass admin consent".
- Got no lane prior to this ADR: confirmed — Wave 14 shipped the authorization seam
  only; no Wave 16–18 lane covered discovery/import/settings before this one.
