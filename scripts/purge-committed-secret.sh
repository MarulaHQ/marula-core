#!/usr/bin/env bash
# purge-committed-secret.sh — remove a committed credential from LOCAL history.
#
#   scripts/purge-committed-secret.sh            preflight only, changes nothing
#   scripts/purge-committed-secret.sh --go       rewrite history
#
# Why this is not part of the fix commit: rewriting history changes every
# commit id after the secret's first appearance, so every worktree and every
# clone must move with it. It is safe only when nothing is mid-landing, which
# a script can check but a lane cannot promise.
#
# The secret this targets is the-hive-ai bearer token in `opencode.json`
# (security-audit finding opencode.json.committed-bearer-token-thehive-ai) and
# the machine-local `.kilo/kilo.json`. Both are untracked and ignored now; this
# only scrubs the old blobs.
#
# IMPORTANT, and the reason rotation still matters: this rewrites the LOCAL
# repository. The blob is in `origin/main` from commit 825c7f4e onward, so the
# remote keeps serving it until someone force-pushes a rewritten history and
# every clone re-clones. Rotation is the control that actually revokes access;
# this is hygiene for an export or a fresh clone.
#
# LIMIT: dropping the files does not drop the literal from every blob. The
# security audit's own evidence quoted the token and is redacted in the tree
# as of 2026-10-20, but the old blobs still hold it. A full content scrub needs
#   git filter-repo --replace-text scripts/purge-secrets-expressions.txt
# (filter-repo is not installed here; filter-branch cannot replace text).
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1

GO=0
[[ "${1:-}" == "--go" ]] && GO=1

PATHS=(opencode.json .kilo/kilo.json)
fail=0
say() { echo "  $*"; }

echo "== preflight =="

# 1. No cargo/pnpm/git process that could be writing the object store.
if pgrep -f "git (commit|merge|rebase|cherry-pick)|cargo (build|test|check)|pnpm" >/dev/null 2>&1; then
  say "REFUSED: a git, cargo or pnpm process is running. Retry when the tree is quiet."
  fail=1
else
  say "ok: no git/cargo/pnpm process"
fi

# 2. Every worktree clean, and every worktree HEAD already in main's history.
if [[ -n "$(git status --porcelain)" ]]; then
  say "REFUSED: this checkout has uncommitted changes."
  fail=1
else
  say "ok: this checkout is clean"
fi

dirty_other=0
unmerged_other=0
while IFS= read -r wt; do
  [[ "$wt" == "$ROOT" ]] && continue
  [[ -d "$wt" ]] || continue
  if [[ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]]; then
    say "REFUSED: worktree has uncommitted changes — $wt"
    dirty_other=1
  fi
  h="$(git -C "$wt" rev-parse HEAD 2>/dev/null)"
  if [[ -n "$h" ]] && ! git merge-base --is-ancestor "$h" refs/heads/main 2>/dev/null; then
    say "REFUSED: worktree HEAD is not in main — $wt ($h)"
    unmerged_other=1
  fi
done < <(git worktree list --porcelain | awk '/^worktree /{print $2}')
[[ $dirty_other -eq 0 ]] && say "ok: no other worktree has uncommitted changes"
[[ $unmerged_other -eq 0 ]] && say "ok: every other worktree HEAD is already merged"

# 3. Main must not be moving. Two reads a few seconds apart. Resolve the
# branch, not HEAD: this script usually runs from a lane worktree, whose HEAD
# is the lane tip and not main.
a="$(git rev-parse refs/heads/main)"; sleep 3; b="$(git rev-parse refs/heads/main)"
if [[ "$a" != "$b" ]]; then
  say "REFUSED: main moved during the check ($a -> $b). A lane is landing."
  fail=1
else
  say "ok: main is quiet at $a"
fi

if [[ $fail -ne 0 ]]; then
  echo
  echo "Something is in flight. Re-run when it settles; the secret is already"
  echo "untracked and ignored, so the exposure is not growing."
  exit 1
fi

echo
echo "== plan =="
say "rewrite $(git rev-list --count refs/heads/main) commits on main, dropping:"
for p in "${PATHS[@]}"; do say "  $p"; done
say "a rollback ref is kept at refs/original/refs/heads/main until you delete it"
say "the remote still carries the blob until a force-push and a re-clone"

if [[ $GO -eq 0 ]]; then
  echo
  echo "Dry run. Re-run with --go to rewrite."
  exit 0
fi

echo
echo "== rewriting =="
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch --prune-empty \
  --index-filter 'git rm --cached --ignore-unmatch opencode.json .kilo/kilo.json' \
  -- --all || { echo "filter-branch failed; nothing to do but inspect"; exit 1; }

echo
echo "== done (local only) =="
say "this checkout: $(git rev-parse --short HEAD)"
say "other worktrees still point at the old ids; reset each one onto main"
say "remote: still exposed until force-push + every clone re-clones"
say "cleanup when satisfied: git for-each-ref --format='%(refname)' refs/original | xargs -n1 git update-ref -d"
