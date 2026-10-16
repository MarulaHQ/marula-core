#!/usr/bin/env python3
"""lane-target-guard.py — provenance guard for the shared cargo target dir.

Wave 47 lane A17 (`p47-lane-env-hygiene`).  Read this before changing it.

WHY THIS EXISTS
---------------
`.cargo/config.toml` pins `build.target-dir` at `.kilo/target-shared`, so every
worktree and the main checkout compile into ONE directory (disk guardrails DX6;
lane-local target dirs duplicated 36GB for two lanes in 2026-09 and are barred).
Cargo's dep-info records paths RELATIVE to the workspace root, so an artifact a
peer worktree built from a different source state resolves to the CURRENT tree's
files and passes cargo's mtime freshness check.  Cargo then links an artifact
that does not match the sources it was built from, and the failure is charged to
whatever innocent crate happened to recompile:

  * `cannot find value ORG_SUSPENSION_CHANGED` in `core-events` while the const
    exists in `core-domain` and `cargo check` passes — a stale RELEASE
    `core-domain` rlib from a tree without the A7 work.
  * `object file (libring-*.rlib) was built for newer 'macOS' version (26.5)
    than being linked (11.0)` — a dependency rlib built under a different
    SDK/toolchain environment (recorded in the p41-ui-operations landing notes).

Both were recorded as a recurring class in `design/REPORT-lane-env-instability.md`
(2026-09-23, shape 2 and shape 4) and again in `TASKLIST.md` (A17).

WHAT IT DOES
------------
It keeps a small stamp beside the artifacts — `<target-dir>/.marula-lane-stamp.json`
— recording, PER PROFILE, which worktree last built into the dir, that tree's
source fingerprint, and the build-environment identity (SDK/rustc).  Before a
build it answers one question: *are the artifacts in this profile dir
trustworthy for this tree?*  If a different tree with different sources wrote
them, the workspace members for that profile are reset so cargo rebuilds them
from the sources in front of it.  `scripts/e2e-stack.sh` calls `check --fix`
before its release builds and `mark` after; any lane can call it by hand.

WHAT IT IS NOT
--------------
* Not a per-lane target dir.  Disk is the binding constraint (44Gi free at 96%
  used on 2026-10-16) and `.kilo/setup-script` strips `CARGO_TARGET_DIR` from
  lane `.env` for that reason.  Third-party dependency artifacts stay SHARED —
  they are the bulk of the 36GB and are content-identical across lanes.
* Not a hook into bare `cargo build`.  Cargo has no pre-build hook; the guard
  runs where lanes actually are: the sanctioned stack boot, plus `check` by
  hand.  A bare `cargo test` after a peer build is still unguarded — the stamp
  makes the next `check` catch it (see `stale-stamp`), and it is recorded as an
  open shape in `design/qa-p47-lane-env-hygiene/REVIEW.md`.

THE RELEASE TRAP (do not regress this)
--------------------------------------
`cargo clean -p core-domain -p core-events` WITHOUT `--release` removes ~120MiB
and looks like it worked, but the poisoned artifact lives in `release/` and
survives; only `cargo clean -p … --release` clears it (proven 2026-10-16,
`.trail/learnings/2026-10-16-cargo-clean-p-without-release-leaves-eclipse.md`).
Every reset here is profile-keyed and passes `--release` when the profile is
release.  `--self-test` asserts the command shape so a later edit cannot quietly
drop the flag.

Exit codes: 0 = safe (or fixed) · 1 = needs action (verdict printed) · 2 = usage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

SCHEMA = 1
STAMP_NAME = ".marula-lane-stamp.json"
# Directories inside a workspace member that never feed a compile.
SKIP_DIRS = {"target", "node_modules", ".next", "dist", ".git", ".turbo", "build", ".venv"}
# Files above this size are recorded by name+size, not content: a member dir
# holding a large binary (a fixture, an icon) must not make the check slow.
BIG_FILE = 8 * 1024 * 1024
# Fingerprint writes land just before `mark` runs; allow clock granularity.
STAMP_SLACK_S = 5.0


# --------------------------------------------------------------------------
# resolution
# --------------------------------------------------------------------------
def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def read_target_dir_config(repo: Path) -> str | None:
    for name in (".cargo/config.toml", ".cargo/config"):
        cfg = repo / name
        if not cfg.is_file():
            continue
        try:
            import tomllib

            data = tomllib.loads(cfg.read_text())
            value = data.get("build", {}).get("target-dir")
            if value:
                return str(value)
        except Exception:
            # A tolerant fallback: the key lives in [build] and never spans lines.
            for line in cfg.read_text().splitlines():
                m = re.match(r'\s*target-dir\s*=\s*"([^"]+)"', line)
                if m:
                    return m.group(1)
    return None


def resolve_target_dir(repo: Path) -> Path:
    """Same precedence cargo uses: CARGO_TARGET_DIR beats .cargo/config.toml.

    Paths are normalized, not realpath'd: the stamp must land in the directory
    cargo writes to, and macOS's /var → /private/var symlink must not make two
    spellings of one dir look like two.
    """
    for value in (os.environ.get("CARGO_TARGET_DIR", "").strip(), read_target_dir_config(repo)):
        if value:
            p = Path(value)
            return Path(os.path.normpath(p if p.is_absolute() else repo / p))
    return repo / "target"


def workspace_members(repo: Path) -> list[str]:
    """Workspace member dirs, relative to the repo root, globs expanded."""
    import glob

    out: list[str] = []
    try:
        import tomllib

        data = tomllib.loads((repo / "Cargo.toml").read_text())
        patterns = list(data.get("workspace", {}).get("members", []))
    except Exception:
        text = (repo / "Cargo.toml").read_text()
        m = re.search(r"members\s*=\s*\[(.*?)\]", text, re.S)
        patterns = re.findall(r'"([^"]+)"', m.group(1)) if m else []
    for pattern in patterns:
        matches = sorted(glob.glob(str(repo / pattern)))
        if matches:
            for match in matches:
                if (Path(match) / "Cargo.toml").is_file():
                    out.append(str(Path(match).relative_to(repo)))
        else:
            out.append(pattern)
    return sorted(set(out))


def source_fingerprint(repo: Path, members: list[str]) -> str:
    """Content hash of what the workspace crates compile.

    Content, not mtime: identical sources in two worktrees must fingerprint
    identically (that is what makes sharing safe), and a peer's build must not
    change it (that is what makes poisoning visible).
    """
    digest = hashlib.sha256()
    roots = [repo / "Cargo.toml", repo / "Cargo.lock"] + [repo / m for m in members]
    for root in roots:
        if root.is_file():
            files = [root]
            base = repo
        else:
            files = []
            base = root
            for dirpath, dirnames, filenames in os.walk(root):
                dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
                files.extend(Path(dirpath) / f for f in sorted(filenames))
        for path in files:
            digest.update(str(path.relative_to(base)).encode())
            try:
                if path.stat().st_size > BIG_FILE:
                    digest.update(f"big:{path.stat().st_size}".encode())
                    continue
                digest.update(hashlib.sha256(path.read_bytes()).digest())
            except OSError:
                digest.update(b"unreadable")
    return "sha256:" + digest.hexdigest()


def env_identity() -> dict[str, str]:
    """Build-environment identity cargo's own fingerprints do NOT cover.

    Cargo keys its fingerprints on the rustc version, so a toolchain bump
    rebuilds.  The SDK / deployment-target / RUSTFLAGS environment is invisible
    to it, and that is how a dependency rlib built for one macOS SDK ends up
    linked by a lane using another (the p41 libring signature).
    """
    ident = {
        key: os.environ.get(key, "")
        for key in (
            "DEVELOPER_DIR",
            "SDKROOT",
            "MACOSX_DEPLOYMENT_TARGET",
            "RUSTFLAGS",
            "CARGO_BUILD_TARGET",
        )
    }
    rustc = shutil.which("rustc")
    ident["rustc"] = ""
    if rustc:
        try:
            ident["rustc"] = subprocess.run(
                [rustc, "-Vv"], capture_output=True, text=True, timeout=20
            ).stdout.strip()
        except Exception:
            ident["rustc"] = "unknown"
    return ident


def env_digest(ident: dict[str, str]) -> str:
    blob = json.dumps(ident, sort_keys=True).encode()
    return "sha256:" + hashlib.sha256(blob).hexdigest()[:32]


def worktree_id(repo: Path) -> str:
    return hashlib.sha256(str(repo.resolve()).encode()).hexdigest()[:16]


def branch_of(repo: Path) -> str:
    try:
        out = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        return out if out and out != "HEAD" else "(detached " + short_sha(repo) + ")"
    except Exception:
        return "(no git)"


def short_sha(repo: Path) -> str:
    try:
        return subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip() or "unknown"
    except Exception:
        return "unknown"


def profile_dir(profile: str) -> str:
    if profile in ("release", "bench"):
        return profile
    return "debug"


# --------------------------------------------------------------------------
# stamp
# --------------------------------------------------------------------------
def stamp_path(target: Path) -> Path:
    return target / STAMP_NAME


def load_stamp(target: Path) -> dict:
    try:
        data = json.loads(stamp_path(target).read_text())
        if isinstance(data, dict) and data.get("schema") == SCHEMA:
            return data
    except Exception:
        pass
    return {"schema": SCHEMA, "profiles": {}}


def save_stamp(target: Path, stamp: dict) -> None:
    target.mkdir(parents=True, exist_ok=True)
    tmp = stamp_path(target).with_suffix(".tmp")
    tmp.write_text(json.dumps(stamp, indent=2, sort_keys=True) + "\n")
    tmp.replace(stamp_path(target))


def newest_write(target: Path, profile: str) -> float:
    """Newest artifact/fingerprint write in this profile, 0 when none.

    Only the two dirs cargo actually fills with compiled units count:
    `<profile>/.fingerprint/` (one entry per unit) and `<profile>/deps/`.
    A bare `mkdir -p release` is not evidence of a build.
    """
    pdir = target / profile_dir(profile)
    newest = 0.0
    for sub in (".fingerprint", "deps"):
        directory = pdir / sub
        if not directory.is_dir():
            continue
        for entry in os.scandir(directory):
            try:
                newest = max(newest, entry.stat().st_mtime)
            except OSError:
                continue
    return newest


# --------------------------------------------------------------------------
# verdict
# --------------------------------------------------------------------------
def verdict(repo: Path, target: Path, profile: str) -> dict:
    """Classify the profile dir for THIS tree.  Pure read; no cargo."""
    profile_dir(profile)  # validate/normalize
    entry = (load_stamp(target).get("profiles") or {}).get(profile)
    newest = newest_write(target, profile)
    me_id, me_fp = worktree_id(repo), None
    result = {
        "profile": profile,
        "target": str(target),
        "worktree": str(repo),
        "worktree_id": me_id,
        "branch": branch_of(repo),
        "newest_write": newest,
    }
    if newest == 0.0:
        result.update(status="empty", action="none", detail="no artifacts for this profile")
        return result

    if not entry:
        result.update(
            status="unknown",
            action="reset",
            detail=(
                "artifacts exist but no stamp records who built them "
                "(dir cleaned, stamp removed, or built by a cargo that predates the guard)"
            ),
        )
        return result

    stamp_age = float(entry.get("built_at_unix", 0))
    if newest > stamp_age + STAMP_SLACK_S:
        result.update(
            status="stale-stamp",
            action="reset",
            detail=(
                "artifacts were written after the last stamped build — something built here "
                "without marking (a bare cargo invocation in a lane or a peer), so the stamp "
                "cannot vouch for them"
            ),
            last=entry,
        )
        return result

    me_fp = source_fingerprint(repo, workspace_members(repo))
    result["source_fingerprint"] = me_fp
    result["last"] = entry
    if entry.get("worktree_id") == me_id:
        result.update(status="safe", action="none", detail="this worktree built this profile last")
        return result
    if entry.get("source_fingerprint") == me_fp and entry.get("env_identity") == env_digest(
        env_identity()
    ):
        result.update(
            status="safe-shared",
            action="none",
            detail="a peer worktree built this profile from byte-identical sources and env",
        )
        return result
    if entry.get("env_identity") != env_digest(env_identity()):
        result.update(
            status="foreign-env",
            action="report",
            detail=(
                "the last build used a different SDK/toolchain/RUSTFLAGS environment; workspace "
                "resets cannot fix a dependency rlib built under another SDK"
            ),
        )
        return result
    result.update(
        status="foreign-source",
        action="reset",
        detail=(
            "the last build came from a different tree with different sources; cargo's relative "
            "dep-info would let those artifacts pass this tree's freshness check"
        ),
    )
    return result


# --------------------------------------------------------------------------
# actions
# --------------------------------------------------------------------------
def any_cargo_running() -> bool:
    try:
        out = subprocess.run(
            ["pgrep", "-f", r"cargo (build|check|test|run|clippy)|rustc "],
            capture_output=True,
            text=True,
            timeout=15,
        )
        return bool(out.stdout.strip())
    except Exception:
        return True  # cannot tell — treat as busy, the caller decides


def wait_for_gap(seconds: int) -> bool:
    """Never yank artifacts out from under a peer's build (report shape 4)."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        if not any_cargo_running():
            return True
        time.sleep(5)
    return not any_cargo_running()


