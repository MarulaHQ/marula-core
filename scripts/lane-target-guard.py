#!/usr/bin/env python3
"""lane-target-guard.py — provenance guard for the shared cargo target dir.

Wave 47 lane A17 (`p47-lane-env-hygiene`).  Read this before changing it.

WHY THIS EXISTS
---------------
A machine that turns on `share-target` (`.cargo/config.toml` cannot carry it —
cargo ranks the including file above the included overlay, so the key lives in
`.cargo/p13-fast.local.toml`, written by `scripts/p13-build-env.sh enable
--share-target`) makes every worktree and the main checkout compile into ONE
directory, the main checkout's `.kilo/target-shared` (disk guardrails DX6;
lane-local target dirs duplicated 36GB for two lanes in 2026-09 and are barred).
Without it — CI, a fresh clone — cargo's default `target/` is used and there is
nothing shared to guard, which this script reports rather than assumes.
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
(2026-09-23, shape 2 and shape 4) and again in the private build tracker (A17).

WHAT IT DOES
------------
It keeps a small stamp beside the artifacts — `<target-dir>/.marula-lane-stamp.json`
— recording, PER PROFILE, which worktree last built into the dir, that tree's
source fingerprint, and the build-environment identity (SDK/rustc).  Before a
build it answers one question: *are the artifacts in this profile dir
trustworthy for this tree?*  If not, artifacts are reset so cargo rebuilds them
from the sources in front of it — the workspace members when a peer tree wrote
them, the whole profile when writes landed outside the member set (see "TWO
REPAIR SCOPES" below).  `scripts/e2e-stack.sh` calls `check --fix` before its
release builds and `mark` after; any lane can call it by hand.

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

BUILD TOOLS ARE RECORDED, NOT JUDGED (Wave 52 P13)
--------------------------------------------------
The build gained two optional levers in P13: a machine-local overlay
(`.cargo/p13-fast.local.toml`, included by `.cargo/config.toml` with
`optional = true`) that can name a faster linker, and sccache as rustc's
wrapper.  Neither belongs in `env_identity()`, and neither changes a verdict:

* A missing linker on one lane and lld on another does not make either lane's
  artifacts STALE.  Cache freshness is cargo's fingerprint plus this stamp; a
  linker choice decides how a final binary is assembled, not whether an rlib
  matches the sources it will be linked with.
* sccache is content-addressed on the compilation inputs, so a hit is an
  artifact for byte-identical inputs, and cargo writes its fingerprints as if
  it had compiled.  Making the wrapper part of the identity would turn every
  `cargo binstall sccache` into a `foreign-env` report for the whole workspace,
  which is friction with nothing behind it.

What the guard does instead is `mark()` them into the stamp (`build_tools`:
overlay presence, linker, wrapper, and any remote-cache variable exported in the
shell) and print them in `status`.  When a link or a compile looks wrong, the
first question is which of these built it, and the stamp answers it per profile.

A REMOTE sccache is the one configuration the guard cannot vouch for, so nothing
in this repo writes one: `SCCACHE_REDIS`, `SCCACHE_BUCKET`, `SCCACHE_S3_BUCKET`,
`SCCACHE_GCS_BUCKET`, `SCCACHE_AZURE_CONNECTION_STRING` and
`SCCACHE_WEBDAV_ENDPOINT` are reported by `status` when a shell exports them,
never configured.  If a remote cache is ever wanted, the lane that adds it owes
this guard a fingerprint over the cache's key inputs first.

THE RELEASE TRAP (do not regress this)
--------------------------------------
`cargo clean -p core-domain -p core-events` WITHOUT `--release` removes ~120MiB
and looks like it worked, but the poisoned artifact lives in `release/` and
survives; only `cargo clean -p … --release` clears it (proven 2026-10-16,
`.trail/learnings/2026-10-16-cargo-clean-p-without-release-leaves-eclipse.md`).
Every reset here is profile-keyed and passes `--release` when the profile is
release.  `--self-test` asserts the command shape so a later edit cannot quietly
drop the flag.

PACKAGE NAMES, NOT DIRECTORY NAMES (Wave 47 A20, do not regress this)
---------------------------------------------------------------------
The repair must pass cargo a PACKAGE name, never a member directory's basename.
Two of the 32 members are not named after their directory (`desktop` is package
`marula-desktop`, `plugins/connectors/mapbox` is `mapbox-routing`), so the old
`-p Path(member).name` emitted `-p desktop -p mapbox`; cargo exits 101
("package ID specification `desktop` did not match any packages") and the boot
the guard exists to protect dies inside the guard's own repair step.  Names are
resolved from each member's manifest (`workspace_packages`), and `--self-test`
asserts every `-p` resolves to a real workspace package with a dir != name case
covered.

TWO REPAIR SCOPES (Wave 47 A20)
-------------------------------
`newest_write()` scans the whole profile dir, so the state the stamp test wants
depends on WHY the dir was flagged:

* `foreign-source` — a peer tree wrote OUR crates' artifacts.  Clearing the
  workspace members reaches it, and keeps the SHARED third-party artifacts (the
  bulk of the dir, content-identical across trees — the reason a shared dir is
  affordable at all).
* `stale-stamp` / `unknown` — writes landed anywhere in the profile: a bare
  `cargo test` writes third-party dev units too, and no member list reaches
  them.  A member-only clean looks clean but leaves deps newer than the stamp,
  so the verdict never clears (lane 19: 6697 files, 1.9GiB).  These clear the
  whole profile, which IS the state the stamp test demands.

CROSS-SESSION ARBITRATION (Wave 47 A20 D, do not regress this)
--------------------------------------------------------------
The reset above is right for one session and destructive across two.  Two
sessions on one machine write into the same `.kilo/target-shared` (with
share-target on), so when the
stamp names another worktree the "clean the members and rebuild" rule wipes
artifacts a PEER session may still be building — and a whole-profile clean takes
the whole debug dir (21.5GiB here) out from under it.

The arbiter is cargo's OWN per-profile build lock, `<target-dir>/<profile>/.cargo-lock`.
Cargo takes an exclusive `flock` on it for the duration of a build, so it is
already held by every actor that can corrupt the dir (any session, worktree,
checkout, or a containerised build writing into a bind-mounted dir), needs no new
protocol and no cooperation, and the kernel drops it when the builder exits, so
there is no stale-lock problem.  The guard probes it with a non-blocking `flock`:
free means nothing is building into this profile, held means refuse.

Refusal is the outcome, never a silent proceed:

* a held lock (or `any_cargo_running()` failing to rule a build out) — wait up to
  `--wait` for the lock to free, then REFUSE and clean nothing, exit 3;
* the lock file cannot be read, so a live build cannot be ruled out — REFUSE;
* `--force` is the explicit override, and the only way to clean under a live
  build.  Nothing else proceeds.

The lock file's ABSENCE reads as free, because cargo creates
`<profile>/.cargo-lock` on first use and never deletes it, so a profile dir with
artifacts always has one.

Exit codes: 0 = safe (or fixed) · 1 = needs action (verdict printed) · 2 = usage
· 3 = refused (a build is using this profile; nothing was cleaned).
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
# Cargo's own per-profile build lock.  The guard never invents a lock of its own:
# see "CROSS-SESSION ARBITRATION" in the module docstring.
BUILD_LOCK_NAME = ".cargo-lock"
# Exit code for "refused — a build is using this profile, nothing was cleaned".
EXIT_REFUSED = 3

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


def _target_dir_in(cfg: Path) -> str | None:
    if not cfg.is_file():
        return None
    try:
        import tomllib

        data = tomllib.loads(cfg.read_text())
        value = data.get("build", {}).get("target-dir")
        if value:
            return str(value)
        return None
    except Exception:
        # A tolerant fallback: the key lives in [build] and never spans lines.
        for line in cfg.read_text().splitlines():
            m = re.match(r'\s*target-dir\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    return None


def _included_configs(cfg: Path) -> list[Path]:
    """The `include` entries of one config file, resolved like cargo does:
    relative to that config file's own directory. Optional paths may be
    absent; a malformed file yields no entries rather than an error."""
    try:
        import tomllib

        data = tomllib.loads(cfg.read_text())
    except Exception:
        return []
    out: list[Path] = []
    for entry in data.get("include", []) or []:
        path = entry.get("path") if isinstance(entry, dict) else None
        if isinstance(path, str) and path:
            out.append((cfg.parent / path).resolve())
    return out


def read_target_dir_config(repo: Path) -> str | None:
    """The configured `build.target-dir`, resolved the way cargo merges config.

    The including file wins over everything it includes, so its own key is
    authoritative; only when it names none do the includes get a say, in the
    order they are listed. This machine has exactly one include — the
    machine-local overlay that carries `share-target` — and CI has none.
    """
    for name in (".cargo/config.toml", ".cargo/config"):
        cfg = repo / name
        if not cfg.is_file():
            continue
        value = _target_dir_in(cfg)
        if value:
            return value
        for included in _included_configs(cfg):
            value = _target_dir_in(included)
            if value:
                return value
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


def read_package_name(manifest: Path, root: Path | None = None) -> str | None:
    """The `[package] name` a member manifest declares, or None if it has none.

    A virtual manifest has no `[package] name`; a member always does.
    """
    try:
        import tomllib

        data = tomllib.loads(manifest.read_text())
    except Exception:
        m = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"', manifest.read_text())
        return m.group(1) if m else None
    name = (data.get("package") or {}).get("name")
    if isinstance(name, str) and name:
        return name
    if isinstance(name, dict) and name.get("workspace") and root is not None:
        # `name.workspace = true` inherits from the root `[workspace.package]`.
        try:
            import tomllib

            root_data = tomllib.loads((root / "Cargo.toml").read_text())
            inherited = (root_data.get("workspace", {}).get("package") or {}).get("name")
            if isinstance(inherited, str) and inherited:
                return inherited
        except Exception:
            pass
    return None


def workspace_packages(repo: Path) -> list[dict]:
    """Every workspace member as its member directory + resolved package name.

    `workspace_members()` yields DIRECTORIES; cargo's `-p` wants the PACKAGE
    name, and the two differ wherever a member directory is not named after its
    package (`desktop` → `marula-desktop`, `plugins/connectors/mapbox` →
    `mapbox-routing`, 2 of 32 here).  Emitting the directory basename made
    `cargo clean` exit 101 and killed the boot the guard exists to protect
    (Wave 47 A20), so the name comes from the member's own manifest.

    `name` falls back to the directory basename only for a manifest with no
    `[package] name` (a virtual manifest, which cannot be a member anyway), and
    `resolved` records which happened so `status` can surface it.
    """
    out: list[dict] = []
    for member in workspace_members(repo):
        manifest = repo / member / "Cargo.toml"
        name = read_package_name(manifest, repo) if manifest.is_file() else None
        out.append(
            {"member": member, "name": name or Path(member).name, "resolved": name is not None}
        )
    return out


def package_names(repo: Path) -> list[str]:
    return [pkg["name"] for pkg in workspace_packages(repo)]


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


# The machine-local overlay P13 introduced.  Machine-local by design (the
# committed config includes it optionally), so a peer lane's copy is not this
# lane's copy: recorded in the stamp, never folded into a verdict.  See "BUILD
# TOOLS ARE RECORDED, NOT JUDGED" in the module docstring.
OVERLAY_REL = Path(".cargo") / "p13-fast.local.toml"
REMOTE_SCCACHE_VARS = (
    "SCCACHE_REDIS",
    "SCCACHE_BUCKET",
    "SCCACHE_S3_BUCKET",
    "SCCACHE_GCS_BUCKET",
    "SCCACHE_AZURE_CONNECTION_STRING",
    "SCCACHE_WEBDAV_ENDPOINT",
)


def build_tools(repo: Path) -> dict:
    """Which optional build levers this tree would use, for the stamp only.

    Reads the overlay the lane actually has plus the wrapper/cache variables in
    the calling shell.  Absent overlay, absent wrapper, no remote: the
    documented stock configuration.
    """
    overlay = repo / OVERLAY_REL
    text = ""
    try:
        if overlay.is_file():
            text = overlay.read_text()
    except OSError:
        text = ""
    linker = None
    m = re.search(r"-fuse-ld=([^\s\"']+)", text)
    if m:
        linker = m.group(1)
    wrapper = None
    m = re.search(r'(?m)^\s*rustc-wrapper\s*=\s*"([^"]+)"', text)
    if m:
        wrapper = m.group(1)
    if not wrapper:
        wrapper = os.environ.get("RUSTC_WRAPPER") or os.environ.get("CARGO_BUILD_RUSTC_WRAPPER") or None
    return {
        "overlay": str(OVERLAY_REL) if overlay.is_file() else None,
        "linker": linker,
        "rustc_wrapper": wrapper,
        "remote_cache_vars": [v for v in REMOTE_SCCACHE_VARS if os.environ.get(v)],
    }


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
        return True  # cannot tell — the caller decides, and here that means refuse


def profile_lock_path(target: Path, profile: str) -> Path:
    """Cargo's own build lock for one profile: `<target>/<profile>/.cargo-lock`."""
    return target / profile_dir(profile) / BUILD_LOCK_NAME


