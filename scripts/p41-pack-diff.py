#!/usr/bin/env python3
"""Wave 41 lane 15 — pack conformance diff.

Compares a captured PNG against the pack PNG it was captured for, at the pack's
own recorded size. Reports a divergence score per state so the vision review can
be pointed at the worst states rather than at all 434.

Scores are structural, not aesthetic: a high score means the capture and the
pack disagree about where ink is, which is what a fidelity audit can act on.
Reading a score still needs the ≤8-image vision batches.

  python3 scripts/p41-pack-diff.py [--pack workbench|surface] [--limit N]
"""
import argparse
import json
import os
import sys

from PIL import Image, ImageChops, ImageStat

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACKS = {
    "workbench": os.path.expanduser("~/Documents/GitHub/Marula/marula-core/design/mockups/marula-app-mockup-images"),
    "surface": "/Users/iordn/Documents/GitHub/Marula/brand/packs/core-surface-pack",
}
CAP = os.path.join(REPO, "design/qa-p41-leftovers-closeout/captures")


def load(path):
    try:
        return Image.open(path).convert("RGB")
    except Exception:
        return None


def score(pack_img, cap_img):
    """Resize to one size, then measure ink disagreement."""
    size = pack_img.size
    cap = cap_img.resize(size, Image.LANCZOS) if cap_img.size != size else cap_img
    diff = ImageChops.difference(pack_img, cap)
    stat = ImageStat.Stat(diff)
    mean = sum(stat.mean) / 3.0
    # Normalised mean absolute difference over 0..255.
    mad = mean / 255.0
    # Ink extent: how much of the frame moved at all.
    gray = diff.convert("L")
    hist = gray.histogram()
    moved = sum(hist[8:]) / float(size[0] * size[1])
    return mad, moved


def resolve_pack_png(pack, state_id):
    """Find the pack PNG a capture was aimed at.

    The mapping is explicit rather than inferred: a capture id names the pack
    state it was captured for, and the pack files its own states under a
    surface the id does not carry. Guessing here would silently compare a
    capture against the wrong PNG, so every row is declared.
    """
    want, theme = state_id.split("--")
    # {t} is the pack's canvas folder: neutral for the default web canvas,
    # dark for dark mode, sand for the cream (marula-warm) column.
    t = "dark" if theme == "dark" else "neutral"
    SURFACE = {
        "tickets-list": f"web/tickets/{t}/states/01-default.png",
        "today-default": f"web/today/{t}/states/01-today-default.png",
        "dispatch-default": f"web/dispatch/{t}/states/01-board-rest.png",
        "approvals-default": f"web/approvals/{t}/states/01-queue-no-grant.png",
        "clients-default": f"web/clients/{t}/states/01-clients-list.png",
        "knowledge-default": f"web/knowledge/{t}/states/K01.png",
        "settings-default": f"web/settings/{t}/states/01-settings-shell.png",
        "workflow-default": f"web/workflow/{t}/states/W01.png",
        "quoting-default": f"web/quoting/{t}/states/Q01.png",
        "portal-default": "portal/sand/states/01-login.png",
        "auth-default": f"auth/{t}/states/01-login.png",
    }
    WORKBENCH = {
        "01-workbench-reply-default": f"desktop/{t}/states/01-workbench-reply-default.png",
        "02-timeline-scrolled-to-top": f"desktop/{t}/states/02-timeline-scrolled-to-top.png",
        "03-composer-internal-note": f"desktop/{t}/states/03-composer-internal-note.png",
        "04-internal-note-added": f"desktop/{t}/states/04-internal-note-added.png",
        "05-reply-sent-status-waiting": f"desktop/{t}/states/05-reply-sent-status-waiting.png",
        "06-status-set-resolved": f"desktop/{t}/states/06-status-set-resolved.png",
        "07-filter-client": f"desktop/{t}/states/07-filter-client.png",
        "08-filter-internal": f"desktop/{t}/states/08-filter-internal.png",
        "09-filter-agent": f"desktop/{t}/states/09-filter-agent.png",
        "10-assistant-script-ran": f"desktop/{t}/states/10-assistant-script-ran.png",
        "11-assistant-suggestion-dismissed": f"desktop/{t}/states/11-assistant-suggestion-dismissed.png",
        "12-assistant-question-answered": f"desktop/{t}/states/12-assistant-question-answered.png",
        "13-timer-paused": f"desktop/{t}/states/13-timer-paused.png",
        "14-quick-note-typed": f"desktop/{t}/states/14-quick-note-typed.png",
        "15-note-lines-open": f"desktop/{t}/states/15-note-lines-open.png",
        "16-note-line-added": f"desktop/{t}/states/16-note-line-added.png",
        "17-keyboard-focus-on-send": f"desktop/{t}/states/17-keyboard-focus-on-send.png",
        "18-wrap-up-resolve": f"desktop/{t}/states/18-wrap-up-resolve.png",
        "19-wrap-up-waiting-on-client": f"desktop/{t}/states/19-wrap-up-waiting-on-client.png",
        "20-wrap-up-schedule-follow-up": f"desktop/{t}/states/20-wrap-up-schedule-follow-up.png",
        "21-wrap-up-keep-working": f"desktop/{t}/states/21-wrap-up-keep-working.png",
        "22-wrap-up-completed-resolved": f"desktop/{t}/states/22-wrap-up-completed-resolved.png",
    }
    if want in SURFACE:
        rel = SURFACE[want]
    elif pack == "workbench":
        if want not in WORKBENCH:
            return None
        rel = WORKBENCH[want].format(t=t)
    else:
        return None
    p = os.path.join(PACKS[pack], rel)
    return p if os.path.exists(p) else None


