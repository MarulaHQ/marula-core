#!/usr/bin/env python3
"""Wave 41 lane 15 — vision-review composites.

Builds side-by-side pack-vs-capture grids, at most 8 IMAGES per grid. A grid is
one model request, so the house rule is a hard cap on the image count, not on
the state count: several states may share a grid, but never more than 8 panes.

  python3 scripts/p41-vision-grid.py --pack surface --batch 1 [--out dir]
"""
import argparse
import json
import os

from PIL import Image, ImageDraw

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACKS = {
    "workbench": os.path.expanduser("~/Documents/GitHub/Marula/marula-core/design/mockups/marula-app-mockup-images"),
    "surface": "/Users/iordn/Documents/GitHub/Marula/brand/packs/core-surface-pack",
}
CAP = os.path.join(REPO, "design/qa-p41-ui-audit/captures")

# Declared, never inferred — a capture id names the pack state it was captured
# for and the pack files that state under a path the id does not carry.
SURFACE_PNG = {
    "tickets-list": "web/tickets/{t}/states/01-default.png",
    "today-default": "web/today/{t}/states/01-today-default.png",
    "dispatch-default": "web/dispatch/{t}/states/01-board-rest.png",
    "approvals-default": "web/approvals/{t}/states/01-queue-no-grant.png",
    "clients-default": "web/clients/{t}/states/01-clients-list.png",
    "knowledge-default": "web/knowledge/{t}/states/K01.png",
    "settings-default": "web/settings/{t}/states/01-settings-shell.png",
    "workflow-default": "web/workflow/{t}/states/W01.png",
    "quoting-default": "web/quoting/{t}/states/Q01.png",
    "portal-default": "portal/sand/states/01-login.png",
    "auth-default": "auth/{t}/states/01-login.png",
}
WORKBENCH_PNG = {
    "01-workbench-reply-default": "desktop/{t}/states/01-workbench-reply-default.png",
    "02-timeline-scrolled-to-top": "desktop/{t}/states/02-timeline-scrolled-to-top.png",
    "03-composer-internal-note": "desktop/{t}/states/03-composer-internal-note.png",
    "07-filter-client": "desktop/{t}/states/07-filter-client.png",
    "08-filter-internal": "desktop/{t}/states/08-filter-internal.png",
    "09-filter-agent": "desktop/{t}/states/09-filter-agent.png",
    "15-note-lines-open": "desktop/{t}/states/15-note-lines-open.png",
    "18-wrap-up-resolve": "desktop/{t}/states/18-wrap-up-resolve.png",
}


def grid(states, out, cols=2, pane_w=560):
    """One grid, ≤8 panes. Pack on the left, capture on the right, labelled."""
    rows = (len(states) + cols - 1) // cols
    label_h, gap, head = 22, 8, 26
    pane_h = int(pane_w * 994 / 1600)
    W = cols * pane_w + (cols + 1) * gap
    H = rows * (pane_h + label_h) + (rows + 1) * gap + head
    img = Image.new("RGB", (W, H), (24, 24, 26))
    d = ImageDraw.Draw(img)
    d.text((gap, 8), "  |  ".join(s["label"] for s in states), fill=(230, 224, 205))
    y = head
    for i, s in enumerate(states):
        col, row = i % cols, i // cols
        x = gap + col * (pane_w + gap)
        yy = y + row * (pane_h + label_h + gap)
        for side in (0, 1):
            src = s["pack"] if side == 0 else s["cap"]
            if src and os.path.exists(src):
                im = Image.open(src).convert("RGB").resize((pane_w, pane_h), Image.LANCZOS)
            else:
                im = Image.new("RGB", (pane_w, pane_h), (60, 58, 54))
                ImageDraw.Draw(im).text((10, 10), "missing", fill=(200, 190, 170))
            img.paste(im, (x, yy))
            d.rectangle([x, yy, x + pane_w, yy + pane_h], outline=(90, 86, 76))
        d.text((x, yy + pane_h + 4), f"{s['label']}   [pack | capture]", fill=(232, 184, 58))
    img.save(out)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pack", required=True, choices=["surface", "workbench", "both"])
    ap.add_argument("--batch", type=int, default=1)
    ap.add_argument("--per", type=int, default=4, help="comparisons per grid (8 panes = 4 comparisons)")
    ap.add_argument("--out", default=None)
    a = ap.parse_args()

    diff = json.load(open(os.path.join(os.path.dirname(CAP), "PACK-DIFF.json")))
    rows = [r for r in diff["rows"] if r["status"] == "compared"]
    rows.sort(key=lambda r: -r["mad"])  # worst first: the review earns its keep there
    if a.pack != "both":
        rows = [r for r in rows if r["pack"] == a.pack]

    outdir = a.out or os.path.join(CAP, "review")
    os.makedirs(outdir, exist_ok=True)
    made = []
    for i in range(0, len(rows), a.per):
        chunk = rows[i : i + a.per]
        n = i // a.per + 1
        path = os.path.join(outdir, f"{a.pack}-batch{n:02d}.png")
        grid(
            [{"label": r["state"], "pack": r["pack_png"], "cap": r["cap"]} for r in chunk],
            path,
            cols=2,
        )
        panes = 2 * len(chunk)
        assert panes <= 8, f"grid would carry {panes} panes — the ≤8 cap is a hard limit"
        made.append({"batch": n, "file": os.path.relpath(path, REPO), "panes": panes, "states": [r["state"] for r in chunk]})
    print(json.dumps(made, indent=2))
    print(f"\n{len(made)} grids, ≤8 panes each → {os.path.relpath(outdir, REPO)}")


if __name__ == "__main__":
    main()
