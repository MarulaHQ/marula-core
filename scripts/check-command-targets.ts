#!/usr/bin/env node
/**
 * Command-target check (Wave 47 lane A25, p47-silent-command-noops).
 *
 * Every control a keyboard command clicks is declared on its registry row
 * (`targets: [testid("new-record")]`, see apps/web/lib/commands/targets.ts).
 * This script imports the REAL registry — no second copy, no text parsing —
 * and resolves each declaration against the source tree:
 *
 *   1. the selector's control must render somewhere in apps/web or packages
 *   2. when the command declares surfaces and the target defaults to
 *      `via: "surface"`, the control must render inside the route directory
 *      of EVERY declared surface
 *
 * Rule 2 is the one that matters for the bug this exists for. The Wave 41
 * pack port renamed the clients page's create button; `/boards/[id]` kept its
 * `data-testid="new-record"`, so a check that only asked "does this testid
 * exist anywhere?" would have stayed green while Alt+N on /clients clicked
 * nothing for months. Asking per surface turns that rename red in the commit
 * that made it.
 *
 * Scope: markup in apps/web and packages. Tests, stories and build output are
 * skipped — a testid only a test file mentions is not a control a user clicks.
 *
 * Read-only, no browser, no build.
 * Usage: pnpm shortcuts:check            (node scripts/check-command-targets.ts via tsx)
 *        pnpm shortcuts:check --self-test
 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { COMMAND_REGISTRY } from "../apps/web/lib/commands/registry";
import type { CommandDef, CommandTarget } from "../apps/web/lib/commands/types";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE_ROOTS = ["apps/web", "packages"];
const APP_ROOT = "apps/web/app";
const SKIP_DIRS = new Set([
  "node_modules",
  ".next",
  "dist",
  "coverage",
  ".turbo",
  "tests",
  "__snapshots__",
]);
const SKIP_FILE = /\.test\.[cm]?[jt]sx?$/;

export interface SourceFile {
  /** Repo-relative path, so reports read the same on every machine. */
  path: string;
  text: string;
}

export interface TargetViolation {
  command: string;
  selector: string;
  reason: string;
}

export interface ResolvedTarget {
  command: string;
  selector: string;
  presence: CommandTarget["presence"];
  /** Repo-relative files that render the control. */
  hits: string[];
}

export interface TargetAudit {
  commands: number;
  targets: number;
  files: number;
  violations: TargetViolation[];
  resolved: ResolvedTarget[];
}

function walk(dir: string, out: string[] = []): string[] {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      walk(join(dir, entry.name), out);
    } else if (/\.(tsx?|jsx?|mjs)$/.test(entry.name) && !SKIP_FILE.test(entry.name)) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
}

/** The markup a browser would see for one attribute, in the forms JSX uses. */
function markupPattern(attribute: string, value: string): RegExp {
  const v = value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const a = attribute.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`${a}=(?:"${v}"|'${v}'|\\{"${v}"\\}|\\{'${v}'\\})`);
}

function markupOf(target: CommandTarget): { attribute: string; value: string } {
  if (target.testId !== null) return { attribute: "data-testid", value: target.testId };
  if (target.attribute !== null) {
    return { attribute: target.attribute.name, value: target.attribute.value };
  }
  throw new Error(`target ${target.selector} carries neither a testId nor an attribute`);
}

/**
 * Does this file belong to the route subtree for `surface`, without wandering
 * into a nested route? `/clients` must be satisfied by the list page, never by
 * `app/clients/[id]/page.tsx` keeping the testid alive on a different surface.
 */
