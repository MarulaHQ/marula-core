#!/usr/bin/env node
/**
 * Fails when the built installer *loads* anything off this machine.
 *
 * Purpose: the wizard promises "no telemetry, works offline once the images are
 * pulled". A remote font, a CDN script or an analytics beacon would make that
 * sentence false while the UI still looked right, so the promise is checked
 * against the emitted files.
 *
 * The rule is about loads, not string literals. React's production bundle
 * carries namespace identifiers (`http://www.w3.org/2000/svg`) and documentation
 * links (`https://react.dev/errors/`) that are never fetched, and a check that
 * failed on those would be switched off within a week. So this looks at the
 * positions a browser actually loads from: `src`/`href` in HTML, `url()` and
 * `@import` in CSS, and `fetch`/`import()`/`Worker`/`EventSource` call sites in
 * JavaScript. The page's CSP (`default-src 'self'`) is the second layer: even a
 * loader this check missed could not reach the network.
 *
 * `--self-test` proves the classifier both ways in memory, so a check that
 * stopped catching anything would fail rather than pass quietly.
 *
 * Non-purpose: it is not a security scanner. It answers one question.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const DIST = new URL("../dist/", import.meta.url).pathname;

const LOCAL = /^https?:\/\/(127\.0\.0\.1|localhost)([:/]|$)/;

const RULES = [
  { kind: "html", pattern: /\b(?:src|href|action)\s*=\s*["'](https?:\/\/[^"']+)/gi },
  { kind: "css", pattern: /url\(\s*["']?(https?:\/\/[^"')\s]+)/gi },
  { kind: "css", pattern: /@import\s+(?:url\()?["']?(https?:\/\/[^"')\s]+)/gi },
  { kind: "js", pattern: /\bfetch\s*\(\s*["'`](https?:\/\/[^"'`]+)/g },
  { kind: "js", pattern: /\bimport\s*\(\s*["'`](https?:\/\/[^"'`]+)/g },
  {
    kind: "js",
    pattern: /\bnew\s+(?:Worker|EventSource|WebSocket)\s*\(\s*["'`](https?:\/\/[^"'`]+)/g,
  },
  { kind: "js", pattern: /\bimportScripts\s*\(\s*["'`](https?:\/\/[^"'`]+)/g },
];

/** Every remote load in one emitted file, as `file: url` lines. */
export function scan(file, text) {
  const extension = file.slice(file.lastIndexOf(".") + 1);
  const offenders = [];
  for (const rule of RULES) {
    if (rule.kind === "html" && extension !== "html") continue;
    if (rule.kind === "css" && extension !== "css") continue;
    if (rule.kind === "js" && !["js", "mjs"].includes(extension)) continue;
    rule.pattern.lastIndex = 0;
    for (const match of text.matchAll(rule.pattern)) {
      const url = match[1];
      if (url && !LOCAL.test(url)) offenders.push(`${file}: ${url}`);
    }
  }
  return offenders;
}

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...walk(path));
    else out.push(path);
  }
  return out;
}

function selfTest() {
  const fixture = [
    {
      file: "index.html",
      text: '<script src="https://cdn.example.com/app.js"></script>',
      caught: true,
    },
    { file: "index.html", text: '<script src="./assets/app.js"></script>', caught: false },
    { file: "a.css", text: "@import url(https://fonts.example.com/x.css);", caught: true },
    { file: "a.css", text: 'body{background:url("./bg.png")}', caught: false },
    { file: "a.js", text: 'fetch("https://api.example.com/collect",{method:"POST"})', caught: true },
    { file: "a.js", text: 'fetch("/api/state")', caught: false },
    { file: "a.js", text: 'fetch("http://127.0.0.1:8081/api/state")', caught: false },
    {
      file: "a.js",
      text: 'const ns="http://www.w3.org/2000/svg";const doc="https://react.dev/errors/418";',
      caught: false,
    },
  ];

  let failures = 0;
  for (const [index, entry] of fixture.entries()) {
    const found = scan(entry.file, entry.text);
    const ok = entry.caught ? found.length > 0 : found.length === 0;
    if (!ok) {
      failures += 1;
      console.error(`self-test ${index}: expected caught=${entry.caught}, got ${found.length}`);
    }
  }
  if (failures > 0) process.exit(1);
  console.log(`offline:check self-test — ${fixture.length} cases, classifier agrees both ways`);
}

if (process.argv.includes("--self-test")) {
  selfTest();
} else {
  let files;
  try {
    files = walk(DIST);
  } catch {
    console.error(`offline:check — no build at ${DIST}. Run \`pnpm build\` first.`);
    process.exit(1);
  }

  const offenders = [];
  for (const file of files) {
    const extension = file.slice(file.lastIndexOf(".") + 1);
    if (!["html", "css", "js", "mjs", "json"].includes(extension)) continue;
    offenders.push(...scan(file.replace(DIST, ""), readFileSync(file, "utf8")));
  }

  if (offenders.length > 0) {
    console.error("offline:check — the bundle loads from off this machine:");
    for (const line of offenders) console.error(`  ${line}`);
    process.exit(1);
  }
  console.log(`offline:check — ${files.length} emitted files, no remote load`);
}
