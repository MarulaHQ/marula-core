#!/usr/bin/env node
// Add a description to every published Core docs page that lacks one.
//
// The source is the first substantive paragraph (skipping the H1, the audience
// blockquote, lists, tables and code), trimmed to 80–160 characters. Pages with
// no prose paragraph use the override below. Generated pages are owned by their
// generators, which emit their own frontmatter.

import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const nav = JSON.parse(readFileSync(join(site, "nav.json"), "utf8"));
const listed = new Set([nav.home, ...nav.sections.flatMap((section) => section.items)]);
const generated = new Set(["reference/api", "reference/upgrade-matrix"]);

const overrides = {
  "explanation/why-marula":
    "Marula's positioning narrative, quoted verbatim: why the PSA was built and what it refuses to repeat.",
  "explanation/ai-deny-by-default":
    "Marula ships AI deny-by-default: the org switch stays off until grants, caps and credentials make a surface audible.",
  "how-to/admin-csat-surveys":
    "Set up satisfaction surveys on ticket resolution, with intervals, link validity and quiet hours.",
  "how-to/client-quote-approval":
    "How a client reads, accepts or declines a quote in the portal, and what changes after acceptance.",
  "how-to/local-demo-stack":
    "Take a fresh clone to a booted, seeded demo with Postgres, Valkey and the sample organization.",
  "how-to/staff-multi-window":
    "How to open a second window on the same work, with a compact shell and the workbar.",
  "how-to/staff-scanning":
    "How a scan resolves to one typed record — ticket, asset or stock item — before Marula acts on it.",
  "how-to/staff-time-categories":
    "How work categories decide what a time entry bills and where it lands.",
  "reference/self-host-local-stack":
    "The canonical local stack: Postgres, Redis, schema migration, core API, worker and web console.",
  "start/technicians":
    "The technician's path through Marula: work the queue, run a service day, log billable time and answer the customer.",
  "tutorials/first-invoice":
    "Walk the invoice cycle end to end in the demo organization, from billable time to a sent invoice.",
};

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory()
      ? walk(path)
      : path.endsWith(".md")
        ? [path]
        : [];
  });
}

function clean(text) {
  return text
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/gs, " ")
    .replace(/\[([^\]]*)\]\([^)]*\)/gs, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/[*_]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function paragraphs(markdown) {
  const lines = markdown.replace(/^---\n[\s\S]*?\n---\n/, "").split("\n");
  const out = [];
  let buffer = [];
  let mode = "paragraph";
  let fenced = false;
  const flush = () => {
    if (!buffer.length) return;
    const text = clean(buffer.join(" "));
    if (text) out.push(text);
    buffer = [];
  };
  for (const raw of lines) {
    const line = raw.trim();
    if (line.startsWith("```")) {
      fenced = !fenced;
      flush();
      continue;
    }
    if (fenced) continue;
    if (!line) {
      flush();
      mode = "paragraph";
      continue;
    }
    if (line.startsWith("# ")) {
      flush();
      continue;
    }
    if (line.startsWith("#")) {
      flush();
      mode = "paragraph";
      continue;
    }
    if (line.startsWith(">")) {
      flush();
      mode = "quote";
      continue;
    }
    if (/^([-*+]|\d+\.)\s/.test(line)) {
      flush();
      mode = "list";
      continue;
    }
    if (line.startsWith("|") || line.startsWith("<") || line.startsWith("!")) {
      flush();
      mode = "other";
      continue;
    }
    if (mode !== "paragraph") continue;
    buffer.push(line);
  }
  flush();
  return out;
}

function shorten(text) {
  const value = text.replace(/\s+/g, " ").trim();
  if (value.length <= 160) return value;
  const window = value.slice(0, 161);
  const sentence = window.match(/^(.{80,}?[.!?])\s/);
  if (sentence) return sentence[1];
  const cut = value.slice(0, 157);
  const space = cut.lastIndexOf(" ");
  return `${(space > 80 ? cut.slice(0, space) : cut).replace(/[,;:]$/, "")}…`;
}

let changed = 0;
for (const file of walk(site)) {
  const rel = relative(site, file).replace(/\\/g, "/").replace(/\.md$/, "");
  if (!listed.has(rel) || generated.has(rel)) continue;
  const markdown = readFileSync(file, "utf8");
  if (/^---\n[\s\S]*?\ndescription:/m.test(markdown)) continue;
  const found = paragraphs(markdown);
  const description =
    overrides[rel] ??
    shorten(found.find((text) => text.length >= 80) ?? found.find((text) => text.length >= 40) ?? found[0] ?? "");
  if (description.length < 40) {
    console.warn(`${rel}: no usable description (${description.length} chars)`);
    continue;
  }
  writeFileSync(
    file,
    `---\ndescription: ${JSON.stringify(description)}\n---\n\n${markdown}`,
  );
  changed += 1;
}

console.log(`descriptions added: ${changed}`);