function inSurfaceSubtree(filePath: string, surface: string): boolean {
  const routeDir = join(APP_ROOT, surface.replace(/^\//, ""));
  const prefix = `${routeDir}/`;
  if (!filePath.startsWith(prefix)) return false;
  const rest = filePath.slice(prefix.length);
  const extraRouteDirs = rest
    .split("/")
    .slice(0, -1)
    .filter((segment) => segment.startsWith("["));
  return extraRouteDirs.length === 0;
}

export function loadedSources(): SourceFile[] {
  const files: SourceFile[] = [];
  for (const scope of SOURCE_ROOTS) {
    for (const file of walk(resolve(root, scope))) {
      files.push({ path: relative(root, file), text: readFileSync(file, "utf8") });
    }
  }
  return files;
}

/** Resolve every declared target against `files`. Pure: the self-test feeds
 * it renamed copies of the same files. */
export function auditTargets(
  commands: ReadonlyArray<CommandDef> = COMMAND_REGISTRY,
  files: ReadonlyArray<SourceFile> = loadedSources(),
): TargetAudit {
  const violations: TargetViolation[] = [];
  const resolved: ResolvedTarget[] = [];
  let targets = 0;

  for (const command of commands) {
    for (const target of command.targets) {
      targets += 1;
      const { attribute, value } = markupOf(target);
      const pattern = markupPattern(attribute, value);
      const hits = files.filter((file) => pattern.test(file.text)).map((file) => file.path);
      resolved.push({ command: command.id, selector: target.selector, presence: target.presence, hits });

      if (hits.length === 0) {
        violations.push({
          command: command.id,
          selector: target.selector,
          reason: "no page or component renders it — the control was renamed or deleted",
        });
        continue;
      }
      if (target.via === "shared" || command.surfaces.length === 0) continue;

      for (const surface of command.surfaces) {
        if (hits.some((hit) => inSurfaceSubtree(hit, surface))) continue;
        violations.push({
          command: command.id,
          selector: target.selector,
          reason:
            `declared for ${surface} but nothing under ${APP_ROOT}${surface}/ ` +
            "renders it — the chord is advertised on a surface that cannot act",
        });
      }
    }
  }

  return { commands: commands.length, targets, files: files.length, violations, resolved };
}

/** The checker's own proof: rename each control in memory and require a
 * violation that names its command. A broken matcher cannot pass quietly. */
export function selfTest(): { checked: number; missed: string[] } {
  const files = loadedSources();
  const missed: string[] = [];
  let checked = 0;
  for (const command of COMMAND_REGISTRY) {
    for (const target of command.targets) {
      const { attribute, value } = markupOf(target);
      const pattern = markupPattern(attribute, value);
      const renamed = `${value}-renamed`;
      const mutated = files.map((file) =>
        pattern.test(file.text)
          ? {
              path: file.path,
              text: file.text.replace(new RegExp(pattern.source, "g"), `${attribute}="${renamed}"`),
            }
          : file,
      );
      const after = auditTargets([command], mutated);
      checked += 1;
      if (!after.violations.some((v) => v.command === command.id)) missed.push(command.id);
    }
  }
  return { checked, missed };
}

function printReport(audit: TargetAudit): boolean {
  const width = Math.max(...audit.resolved.map((r) => r.command.length), 12);
  for (const target of audit.resolved) {
    const label = `${target.command.padEnd(width)} ${target.selector}`;
    const failed = audit.violations.some(
      (v) => v.command === target.command && v.selector === target.selector,
    );
    if (failed) {
      console.log(`  FAIL ${label}`);
    } else {
      const places = target.hits.length === 1 ? target.hits[0] : `${target.hits[0]} (+${target.hits.length - 1})`;
      console.log(`  ok   ${label}  ${places}`);
    }
  }
  console.log(
    `\n${audit.targets - audit.violations.length}/${audit.targets} command targets resolve ` +
      `across ${audit.commands} commands (${audit.files} files scanned).`,
  );
  return audit.violations.length === 0;
}

function main(argv: ReadonlyArray<string>): number {
  if (argv.includes("--self-test")) {
    const { checked, missed } = selfTest();
    if (missed.length > 0) {
      console.error(`✗ self-test failed — a rename went unnoticed for: ${missed.join(", ")}`);
      return 1;
    }
    console.log(`self-test ok — ${checked} target${checked === 1 ? "" : "s"}, every rename detected.`);
    return 0;
  }

  const audit = auditTargets();
  console.log(`command targets — ${audit.targets} declaration(s) across ${audit.commands} commands`);
  const clean = printReport(audit);
  if (clean) return 0;

  for (const violation of audit.violations) {
    console.error(`\n✗ ${violation.command} → ${violation.selector}\n    ${violation.reason}`);
  }
  console.error(
    "\nA command clicks a control its surface does not render. Restore the control's testid or " +
      "update the command's target: a dead chord stays advertised in the overlay and the cheat sheet.",
  );
  return 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(main(process.argv.slice(2)));
}
