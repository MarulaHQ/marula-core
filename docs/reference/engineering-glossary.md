# Engineering vocabulary

Words used in this repository's working documents — `TASKLIST.md`, lane evidence
directories, commit messages. They are internal shorthand, not product terms, and
none of them belongs on the published docs site. Product vocabulary lives in
[`site/reference/glossary.md`](../site/reference/glossary.md).

## Work structure

| Term | What it means here |
|---|---|
| **Wave** | A numbered batch of lanes planned together against one premise. Wave 52 is the reliability wave; the number appears in lane names and evidence directory names. |
| **Lane** | One unit of work with one owner, one branch and one evidence directory. A lane is done when its branch is merged into local `main` and its tracker row moves. |
| **Row** | One entry in `TASKLIST.md`: a lane id (`R8a`, `P3`), the owner worktree name, the premise, and what it needs. The premise is written as a defect, not a task. |
| **Track** | The grouping letter in a lane id. `P` is platform work, `R` is remediation-found work. |
| **Merge group** | Parallel lanes that share one integration branch and one gate run, so the workspace is built once instead of once per lane. |
| **Integration owner** | The lane in a group that merges the others and runs the group gate. It is a lane, not a separate coordination role. |
| **Merge group gate** | The single heavy verification run per group: fmt, check, clippy, workspace tests, E2E as the plan requires. Run once, on the merged tree. |
| **Light checks** | What a lane may run on its own tree: fmt, `cargo check` on touched crates, package-level unit tests, typecheck and lint. No workspace-wide suite, no E2E, no stack boot. |
| **Gate pause** | The point where a lane stops and reports `READY FOR GATES`, listing the exact commands it will run, and waits for the user's go-ahead instead of starting them. |
| **Worktree lane** | A lane that works in its own `git worktree`, never in the primary checkout. The branch dies with the worktree. |

## Evidence

| Term | What it means here |
|---|---|
| **PLAN** | The lane's `internal/design/qa-<lane>/PLAN.md`: its scope and deliverables, plus the work it explicitly leaves out. |
| **REVIEW** | The lane's verdict document: what was found, what was changed, what is still open, with counts and file:line citations rather than impressions. |
| **GATES** | The lane's gate record: the exact commands run, their result, and a pointer to the group run when the lane did not run its own. |
| **LANDING** | The landing record written when the lane merges: what landed, the merge sha, and the trail token saving. |
| **Evidence directory** | `internal/design/qa-<lane>/` — captures, probe scripts, verdicts. The checker `scripts/check-evidence-writes.mjs` keeps writes inside these paths. |
| **Landing** | Merging into **local** `main`. Not a push, not a PR. |
| **Desk check** | Verifying a claim against running code or a captured screen instead of against a comment. |

## Remediation vocabulary

| Term | What it means here |
|---|---|
| **Remediation** | The R-track: defects the security audit or a review found in already-shipped code, as opposed to new platform work. |
| **Honest gap** | A surface that shipped incomplete, recorded where the reader can see it, instead of being described as done. |
| **Typed outcome with no consumer** | A result value or error variant that nothing reads. Treated as a defect, because it looks handled and is not. |
| **Metric with no reader** | A counter or gauge nothing scrapes or alerts on. Same verdict as above. |
| **Premise** | The claim in a tracker row that the lane exists to test. A stale premise is a reason to stop the lane, not to invent adjacent work. |
| **Stale row** | A tracker row whose work is already merged, or whose cited path no longer exists. `scripts/check-tracker-freshness.mjs` reports these; a human decides. |

## Tooling

| Term | What it means here |
|---|---|
| **trail** | The repo's code graph (`trail ask`, `trail grep`, `trail callers`, `trail skeleton`), rebuilt with `trail build`. Lookups start here, not with grep. |
| **graphify** | The structural graph, for architecture-shaped questions: subsystems, god nodes, communities. |
| **SlopMonster** | The prose linter for documents written for humans. Ships at 5/5 or it does not ship. |
| **Descriptor** | `marula deploy describe` output: every runtime parameter in machine-readable form. A lever that is not in the descriptor does not exist to a deployment. |
| **Preflight** | The check a lane runs before writing anything: tracker freshness, then the code the row cites. For an upgrade, the analogous command is `marula upgrade --check`. |