def main():
    global CAP
    ap = argparse.ArgumentParser()
    ap.add_argument("--pack", default=None)
    ap.add_argument("--limit", type=int, default=60)
    # Overridable so a later lane can score its own captures without editing
    # this tool (Wave 47 lane A13). The run record the diff reads is
    # <capture-dir>/_run.json; the report is written beside it.
    ap.add_argument("--capture-dir", default=None)
    a = ap.parse_args()
    if a.capture_dir:
        CAP = a.capture_dir if os.path.isabs(a.capture_dir) else os.path.join(REPO, a.capture_dir)

    run = json.load(open(os.path.join(CAP, "_run.json")))
    rows = []
    for r in run["results"]:
        if not r["ok"]:
            continue
        # r["file"] is <out>/<pack>/<state>.png; the id alone carries no pack.
        parts = r["file"].replace(os.sep, "/").split("/")
        pack = parts[-2]
        state_id = parts[-1].replace(".png", "")
        cap = load(r["file"])
        if cap is None:
            continue
        png = resolve_pack_png(pack, state_id)
        if png is None:
            rows.append({"state": state_id, "pack": pack, "mad": None, "moved": None, "pack_png": None, "status": "no-pack-png"})
            continue
        pk = load(png)
        if pk is None:
            rows.append({"state": state_id, "pack": pack, "mad": None, "moved": None, "pack_png": png, "status": "pack-unreadable"})
            continue
        mad, moved = score(pk, cap)
        rows.append({
            "state": state_id, "pack": pack, "mad": round(mad, 4), "moved": round(moved, 4),
            "pack_png": os.path.relpath(png, REPO), "cap": r["file"],
            "sizes": [pk.size, cap.size], "status": "compared",
        })

    done = [r for r in rows if r["status"] == "compared"]
    nodiff = [r for r in rows if r["status"] != "compared"]
    done.sort(key=lambda r: -r["mad"])

    print(f"=== pack conformance diff ({len(done)} compared, {len(nodiff)} without a pack PNG) ===\n")
    if nodiff:
        for r in nodiff:
            print(f"  [skip] {r['state']}: {r['status']}")
        print()
    print(f"  {'state':46s} {'MAD':>7s} {'ink':>6s}  verdict")
    print("  " + "-" * 74)
    for r in done[: a.limit]:
        verdict = "aligned" if r["mad"] < 0.02 else ("diverged" if r["mad"] < 0.10 else "MATERIALLY DIVERGED")
        print(f"  {r['state']:46s} {r['mad']:7.4f} {r['moved']:6.3f}  {verdict}")
    print()
    out = os.path.join(os.path.dirname(CAP), "PACK-DIFF.json")
    json.dump({"generated": __import__("datetime").datetime.utcnow().isoformat() + "Z", "rows": rows}, open(out, "w"), indent=2)
    print(f"full machine-readable record → {os.path.relpath(out, REPO)}")


if __name__ == "__main__":
    sys.exit(main())