def lock_held(path: Path) -> bool | None:
    """Is cargo's build lock at `path` held right now?

    `flock` is the primitive cargo itself uses, so a non-blocking `LOCK_EX` that
    fails means a build owns this profile dir.  True = held, False = free,
    None = cannot tell (no `fcntl`, or the file cannot be opened).
    """
    try:
        import fcntl
    except ImportError:
        return None
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return None
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return True
    else:
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    finally:
        os.close(fd)


def profile_build_state(target: Path, profile: str) -> str:
    """`held` | `free` | `unknown` — is a build using this profile dir now?

    Absence of the lock file reads as `free`: cargo creates it on first use and
    never deletes it, so a profile dir with artifacts always has one.
    """
    lock = profile_lock_path(target, profile)
    if not lock.exists():
        return "free"
    held = lock_held(lock)
    if held is None:
        # Cannot read cargo's own lock, so a live build cannot be ruled out.
        return "held" if any_cargo_running() else "unknown"
    return "held" if held else "free"


def live_build_cwds() -> list[str]:
    """Best-effort cwds of live cargo/rustc processes, for the refusal message."""
    try:
        out = subprocess.run(
            ["pgrep", "-f", r"cargo (build|check|test|run|clippy)|rustc "],
            capture_output=True,
            text=True,
            timeout=15,
        ).stdout
    except Exception:
        return []
    cwds: list[str] = []
    for pid in out.split():
        try:
            listing = subprocess.run(
                ["lsof", "-a", "-p", pid, "-d", "cwd", "-Fn"],
                capture_output=True,
                text=True,
                timeout=10,
            ).stdout
        except Exception:
            continue
        cwds += [line[1:] for line in listing.splitlines() if line.startswith("n")]
    return sorted(set(cwds))


