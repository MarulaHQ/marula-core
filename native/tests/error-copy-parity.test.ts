// Parity proof: the mobile reason-code copy map is byte-identical to the
// web map (apps/web/lib/error-copy.ts). This is the mechanical half of
// "typed 404/409 honest states identical to web" — a drift on either side
// fails this test with the exact key named.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  GENERIC_REJECTION,
  OFFLINE_QUEUED_COPY,
  OFFLINE_RETRY_COPY,
  REASON_COPY,
  TIMEOUT_QUEUED_COPY,
  TIMEOUT_RETRY_COPY,
} from "@/core/errors";

const here = path.dirname(fileURLToPath(import.meta.url));
const webSource = fs.readFileSync(
  path.resolve(here, "../../apps/web/lib/error-copy.ts"),
  "utf8",
);

function blockOf(name: string): string {
  const start = webSource.indexOf(name);
  if (start === -1) throw new Error(`web source is missing ${name}`);
  const open = webSource.indexOf("{", start);
  const end = webSource.indexOf("\n};", open);
  if (open === -1 || end === -1) throw new Error(`cannot isolate ${name} block`);
  return webSource.slice(open + 1, end);
}

function entriesOf(block: string): Record<string, string> {
  const entries: Record<string, string> = {};
  const re = /([A-Za-z_][\w]*)\s*:\s*"((?:[^"\\]|\\.)*)"/g;
  let match = re.exec(block);
  while (match !== null) {
    entries[match[1] as string] = (match[2] as string)
      .replace(/\\"/g, '"')
      .replace(/\\\\/g, "\\");
    match = re.exec(block);
  }
  return entries;
}

function constString(name: string): string {
  const match = new RegExp(`(?:const|export const) ${name} =\\s*"((?:[^"\\\\]|\\\\.)*)"`).exec(
    webSource,
  );
  if (match === null) throw new Error(`web source is missing string const ${name}`);
  return (match[1] as string).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
}

describe("error copy parity with web", () => {
  it("REASON_COPY matches key-for-key and value-for-value", () => {
    const webEntries = entriesOf(blockOf("REASON_COPY"));
    expect(Object.keys(webEntries).sort()).toEqual(Object.keys(REASON_COPY).sort());
    for (const [reason, copy] of Object.entries(webEntries)) {
      expect(REASON_COPY[reason], `reason ${reason}`).toBe(copy);
    }
  });

  it("shared register constants match", () => {
    expect(GENERIC_REJECTION).toBe(constString("GENERIC_REJECTION"));
    expect(OFFLINE_QUEUED_COPY).toBe(constString("OFFLINE_QUEUED_COPY"));
    expect(OFFLINE_RETRY_COPY).toBe(constString("OFFLINE_RETRY_COPY"));
    expect(TIMEOUT_QUEUED_COPY).toBe(constString("TIMEOUT_QUEUED_COPY"));
    expect(TIMEOUT_RETRY_COPY).toBe(constString("TIMEOUT_RETRY_COPY"));
  });
});
