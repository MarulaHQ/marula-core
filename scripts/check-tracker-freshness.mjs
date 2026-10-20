#!/usr/bin/env node
/**
 * check-tracker-freshness.mjs — a live tracker row is a claim; test the claim.
 *
 * Wave 47 lane A18 (`p47-tracker-freshness-guard`).  The incident: five lanes
 * launched against rows whose premise was already satisfied — `p43-signature-pad`,
 * `p45-rag-vector-namespace` and `p47-org-resource-quota-api` (reconciled by hand
 * on 2026-10-16), `p44-ui-shared-css-closeout` (launched to move CSS that had
 * already moved), and the Wave 49 rows, which sat live for a day after every one
 * of them had merged.  Each one cost a full lane launch to discover what
 * `git log --merges` answers in a second.
 *
 * This check does NOT decide a row is done.  It reports what it can prove, and a
 * human decides whether the row is stale or the check is wrong.
 *
 * Signals, strongest first:
 *   MERGED            the row's own branch slug has a merge commit on `main`
 *   ROW_SAYS_LANDED   the row's text opens with "**LANDED"
 *   SHA_NOT_ON_MAIN   the row claims a merge/tip sha that is not in `main`'s history
 *   SHA_UNKNOWN       the row cites a sha that does not exist in this clone
 *   MISSING_PATH      a `path:line` the row cites does not exist
 *   IMPOSSIBLE_LINE   the cited line number is past the end of the file
 *   GATE_GREEN        (--run-gates) a check/review script the row quotes passes today
 *   PREAMBLE_STALE    fire-order prose says IN FLIGHT / merge pending for a merged slug
 *   SNAPSHOT_STALE    the "main:" snapshot sha is not an ancestor of `main`
 *
 *   node scripts/check-tracker-freshness.mjs                 # every live row
 *   node scripts/check-tracker-freshness.mjs --row A18       # one row, lane preflight
 *   node scripts/check-tracker-freshness.mjs --json          # machine-readable
 *   node scripts/check-tracker-freshness.mjs --run-gates     # slower: runs quoted scripts
 *   node scripts/check-tracker-freshness.mjs --self-test     # the parsers' own rules
 *   node scripts/check-tracker-freshness.mjs --tracker X.md  # check a proposed tracker
 *
 * Exit 0 = no flags · 1 = at least one flag · 2 = usage/tracker error.
 * Run it from anywhere; it resolves the repo from its own path.
 */
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO = resolve(import.meta.dirname, "..");
const TRACKER = join(REPO, "TASKLIST.md");

// Row ids the tracker actually uses: `A18`, `S3b`, `P1c`, `12`, and hyphenated
// `55-3`. The hyphen was missing, and `parseRows` DROPS rows whose id does not
// match — so every Wave 55 row was invisible to `--row`, which answers "no such
// row". A check that cannot see a row reads like a pass, which is worse than no
// check. Tracked as lane 55-4; the self-test below now guards the shape.
const ROW_ID = /^(?:\*\*)?([A-Z]?\d+[a-z]?(?:-\d+)?)(?:\*\*)?$/;
const PATH_ROOTS = "apps|crates|packages|scripts|docs|design|e2e|deploy|native|desktop|fixtures|brand|plugins|\\.kilo";
const CITATION = new RegExp(
  `((?:${PATH_ROOTS})/[\\w.@/-]+|(?:TASKLIST|TASKLIST_COMPLETED|AGENTS|README|package)\\.(?:md|json))` +
    `:(\\d+)((?:,\\d+)*)(?:-(\\d+))?`,
  "g",
);
const FILE_EXT = /\.(?:tsx?|mjs|cjs|js|json|md|rs|sql|sh|py|toml|ya?ml|css|html|svg|png)$/;
const SLUG = /`(p\d+-[a-z0-9-]+)`/g;
const RED_CLAIM = /\b(?:fails?|failing|red|broken|regressed?)\b/i;
const GATE = /(?:scripts\/)?((?:check|review)-[a-z0-9-]+\.(?:mjs|sh|py))/g;
const MERGE_PREFIX = (slug) =>
  new RegExp(`^(?:merge\\s+${slug}(?![\\w-])|Merge branch '${slug}'(?:\\s|$))`);

function git(args, opts = {}) {
  return execFileSync("git", args, { cwd: REPO, encoding: "utf8", ...opts });
}

function mergeIndex() {
  let out = "";
  try {
    out = git(["log", "main", "--merges", "--format=%H%x09%s"]);
  } catch {
    return [];
  }
  return out
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const [sha, ...subject] = line.split("\t");
      return { sha, subject: subject.join("\t") };
    });
}