def wait_for_free_lock(target: Path, profile: str, seconds: int) -> str:
    """Poll the profile's build lock until it frees, or the budget runs out.

    There is no way to yank a peer's artifacts without breaking its build, so
    waiting is all a reset can honestly do; what it must NOT do is proceed when
    the wait runs out (that was the old warn-and-clean-anyway).
    """
    state = profile_build_state(target, profile)
    if state == "free" or seconds <= 0:
        return state
    print(f"lane-target-guard: waiting up to {seconds}s for the {profile} build lock to free …")
    deadline = time.time() + seconds
    while state != "free" and time.time() < deadline:
        time.sleep(1)
        state = profile_build_state(target, profile)
    return state


def cargo_profile(profile: str) -> str:
    """The profile NAME cargo's `--profile` accepts for a guard profile.

    Cargo has no profile called `debug` (`cargo clean --profile debug` exits 2
    and points at the name `dev`), and the `dev`/`test` profiles both write
    `target/debug` — the same directory `profile_dir()` maps them to.
    """
    if profile in ("release", "bench"):
        return profile
    return "dev"


def scope_for_status(status: str) -> str:
    """The clean scope that reaches the state the stamp test demands.

    `newest_write()` scans the whole profile dir, so a `stale-stamp` or
    `unknown` verdict cannot be repaired by clearing the workspace members
    alone — the newer writes are third-party dev units a member list never
    reaches.  `foreign-source` is exactly the members' poison, so it stays
    surgical and the shared third-party artifacts survive.
    """
    return "profile" if status in ("stale-stamp", "unknown") else "members"


