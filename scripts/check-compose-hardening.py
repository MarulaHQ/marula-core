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

HOW IT READS THE FILES
----------------------
Not with a bare line regex. Three YAML spellings of the same forbidden setting
are idiomatic and a naive scan misses all three:

  privileged: true            # trailing comment: an end-of-line pattern fails
  services: {web: {privileged: true}}   # flow style: no line starts with it
  "privileged": true          # a quoted key is still the same key

So a line's comment is stripped first, with quotes respected, and the patterns
then match the key at a position where YAML can start one (start of line, after
`{` or after `,`) with an optional quote. A `#` inside a quoted scalar is text,
not a comment, and a `#` only opens a comment at a word boundary.

This is a scanner, not a YAML parser: PyYAML is not in the standard library and
a workflow that shells out to `pip install` to lint a file is a worse trade than
a scan that reads the three spellings above. What it cannot see is a setting
assembled by an anchor or a merge key, and a value that a quoted scalar spells
out (`command: ["privileged: true"]`) is skipped rather than reported, because
the closing quote is not where a value can end. Anything it does report is worth
a look: the setting is either there or it was written in a way Compose would
also misread.

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

# YAML writes a boolean several ways; Compose accepts all of them.
BOOL = r"(?:true|True|TRUE|yes|Yes|YES|on|On|ON)"
HOST = r"(?:host|Host|HOST)"
# A key can start a line, or follow `{` or `,` in a flow mapping. Quoting the
# key does not change it. The value may be quoted too.
KEY_START = r"(?:^|[\s{,])"


def rule(key: str, value: str) -> re.Pattern[str]:
    return re.compile(
        rf"{KEY_START}[\"']?{key}[\"']?\s*:\s*[\"']?{value}[\"']?(?=[\s,}}]|$)"
    )


# (pattern, what a human should read instead of the finding)
RULES: list[tuple[re.Pattern[str], str]] = [
    (rule("privileged", BOOL),
     "`privileged: true` hands the container every capability the host has"),
    (rule("network_mode", HOST),
     "`network_mode: host` puts the container on the host's network stack"),
    (rule("pid", HOST),
     "`pid: host` lets the container see and signal host processes"),
    (rule("ipc", HOST),
     "`ipc: host` shares host shared memory"),
    (re.compile(r"/var/run/docker\.sock"),
     "the Docker socket is root on the host, and a mount hands it over"),
]


def strip_comment(line: str) -> str:
    """Return `line` without its trailing comment, respecting quoted scalars.

    A `#` opens a comment only outside quotes and only at a word boundary
    (`privileged: true#4` is a scalar, `privileged: true # note` is a comment).
    A double-quoted scalar honours backslash escapes; a single-quoted one escapes
    by doubling the quote.
    """
    kept: list[str] = []
    quote: str | None = None
    index = 0
    while index < len(line):
        char = line[index]
        if quote is None:
            if char in "\"'":
                quote = char
                kept.append(char)
            elif char == "#" and (index == 0 or line[index - 1] in " \t"):
                break
            else:
                kept.append(char)
        else:
            kept.append(char)
            if quote == '"' and char == "\\" and index + 1 < len(line):
                kept.append(line[index + 1])
                index += 1
            elif char == quote:
                quote = None
        index += 1
    return "".join(kept)


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
        code = strip_comment(line)
        if not code.strip():
            continue
        for pattern, reason in RULES:
            if pattern.search(code):
                if not exemptions_above(lines, index):
                    findings.append((index + 1, line.strip(), reason))
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

# The three spellings a line-anchored regex misses. Each one is the same
# setting to Compose, so each one is the same finding here.
INLINE_COMMENT = """services:
  escape:
    image: alpine:3.20
    privileged: true   # required temporarily
"""

FLOW_STYLE = """services: {web: {image: alpine:3.20, privileged: true}, api: {image: x, pid: host}}
"""

QUOTED_KEY = """services:
  escape:
    image: alpine:3.20
    "network_mode": "host"
    'ipc': 'host'
"""

# A `#` inside a quoted scalar is text, and a `#` glued to a value is not the
# start of a comment either. Neither line sets anything.
NOT_A_COMMENT = """services:
  web:
    image: alpine:3.20
    command: ["sh", "-c", "echo #1 privileged: true"]
"""


def self_test() -> int:
    """Prove the gate can fail, and can tell a comment from a directive."""
    failures: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        cases = {
            "clean.yml": CLEAN,
            "exempt.yml": EXEMPT,
            "bad.yml": BAD,
            "inline.yml": INLINE_COMMENT,
            "flow.yml": FLOW_STYLE,
            "quoted.yml": QUOTED_KEY,
            "notacomment.yml": NOT_A_COMMENT,
        }
        for name, body in cases.items():
            (root / name).write_text(body, encoding="utf-8")

        found = {p.name: len(scan_file(p)) for p in sorted(root.glob("*.yml"))}
        expected = {
            "clean.yml": 0,
            "exempt.yml": 0,
            "bad.yml": 5,
            "inline.yml": 1,
            # Both settings sit on one line, and a finding is per line, so it
            # names the first of the two. The line is reported either way.
            "flow.yml": 1,
            "quoted.yml": 2,
            # A quoted scalar is not a key, and its closing quote stops the value
            # from ending there, so nothing is reported.
            "notacomment.yml": 0,
        }
        for name, count in expected.items():
            if found[name] != count:
                failures.append(f"{name}: reported {found[name]} finding(s), expected {count}")

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
    print("compose hardening self-test: 8 fixtures behave as intended")
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
