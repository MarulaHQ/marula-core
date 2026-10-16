## Repo lookups must go through trail first (BINDING, top-of-file)

Before ANY Grep/Glob/Read search of this codebase: run trail first — the code
map is the graft map, and `trail` is the current command (`graft` still works
and runs the same code):
`trail ask "<q>" --source` / `trail grep "<literal>"` / `trail callers <sym>` /
`trail skeleton <file>` (CLI, always available) or the `trail_*` MCP tools.
If trail MCP tools appear deferred (names listed, schemas withheld), load ALL
of them in ONE ToolSearch lookup (`select:mcp__trail__trail_find_code,mcp__trail__trail_find_all,mcp__trail__trail_trace_calls,mcp__trail__trail_file_api,mcp__trail__trail_repo_map`)
before your first lookup — grep-first is WRONG by default: a trail hit carries
exact file:line spans for ~3% of the tokens reading or grepping the same code
costs. Use plain grep only AFTER a trail attempt missed the file (unindexed
paths: fixtures/, .sqlx/, dist/, evidence dirs). Full routing detail lives in
"trail vs graphify" below. Record trail token savings in the lane landing
record ("🌱 trail saved ~N tokens").

**Worktree lanes: build your own graph at lane start (2026-09-18).** Every
worktree lane runs `trail build` ONCE at lane start in its worktree (deterministic
AST-only, free) so its indexes reflect the lane's branch, then re-runs it after
big changes — instead of sharing the main repo's graph via symlink (the old
setup-script symlink is retired; stale shared spans caused wrong file:line hits
after merges). The map is GITIGNORED wherever it lands — `.trail/cache/` through
`.trail/.gitignore`, or the top-level `graft/` directory through `.gitignore` in a
checkout with no committed `.trail/` wiring yet — never commit, symlink, or
force-add it; if a lane ever sees either in `git status`, fail the check and
re-ignore rather than committing the graph.

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

## Design research — lazyweb + inspo MCPs (binding for every UI/frontend lane)

Every UI/frontend lane (any lane touching pages, views, flows, or UI polish)
MUST run design research through BOTH MCPs BEFORE writing UI, so surfaces are
designed for ultimate UX and looks, not invented from scratch (user directive
2026-09-25 — inspo joins lazyweb as a mandatory leg, not a substitute):

- **lazyweb MCP:** `/lazyweb-search-screens` for the surface's UI pattern (real
  product references ranked by visual search), `/lazyweb-search-flows` for
  multi-step journeys (onboarding, checkout, paywalls, sign-up), and
  `/lazyweb-search-experiments` when a conversion decision is involved.
- **inspo MCP:** `recommend` for a brief (or `search_screens` /
  `find_components` for a named pattern) to study real shipped composition,
  palette, type and density; pull `get_design_system` / `get_reference_jsx`
  only for the references actually kept. Take their composition, not their
  compliance — project tokens always win.
- **Aesthetic reference (2026-09-30, user directive — repointed from the retired
  Workbench mockup):**
  `design/mockups/marula-app-mockup-images/` is the canonical look-and-feel
  reference. Open `source/marula-app-mockups.html` for the interactive mockup and
  `DESIGN-NOTES.md` for the rules behind each state; `desktop/neutral/states/`
  and `mobile/light/states/` hold the per-state PNGs, `manifest.json` indexes all
  70. It sets the bar for layout, hierarchy, density and both modes — use it
  alongside lazyweb research, which supplies pattern breadth. The pre-rebrand
  `meridian-ticket-workbench-html-mockup-v2.html` stays in `design/mockups/` as
  history and is checked for geometry only.
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

## Prose must pass SlopMonster (BINDING, all agents)

