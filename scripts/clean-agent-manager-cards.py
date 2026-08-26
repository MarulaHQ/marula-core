#!/usr/bin/env python3
"""Remove dead Agent Manager worktree cards.

Run ONLY while VS Code is fully quit (the extension flushes its in-memory
state over this file on shutdown/change, which is why edits while it runs
get clobbered).

Removes every entry under `worktrees` whose `path` no longer exists on
disk, plus dangling references in order/session maps. Prints a summary.
"""
import json, os, sys

P = os.path.expanduser(
    "~/Documents/GitHub/meridian-open-psa-core/.kilo/agent-manager.json"
)

with open(P) as f:
    d = json.load(f)

wts = d.get("worktrees", {})
removed, kept = [], []
for wid, e in list(wts.items()):
    if not os.path.isdir(e.get("path", "")):
        removed.append(f"{wid} ({e.get('label')})")
        del wts[wid]
    else:
        kept.append(wid)
d["worktrees"] = wts

for key in ("tabOrder", "worktreeOrder"):
    v = d.get(key)
    if isinstance(v, list):
        d[key] = [x for x in v if x in wts]

sess = d.get("sessions")
if isinstance(sess, dict):
    for k in [k for k, v in sess.items()
              if isinstance(v, dict) and v.get("worktreeId") not in wts]:
        del sess[k]

with open(P, "w") as f:
    json.dump(d, f, indent=2)
    f.write("\n")

print(f"removed {len(removed)} dead cards:")
for r in removed:
    print("  -", r)
print(f"kept {len(kept)} live worktrees")