def reset_command(repo: Path, profile: str, scope: str) -> list[str]:
    """The cargo command that clears what a verdict flagged — nothing more.

    See "TWO REPAIR SCOPES" in the module docstring.  Package names, never
    member directory basenames: see "PACKAGE NAMES, NOT DIRECTORY NAMES".
    """
    if scope == "profile":
        if profile == "release":
            # See THE RELEASE TRAP: `cargo clean --profile release` is the same
            # wipe, spelled with the flag the trap is about.
            return ["cargo", "clean", "--release"]
        return ["cargo", "clean", "--profile", cargo_profile(profile)]
    cmd = ["cargo", "clean"]
    packages = workspace_packages(repo)
    unresolved = [p["member"] for p in packages if not p["resolved"]]
    if unresolved:
        # The emit below would fall back to a directory basename for these, which
        # is the A20 defect. Say so rather than emit a `-p` cargo rejects.
        print(
            "lane-target-guard: WARN could not read [package] name for "
            + ", ".join(unresolved)
            + " — those -p arguments fall back to the directory name",
            file=sys.stderr,
        )
    for name in (p["name"] for p in packages):
        cmd += ["-p", name]
    if profile == "release":
        # See THE RELEASE TRAP above: without this flag the poisoned artifact
        # survives (only the dev profile is cleared) and the guard would lie.
        cmd.append("--release")
    return cmd


