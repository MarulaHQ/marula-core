#!/usr/bin/env python3
"""check-compose-hardening.py — the compose rules the container gate enforces.

WHY THIS EXISTS
---------------
`deploy/` ships compose files, and until now nothing read them. Trivy's config
scan handles Dockerfiles, Kubernetes manifests and Helm charts; it does not read
`docker-compose.yml` at all — point it at a compose file with `privileged: true`
and it reports `Detected config files num=0`. hadolint reads Dockerfiles too. So
the properties those files rely on (no `privileged`, no host network, no Docker
socket) were conventions held by review, not by a gate, and the trivy job's own
comment used to claim otherwise.

WHAT IT CHECKS
--------------
Five settings, each one a container that reaches past its own boundary:

  privileged: true            every capability, and the container escapes the
                             isolation the others in the file depend on
  network_mode: host          the container owns the host's network stack
  pid: host                   it can see and signal host processes
  ipc: host                   it shares host shared memory
  /var/run/docker.sock        it controls the daemon, which is root on the host

`cap_add` and `read_only` are deliberately not checked: the first is a narrow
grant the edge file uses for NET_BIND_SERVICE, the second already carries an
`EXEMPTION (read_only)` convention in the files themselves and a ratchet that
guesses at it would be wrong more often than right.

EXEMPTIONS
----------
A violation is exempt when the comment block immediately above it carries
`EXEMPTION (compose)` with a reason. That is the convention these files already
use (`EXEMPTION (read_only)`), so an exemption reads the same way as the others.

USAGE
-----
  python3 scripts/check-compose-hardening.py             # scan deploy/
  python3 scripts/check-compose-hardening.py --self-test # prove it can fail
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SCAN_DIR = ROOT / "deploy"

MARKER = "EXEMPTION (compose)"

# (pattern, what a human should read instead of the finding)
RULES: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"^\s*privileged\s*:\s*(?:true|yes|on|\"true\"|'true')\s*$"),
     "`privileged: true` hands the container every capability the host has"),
    (re.compile(r"^\s*network_mode\s*:\s*[\"']?host[\"']?\s*$"),
     "`network_mode: host` puts the container on the host's network stack"),
    (re.compile(r"^\s*pid\s*:\s*[\"']?host[\"']?\s*$"),
     "`pid: host` lets the container see and signal host processes"),
    (re.compile(r"^\s*ipc\s*:\s*[\"']?host[\"']?\s*$"),
     "`ipc: host` shares host shared memory"),
    (re.compile(r"/var/run/docker\.sock"),
     "the Docker socket is root on the host, and a mount hands it over"),
]


def exemptions_above(lines: list[str], index: int) -> bool:
    """True when the comment block immediately above `index` exempts it.

    Only the contiguous run of comment lines above counts, so a marker far up
    the file cannot quietly cover a new line somebody added later.
    """
    cursor = index - 1
    while cursor >= 0:
        stripped = lines[cursor].strip()
        if not stripped:
            cursor -= 1
            continue
        if not stripped.startswith("#"):
            return False
        if MARKER in stripped:
            return True
        cursor -= 1
    return False


def scan_file(path: Path) -> list[tuple[int, str, str]]:
    findings: list[tuple[int, str, str]] = []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("#"):
            continue
        for pattern, reason in RULES:
            if pattern.search(line):
                if not exemptions_above(lines, index):
                    findings.append((index + 1, stripped, reason))
                break
    return findings


def scan(directory: Path) -> list[tuple[Path, int, str, str]]:
    findings: list[tuple[Path, int, str, str]] = []
    for path in sorted(directory.rglob("*")):
        if path.suffix not in (".yml", ".yaml"):
            continue
        for line_no, text, reason in scan_file(path):
            findings.append((path, line_no, text, reason))
    return findings


def run(directory: Path) -> int:
    findings = scan(directory)
    if not findings:
        count = sum(1 for p in directory.rglob("*") if p.suffix in (".yml", ".yaml"))
        print(f"compose hardening: clean ({count} compose file(s) under {directory.name}/)")
        return 0
    for path, line_no, text, reason in findings:
        rel = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
        print(f"{rel}:{line_no}: {text}", file=sys.stderr)
        print(f"    {reason}. Remove it, or record why with `# {MARKER}: ...` above it.",
              file=sys.stderr)
    print(f"compose hardening: {len(findings)} finding(s)", file=sys.stderr)
    return 1


CLEAN = """services:
  web:
    image: alpine:3.20
    read_only: true
    security_opt: ["no-new-privileges"]
  # a comment that mentions privileged: true must not trip the scan
"""

EXEMPT = """services:
  proxy:
    image: alpine:3.20
    # EXEMPTION (compose): this test fixture binds the host network on purpose.
    network_mode: host
"""

BAD = """services:
  escape:
    image: alpine:3.20
    privileged: true
    network_mode: host
    pid: host
    ipc: host
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
"""


def self_test() -> int:
    """Prove the gate can fail, and can tell a comment from a directive."""
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        cases = {"clean": CLEAN, "exempt": EXEMPT, "bad": BAD}
        for name, body in cases.items():
            (root / f"{name}.yml").write_text(body, encoding="utf-8")

        found = {p.name: len(scan_file(p)) for p in sorted(root.glob("*.yml"))}
        if found["clean.yml"] != 0:
            failures.append(f"a clean file reported {found['clean.yml']} finding(s)")
        if found["exempt.yml"] != 0:
            failures.append(f"an exempted line reported {found['exempt.yml']} finding(s)")
        if found["bad.yml"] != 5:
            failures.append(f"the violating fixture reported {found['bad.yml']} of 5 settings")

        # The exemption must be literally adjacent: a marker separated by a
        # non-comment line must not cover anything.
        detached = root / "detached.yml"
        detached.write_text(
            "services:\n"
            "  a:\n"
            "    image: alpine:3.20\n"
            f"    # {MARKER}: stale marker\n"
            "    command: ['true']\n"
            "    privileged: true\n",
            encoding="utf-8",
        )
        if len(scan_file(detached)) != 1:
            failures.append("a stale exemption marker covered a line it should not")

    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    if failures:
        return 1
    print("compose hardening self-test: 4 fixtures behave as intended")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    directory = DEFAULT_SCAN_DIR
    if len(argv) > 1 and not argv[1].startswith("-"):
        directory = Path(argv[1]).resolve()
    if not directory.is_dir():
        print(f"no such directory: {directory}", file=sys.stderr)
        return 2
    return run(directory)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
