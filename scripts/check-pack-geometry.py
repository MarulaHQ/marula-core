#!/usr/bin/env python3
"""Diff the lane's ledger geometry against the surface pack's own numbers.

The pack ships the HTML each mockup was rendered from
(`brand/packs/core-surface-pack/source/*.html`), so the rendered
ground truth is readable as measurements rather than only as a picture. This
compares the values that must agree — the density scale, the pill box, the
chip box, the toolbar row, the table's header and cell insets, and the
selected-row treatment — and exits non-zero on a mismatch.

Run: python3 scripts/check-pack-geometry.py
"""
from __future__ import annotations

import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "apps/web/app/ledger.css"

# The pack was moved out of the repo on 2026-10-10: the canonical copy now lives
# in the private org home at brand/packs/core-surface-pack, and the old
# brand/marula-mockups/marula-surface-pack path is stale. Resolve it the same way
# check-surface-pack.mjs does — an env override, then a walk up from the repo,
# because a worktree lane's org home is several levels above its cwd.
PACK_CANDIDATES = [
    Path(__file__).resolve().parent.parent / "brand/packs/core-surface-pack/source/tickets-list.html",
    REPO / "brand/marula-mockups/marula-surface-pack/source/tickets-list.html",
]
for _up in range(6):
    _p = Path(__file__).resolve()
    for _ in range(_up + 1):
        _p = _p.parent
    PACK_CANDIDATES.append(_p / "brand/packs/core-surface-pack/source/tickets-list.html")
PACK = Path(os.environ["MARULA_PACK_ROOT"]) / "source/tickets-list.html" \
    if os.environ.get("MARULA_PACK_ROOT") else next((c for c in PACK_CANDIDATES if c.exists()), PACK_CANDIDATES[0])

# (label, the pack's declared value, what the lane must resolve it to)
EXPECTED: list[tuple[str, str, str]] = [
    ("density comfortable --ctl", "44px", "--m-density-control"),
    ("density comfortable --row", "48px", "--m-density-row"),
    ("density comfortable --hdr", "44px", "--m-ledger-hdr"),
    ("density comfortable --pill", "24px", "--m-ledger-pill"),
    ("density compact --pill", "22px", "compact --m-ledger-pill"),
    ("density dense --pill", "20px", "dense --m-ledger-pill"),
    ("density compact --hdr", "40px", "compact --m-ledger-hdr"),
    ("density dense --hdr", "32px", "dense --m-ledger-hdr"),
]


def pack_css() -> str:
    return PACK.read_text(encoding="utf-8")


def ledger_css() -> str:
    return LEDGER.read_text(encoding="utf-8")


def block(css: str, selector: str, prop: str) -> str | None:
    """The property inside the first rule whose selector ends with `selector`.

    The pack ships MINIFIED css (`.pill{...}` with no spaces), so the selector is
    matched loosely and the block is whatever follows the next `{`.
    """
    pattern = re.compile(r"(?<![\w.\-#])" + re.escape(selector) + r"\s*\{([^}]*)\}")
    for match in pattern.finditer(css):
        found = re.search(
            r"(?<![\w-])" + re.escape(prop) + r"\s*:\s*([^;}]+)", match.group(1)
        )
        if found:
            return found.group(1).strip()
    return None