def reset(
    repo: Path,
    profile: str,
    scope: str,
    dry_run: bool = False,
    wait: int = 120,
    force: bool = False,
) -> int:
    target = resolve_target_dir(repo)
    # Cross-session arbitration first: never clean under a live build.  See
    # "CROSS-SESSION ARBITRATION" in the module docstring.
    if not force:
        state = wait_for_free_lock(target, profile, wait)
        if state != "free":
            lock = profile_lock_path(target, profile)
            print(
                f"lane-target-guard: REFUSED {profile} — "
                + (
                    f"a cargo build holds {lock}"
                    if state == "held"
                    else f"could not read {lock}, so a live build cannot be ruled out"
                ),
                file=sys.stderr,
            )
            cwds = live_build_cwds() if state == "held" else []
            if cwds:
                print("lane-target-guard:   live cargo cwd: " + ", ".join(cwds), file=sys.stderr)
            print(
                "lane-target-guard:   NOTHING WAS CLEANED. Re-run when that build finishes "
                "(another session or worktree is using .kilo/target-shared). Do not set "
                "MARULA_LANE_GUARD=0 — it also skips the mark steps.",
                file=sys.stderr,
            )
            return EXIT_REFUSED
    cmd = reset_command(repo, profile, scope)
    where = (
        "the whole profile (the flagged writes land outside the workspace members)"
        if scope == "profile"
        else "the workspace members"
    )
    print("lane-target-guard: reset — " + " ".join(cmd) + f"   [{where}]")
    if dry_run:
        return 0
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
        "build_tools": build_tools(repo),
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
    # With --json, stdout carries only JSON (one document per profile) and the
    # human progress lines go to stderr, so a machine consumer never sees
    # "Extra data".
    sink = sys.stderr if args.json else sys.stdout
    rc = 0
    for profile in profiles_for(args, target):
        result = verdict(repo, target, profile)
        result["repo"] = str(repo)
        scope = scope_for_status(result["status"])
        if result["action"] == "reset":
            result["fix_scope"] = scope
            result["fix"] = reset_command(repo, profile, scope)
            if not args.fix:
                result["fix_blocked_by_live_build"] = (
                    not args.force and profile_build_state(target, profile) != "free"
                )
        emit(result, args.json)
        if result["action"] == "reset":
            if args.fix:
                print(f"lane-target-guard: fixing — {result['detail']}", file=sink)
                rrc = reset(
                    repo, profile, scope,
                    dry_run=args.dry_run, wait=args.wait, force=args.force,
                )
                if rrc == EXIT_REFUSED:
                    print(
                        "lane-target-guard: fix withheld — nothing was cleaned (see REFUSED above).",
                        file=sys.stderr,
                    )
                    rc = EXIT_REFUSED
                elif rrc != 0:
                    print(f"lane-target-guard: FAIL cargo clean exited {rrc}", file=sys.stderr)
                    rc = 1
                else:
                    print(
                        "lane-target-guard: reset done — the build that follows compiles from "
                        "THIS tree's sources.",
                        file=sink,
                    )
            else:
                print("lane-target-guard: FIX — " + " ".join(result["fix"]), file=sink)
                if result["fix_blocked_by_live_build"]:
                    print(
                        "lane-target-guard: NOTE — a build is using this profile right now; wait "
                        "for it before running that fix (--force cleans under it instead).",
                        file=sink,
                    )
                rc = 1
        elif result["action"] == "report":
            if profile == "release":
                print(
                    "lane-target-guard: FIX — cargo clean --release (a full release wipe; a "
                    "workspace-member reset cannot reach a dependency rlib built under another SDK), "
                    "then re-run with the same DEVELOPER_DIR on every lane.",
                    file=sink,
                )
            else:
                print(
                    "lane-target-guard: FIX — cargo clean, then re-run with one SDK environment.",
                    file=sink,
                )
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
    return reset(
        repo, args.profile, args.scope, dry_run=args.dry_run, wait=args.wait, force=args.force
    )


