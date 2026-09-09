## UI skills (vendored design-engineering skills)

`/skills` on ui-skills.com, vendored to `.kilo/skills/` (ui-skills-root, improve-ui,
web-design-guidelines, better-ui, interface-design, fixing-accessibility, rams).
When doing UI work (building pages, components, polish, audits), consult these:
- Routing: start from `.kilo/skills/ui-skills-root/SKILL.md`.
- Audits/reviews: improve-ui (audit → self-contained plan for another agent),
  web-design-guidelines, fixing-accessibility.
- Polish/craft: better-ui, interface-design (dashboards/admin/SaaS), rams.

**Override rules (always win over skill guidance):** `docs/theme.md` + Workbench
tokens (no off-palette colors, Ember interactive-only ≤5%) · Inter/JetBrains Mono
typography · motion law (≤120ms, transform/opacity, prefers-reduced-motion) ·
instant-first envelope patterns in `docs/instant-first.md`. Skills advise on
craft; they never change palette, typography, stack, or feature decisions.
Skill output that violates the token set or motion law is rejected, not applied.
Enforcement stays mechanical: CI hex-scan, typecheck/tests, the p11-ui-quality
lane's screenshot review.

## Backend skills (vendored)

`.kilo/skills/` also carries stack skills: rust-best-practices (Apollo handbook),
postgres-best-practices (Supabase; includes RLS performance + connection pooling
references), stripe-best-practices (for the future PaymentProvider adapter),
insecure-defaults (Trail of Bits; audit command + pattern catalogs for the
Phase-4 security pass). Backend lanes: load rust-best-practices for all Rust
work, postgres-best-practices when writing migrations/queries/RLS policies.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
