# Project instructions for Claude Code

`AGENTS.md` carries the binding project rules. Read it before acting; this file
only fixes the lookup order so a Claude Code session does not start from the
wrong graph.

## Lookups

- **Trail first, when installed.** Run `trail ask "<question>" --source` before
  grep or file reads, or use the trail MCP tools. `trail grep` for every
  occurrence, `trail callers` / `trail skeleton` for structure. Fall back to
  grep only for unindexed paths (`fixtures/`, `.sqlx/`, `dist/`, evidence
  dirs) or after a trail query missed.
- **Trail is optional.** If `trail` is not installed, use grep/read normally
  and read `.trail/learnings/` when that directory exists. You do not need
  trail to build, test, or contribute.
- **graphify is optional.** Use it only when `graphify-out/graph.json` exists
  and the `graphify` command is installed, and only for structural or semantic
  questions. If either is missing, say which and use trail or file tools.
- When the user types `/graphify`, use the installed graphify skill first.