def cmd_status(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    target = resolve_target_dir(repo)
    if getattr(args, "print_target", False):
        # Machine-readable form for scripts (dev-clean.sh is the caller): the
        # one line they need, resolved the same way cargo and `check` do.
        print(target)
        return 0
    stamp = load_stamp(target)
    packages = workspace_packages(repo)
    renamed = [p for p in packages if p["name"] != Path(p["member"]).name]
    print(f"lane-target-guard: target {target}")
    print(f"  exists      {target.is_dir()}")
    print(f"  worktree    {branch_of(repo)} (id {worktree_id(repo)})")
    print(f"  members     {len(packages)} ({len(packages) - len(renamed)} named after their dir, "
          f"{len(renamed)} not)")
    for pkg in renamed:
        print(f"    -p {pkg['name']}  ({pkg['member']})")
    unresolved = [pkg["member"] for pkg in packages if not pkg["resolved"]]
    if unresolved:
        print(f"  WARN        no [package] name readable for: {', '.join(unresolved)}")
    print(f"  stamp       {stamp_path(target)}")
    tools = build_tools(repo)
    print(
        "  build tools linker={} wrapper={} overlay={}{}".format(
            tools["linker"] or "stock",
            tools["rustc_wrapper"] or "none",
            tools["overlay"] or "absent",
            ("  REMOTE CACHE: " + ", ".join(tools["remote_cache_vars"]))
            if tools["remote_cache_vars"]
            else "",
        )
    )
    for profile, entry in sorted((stamp.get("profiles") or {}).items()):
        state = profile_build_state(target, profile)
        now = "a build holds it — a reset would refuse" if state != "free" else "free"
        print(
            f"    {profile:8} built {entry.get('built_at', '?')} by {entry.get('branch', '?')} "
            f"(id {entry.get('worktree_id', '?')})  build lock: {now}"
        )
    return 0


# --------------------------------------------------------------------------
# self-test — pure Python, no cargo, no target dir needed
# --------------------------------------------------------------------------
def self_test() -> int:
    import tempfile

    import contextlib
    import io

    failures: list[str] = []
    checks = 0

    def expect(label: str, got, want) -> None:
        nonlocal checks
        checks += 1
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        # A synthetic workspace shaped like the real one: four members across
        # three globs/roots, and TWO whose directory is not their package name
        # — the A20 shape (real: desktop → marula-desktop,
        # plugins/connectors/mapbox → mapbox-routing).
        for src, manifest in (
            ("crates/alpha/src", "[package]\nname = \"alpha\"\n"),
            ("crates/beta/src", "[package]\nname = \"beta\"\n"),
            ("desktop/src", "[package]\nname = \"marula-desktop\"\n"),
            ("plugins/connectors/mapbox/src", "[package]\nname = \"mapbox-routing\"\n"),
        ):
            (root / src).mkdir(parents=True)
            (root / src / "lib.rs").write_text("pub const A: u8 = 1;\n")
            (root / Path(src).parent / "Cargo.toml").write_text(manifest)
        (root / "Cargo.toml").write_text(
            "[workspace]\nmembers = [\"crates/*\", \"desktop\", \"plugins/connectors/*\"]\n",
            encoding="utf-8",
        )
        (root / "Cargo.lock").write_text("# lock\n")

        members = workspace_members(root)
        expect(
            "members parsed",
            members,
            ["crates/alpha", "crates/beta", "desktop", "plugins/connectors/mapbox"],
        )
        pkg_names = set(package_names(root))
        expect(
            "member directory != package name is resolved",
            pkg_names,
            {"alpha", "beta", "marula-desktop", "mapbox-routing"},
        )
        expect("directory basename is never a package name", "desktop" in pkg_names, False)
        # The A20 assertion above compares the emit path against the resolve
        # path, so it also has to assert the resolve path itself did not fall
        # back to a basename. Otherwise a total fallback passes both.
        expect(
            "every member package name came from its manifest",
            [p["member"] for p in workspace_packages(root) if not p["resolved"]],
            [],
        )

        # target-dir resolution: config, then env override.
        (root / ".cargo").mkdir()
        (root / ".cargo/config.toml").write_text(
            '[build]\ntarget-dir = "shared-target"\n'
        )
        expect("target-dir from config", resolve_target_dir(root), root / "shared-target")
        os.environ["CARGO_TARGET_DIR"] = str(root / "env-target")
        expect("target-dir from env", resolve_target_dir(root), root / "env-target")
        del os.environ["CARGO_TARGET_DIR"]

        # share-target: the key rides in the include, because cargo ranks the
        # including file above it. Resolve the chain the way cargo merges it.
        (root / ".cargo/config.toml").write_text(
            'include = [{ path = "overlay.toml", optional = true }]\n'
        )
        expect("no key, no overlay -> cargo default", resolve_target_dir(root), root / "target")
        (root / ".cargo/overlay.toml").write_text('[build]\ntarget-dir = "joined"\n')
        expect("target-dir from the include", resolve_target_dir(root), root / "joined")
        (root / ".cargo/config.toml").write_text(
            'include = [{ path = ".cargo/overlay.toml", optional = true }]\n'
            '[build]\ntarget-dir = "shared-target"\n'
        )
        expect("including file beats the include", resolve_target_dir(root), root / "shared-target")

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
        for rel in ("crates", "desktop", "plugins"):
            shutil.copytree(root / rel, peer / rel)
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

        # ---- (E) P13 build tools: recorded in the stamp, never judged -----
        # An overlay naming a linker and a wrapper is what a fast machine has.
        # It must reach the stamp and must NOT perturb the verdict: a lane with
        # lld and a lane without still share artifacts.
        (root / ".cargo/p13-fast.local.toml").write_text(
            '[build]\nrustc-wrapper = "sccache"\n\n'
            "[target.'cfg(target_os = \"macos\")']\n"
            'rustflags = ["-C", "link-arg=-fuse-ld=/opt/homebrew/opt/lld/bin/ld64.lld"]\n'
        )
        tools = build_tools(root)
        expect("overlay recorded", tools["overlay"], ".cargo/p13-fast.local.toml")
        expect("linker recorded", tools["linker"], "/opt/homebrew/opt/lld/bin/ld64.lld")
        expect("wrapper recorded", tools["rustc_wrapper"], "sccache")
        expect("no remote cache by default", tools["remote_cache_vars"], [])
        os.environ["SCCACHE_REDIS"] = "redis://example:6379"
        expect(
            "an exported remote cache is reported",
            build_tools(root)["remote_cache_vars"],
            ["SCCACHE_REDIS"],
        )
        del os.environ["SCCACHE_REDIS"]
        mark(root, target, "release", "self-test tools")
        entry = load_stamp(target)["profiles"]["release"]
        expect(
            "the stamp carries the tools",
            (entry.get("build_tools") or {}).get("linker"),
            "/opt/homebrew/opt/lld/bin/ld64.lld",
        )
        # The verdict with the tools must equal the verdict without them: that
        # is the whole "recorded, never judged" claim.
        with_tools = verdict(root, target, "release")["status"]
        (root / ".cargo/p13-fast.local.toml").unlink()
        mark(root, target, "release", "self-test stock")
        without_tools = verdict(root, target, "release")["status"]
        expect("tools do not change the verdict", with_tools, without_tools)
        # And the overlay is not part of the source fingerprint, so a peer with
        # the same sources and a different overlay is safe-shared, not foreign.
        expect(
            "the overlay is not source",
            "p13-fast" in source_fingerprint(root, workspace_members(root)),
            False,
        )

        # The reset commands carry the whole repair, so assert their shape.
        # Two invariants matter:
        #   * every `-p` is a REAL workspace package name (A20 — the old
        #     `-p <dir basename>` emitted `-p desktop`, and cargo exits 101);
        #   * the scope reaches the state the stamp test demands (A20 B).
        argv: list[str] = []

        def fake_run(cmd, cwd=None, **kwargs):
            # Only cargo invocations are the command under test; the arbiter's
            # pgrep/lsof probes also go through subprocess.run and must not
            # pollute the recorded argv.
            if isinstance(cmd, (list, tuple)) and cmd and cmd[0] == "cargo":
                argv.extend(cmd)

            class R:
                returncode = 0
                stdout = ""
                stderr = ""

            return R()

        def emitted() -> list[str]:
            return [argv[i + 1] for i, tok in enumerate(argv) if tok == "-p"]

        real_run = subprocess.run
        subprocess.run = fake_run  # type: ignore[assignment]
        try:
            # foreign-source, release: members only, plus the trap flag.
            reset(root, "release", "members", dry_run=False, wait=0)
            expect("release member reset clears first", argv[:2], ["cargo", "clean"])
            expect(
                "release member reset passes --release (the 2026-10-16 trap)",
                argv[-1] if argv else None,
                "--release",
            )
            expect("release member reset has one -p per member", argv.count("-p"), len(members))
            expect(
                "every -p resolves to a real workspace package name",
                [spec for spec in emitted() if spec not in pkg_names],
                [],
            )
            expect(
                "the dir != package member emits the PACKAGE name",
                "marula-desktop" in emitted(),
                True,
            )
            expect(
                "the dir != package member never emits its directory",
                "desktop" in emitted(),
                False,
            )

            # foreign-source, debug: still surgical, still no --release.
            argv.clear()
            reset(root, "debug", "members", dry_run=False, wait=0)
            expect("debug member reset has no --release", "--release" in argv, False)

            # stale-stamp / unknown: a member clean cannot reach the newer
            # third-party writes, so the WHOLE profile goes — the state the
            # stamp test demands (lane 19 measured 6697 files, 1.9GiB).
            argv.clear()
            reset(root, "debug", scope_for_status("stale-stamp"), dry_run=False, wait=0)
            expect(
                "stale-stamp reset clears the whole dev profile",
                argv,
                ["cargo", "clean", "--profile", "dev"],
            )
            argv.clear()
            reset(root, "debug", scope_for_status("unknown"), dry_run=False, wait=0)
            expect(
                "unknown reset clears the whole profile too",
                argv,
                ["cargo", "clean", "--profile", "dev"],
            )
            argv.clear()
            reset(root, "release", scope_for_status("stale-stamp"), dry_run=False, wait=0)
            expect(
                "stale-stamp release reset is release-scoped",
                argv,
                ["cargo", "clean", "--release"],
            )
            argv.clear()
            reset(root, "debug", scope_for_status("foreign-source"), dry_run=False, wait=0)
            expect("foreign-source reset stays member-scoped", argv.count("-p"), len(members))

            # cargo has no `debug` profile, so a whole-profile clean must name
            # the profile cargo accepts.
            expect("cargo profile name for debug is dev", cargo_profile("debug"), "dev")
            expect("cargo profile name for test is dev", cargo_profile("test"), "dev")
            expect("cargo profile name for release is release", cargo_profile("release"), "release")

            # ---- (D) cross-session arbitration ---------------------------
            # Cargo's own per-profile build lock is the arbiter.  A second real
            # process holds the flock exactly as a live build does.
            lock_file = target / "release/.cargo-lock"
            expect("an absent lock file reads free", profile_build_state(target, "release"), "free")
            expect("an absent lock file cannot be probed", lock_held(lock_file), None)

            lock_file.write_text("")  # cargo's own name, created on first use
            expect("an unheld lock file reads free", lock_held(lock_file), False)
            expect(
                "an unheld lock file is a free profile",
                profile_build_state(target, "release"),
                "free",
            )

            holder = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import fcntl,sys,time\n"
                    "f = open(sys.argv[1], 'r+')\n"
                    "fcntl.flock(f, fcntl.LOCK_EX)\n"
                    "sys.stdout.write('held\\n')\n"
                    "sys.stdout.flush()\n"
                    "time.sleep(120)\n",
                    str(lock_file),
                ],
                stdout=subprocess.PIPE,
                text=True,
            )
            try:
                expect("the holder announces itself", holder.stdout.readline().strip(), "held")
                expect("a held lock reads held", lock_held(lock_file), True)
                expect("a held lock is a held profile", profile_build_state(target, "release"), "held")
                # a build in one profile must not block the other profile
                expect(
                    "a release build does not block the debug profile",
                    profile_build_state(target, "debug"),
                    "free",
                )

                argv.clear()
                rc = reset(root, "release", "members", dry_run=False, wait=0)
                expect("a held lock REFUSES the reset", rc, EXIT_REFUSED)
                expect("a refused reset runs no cargo at all", argv, [])

                argv.clear()
                rc = reset(root, "release", scope_for_status("stale-stamp"), dry_run=False, wait=0)
                expect("a held lock refuses a whole-profile reset too", rc, EXIT_REFUSED)
                expect("the refused whole-profile reset cleaned nothing", argv, [])

                argv.clear()
                rc = reset(root, "release", "members", dry_run=False, wait=1)
                expect("a wait that times out still REFUSES", rc, EXIT_REFUSED)
                expect("a timed-out wait runs no cargo", argv, [])

                # --force is the one explicit override to clean under a build
                argv.clear()
                rc = reset(root, "release", "members", dry_run=False, wait=0, force=True)
                expect("--force resets despite a held lock", rc, 0)
                expect("--force runs the clean", argv[:2], ["cargo", "clean"])

                # "cannot tell" must refuse, never proceed silently
                real_lock_held, real_running = lock_held, any_cargo_running
                globals()["lock_held"] = lambda _path: None
                globals()["any_cargo_running"] = lambda: False
                try:
                    expect(
                        "an unreadable lock with no cargo is unknown",
                        profile_build_state(target, "release"),
                        "unknown",
                    )
                    argv.clear()
                    rc = reset(root, "release", "members", dry_run=False, wait=0)
                    expect("unknown REFUSES (never a silent proceed)", rc, EXIT_REFUSED)
                    expect("an unknown-state reset runs no cargo", argv, [])
                finally:
                    globals()["lock_held"] = real_lock_held
                    globals()["any_cargo_running"] = real_running
            finally:
                holder.kill()
                holder.wait()

            expect("after the holder exits the lock reads free", lock_held(lock_file), False)
            argv.clear()
            rc = reset(root, "release", "members", dry_run=False, wait=0)
            expect("a free lock resets", rc, 0)
            expect("a free lock runs the clean", argv[:2], ["cargo", "clean"])

            # A transient build must not hard-fail the boot: with a wait budget,
            # the guard polls the lock and resets once it frees.  (Without this,
            # "skip the wait and refuse at once" would pass every other check.)
            short = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import fcntl,sys,time\n"
                    "f = open(sys.argv[1], 'r+')\n"
                    "fcntl.flock(f, fcntl.LOCK_EX)\n"
                    "sys.stdout.write('held\\n')\n"
                    "sys.stdout.flush()\n"
                    "time.sleep(1)\n",
                    str(lock_file),
                ],
                stdout=subprocess.PIPE,
                text=True,
            )
            try:
                expect("the short holder announces itself", short.stdout.readline().strip(), "held")
                argv.clear()
                rc = reset(root, "release", "members", dry_run=False, wait=5)
                expect("a transient build is waited out, then reset", rc, 0)
                expect("the waited-out reset runs the clean", argv[:2], ["cargo", "clean"])
            finally:
                short.wait()
        finally:
            subprocess.run = real_run  # type: ignore[assignment]

        # --json must leave stdout as parseable JSON even when the verdict wants
        # a reset — the FIX line used to be appended to the document, so
        # `json.load` saw "Extra data".  Real subprocesses here (no fake_run).
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc_json = main(["check", "--profile", "release", "--json", "--repo", str(root)])
        try:
            parsed = json.loads(buf.getvalue())
            json_ok = isinstance(parsed, dict) and parsed.get("action") == "reset"
        except ValueError:
            json_ok = False
        expect("--json emits one parseable document", json_ok, True)
        expect("--json with a pending reset exits non-zero", rc_json, 1)

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
    check.add_argument(
        "--force",
        action="store_true",
        help="reset even while a build holds this profile's lock (you accept breaking it)",
    )
    check.add_argument("--wait", type=int, default=int(os.environ.get("MARULA_LANE_GUARD_WAIT", 120)))
    check.set_defaults(func=cmd_check)

    markp = sub.add_parser("mark", help="record this worktree as the profile's last builder")
    add_profile(markp)
    markp.add_argument("--note", default="")
    markp.set_defaults(func=cmd_mark)

    rs = sub.add_parser("reset", help="clear what a verdict flagged for a profile")
    add_profile(rs)
    rs.add_argument(
        "--scope",
        default="profile",
        choices=["profile", "members"],
        help=(
            "profile (default) clears the whole profile — the state the stamp test demands; "
            "members clears only the workspace members and keeps the shared third-party artifacts"
        ),
    )
    rs.add_argument("--dry-run", action="store_true")
    rs.add_argument(
        "--force",
        action="store_true",
        help="reset even while a build holds this profile's lock (you accept breaking it)",
    )
    rs.add_argument("--wait", type=int, default=int(os.environ.get("MARULA_LANE_GUARD_WAIT", 120)))
    rs.set_defaults(func=cmd_reset)

    st = sub.add_parser("status", help="show the stamp and what this tree would see")
    st.add_argument("--repo", default=str(repo_root()))
    st.add_argument(
        "--print-target",
        action="store_true",
        help="print only the resolved target dir (for scripts)",
    )
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
