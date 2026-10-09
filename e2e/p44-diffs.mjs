// Wave 44 lane 1 `p44-ui-today` — capture-vs-mockup diff composites.
//
// The canonical surface-pack PNGs (brand/marula-mockups/marula-surface-pack/
// web/today/) were removed from disk by another lane's cleanup; the same
// seven states, same names, survive vendored in the p41 lane's evidence
// (design/qa-p41-ui-kit-sweep/explore/today/) and are the reference here.
//
// For each state × theme: a side-by-side (mockup left, capture right) and
// an amplified difference heatmap with a coverage percentage. Captures are
// 1440×1000 viewport shots; the mockup canvas is 1600×994 — the capture is
// scaled to the mockup width so the geometry lines up.
import { mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";

const OUT = "design/qa-p44-today/diffs";
const PACK = "design/qa-p41-ui-kit-sweep/explore/today";
mkdirSync(OUT, { recursive: true });

// state number → [pack file, capture suffix]
const STATES = [
  ["01", "01-today-default.png", "01-today-default.png"],
  ["02", "02-quick-log-default.png", "02-quick-log-default.png"],
  ["03", "03-quick-log-corrected.png", "03-quick-log-corrected.png"],
  ["04", "04-quick-log-non-billable.png", "04-quick-log-non-billable.png"],
  ["05", "05-quick-log-error.png", "05-quick-log-error.png"],
  ["06", "06-after-logging.png", "06-after-logging.png"],
  ["07", "07-quiet-day.png", "07-day-empty.png"],
];

const results = [];
for (const [num, packFile, capSuffix] of STATES) {
  for (const theme of ["light", "dark"]) {
    const out = execFileSync(
      "python3",
      [
        "-c",
        `
from PIL import Image, ImageChops, ImageDraw
pack = Image.open("${PACK}/${packFile}").convert("RGB")
cap = Image.open("design/qa-p44-today/captures/${theme}-${capSuffix}").convert("RGB")
W = pack.width
cap = cap.resize((W, int(cap.height * W / cap.width)))
# crop the capture to the mockup's viewport height (the capture also has a
# taller page) so the heatmap compares the same canvas
h = pack.height
cap = cap.crop((0, 0, W, min(h, cap.height)))
if cap.height < h:
    pad = Image.new("RGB", (W, h), (255, 255, 255))
    pad.paste(cap, (0, 0)); cap = pad
diff = ImageChops.difference(pack, cap)
# Metric on the LAYOUT, not the raster: downscale both to a small grayscale
# canvas so viewport/AA/font differences don't drown the signal.
sg = 320
p_small = pack.convert("L").resize((sg, int(sg * h / W)))
c_small = cap.convert("L").resize((sg, int(sg * h / W)))
sd = ImageChops.difference(p_small, c_small)
hist = sd.histogram()
changed = sum(hist[10:]) / (p_small.width * p_small.height)
# heatmap at full res: identical pixels black, differences bright
b = diff.convert("L")
ba = b.point(lambda v: min(255, v * 3))
heat = Image.merge("RGB", [ba, ba.point(lambda v: int(v * 0.85)), ba.point(lambda v: int(max(0, 255 - v * 6)))])
# side by side with labels
label = 28
canvas = Image.new("RGB", (W * 2 + 16, h + label), (240, 240, 240))
d = ImageDraw.Draw(canvas)
canvas.paste(pack, (0, label))
canvas.paste(cap, (W + 16, label))
d.rectangle([0, label, W, label + h], outline=(180, 180, 180))
d.rectangle([W + 16, label, W * 2 + 16, label + h], outline=(180, 180, 180))
d.text((8, 6), "MOCKUP ${packFile}", fill=(40, 40, 40))
d.text((W + 24, 6), "CAPTURE ${theme} (${capSuffix})", fill=(40, 40, 40))
heat.save("${OUT}/${theme}-${num}-diff.png")
canvas.save("${OUT}/${theme}-${num}-side-by-side.png")
print(round(changed * 100, 1))
`,
      ],
      { encoding: "utf8" },
    ).trim();
  results.push(`${theme} ${num}: ${out}% pixels differ`);
  }
}
console.log(results.join("\n"));
