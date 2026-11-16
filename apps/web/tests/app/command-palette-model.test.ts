// Pure models behind the palette: grouping order and labels, which results
// the cursor may land on, and the context chip's fragments.
import { describe, expect, it } from "vitest";
import {
  ACTIONS,
  flatOpenable,
  groupResults,
  resultHref,
  resultMeta,
} from "../../components/command-palette/search-model";
import { describeScreenContext, screenLabelFor } from "../../components/command-palette/screen-context";

type Hit = Parameters<typeof groupResults>[number][number];

function hit(overrides: Partial<Hit> = {}): Hit {
  return {
    entity_kind: "ticket",
    entity_id: "71000000-0000-4000-8000-000000000001",
    parent_id: null,
    title: "#18432 VPN disconnects every 20–30 minutes",
    snippet: "Acme Corporation · In progress",
    score: 0.9,
    ...overrides,
  } as Hit;
}

describe("palette result grouping", () => {
  it("keeps the /search page's group order and labels", () => {
    const groups = groupResults([
      hit({ entity_kind: "kb_article", entity_id: "k1", title: "Runbook" }),
      hit({ entity_kind: "ticket", entity_id: "t1" }),
      hit({ entity_kind: "client", entity_id: "c1", title: "Acme" }),
    ]);
    expect(groups.map((group) => group.label)).toEqual(["Tickets", "Clients", "KB articles"]);
    expect(groups[0]?.hits).toHaveLength(1);
  });

  it("opens records that have a route and leaves the rest as read-only rows", () => {
    expect(resultHref(hit({ entity_kind: "ticket", entity_id: "t1" }))).toBe("/tickets/t1");
    expect(
      resultHref(hit({ entity_kind: "comment", entity_id: "x", parent_id: "t1" })),
    ).toBe("/tickets/t1");
    expect(resultHref(hit({ entity_kind: "kb_article", entity_id: "k1" }))).toBe("/kb/k1");
    expect(resultHref(hit({ entity_kind: "site", entity_id: "s1" }))).toBeNull();
    expect(resultHref(hit({ entity_kind: "contact", entity_id: "p1" }))).toBeNull();
  });

  it("only lets the keyboard cursor land on openable rows, then the actions", () => {
    const groups = groupResults([
      hit({ entity_kind: "site", entity_id: "s1", title: "Head office" }),
      hit({ entity_kind: "ticket", entity_id: "t1" }),
    ]);
    const rows = flatOpenable(groups, ACTIONS);
    expect(rows[0]).toMatchObject({ kind: "result", href: "/tickets/t1" });
    expect(rows.slice(1).every((row) => row.kind === "action")).toBe(true);
    expect(rows).toHaveLength(1 + ACTIONS.length);
  });

  it("uses the snippet as the row's second line, never an invented client status", () => {
    expect(resultMeta(hit())).toBe("Acme Corporation · In progress");
  });
});

describe("screen context", () => {
  it("names the route's surface", () => {
    expect(screenLabelFor("/tickets/123")).toBe("Tickets");
    expect(screenLabelFor("/kb")).toBe("Knowledge");
    expect(screenLabelFor("/")).toBe("Marula");
  });

  it("builds the chip in the pack's order and skips a zero selection", () => {
    expect(
      describeScreenContext({ label: "Tickets", view: "My queue", rowCount: 12, selectedCount: 3 }),
    ).toEqual(["Tickets", "My queue", "12 rows", "3 selected"]);
    expect(describeScreenContext({ label: "Tickets", selectedCount: 0 })).toEqual(["Tickets"]);
    expect(describeScreenContext({ label: "Tickets", rowCount: 1 })).toEqual(["Tickets", "1 row"]);
  });
});
