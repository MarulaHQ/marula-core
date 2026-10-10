// Wave 44 lane 8 `p44-ui-insight` — capture-vs-pack composites.
//
// Pack references are the vendored copies in the kit-sweep's evidence dir
// (the canonical surface-pack PNGs were removed from disk by another lane's
// cleanup; the same states, same names, survive there):
//   tickets-list/01-default            — the TL data-surface frame
//   tickets-list/10-empty-no-results   — the empty-with-filters state
//   dispatch/18-planning-loaded        — the Planning-card composition
//
// For each pair × theme: a side-by-side (pack left, capture right) plus an
// amplified difference heatmap with a layout-change percentage. The metric
// is computed on a small grayscale canvas so viewport, AA and font
// differences do not drown the structural signal.
import { mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";

const OUT = "design/qa-p44-ui-insight/diffs";
const PACK = "design/qa-p41-ui-kit-sweep/explore";
const CAPS = "design/qa-p44-ui-insight/captures";
mkdirSync(OUT, { recursive: true });

// [capture state, pack area, pack file]
const PAIRS = [
  ["02-reports-saved", "tickets-list", "01-default.png"],
  ["10-audit-default", "tickets-list", "01-default.png"],
  ["09-provenance-filtered", "tickets-list", "10-empty-no-results.png"],
  ["04-dashboards-default", "dispatch", "18-planning-loaded.png"],
];

const results = [];
for (const [state, area, packFile] of PAIRS) {
  for (const theme of ["light", "dark"]) {
    const pct = execFileSync(
      "python3",
      [
        "-c",
        `
from PIL import Image, ImageChops, ImageDraw
pack = Image.open("${PACK}/${area}/${packFile}").convert("RGB")
cap = Image.open("${CAPS}/${theme}-${state}.png").convert("RGB")
W = pack.width
cap = cap.resize((W, int(cap.height * W / cap.width)))
h = pack.height
cap = cap.crop((0, 0, W, min(h, cap.height)))
if cap.height < h:
    pad = Image.new("RGB", (W, h), (255, 255, 255))
    pad.paste(cap, (0, 0)); cap = pad
diff = ImageChops.difference(pack, cap)
sg = 320
p_small = pack.convert("L").resize((sg, int(sg * h / W)))
c_small = cap.convert("L").resize((sg, int(sg * h / W)))
sd = ImageChops.difference(p_small, c_small)
hist = sd.histogram()
changed = sum(hist[10:]) / (p_small.width * p_small.height)
b = diff.convert("L")
ba = b.point(lambda v: min(255, v * 3))
heat = Image.merge("RGB", [ba, ba.point(lambda v: int(v * 0.85)), ba.point(lambda v: int(max(0, 255 - v * 6)))])
label = 28
canvas = Image.new("RGB", (W * 2 + 16, h + label), (240, 240, 240))
d = ImageDraw.Draw(canvas)
canvas.paste(pack, (0, label))
canvas.paste(cap, (W + 16, label))
d.rectangle([0, label, W, label + h], outline=(180, 180, 180))
d.rectangle([W + 16, label, W * 2 + 16, label + h], outline=(180, 180, 180))
d.text((8, 6), "PACK ${area}/${packFile}", fill=(40, 40, 40))
d.text((W + 24, 6), "CAPTURE ${theme} ${state}", fill=(40, 40, 40))
heat.save("${OUT}/${theme}-${state}-diff.png")
canvas.save("${OUT}/${theme}-${state}-side-by-side.png")
print(round(changed * 100, 1))
`,
      ],
      { encoding: "utf8" },
    ).trim();
    results.push(`${theme} ${state} vs ${area}/${packFile}: ${pct}% layout differs`);
  }
}
console.log(results.join("\n"));