function commitExists(sha) {
  try {
    execFileSync("git", ["cat-file", "-e", `${sha}^{commit}`], { cwd: REPO, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function onMain(sha) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", sha, "main"], {
      cwd: REPO,
      stdio: "ignore",
    });
    return true;
  } catch {
    return false;
  }
}

/** Live rows only: a row that says it moved to the completed log is not a claim. */
export function parseRows(text) {
  const lines = text.split(/\r?\n/);
  const rows = [];
  let wave = "(top)";
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (line.startsWith("### ")) wave = line.slice(4).split("—")[0].trim();
    if (!line.startsWith("| ")) continue;
    const cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());
    if (cells.length < 3) continue;
    const idMatch = cells[0].match(ROW_ID);
    if (!idMatch) continue;
    const body = cells[2];
    if (/MOVED to/.test(body)) continue;
    rows.push({
      id: idMatch[1],
      wave,
      lane: cells[1],
      body,
      deps: cells[cells.length - 1],
      line: i + 1,
      raw: line,
    });
  }
  return rows;
}

export function ownSlug(laneCell) {
  const m = laneCell.match(/`(p\d+-[a-z0-9-]+)`/);
  return m ? m[1] : null;
}

export function isMergeOf(subject, slug) {
  return MERGE_PREFIX(slug).test(subject);
}

/** `apps/web/x.tsx:159,222` and `pkg/a.rs:10-14` are three citations, not one. */
export function citations(body) {
  const out = [];
  for (const m of body.matchAll(CITATION)) {
    const [, path, first, more, rangeEnd] = m;
    if (!FILE_EXT.test(path)) continue;
    const lines = [Number(first), ...[...more.matchAll(/\d+/g)].map((x) => Number(x[0]))];
    if (rangeEnd) lines.push(Number(rangeEnd));
    out.push({ path, lines });
  }
  return out;
}

function sentenceAround(body, needle) {
  const parts = body.split(/(?<=[.;:])\s+/);
  return parts.find((p) => p.includes(needle)) ?? body;
}

export function shasIn(body) {
  return [...body.matchAll(/\b([0-9a-f]{7,40})\b/g)]
    .map((m) => m[1])
    .filter((s) => /[a-f]/.test(s));
}

function rowFlags(row, ctx) {
  const signals = [];
  const slug = ownSlug(row.lane);
  if (slug) {
    const merge = ctx.merges.find((m) => isMergeOf(m.subject, slug));
    if (merge) {
      signals.push({
        code: "MERGED",
        detail: `${merge.sha.slice(0, 8)} ${merge.subject.slice(0, 90)}`,
      });
    }
  }
  if (/^\*\*LANDED\b/.test(row.body)) {
    signals.push({ code: "ROW_SAYS_LANDED", detail: "row text opens with **LANDED" });
  }
  for (const sha of new Set(shasIn(row.body))) {
    if (!commitExists(sha)) {
      signals.push({ code: "SHA_UNKNOWN", detail: sha });
      continue;
    }
    if (!onMain(sha)) {
      const claimed = new RegExp(`(?:merged|merge|tip)\\s+\`?${sha}`).test(row.body);
      signals.push({
        code: "SHA_NOT_ON_MAIN",
        detail: `${sha}${claimed ? " (cited as merged/tip)" : ""}`,
      });
    }
  }
  for (const cite of citations(row.body)) {
    const full = join(REPO, cite.path);
    if (!existsSync(full)) {
      signals.push({ code: "MISSING_PATH", detail: cite.path });
      continue;
    }
    const total = ctx.lineCount(cite.path);
    const worst = Math.max(...cite.lines);
    if (total && worst > total) {
      signals.push({ code: "IMPOSSIBLE_LINE", detail: `${cite.path}:${worst} of ${total}` });
    }
  }
  if (ctx.runGates) {
    for (const name of new Set([...row.body.matchAll(GATE)].map((m) => m[1]))) {
      const script = `scripts/${name}`;
      if (!existsSync(join(REPO, script))) continue;
      const state = ctx.gateState(script);
      // A green gate only contradicts the row when the sentence naming it calls it red.
      const claimsRed = RED_CLAIM.test(sentenceAround(row.body, name));
      row.gates = [...(row.gates ?? []), { script, state, claimsRed }];
      if (state === "green" && claimsRed) {
        signals.push({ code: "GATE_GREEN", detail: `${script} passes today` });
      }
    }
  }
  return signals;
}