Any prose an agent writes or edits for humans — landing/marketing copy, READMEs, docs
prose, emails, UI copy with full sentences — must be evaluated with the SlopMonster
skill (`.kilo/skills/slopmonster/`). Load it (`skill: slopmonster`) before writing the
prose, then verify with `python3 .kilo/skills/slopmonster/tools/deslop.py <file>`.
Ship only at **5/5 CLEAN** (use `--allow-proof` only when a number is real and
evidenced). If the score is below 5, clean it following the skill's loop
(kill vocabulary → kill constructions → put a person back in) and re-lint.
This is not needed for code, code comments, logs, git messages, or prose inside
`code` spans/fenced blocks — the scorer skips those too.

## Env orchestration: ONE entry point for lane stack boots (BINDING, all lanes)

Every lane that boots the demo/test stack — Playwright E2E checks, manual
boot, openapi pin regen, scratch verification — routes through
`scripts/e2e-stack.sh` (Wave 37 lane 1, `p37-env-orchestration`):

- `scripts/e2e-stack.sh` up: per-run DB name + random high ports + run-id-keyed
  container names (pgvector + Valkey), mechanical deterministic reseed
  (seed-demo --force + auth-lockout/onboarding/tours reset), boots core-api +
  realtime, and writes `e2e/.e2e-stack.env` (which Playwright's config and
  `scripts/e2e-build-web.sh` read — never set NEXT_PUBLIC_API_URL/ports by
  hand).
- Teardown: `scripts/e2e-stack.sh down <RUN_ID>` — removes ONLY that run id's
  processes/containers; never a parallel lane's DB. (E2E_FOREGROUND traps on
  its own run id too.)
- `scripts/e2e-stack.sh` up RETURNS as soon as core-api answers its health
  check: core-api and realtime detach with their own stdio (`</dev/null`, logs
  at `.kilo/e2e-logs/<run>/{core-api,realtime}.log`), so piping `up` can never
  block. Never wait on it with `| tail`/`| head`, and read a boot failure from
  the log path rather than from a call that never returns.
- `scripts/dev-status.sh` shows every registered run (pids, ports, env
  fingerprint) — check it BEFORE killing anything that looks like a straggler,
  and never kill processes outside your own run id. It resolves the SHARED root
  (the main checkout) and also scans worktree registries, so a stack booted
  inside a lane worktree is visible from `main`. The run registry and service
  logs live under that shared root; only `e2e/.e2e-stack.env` stays in the
  checkout that ran the boot.
- `scripts/regen-openapi-pins.sh` is the only sanctioned way to re-pin the
  OpenAPI snapshots (compact byte-identical pair + SDK regen + gate).
- Frontend verification stays frontend-tooling-only (typecheck/lint/targeted
  Playwright), per ui-tranches_cargo_check_limits.
- Flow-author rules (order independence, durable-gate login retry): `e2e/README.md`.

## Lane gate pause — wait for user go-ahead (BINDING, all new lanes)

Every NEW lane/agent session stops once and waits for the user before running
heavy verification (user directive 2026-09-25 — now the standing standard for
all lanes):

- Research, implementation, and light frontend checks (typecheck/lint/unit
  tests) proceed freely.
- Before the FIRST heavy gate run — any workspace `cargo` command (build/
  check/test/clippy/sqlx prepare) or any full suite / E2E / Playwright /
  Maestro run / `scripts/e2e-stack.sh` boot — the lane ENDS ITS TURN with a
  `READY FOR GATES — awaiting user go-ahead` report listing the exact commands
  it will run, in order, and the evidence it will capture. It must not start
  them, background them, or continue until the user replies.
- The lane's native to-do list marks the gate step pending with the pause
  noted, so an idle lane reads as "waiting for go-ahead", not stalled.
- Merge only after the user-approved gate run is green (repo close-out rules
  unchanged: landing record, TASKLIST row, no worktree removal by tooling).

## Lane to-do lists (all agent worktrees)

Every worktree lane decomposes its assigned scope into a to-do list using
Kilo's NATIVE to-do list tool (`todowrite`) at lane start — one item per
concrete deliverable — and updates it in real time: `in_progress` while
working, `completed` the moment an item is done, blockers called out inline.
Do NOT track lane work in markdown files (LANE-TASKLIST.md or similar) — the
native to-do list is the single source of truth and is visible in the Agent
Manager UI. An out-of-date to-do list reads as a stalled lane.

