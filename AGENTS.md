## Repo lookups must go through graft first (BINDING, top-of-file)

Before ANY Grep/Glob/Read search of this codebase: run graft first —
`graft ask "<q>" --source` / `graft grep "<literal>"` / `graft callers <sym>` /
`graft skeleton <file>` (CLI, always available) or the `graft_*` MCP tools.
If graft MCP tools appear deferred (names listed, schemas withheld), load ALL
of them in ONE ToolSearch lookup (`select:mcp__graft__graft_find_code,mcp__graft__graft_find_all,mcp__graft__graft_trace_calls,mcp__graft__graft_file_api,mcp__graft__graft_repo_map`)
before your first lookup — grep-first is WRONG by default: a graft hit carries
exact file:line spans for ~3% of the tokens reading or grepping the same code
costs. Use plain grep only AFTER a graft attempt missed the file (unindexed
paths: fixtures/, .sqlx/, dist/, evidence dirs). Full routing detail lives in
"graft vs graphify" below. Record graft token savings in the lane landing
record ("🌱 graft saved ~N tokens").

**Worktree lanes: build your own graph at lane start (2026-09-18).** Every
worktree lane runs `graft build` ONCE at lane start in its worktree (deterministic
AST-only, free) so its indexes reflect the lane's branch, then re-runs it after
big changes — instead of sharing the main repo's graph via symlink (the old
setup-script symlink is retired; stale shared spans caused wrong file:line hits
after merges). `graft/` is GITIGNORED (`.gitignore /graft/`) — never commit,
symlink, or force-add it; if a lane ever sees `graft/` in `git status`, fail the
check and re-ignore rather than committing the graph.

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

## Design research — lazyweb MCP (binding for every frontend lane)

Every frontend lane (any lane touching pages, views, flows, or UI polish) MUST
run design research through the lazyweb MCP BEFORE writing UI, so surfaces are
designed for ultimate UX and looks, not invented from scratch:

- `/lazyweb-search-screens` for the surface's UI pattern (real product
  references ranked by visual search), `/lazyweb-search-flows` for multi-step
  journeys (onboarding, checkout, paywalls, sign-up), and
  `/lazyweb-search-experiments` when a conversion decision is involved.
- **Aesthetic reference (2026-09-16, user directive):**
  `design/mockups/meridian-ticket-workbench-html-mockup-v2.html` (+ its
  `-v2-light.png` / `-v2-dark.png`) is the canonical look-and-feel reference —
  use it appropriately alongside lazyweb research (it sets the bar for layout,
  hierarchy, density, and both modes; lazyweb supplies pattern breadth).
- **Motion:** subtle, tasteful animation is encouraged — entrance/transition
  motion, staggered reveals, hover micro-feedback. Guardrails: transform/opacity
  only, ≤120ms, `prefers-reduced-motion` honored (`docs/theme.md` §5).
- Prefer finalizing useful references with
  `lazyweb_agentic_search_finalize` and citing them in the lane's evidence dir
  (`design/qa-*/`) alongside the REVIEW — the chosen references become part of
  the lane's verification record.

### Vision-review image batching (binding — provider hard limit)

Every browser vision review that evaluates screenshot evidence MUST batch the
captures: **NEVER send more than 30 images to the model provider in a single
request/context** — larger batches (e.g. 32) are rejected by the provider, the
review session dies mid-verdict, and recovery costs a minified-context re-run.
Proven pattern (Wave 17 lanes 8/12): evaluate captures in **≤8-image batches**,
record a per-batch verdict line in `REVIEW.md` as you go, then summarize the
overall verdict — a review tool call that would attach more than 30 images is a
bug, not a shortcut. This applies to review sessions, review subagents, and any
peer-agent request that carries captures.
- `/lazyweb-growth-report` only when the user explicitly asks for one; never
  auto-start reports.

**Override rules apply to lazyweb output too:** `docs/theme.md` + Workbench
tokens (no off-palette colors, Ember interactive-only ≤5%) · Inter/JetBrains
Mono typography · motion law (≤120ms, transform/opacity, prefers-reduced-motion)
· instant-first envelope patterns in `docs/instant-first.md`. Lazyweb
references inspire layout, hierarchy, density, and flow; they never change
palette, typography, stack, or feature decisions. Output that violates the
token set or motion law is rejected, not applied.

## Backend skills (vendored)

