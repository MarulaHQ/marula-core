# ADR 0009 — Plugin & skill distribution: sources, staged installs, and manifest-rendered settings

**Status:** Accepted (user directive + design session) · 2026-09-16
**Scope:** `crates/plugin-host` install lifecycle, `plugins/` + `packages/plugin-kit`,
agent skills (`AgentSkill`), marketplace/registry surface, settings IA (Plugins + Agent
Skills sections)

## Context

The Wave 14 plugin runtime v2 landed the SEAM: declarative manifests only
(no dynamic eval loading), catalog-entry pinning, default-deny admin
enable/disable/revoke/uninstall, audit events, an egress allowlist posture, and the
`PluginInstallation` store (migration 0076). What does not exist yet is
*distribution*: no way to browse a public marketplace, import a zip, pull from a
GitHub repo, update an install, or configure one. There is also no story yet for agent
skills sourced from community marketplaces or repositories. Both are now in first scope
(dropped into **Wave 16** so the UX pass and AI waves build on a complete management
surface).

## Decision

1. **Two artifacts, two governance profiles, one management surface.** Plugins
   (WASM executables + HTTP connectors) carry a large blast radius: capability consent,
   digest verification, an egress allowlist, and per-install settings. Agent skills
   (declarative prompts/rules manifests) are data: versioned, reversible via rollback,
   and low-blast-radius. Skill content is still treated as UNTRUSTED just like retrieved
   RAG content (`docs/10-agent-platform.md` §4): never executed as instructions,
   and always quoted and scoped into the rules layers. Settings IA gains **Plugins** and
   **Agent Skills** sections. They share the install and provenance machinery, never
   the lifecycle semantics.
2. **Sources are adapters behind one distribution seam.** v1 sources:
   - **Public marketplace registry** (the BUILD-PROMPT v1 format: static JSON
     manifest + git tag, code-signed WASM artifacts. Committed catalog fixtures are
     the fixture shape.)
   - **GitHub repo pull** — tag/release ONLY (never HEAD). Install records pin
     `(owner/repo/tag/commit/digest)` the same way the marketplace entry does, and repos
     without pinned tags are refused.
   - **Local zip import** — sandboxed validation before anything registers:
     manifest schema check, artifact digests, size/entry caps, and capability list
     diffed into the same consent screen.
   - **Skills from community marketplaces/repo APIs (free and open sources only)**
     and manual skill uploads.
   Every source records the SAME provenance chain: `{source_kind, source_url,
   fetched_ref, artifact_digest, uploader}` — append-only, shown on the install
   record, feeding the audit trail.
3. **User uploads go through a staged pipeline.** A member import (skill or
   plugin) lands **PENDING/staged**: validated, never executed, and invisible except to
   its uploader and admins. An owner or admin reviews the consent diff and enables it
   (the default-deny lifecycle already shipped — staging is the row *before*
   install, not a bypass of it).
4. **Settings render from manifests.** A plugin declares its settings as
   JSON Schema in the manifest. The Plugins section renders per-install config FROM
   that schema (same typed-registry doctrine as KB embeds / widget props). There are no
   hand-authored per-plugin settings pages, and a plugin with no declared schema gets
   consent/enable/disable only.
5. **Lifecycle completeness (binding):** install (from any source) → consent diff
   (capability scopes (+ settings schema shown)) → enable/disable → **update** (new
   version = re-consent diff, with the old version pinned until consented) → disable/enable
   → **uninstall is terminal** (system-of-record data the plugin wrote lives in
   primary data owned by the org and is never deleted with the plugin, while link rows survive
   like the vault/document semantics).
6. **WASM and HTTP connectors ship in one lifecycle track** (distinct tabs —
   connectors declare webhook endpoints + scopes, WASM declares capabilities +
   resource profile) so no source (zip/registry/GitHub) creates a second governance
   regime. Skills reuse only the skills half: import/edit/enable/version/rollback —
   no executable lifecycle.

## Consequences

- **Wave 16 gains lane 7 `p16-plugin-skill-mgmt` (migration 0079 / port 54379)**
  and lane 8 `p16-plugin-skills-closeout` (UI, no migration). Renumbering
  knock-ons: Wave 17 workbar pair → **0080/54380**, and Wave 18 AI pairs
  (`p18-ai-policy-ledger` / `p18-agent-chat-runner`) → **0081 / 0082** (ADR 0008
  consequence note updated to match).
- Lane 7 builds on the Wave 14 seam — it EXTENDS the lifecycle (sources, update
  path, settings schema, staged state), it does not fork it. Consent screens stay
  one component shared by all sources.
- Skills imported from public marketplaces land staged even when the source is
  "public registry" reviewed by admins. The per-user help is browsing, not bypass.
- Supply-chain invariants: default-deny posture, admin consent, pinned digests,
  egress allowlists, and audit logs remain unchanged from the runtime v2 seam. In short,
  nothing executes that did not arrive digest-verified with provenance AND pass admin consent.
- Got no lane prior to this ADR: confirmed. Wave 14 shipped the authorization seam
  only, and no Wave 16–18 lane covered discovery/import/settings before this one.

## Amendment — Wave 41 lane 2b: theme packs are restyle-only, and density is not a pack's business

**Added 2026-09-30 by `p41-marula-theme-2b`.** This ADR's V1 decision covers
staged imports, consent and the zero-capability rule for plugins and skills.
Wave 41 turned the same distribution machinery onto **theme packs**
(`artifact_kind: "theme"`), and two facts about that path are worth writing
down here rather than leaving to the theme doc.

**1. A theme pack restyles surfaces and nothing else.** The pack manifest may
move only the neutral, chrome and accent-tint axes. It may not restyle the
primary or its ink-to-gold mode flip, `destructive`, the status pairs, or the
assistant inversion. The mechanism is that the fixed roles are ABSENT from
`ROLE_TO_TOKEN`, so a preset naming one is dropped rather than repainted, and
`assertPresetIsLegal` rejects it at the pack boundary. This is the same
zero-capability posture as the plugin path, applied to colour: a pack is a
declaration of what a theme looks like, and the moment it can move semantics
it is a different kind of artifact with a different risk profile.

**2. Density is a user preference, not a theme-pack concern.** A pack that
shipped a density would be shipping a layout to one user and not the next,
which is the part of appearance that is never a brand decision. Density is the
USER-scope key `appearance.density` and a `data-density` attribute, outside the
preset vocabulary by construction. The same reasoning puts the per-surface
display state (named view, visible columns, filters, sort) in
`appearance.data_surfaces` rather than in a pack: those are one person's working
set, and a pack has no business carrying them between people.

**3. The restyle-only boundary is also an accessibility boundary, and the gate
enforces it per pack.** `scripts/check-theme-packs.mjs` runs the retired-hex
blacklist, the AA and 3:1 floors, and the fixed-axis diff over every shipped
built-in preset in both modes, and scores any installed pack manifest passed as
a CLI arg or through `THEME_PACK_MANIFESTS` in CI. An off-schema token in any
theme is mechanically rejected rather than applied.

**Not amended:** the sources, staging, consent, update and uninstall rules above
are unchanged. A theme pack travels the same road as a plugin import and is held
to the same staged-until-consented posture.
