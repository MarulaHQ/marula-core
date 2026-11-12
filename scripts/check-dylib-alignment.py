#!/usr/bin/env python3
"""Find macOS dylibs dyld will refuse to load.

dyld 27.x requires the LC_SYMTAB string table to start on an 8-byte boundary
(`stroff % 8 == 0`). Apple's ld sometimes lands it on a 4 instead, and the
dylib then fails to load with "mis-aligned LINKEDIT string pool". rustc
reports that as unresolved imports in the crate that uses the proc macro, so
the failure reads like a code defect in `sqlx`, not a linker defect.

This walks the dylibs a build left behind and reports the ones that cannot
load, which turns a confusing E0432 into one line naming the file and the
remedy.

Usage:
  scripts/check-dylib-alignment.py [DIR ...]   scan DIRs (default: target dirs)
  scripts/check-dylib-alignment.py --selftest  prove the parser on built files

Exit status: 0 when every dylib can load (or there are none), 1 when at least
one cannot, 2 on usage error. Records without LC_SYMTAB (object files, stubs)
and files that are not Mach-O are skipped silently.
"""

from __future__ import annotations

import argparse
import glob
import os
import struct
import sys
import tempfile

MH_MAGIC_64 = 0xFEEDFACF
MH_CIGAM_64 = 0xCFFAEDFE
FAT_MAGIC = 0xCAFEBABE
FAT_CIGAM = 0xBEBAFECA
FAT_MAGIC_64 = 0xCAFEBABF
LC_SYMTAB = 0x2
ALIGN = 8
REMEDY = (
    'export RUSTFLAGS="-C strip=none" CARGO_TARGET_APPLIES_TO_HOST=true, '
    "then rebuild the crate that owns the dylib (a relink changes the layout)"
)


def _symtab(data: bytes, base: int) -> tuple[int, int] | None:
    """(stroff, nsyms) for a thin Mach-O image at base, or None."""
    if len(data) < base + 32:
        return None
    magic = struct.unpack_from("<I", data, base)[0]
    if magic == MH_CIGAM_64:
        endian = ">"
    elif magic == MH_MAGIC_64:
        endian = "<"
    else:
        return None
    ncmds = struct.unpack_from(endian + "I", data, base + 16)[0]
    off = base + 32
    for _ in range(ncmds):
        if off + 8 > len(data):
            return None
        cmd, size = struct.unpack_from(endian + "II", data, off)
        if size < 8:
            return None
        if cmd == LC_SYMTAB and off + 24 <= len(data):
            stroff, nsyms = struct.unpack_from(endian + "II", data, off + 16)
            return stroff, nsyms
        off += size
    return None


def symtabs(data: bytes) -> list[tuple[int, int]]:
    """Every LC_SYMTAB in a thin or fat Mach-O file."""
    if len(data) < 8:
        return []
    magic = struct.unpack_from(">I", data, 0)[0]
    if magic in (FAT_MAGIC, FAT_MAGIC_64):
        count = struct.unpack_from(">I", data, 4)[0]
        if count == 0 or count > 64:
            return []
        entry = 32 if magic == FAT_MAGIC_64 else 20
        out = []
        for i in range(count):
            start = 8 + i * entry
            if start + entry > len(data):
                break
            if magic == FAT_MAGIC_64:
                offset = struct.unpack_from(">Q", data, start + 8)[0]
            else:
                offset = struct.unpack_from(">I", data, start + 8)[0]
            found = _symtab(data, offset)
            if found:
                out.append(found)
        return out
    found = _symtab(data, 0)
    return [found] if found else []


def scan(paths: list[str]) -> list[tuple[str, int, int]]:
    bad = []
    for path in paths:
        try:
            with open(path, "rb") as fh:
                data = fh.read()
        except OSError:
            continue
        for stroff, nsyms in symtabs(data):
            if stroff % ALIGN != 0:
                bad.append((path, stroff, nsyms))
    return bad