function lineCounter(pathCache) {
  return (relPath) => {
    if (!pathCache.has(relPath)) {
      try {
        const text = readFileSync(join(REPO, relPath), "utf8");
        pathCache.set(relPath, text.split(/\r?\n/).length);
      } catch {
        pathCache.set(relPath, 0);
      }
    }
    return pathCache.get(relPath);
  };
}

function gateState(script) {
  const runner = script.endsWith(".mjs") ? "node" : script.endsWith(".sh") ? "bash" : "python3";
  try {
    execFileSync(runner, [script], { cwd: REPO, stdio: "ignore", timeout: 120_000 });
    return "green";
  } catch {
    return "red";
  }
}

function preambleFlags(text, merges) {
  const flags = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (line.startsWith("| ")) continue;
    if (!/IN FLIGHT|merge pending/i.test(line)) continue;
    for (const [, slug] of line.matchAll(SLUG)) {
      const merge = merges.find((m) => isMergeOf(m.subject, slug));
      if (merge) {
        flags.push({
          line: i + 1,
          slug,
          detail: `${merge.sha.slice(0, 8)} ${merge.subject.slice(0, 90)}`,
        });
      }
    }
  }
  return flags;
}

function snapshot(text, merges) {
  const m = text.match(/^\s*-\s+\*\*main:\*\*\s+`?([0-9a-f]{7,40})`?/m);
  if (!m) return null;
  const sha = m[1];
  if (!commitExists(sha)) return { sha, stale: true, detail: "snapshot sha does not exist" };
  if (!onMain(sha)) return { sha, stale: true, detail: "snapshot sha is not an ancestor of main" };
  let behind = 0;
  try {
    behind = Number(git(["rev-list", "--count", `${sha}..main`]).trim());
  } catch {
    behind = 0;
  }
  return { sha, stale: false, behind };
}