def reset(repo: Path, profile: str, dry_run: bool = False, wait: int = 120) -> int:
    members = workspace_members(repo)
    cmd = ["cargo", "clean"]
    for member in members:
        cmd += ["-p", Path(member).name]
    if profile == "release":
        # See THE RELEASE TRAP above: without this flag the poisoned artifact
        # survives (only the dev profile is cleared) and the guard would lie.
        cmd.append("--release")
    print("lane-target-guard: reset — " + " ".join(cmd))
    if dry_run:
        return 0
    if wait > 0 and not wait_for_gap(wait):
        print(
            f"lane-target-guard: WARN another cargo build is still running after {wait}s — "
            "resetting anyway; a peer may see a rebuilt workspace (report shape 4).",
            file=sys.stderr,
        )
    proc = subprocess.run(cmd, cwd=str(repo))
    return proc.returncode


def mark(repo: Path, target: Path, profile: str, note: str) -> int:
    stamp = load_stamp(target)
    members = workspace_members(repo)
    stamp.setdefault("profiles", {})[profile] = {
        "worktree": str(repo),
        "worktree_id": worktree_id(repo),
        "branch": branch_of(repo),
        "head": short_sha(repo),
        "built_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "built_at_unix": time.time(),
        "source_fingerprint": source_fingerprint(repo, members),
        "env_identity": env_digest(env_identity()),
        "members": len(members),
        "note": note,
    }
    try:
        save_stamp(target, stamp)
    except OSError as exc:  # a successful build must not fail on a stamp write
        print(f"lane-target-guard: WARN could not write {stamp_path(target)}: {exc}", file=sys.stderr)
        return 0
    print(f"lane-target-guard: marked {profile} as built by {branch_of(repo)} (id {worktree_id(repo)})")
    return 0