def main() -> int:
    pack = pack_css()
    mine = ledger_css()
    failures: list[str] = []
    notes: list[str] = []

    # 1. The pack's own density block, read straight out of the mockup source.
    for density, expect_ctl, expect_pill in (
        ("comfortable", "44px", "24px"),
        ("compact", "36px", "22px"),
        ("dense", "32px", "20px"),
    ):
        row = re.search(rf'data-density="{density}"\]\{{([^}}]*)\}}', pack)
        if row is None:
            failures.append(f"pack has no [data-density={density}] block")
            continue
        declared = dict(
            (k, v) for k, v in re.findall(r"--([\w-]+):([^;]+)", row.group(1))
        )
        for key, want in (("ctl", expect_ctl), ("pill", expect_pill), ("hdr", None)):
            if want and declared.get(key) != want:
                failures.append(
                    f"pack [data-density={density}] --{key} is {declared.get(key)}, expected {want}"
                )
        notes.append(
            f"pack {density}: ctl={declared.get('ctl')} row={declared.get('row')} "
            f"hdr={declared.get('hdr')} pill={declared.get('pill')} px={declared.get('px')}"
        )

    # 2. This lane must state the same scale.
    for density, pill, hdr in (
        ("comfortable", "24px", "44px"),
        ("compact", "22px", "40px"),
        ("dense", "20px", "32px"),
    ):
        prefix = "" if density == "comfortable" else f':root[data-density="{density}"] '
        got_pill = re.search(
            rf"{re.escape(prefix)}\{{[^}}]*--m-ledger-pill:\s*(\d+px)", mine
        )
        got_hdr = re.search(
            rf"{re.escape(prefix)}\{{[^}}]*--m-ledger-hdr:\s*(\d+px)", mine
        )
        if got_pill is None or got_pill.group(1) != pill:
            failures.append(f"lane {density} --m-ledger-pill is not {pill}")
        if got_hdr is None or got_hdr.group(1) != hdr:
            failures.append(f"lane {density} --m-ledger-hdr is not {hdr}")

    # 3. The pill box, measured off the pack's rule.
    pack_pill = block(pack, ".pill", "gap") or ""
    if pack_pill.replace(" ", "") != "6px":
        failures.append(f"pack .pill gap changed: {pack_pill!r}")
    mine_gap = block(mine, ".m-ledger-sev", "gap")
    if mine_gap != "6px":
        failures.append(f"lane .m-ledger-sev gap is {mine_gap}, pack says 6px")
    mine_pad = block(mine, ".m-ledger-sev", "padding")
    if mine_pad != "0 10px":
        failures.append(f"lane .m-ledger-sev padding is {mine_pad}, pack says 0 10px")
    if "14px" not in (block(mine, ".m-ledger-sev svg", "width") or ""):
        failures.append("lane pill glyph is not the pack's 14px")

    # 4. The attention chip: the pack sizes it --ctl minus 8.
    pack_chip = block(pack, ".chip", "height") or ""
    if "calc(var(--ctl)-8px)" not in pack_chip.replace(" ", ""):
        failures.append(f"pack .chip height changed: {pack_chip!r}")
    mine_chip = block(mine, ".m-ledger-shortcut", "height")
    if (mine_chip or "").replace(" ", "") != "calc(var(--m-density-control)-8px)":
        failures.append(f"lane shortcut chip height is {mine_chip}, pack says --ctl minus 8")

    # 5. The toolbar row: fixed height, one row.
    if "calc(var(--ctl)+16px)" not in (block(pack, ".tb", "height") or "").replace(" ", ""):
        failures.append("pack .tb height is no longer --ctl + 16")
    if "calc(var(--m-density-control) + 16px)" not in mine:
        failures.append("lane toolbar is not pinned to --ctl + 16")
    if "flex-wrap: nowrap" not in mine:
        failures.append("lane toolbar can still wrap; the pack's cannot")

    # 6. Table header treatment.
    pack_th = block(pack, ".th .tc", "font-size") or ""
    if "12px" not in pack_th:
        failures.append(f"pack header type changed: {pack_th!r}")
    for prop, want in (("font-size", "12px"), ("font-weight", "600"), ("letter-spacing", "0.06em")):
        got = block(mine, ".m-ledger-card .m-table th", prop)
        if (got or "").replace(" ", "") != want:
            failures.append(f"lane ledger th {prop} is {got}, pack says {want}")
    mine_pad = block(mine, ".m-ledger-card .m-table td", "padding")
    if mine_pad != "0 12px":
        failures.append(f"lane cell padding is {mine_pad}, pack says 0 var(--px)=12px")

    # 7. The selected row wears --acc, and --m-accent is that value.
    pack_acc = re.search(r"--acc:\s*(#[0-9A-Fa-f]{6})", pack)
    tokens = (REPO / "packages/ui/src/tokens.css").read_text(encoding="utf-8")
    mine_accent = re.search(r"--m-accent:\s*(#[0-9a-f]{6})", tokens)
    if pack_acc and mine_accent:
        if pack_acc.group(1).lower() != mine_accent.group(1).lower():
            failures.append(
                f"--m-accent ({mine_accent.group(1)}) is not the pack's --acc ({pack_acc.group(1)})"
            )
    sel = re.search(r'tbody tr\[data-selected="true"\]\s*\{([^}]*)\}', mine)
    if sel is None or "var(--m-accent)" not in sel.group(1):
        failures.append("lane selected row does not wear --m-accent (the pack's --acc)")

    for line in notes:
        print(f"  · {line}")
    if failures:
        print(f"\n✗ pack geometry: {len(failures)} mismatch(es)")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\n✓ pack geometry: the lane's ledger matches the mockup's own numbers")
    return 0


if __name__ == "__main__":
    sys.exit(main())