def default_dirs() -> list[str]:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    target = os.environ.get("P13_TARGET_DIR") or os.path.join(
        os.environ.get("CARGO_TARGET_DIR") or os.path.join(root, "target")
    )
    out = []
    for profile in ("debug", "release"):
        deps = os.path.join(target, profile, "deps")
        if os.path.isdir(deps):
            out.append(deps)
    return out


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(add_help=True, description=__doc__.splitlines()[0])
    parser.add_argument("dirs", nargs="*", help="directories to scan (default: target/debug|release/deps)")
    parser.add_argument("--selftest", action="store_true", help="prove the parser on synthetic Mach-O files")
    args = parser.parse_args(argv)

    if args.selftest:
        return selftest()

    dirs = args.dirs or default_dirs()
    if not dirs:
        print("no dylib directories to scan")
        return 0

    paths: list[str] = []
    for directory in dirs:
        paths.extend(glob.glob(os.path.join(directory, "*.dylib")))
    if not paths:
        print("no dylibs to check")
        return 0

    bad = scan(paths)
    if not bad:
        print(f"{len(paths)} dylibs, every LC_SYMTAB aligned to {ALIGN}")
        return 0

    for path, stroff, nsyms in bad:
        print(f"cannot load  {path}  stroff=0x{stroff:x} (mod {ALIGN} = {stroff % ALIGN}), {nsyms} symbols")
    print()
    print(
        f"{len(bad)} of {len(paths)} dylibs cannot be loaded by dyld: the LC_SYMTAB "
        f"string table must start on a {ALIGN}-byte boundary."
    )
    print(f"remedy: {REMEDY}")
    return 1


def _thin(stroff: int) -> bytes:
    """A minimal 64-bit Mach-O with one LC_SYMTAB at stroff."""
    header = struct.pack(
        "<IiiIIIII",
        MH_MAGIC_64,
        0x0100000C,  # CPU_TYPE_ARM64
        0,
        6,  # MH_DYLIB
        1,  # ncmds
        24,  # sizeofcmds
        0,
        0,
    )
    command = struct.pack("<IIIIII", LC_SYMTAB, 24, 0, 0, stroff, 0)
    return header + command


def selftest() -> int:
    failures = 0

    def check(label: str, got: object, want: object) -> None:
        nonlocal failures
        if got != want:
            failures += 1
            print(f"  FAIL {label}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as tmp:
        aligned = os.path.join(tmp, "libaligned.dylib")
        skewed = os.path.join(tmp, "libskewed.dylib")
        with open(aligned, "wb") as fh:
            fh.write(_thin(0x1000))
        with open(skewed, "wb") as fh:
            fh.write(_thin(0x1004))

        check("aligned stroff parses", symtabs(_thin(0x1000)), [(0x1000, 0)])
        check("aligned dylib scans clean", scan([aligned]), [])
        check(
            "skewed dylib is reported",
            [(os.path.basename(p), s) for p, s, _ in scan([skewed])],
            [("libskewed.dylib", 0x1004)],
        )

        # A fat file carries the same check per architecture.
        inner = _thin(0x1004)
        header = struct.pack(">II", FAT_MAGIC, 1)
        entry = struct.pack(">iiIII", 0x0100000C, 0, 0x1000, len(inner), 12)
        fat = header + entry + b"\0" * (0x1000 - len(header) - len(entry)) + inner
        check("fat skewed arch is reported", [stroff for stroff, _ in symtabs(fat)], [0x1004])

        # Non-Mach-O input is skipped, not an error.
        junk = os.path.join(tmp, "junk.dylib")
        with open(junk, "wb") as fh:
            fh.write(b"not a mach-o file")
        check("non-Mach-O is skipped", scan([junk]), [])

    if failures:
        print(f"dylib-alignment selftest: {failures} failure(s)")
        return 1
    print("dylib-alignment selftest: 5 checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