## Codebase lookups: trail vs graphify (routing rule)

Two graph tools coexist in this repo. Route by question shape — never run both
speculatively in parallel for the same question:

- **trail (default for all concrete work):** any question whose answer lives at
  specific code locations — "where is X handled", "how does this function work",
  "who calls this", scoping an edit, skimming an API surface, exhaustive
  occurrence sweeps. Start with `trail ask "<q>" --source`; `trail grep` when
  you need every occurrence; `trail callers` / `trail skeleton` for structure.
  Answers carry exact file:line spans — cite them and edit straight from them.
- **graphify (structural / semantic questions):** architecture-level asks where
  the answer is about shape, not locations — "what are the major subsystems",
  "which modules are god nodes / highest fan-in", community/cluster views,
  concept explanations spanning many areas, onboarding orientation. Use
  `graphify query` / `path` / `explain`, `graphify-out/wiki/index.md` (when it
  exists), or GRAPH_REPORT.md for broad review. Also always when the user types
  `/graphify`.
- **Tie-breaker:** if a question is both (e.g. "explain payment sync"), run one
  cheap `trail ask` first; escalate to graphify only if the answer spans
  multiple subsystems or the trail results don't connect the pieces.

Freshness after code changes: `graphify update .` for any change, `trail build`
after big changes — both deterministic/AST-only, no API cost.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before anything else.

Rules:
- Route codebase questions per the "trail vs graphify (routing rule)" section above: graphify handles structural/semantic questions (subsystems, god nodes, communities, cross-cutting concepts), NOT the default for concrete code lookups — those go to trail.
- Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

<!-- trail:start -->
## Trail — repo context graph

This repo is indexed in `.trail/cache/`: small linked markdown nodes that explain each
system and carry exact file:line spans, kept in sync with the code through git.

For ANY task here — understanding how something works, finding where code lives,
or scoping a change — get context from the graph before grepping or opening
source files. Re-ask freely (it's cheap) and reuse literal identifiers you
already have (symbol, error string, file name) as the query. New to this repo?
Run `trail map` first — a token-budgeted orientation (dir clusters, hubs,
hotspots), no LLM, no key.

- Run `trail ask "<your question>" --source` → ranked nodes with the relevant
  code spans inlined (each hit's ≤8-line crux by default; `--full` for whole
  definitions when the crux isn't enough). Match the tool to the task shape:
  for understanding or editing, the top node IS the answer — cite its
  `covers:` file:line spans and edit straight from `--source`. For
  exhaustive tasks ("every occurrence / every caller of this pattern"), ranked
  results are top-N, not complete — run `trail grep "<literal>"` instead
  (exhaustive over indexed files, grouped by enclosing symbol), falling back
  to raw `grep -rn` only for unindexed files.
- `trail skeleton <file>` → every definition's signature + span, ~10× cheaper
  than reading the file; use it to skim an API surface.
- `trail callers <symbol>` gives precomputed, exact edges — who calls this.
  Add `--direction out` for what it calls, or `--depth N` to walk
  transitively for the full blast radius. For structural questions, skip
  ranking and use this directly.
- Or browse: `.trail/cache/INDEX.md` lists every node; follow the links.
- Monorepos and folders of multiple repos rank fairly across sub-projects —
  hits carry `[scope/]` labels naming which one they're from. Narrow with
  `trail ask "<task>" --in <scope>/` once you know where you're working.

If a returned span is truncated ("+N more lines"), open the file at that exact
range before finalizing. Only open source files when a node genuinely lacks a
needed detail, and then at the exact file:line the node points to — never
re-read whole files.

After big code changes, refresh the graph with `trail build` (deterministic,
no API key, $0).
<!-- trail:end -->