`.kilo/skills/` also carries stack skills: rust-best-practices (Apollo handbook),
postgres-best-practices (Supabase; includes RLS performance + connection pooling
references), stripe-best-practices (for the future PaymentProvider adapter),
insecure-defaults (Trail of Bits; audit command + pattern catalogs for the
Phase-4 security pass). Backend lanes: load rust-best-practices for all Rust
work, postgres-best-practices when writing migrations/queries/RLS policies.
Builds share `.kilo/target-shared` via `.cargo/config.toml` + `.kilo/setup-script`
(disk guardrails DX6) — never build into a worktree-local `target/`; disk
housekeeping is `scripts/dev-clean.sh` (dry-run default, `--go` applies).

## Lane to-do lists (all agent worktrees)

Every worktree lane decomposes its assigned scope into a to-do list using
Kilo's NATIVE to-do list tool (`todowrite`) at lane start — one item per
concrete deliverable — and updates it in real time: `in_progress` while
working, `completed` the moment an item is done, blockers called out inline.
Do NOT track lane work in markdown files (LANE-TASKLIST.md or similar) — the
native to-do list is the single source of truth and is visible in the Agent
Manager UI. An out-of-date to-do list reads as a stalled lane.

## Codebase lookups: graft vs graphify (routing rule)

Two graph tools coexist in this repo. Route by question shape — never run both
speculatively in parallel for the same question:

- **graft (default for all concrete work):** any question whose answer lives at
  specific code locations — "where is X handled", "how does this function work",
  "who calls this", scoping an edit, skimming an API surface, exhaustive
  occurrence sweeps. Start with `graft ask "<q>" --source`; `graft grep` when
  you need every occurrence; `graft callers` / `graft skeleton` for structure.
  Answers carry exact file:line spans — cite them and edit straight from them.
- **graphify (structural / semantic questions):** architecture-level asks where
  the answer is about shape, not locations — "what are the major subsystems",
  "which modules are god nodes / highest fan-in", community/cluster views,
  concept explanations spanning many areas, onboarding orientation. Use
  `graphify query` / `path` / `explain`, `graphify-out/wiki/index.md` (when it
  exists), or GRAPH_REPORT.md for broad review. Also always when the user types
  `/graphify`.
- **Tie-breaker:** if a question is both (e.g. "explain payment sync"), run one
  cheap `graft ask` first; escalate to graphify only if the answer spans
  multiple subsystems or the graft results don't connect the pieces.

Freshness after code changes: `graphify update .` for any change, `graft build`
after big changes — both deterministic/AST-only, no API cost.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before anything else.

Rules:
- Route codebase questions per the "graft vs graphify (routing rule)" section above: graphify handles structural/semantic questions (subsystems, god nodes, communities, cross-cutting concepts), NOT the default for concrete code lookups — those go to graft.
- Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

<!-- graft:start -->
## Graft — repo context graph

This repo is indexed in `graft/`: small linked markdown nodes that explain each
system and carry exact file:line spans, kept in sync with the code through git.

For ANY task here — understanding how something works, finding where code lives,
or scoping a change — get context from the graph before grepping or opening
source files. Re-ask freely (it's cheap) and reuse literal identifiers you
already have (symbol, error string, file name) as the query. New to this repo?
Run `graft map` first — a token-budgeted orientation (dir clusters, hubs,
hotspots), no LLM, no key.

- Run `graft ask "<your question>" --source` → ranked nodes with the relevant
  code spans inlined (each hit's ≤8-line crux by default; `--full` for whole
  definitions when the crux isn't enough). Match the tool to the task shape:
  for understanding or editing, the top node IS the answer — cite its
  `covers:` file:line spans and edit straight from `--source`. For
  exhaustive tasks ("every occurrence / every caller of this pattern"), ranked
  results are top-N, not complete — run `graft grep "<literal>"` instead
  (exhaustive over indexed files, grouped by enclosing symbol), falling back
  to raw `grep -rn` only for unindexed files.
- `graft skeleton <file>` → every definition's signature + span, ~10× cheaper
  than reading the file; use it to skim an API surface.
- `graft callers <symbol>` gives precomputed, exact edges — who calls this.
  Add `--direction out` for what it calls, or `--depth N` to walk
  transitively for the full blast radius. For structural questions, skip
  ranking and use this directly.
- Or browse: `graft/INDEX.md` lists every node; follow the links.
- Monorepos and folders of multiple repos rank fairly across sub-projects —
  hits carry `[scope/]` labels naming which one they're from. Narrow with
  `graft ask "<task>" --in <scope>/` once you know where you're working.

If a returned span is truncated ("+N more lines"), open the file at that exact
range before finalizing. Only open source files when a node genuinely lacks a
needed detail, and then at the exact file:line the node points to — never
re-read whole files.

After big code changes, refresh the graph with `graft build` (deterministic,
no API key, $0).
<!-- graft:end -->
