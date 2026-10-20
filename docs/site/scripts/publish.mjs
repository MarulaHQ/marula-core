// publish.mjs — the docs-site publisher (ADR 0009).
//
// Renders every Diátaxis markdown page in docs/site into a static,
// dependency-free HTML publication. The markdown under docs/site remains
// the ONLY source of truth; this script renders it without forking or
// rewriting a single byte. The consumer is the separate site package, which
// lives outside this repository: at build time it copies `docs/site/dist/`
// next to its public assets — until then the bundle is self-contained and
// openable from the filesystem.
//
// Usage:  node docs/site/scripts/publish.mjs [--out <dir>]
//
// Determinism contract (verified by check-docs-site.mjs): same sources ⇒
// byte-identical output. No timestamps, no locale, no directory order.

import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { render } from "./lib/minidown.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const site = join(scriptDir, "..");

// Publishing destination (default docs/site/dist). --out lets the
// site checker publish into a scratch directory for its determinism run.
const outArg = process.argv.indexOf("--out");
const outDir =
  outArg > -1 ? resolve(process.argv[outArg + 1]) : join(site, "dist");

const DIST_CSS = `:root {
  --bg: #faf9f7; --fg: #2a2621; --muted: #756b5e; --border: #e4ddcf;
  --accent: #e05d38; --code-bg: #f3efe7; --link: #b8500f;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #191612; --fg: #ece5da; --muted: #9a8f80; --border: #332d26;
    --accent: #f07046; --code-bg: #221e19; --link: #ff9a68;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--fg);
  font: 16px/1.65 ui-sans-serif, system-ui, "Inter", "Helvetica Neue", Arial, sans-serif;
  -webkit-font-smoothing: antialiased;
}
nav.bar {
  display: flex; flex-wrap: wrap; gap: 1.25rem; padding: 0.75rem 1.5rem;
  border-bottom: 1px solid var(--border); font-size: 0.9rem;
}
nav.bar a { color: var(--muted); text-decoration: none; }
nav.bar a.title { color: var(--fg); font-weight: 650; }
nav.bar a[aria-current] { color: var(--accent); }
main { max-width: 46rem; margin: 0 auto; padding: 1.75rem 1.5rem 4rem; }
h1, h2, h3, h4 { line-height: 1.25; letter-spacing: -0.01em; }
h1 { font-size: 1.9rem; margin: 1.6rem 0 0.9rem; }
h2 { font-size: 1.3rem; margin: 2rem 0 0.6rem; border-bottom: 1px solid var(--border); padding-bottom: 0.3rem; }
h3 { font-size: 1.06rem; margin: 1.5rem 0 0.5rem; }
a { color: var(--link); }
code, pre { font-family: ui-monospace, "JetBrains Mono", Menlo, Consolas, monospace; }
code { background: var(--code-bg); border-radius: 6px; padding: 0.1em 0.35em; font-size: 0.875em; }
pre { background: var(--code-bg); border: 1px solid var(--border); border-radius: 10px; padding: 0.9rem 1.1rem; overflow-x: auto; }
pre code { background: none; padding: 0; font-size: 0.82rem; }
table { border-collapse: collapse; width: 100%; font-size: 0.9rem; margin: 1rem 0; }
th, td { border-bottom: 1px solid var(--border); text-align: left; padding: 0.4rem 0.7rem; vertical-align: top; }
th { color: var(--muted); font-weight: 600; }
blockquote { border-left: 3px solid var(--accent); margin: 1rem 0; padding: 0.2rem 1.1rem; color: var(--muted); }
footer { color: var(--muted); font-size: 0.8rem; margin-top: 3rem; border-top: 1px solid var(--border); padding-top: 1rem; }
@media (prefers-reduced-motion: reduce) { * { transition: none !important; animation: none !important; } }
`;

const SITE_TITLE = "Marula documentation";
const SITE_CSS = "site.css";
const QUADRANTS = [
  ["start", "Start here"],
  ["tutorials", "Tutorials"],
  ["how-to", "How-to guides"],
  ["reference", "Reference"],
  ["explanation", "Explanation"],
];

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : p.endsWith(".md") ? [p] : [];
  });
}

/** A link target's published form, or null when the publication does not
 *  carry the target (repo-native references beyond docs/site). */
function publishedHref(fromFile, target) {
  if (/^(https?:|mailto:|#)/.test(target)) return target;
  const hashAt = target.indexOf("#");
  const path = hashAt === -1 ? target : target.slice(0, hashAt);
  const hash = hashAt === -1 ? "" : target.slice(hashAt);
  if (!path) return target;
  const resolved = join(dirname(fromFile), decodeURIComponent(path));
  if (!existsSync(resolved)) return null; // broken links must not publish
  const insideSite = relative(site, resolved).split("..").length === 1;
  if (!insideSite) return null; // repo-native reference: dropped in dist
  const rel = relative(dirname(fromFile), resolved).split("\\").join("/");
  // Only markdown pages are published; non-page artifacts (.mjs, READMEs
  // of scripts) are dropped to plain text rather than linked dead.
  if (!rel.endsWith(".md")) return null;
  return `${rel.slice(0, -3)}.html${hash}`;
}

function crumb(activeQuadrant, depth) {
  const up = "../".repeat(depth);
  const nav = [`<a class="title" href="${up}index.html">${SITE_TITLE}</a>`];
  for (const [dir, label] of QUADRANTS) {
    nav.push(
      `<a href="${up}${dir}/index.html"${dir === activeQuadrant ? ' aria-current="page"' : ""}>${label}</a>`,
    );
  }
  return `<nav class="bar">${nav.join("")}</nav>`;
}

// --- publish ------------------------------------------------------------------

const pages = walk(site).filter(
  (file) => !relative(site, file).startsWith("scripts"),
);

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, SITE_CSS), DIST_CSS);

let publishedCount = 0;
for (const file of pages) {
  const relSite = relative(site, file).split("\\").join("/");
  const markdown = readFileSync(file, "utf8");
  const title =
    markdown.match(/^#\s+(.+)$/m)?.[1] ??
    (relSite === "README.md" ? "Docs pipeline" : SITE_TITLE);
  const quadrant =
    QUADRANTS.find(([dir]) => relSite.startsWith(`${dir}/`))?.[0] ?? null;
  const depth = relSite.split("/").length - 1;

  const htmlPath = relSite.endsWith(".md")
    ? `${relSite.slice(0, -3)}.html`
    : relSite;
  const actor = join(outDir, htmlPath);
  const html =
    `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<link rel="stylesheet" href="${SITE_CSS}">
<title>${title} — ${SITE_TITLE}</title>
</head>
<body>
${crumb(quadrant, depth)}
<main>
${render(markdown, { href: (target) => publishedHref(file, target) })}
<footer>Rendered deterministically from ${relSite} by docs/site/scripts/publish.mjs (ADR 0009) — the markdown stays the source of truth.</footer>
</main>
</body>
</html>
`;

  mkdirSync(dirname(actor), { recursive: true });
  writeFileSync(actor, html);
  publishedCount++;
}

console.log(`published ${publishedCount} pages + stylesheet to ${relative(site, outDir) || outDir}`);