def emit(result: dict, as_json: bool) -> None:
    if as_json:
        print(json.dumps(result, indent=2, sort_keys=True, default=str))
        return
    print(f"lane-target-guard: profile {result['profile']} — {result['target']}")
    print(f"  worktree   {result['branch']} (id {result['worktree_id']})  {result['worktree']}")
    last = result.get("last")
    if last:
        print(
            f"  last build {last.get('built_at', '?')} by {last.get('branch', '?')} "
            f"(id {last.get('worktree_id', '?')})  fp {str(last.get('source_fingerprint'))[7:19]}…"
        )
    print(f"  verdict    {result['status'].upper()} — {result['detail']}")


def cmd_check(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    target = resolve_target_dir(repo)
    rc = 0
    for profile in profiles_for(args, target):
        result = verdict(repo, target, profile)
        result["repo"] = str(repo)
        emit(result, args.json)
        if result["action"] == "reset":
            if args.fix:
                print(f"lane-target-guard: fixing — {result['detail']}")
                rrc = reset(repo, profile, dry_run=args.dry_run, wait=args.wait)
                if rrc != 0:
                    print(f"lane-target-guard: FAIL cargo clean exited {rrc}", file=sys.stderr)
                    rc = 1
                else:
                    print(
                        "lane-target-guard: reset done — the build that follows compiles from "
                        "THIS tree's sources."
                    )
            else:
                members = workspace_members(repo)
                print(
                    "lane-target-guard: FIX — cargo clean "
                    + " ".join(f"-p {Path(m).name}" for m in members)
                    + (" --release" if profile == "release" else "")
                )
                rc = 1
        elif result["action"] == "report":
            if profile == "release":
                print(
                    "lane-target-guard: FIX — cargo clean --release (a full release wipe; a "
                    "workspace-member reset cannot reach a dependency rlib built under another SDK), "
                    "then re-run with the same DEVELOPER_DIR on every lane."
                )
            else:
                print("lane-target-guard: FIX — cargo clean, then re-run with one SDK environment.")
            rc = 1
    return rc


def profiles_for(args: argparse.Namespace, target: Path) -> list[str]:
    if getattr(args, "all_profiles", False):
        found = []
        for name in ("release", "debug"):
            if (target / name).is_dir():
                found.append("release" if name == "release" else "debug")
        return found or ["release", "debug"]
    return [args.profile]


def cmd_mark(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    return mark(repo, resolve_target_dir(repo), args.profile, args.note)


def cmd_reset(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    return reset(repo, args.profile, dry_run=args.dry_run, wait=args.wait)


def cmd_status(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    target = resolve_target_dir(repo)
    stamp = load_stamp(target)
    print(f"lane-target-guard: target {target}")
    print(f"  exists      {target.is_dir()}")
    print(f"  worktree    {branch_of(repo)} (id {worktree_id(repo)})")
    print(f"  members     {len(workspace_members(repo))}")
    print(f"  stamp       {stamp_path(target)}")
    for profile, entry in sorted((stamp.get("profiles") or {}).items()):
        print(
            f"    {profile:8} built {entry.get('built_at', '?')} by {entry.get('branch', '?')} "
            f"(id {entry.get('worktree_id', '?')})"
        )
    return 0


# --------------------------------------------------------------------------
# self-test — pure Python, no cargo, no target dir needed
# --------------------------------------------------------------------------
def self_test() -> int:
    import tempfile

    failures: list[str] = []
    checks = 0

    def expect(label: str, got, want) -> None:
        nonlocal checks
        checks += 1
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        # A synthetic workspace with two members (one is a glob match).
        (root / "crates/alpha/src").mkdir(parents=True)
        (root / "crates/beta/src").mkdir(parents=True)
        (root / "crates/alpha/src/lib.rs").write_text("pub const A: u8 = 1;\n")
        (root / "crates/beta/src/lib.rs").write_text("pub use alpha;\n")
        (root / "crates/alpha/Cargo.toml").write_text("[package]\nname = \"alpha\"\n")
        (root / "crates/beta/Cargo.toml").write_text("[package]\nname = \"beta\"\n")
        (root / "Cargo.toml").write_text(
            "[workspace]\nmembers = [\"crates/*\"]\n", encoding="utf-8"
        )
        (root / "Cargo.lock").write_text("# lock\n")

        expect("members parsed", workspace_members(root), ["crates/alpha", "crates/beta"])

        # target-dir resolution: config, then env override.
        (root / ".cargo").mkdir()
        (root / ".cargo/config.toml").write_text(
            '[build]\ntarget-dir = "shared-target"\n'
        )
        expect("target-dir from config", resolve_target_dir(root), root / "shared-target")
        os.environ["CARGO_TARGET_DIR"] = str(root / "env-target")
        expect("target-dir from env", resolve_target_dir(root), root / "env-target")
        del os.environ["CARGO_TARGET_DIR"]

        target = root / "shared-target"
        (target / "release/.fingerprint").mkdir(parents=True)

        # empty profile dir -> safe
        expect("empty verdict", verdict(root, target, "release")["status"], "empty")

        # artifacts but no stamp -> unknown/reset
        (target / "release/.fingerprint/alpha-1").mkdir()
        expect("no stamp -> unknown", verdict(root, target, "release")["status"], "unknown")
        expect(
            "unknown needs reset", verdict(root, target, "release")["action"], "reset"
        )

        # mark -> safe
        mark(root, target, "release", "self-test")
        expect("marked -> safe", verdict(root, target, "release")["status"], "safe")

        # a peer tree, identical sources -> safe-shared
        peer = root / "peer"
        shutil.copytree(root / "crates", peer / "crates")
        for name in ("Cargo.toml", "Cargo.lock"):
            shutil.copy2(root / name, peer / name)
        stamp = load_stamp(target)
        stamp["profiles"]["release"]["worktree_id"] = worktree_id(peer)
        stamp["profiles"]["release"]["source_fingerprint"] = source_fingerprint(
            peer, workspace_members(peer)
        )
        save_stamp(target, stamp)
        expect("peer, same sources -> safe-shared", verdict(root, target, "release")["status"],
               "safe-shared")

        # a peer tree with different sources -> foreign-source/reset
        (peer / "crates/alpha/src/lib.rs").write_text("pub const A: u8 = 2;\n")
        stamp["profiles"]["release"]["source_fingerprint"] = source_fingerprint(
            peer, workspace_members(peer)
        )
        save_stamp(target, stamp)
        v = verdict(root, target, "release")
        expect("peer, other sources -> foreign-source", v["status"], "foreign-source")
        expect("foreign-source needs reset", v["action"], "reset")

        # a peer tree with a different SDK environment -> foreign-env/report
        stamp["profiles"]["release"]["source_fingerprint"] = source_fingerprint(
            root, workspace_members(root)
        )
        stamp["profiles"]["release"]["env_identity"] = "sha256:not-ours"
        save_stamp(target, stamp)
        v = verdict(root, target, "release")
        expect("peer, other env -> foreign-env", v["status"], "foreign-env")
        expect("foreign-env reports, never auto-cleans", v["action"], "report")

        # a write after the stamp -> stale-stamp/reset (bare cargo in a lane)
        mark(root, target, "release", "self-test")
        late = target / "release/.fingerprint/alpha-2"
        late.mkdir()
        os.utime(late, (time.time() + 60, time.time() + 60))
        expect("write after stamp -> stale-stamp", verdict(root, target, "release")["status"],
               "stale-stamp")

        # profile independence: a release-only stamp must not vouch for debug
        expect("debug separate from release", verdict(root, target, "debug")["status"], "empty")

        # release reset command carries --release (the 2026-10-16 trap)
        argv: list[str] = []

        def fake_run(cmd, cwd=None, **kwargs):
            argv.extend(cmd)

            class R:
                returncode = 0

            return R()

        real_run = subprocess.run
        subprocess.run = fake_run  # type: ignore[assignment]
        try:
            reset(root, "release", dry_run=False, wait=0)
            expect("release reset clears first", argv[:2], ["cargo", "clean"])
            expect("release reset passes --release", argv[-1], "--release")
            expect("release reset names both members", all(m in argv for m in ("alpha", "beta")), True)
            expect("release reset has one -p per member", argv.count("-p"), 2)
            argv.clear()
            reset(root, "debug", dry_run=False, wait=0)
            expect("debug reset has no --release", "--release" in argv, False)
        finally:
            subprocess.run = real_run  # type: ignore[assignment]

    if failures:
        print(f"lane-target-guard self-test: {len(failures)} of {checks} failed")
        for line in failures:
            print(f"  FAIL  {line}")
        return 1
    print(f"lane-target-guard self-test: {checks} checks pass (no cargo, no target dir touched)")
    return 0


# --------------------------------------------------------------------------
def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="lane-target-guard.py",
        description="Provenance guard for the shared cargo target dir (Wave 47 A17).",
    )
    sub = p.add_subparsers(dest="cmd")

    def add_profile(sp):
        sp.add_argument("--profile", default="release", choices=["release", "debug", "dev", "test"])
        sp.add_argument("--repo", default=str(repo_root()))

    check = sub.add_parser("check", help="classify the target dir for this tree")
    add_profile(check)
    check.add_argument("--fix", action="store_true", help="reset workspace members when foreign")
    check.add_argument("--dry-run", action="store_true", help="print the reset, do not run it")
    check.add_argument("--json", action="store_true")
    check.add_argument("--all-profiles", action="store_true")
    check.add_argument("--wait", type=int, default=int(os.environ.get("MARULA_LANE_GUARD_WAIT", 120)))
    check.set_defaults(func=cmd_check)

    markp = sub.add_parser("mark", help="record this worktree as the profile's last builder")
    add_profile(markp)
    markp.add_argument("--note", default="")
    markp.set_defaults(func=cmd_mark)

    rs = sub.add_parser("reset", help="clear the workspace members for a profile")
    add_profile(rs)
    rs.add_argument("--dry-run", action="store_true")
    rs.add_argument("--wait", type=int, default=int(os.environ.get("MARULA_LANE_GUARD_WAIT", 120)))
    rs.set_defaults(func=cmd_reset)

    st = sub.add_parser("status", help="show the stamp and what this tree would see")
    st.add_argument("--repo", default=str(repo_root()))
    st.set_defaults(func=cmd_status)

    sst = sub.add_parser("self-test", help="run the guard's own checks (no cargo)")
    sst.set_defaults(func=lambda _a: self_test())
    return p


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not getattr(args, "func", None):
        parser.print_help()
        return 2
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
