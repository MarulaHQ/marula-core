// Live Activity drift guard: the app module's attributes and the widget
// extension's attributes are two Swift files that ActivityKit matches by
// type name and shape. If they drift, the activity silently fails to render.
// This test compares the declarations field by field.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = path.dirname(fileURLToPath(import.meta.url));

const appSide = fs.readFileSync(
  path.resolve(
    here,
    "../modules/meridian-live-activity/ios/TimerActivityAttributes.swift",
  ),
  "utf8",
);
const widgetSide = fs.readFileSync(
  path.resolve(here, "../targets/live-timer/TimerActivityAttributes.swift"),
  "utf8",
);

function contentStateFields(source: string): { name: string; type: string }[] {
  const block = source.slice(source.indexOf("public struct ContentState"));
  const body = block.slice(block.indexOf("{"), block.indexOf("public init"));
  return [...body.matchAll(/public var (\w+): ([\w<>\[\]. ]+)/g)].map((match) => ({
    name: match[1] ?? "",
    type: (match[2] ?? "").trim(),
  }));
}

function attributesField(source: string): string {
  // The fixed attribute is declared after ContentState, so the last String
  // field in the file is the one.
  const matches = [...source.matchAll(/public var (\w+): String/g)];
  return matches.at(-1)?.[1] ?? "";
}

describe("live activity attribute parity", () => {
  it("declares the same ActivityAttributes type name on both sides", () => {
    expect(appSide).toContain("public struct TimerActivityAttributes: ActivityAttributes");
    expect(widgetSide).toContain("public struct TimerActivityAttributes: ActivityAttributes");
  });

  it("keeps ContentState fields identical", () => {
    const app = contentStateFields(appSide);
    const widget = contentStateFields(widgetSide);
    expect(app.length).toBeGreaterThan(0);
    expect(app).toEqual(widget);
  });

  it("keeps the fixed attribute field identical", () => {
    expect(attributesField(appSide)).toBe("sessionId");
    expect(attributesField(widgetSide)).toBe("sessionId");
  });
});