function selfTest() {
  const failures = [];
  let checks = 0;
  const eq = (name, actual, expected) => {
    checks += 1;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push(`${name}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    }
  };
  eq("merge subject, lane form", isMergeOf("merge p49-capacity-model — lane R8", "p49-capacity-model"), true);
  eq("merge subject, branch form", isMergeOf("Merge branch 'p47-lane-env-hygiene' into main", "p47-lane-env-hygiene"), true);
  eq("merge subject, reverse direction", isMergeOf("Merge branch 'main' into p47-drift-recheck", "p47-drift-recheck"), false);
  eq("merge subject, substring is not a match", isMergeOf("merge p49-capacity-model-v2", "p49-capacity-model"), false);

  const cites = citations(
    "see `apps/web/x.tsx:159,222` and `crates/a.rs:10-14`, plus `docs/gone.md:3`",
  );
  eq("citation count", cites.length, 3);
  eq("citation comma list", cites[0], { path: "apps/web/x.tsx", lines: [159, 222] });
  eq("citation range", cites[1], { path: "crates/a.rs", lines: [10, 14] });

  const text = [
    "### Wave 47 — demo",
    "| # | Lane (branch) | Tackles | Migration | Depends on |",
    "|---|---|---|---|---|",
    "| A15 | `p47-drift-recheck` | **Do the thing.** | none | none |",
    "| A16 | — | MOVED to `TASKLIST_COMPLETED.md` | none | none |",
    "| B2 | `p47-byok` | **LANDED 2026-10-17** tip `abc1234` | none | none |",
  ].join("\n");
  const rows = parseRows(text);
  eq("live row count", rows.length, 2);
  eq("row ids", rows.map((r) => r.id), ["A15", "B2"]);

  // Lane 55-4: a hyphenated id must parse, and no id the tracker uses may be
  // invisible. Counting parseRows output cannot prove the second half — it drops
  // unmatched rows — so this guard reads the raw table's first cells instead.
  const hyphenated = parseRows(
    [
      "| # | Lane (branch) | Tackles | Migration | Depends on |",
      "|---|---|---|---|---|",
      "| 55-3 | `p55-domain-residue` | **Do the thing.** | none | none |",
    ].join("\n"),
  );
  eq("hyphenated row id parses", hyphenated.map((r) => r.id), ["55-3"]);
  if (existsSync(TRACKER)) {
    const idShaped = readFileSync(TRACKER, "utf8")
      .split(/\r?\n/)
      .filter((l) => l.startsWith("| "))
      .map((l) => l.trim().replace(/^\|/, "").split("|")[0].trim())
      .filter((c) => /^[A-Za-z]?\d/.test(c));
    const invisible = idShaped.filter((c) => !ROW_ID.test(c));
    eq("every id-shaped row cell in the tracker parses", invisible, []);
  }
  eq("own slug", ownSlug("`p41-ios-simulator` **(added 2026-09-30)**"), "p41-ios-simulator");
  eq(
    "gate sentence, not the whole cell",
    RED_CLAIM.test(sentenceAround("A gate (`check-shared-chrome.mjs`) is green; another fails", "check-shared-chrome.mjs")),
    false,
  );
  eq("red claim, word boundary", RED_CLAIM.test("the shared chrome gate is green"), false);
  eq("red claim, real", RED_CLAIM.test("the gate is red"), true);

  if (failures.length) {
    for (const f of failures) console.error(`SELFTEST FAIL ${f}`);
    process.exit(1);
  }
  console.log(`self-test ok — ${checks} parser rules`);
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.includes("--self-test")) return selfTest();

  const json = argv.includes("--json");
  const runGates = argv.includes("--run-gates");
  const trackerArg = argv.includes("--tracker") ? argv[argv.indexOf("--tracker") + 1] : null;
  const TRACKER_FILE = trackerArg ? resolve(REPO, trackerArg) : TRACKER;
  const rowFilter = argv.includes("--row") ? argv[argv.indexOf("--row") + 1] : null;

  let text;
  try {
    text = readFileSync(TRACKER_FILE, "utf8");
  } catch {
    console.error(`cannot read ${TRACKER_FILE}`);
    process.exit(2);
  }

  const merges = mergeIndex();
  const rows = parseRows(text);
  const pathCache = new Map();
  const ctx = { merges, runGates, gateState, lineCount: lineCounter(pathCache) };

  let selected = rows;
  if (rowFilter) {
    selected = rows.filter((r) => r.id === rowFilter);
    if (!selected.length) {
      const known = rows.map((r) => r.id).join(", ");
      console.error(`no live row "${rowFilter}" in TASKLIST.md (live: ${known})`);
      process.exit(2);
    }
  }

  const evaluated = selected.map((row) => ({ row, signals: rowFlags(row, ctx) }));
  const flagged = evaluated.filter((entry) => entry.signals.length);
  const preamble = preambleFlags(text, merges);
  const snap = snapshot(text, merges);
  let head = "unknown";
  try {
    head = git(["rev-parse", "--short", "main"]).trim();
  } catch {
    head = "unknown";
  }

  if (json) {
    console.log(
      JSON.stringify(
        {
          tracker: "TASKLIST.md",
          main: head,
          snapshot: snap,
          liveRows: rows.length,
          checkedRows: selected.length,
          flagged: flagged.map(({ row, signals }) => ({
            id: row.id,
            wave: row.wave,
            slug: ownSlug(row.lane),
            line: row.line,
            signals,
            ...(row.gates ? { gates: row.gates } : {}),
          })),
          preamble,
        },
        null,
        2,
      ),
    );
  } else {
    console.log(
      `Tracker freshness — ${selected.length} live row${selected.length === 1 ? "" : "s"} checked ` +
        `(main ${head}${snap ? `, snapshot ${snap.sha.slice(0, 8)}${snap.behind ? `, ${snap.behind} commits behind` : ""}` : ""})`,
    );
    if (snap?.stale) console.log(`  SNAPSHOT_STALE  ${snap.sha} — ${snap.detail}`);
    for (const { row, signals } of flagged) {
      console.log(`\n${row.wave} ${row.id}${ownSlug(row.lane) ? ` ${ownSlug(row.lane)}` : ""}  (line ${row.line})`);
      for (const s of signals) console.log(`  ${s.code.padEnd(16)} ${s.detail}`);
    }
    if (runGates) {
      for (const { row } of evaluated) {
        if (!row.gates) continue;
        console.log(
          `\n${row.wave} ${row.id} quoted gates: ` +
            row.gates.map((g) => `${g.script} ${g.state}`).join(" · "),
        );
      }
    }
    if (preamble.length) {
      console.log("\nPreamble claims contradicted by `main`:");
      for (const p of preamble) console.log(`  PREAMBLE_STALE   line ${p.line} · ${p.slug} — ${p.detail}`);
    }
    const total = flagged.length + preamble.length + (snap?.stale ? 1 : 0);
    console.log(
      total === 0
        ? "\n0 flags — every live row's premise still stands as written."
        : `\n${total} flag${total === 1 ? "" : "s"} — a row is a claim; verify before launching a lane against it.`,
    );
  }

  process.exit(flagged.length || preamble.length || snap?.stale ? 1 : 0);
}

main();
